//! WASAPI audio capture for recording: system (render-endpoint loopback) and/or microphone,
//! mixed to a canonical 48 kHz stereo 16-bit PCM stream delivered in ~10 ms chunks.
//!
//! cpal does not expose WASAPI loopback, so this uses raw WASAPI. A render client on the default
//! output endpoint is opened alongside loopback: it feeds silence (a "keep-alive") so the
//! loopback capture doesn't starve when nothing else is playing — or, for the functional test, a
//! sine tone so the captured audio has a known signal.
//!
//! Each source is set up independently: one that fails (a missing or busy microphone, an
//! unsupported format) is skipped with a note for the user, and the others still record. Notes
//! go to the recording's [`Notes`], which the app shows once the recording is saved.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::core::{GUID, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IAudioRenderClient, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};

use crate::Notes;

/// Canonical capture rate and channel count.
pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

/// How long device setup may take before the recording goes ahead without sound. Setup is a
/// handful of COM calls, but a Bluetooth headset switching profile can take seconds.
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

// KSDATAFORMAT_SUBTYPE_{PCM,IEEE_FLOAT}: {0000000X-0000-0010-8000-00aa00389b71}.
const SUBTYPE_PCM: GUID = GUID::from_values(
    1,
    0,
    0x0010,
    [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
);
const SUBTYPE_FLOAT: GUID = GUID::from_values(
    3,
    0,
    0x0010,
    [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71],
);

/// What to capture.
#[derive(Debug, Clone, Default)]
pub struct AudioRequest {
    pub system: bool,
    pub mic: bool,
    /// Microphone to use, matched against the endpoint id or (case-insensitively) its name;
    /// `None` or blank = the system default microphone.
    pub mic_device: Option<String>,
    /// Test-only: render this tone through the keep-alive so loopback has a known signal.
    pub tone_hz: Option<f32>,
}

/// A running audio capture. Dropping it stops the thread.
pub struct AudioEngine {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    ready: Receiver<bool>,
    notes: Notes,
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine").finish_non_exhaustive()
    }
}

impl AudioEngine {
    /// Start capture on a background thread without waiting for the devices; call
    /// [`wait_ready`](Self::wait_ready) before relying on the chunk receiver. Chunks are
    /// interleaved-stereo i16 at [`RATE`].
    pub fn start(req: AudioRequest, notes: Notes) -> (AudioEngine, Receiver<Vec<i16>>) {
        let (chunk_tx, chunk_rx) = mpsc::channel::<Vec<i16>>();
        let (ready_tx, ready_rx) = mpsc::channel::<bool>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let thread_notes = notes.clone();

        let handle = std::thread::Builder::new()
            .name("rustshot-audio".into())
            .spawn(move || audio_thread(req, chunk_tx, ready_tx, stop_thread, thread_notes));
        let handle = match handle {
            Ok(h) => Some(h),
            Err(e) => {
                notes.push(format!("audio could not be started ({e})"));
                None
            }
        };
        (
            AudioEngine {
                stop,
                handle,
                ready: ready_rx,
                notes,
            },
            chunk_rx,
        )
    }

    /// Wait until `deadline` for the devices to come up. True if at least one source is
    /// capturing. On a timeout the thread is detached (so stopping can't hang on a stuck
    /// device) and a note says the recording has no sound.
    pub fn wait_ready(&mut self, deadline: Instant) -> bool {
        if self.handle.is_none() {
            return false;
        }
        let timeout = deadline.saturating_duration_since(Instant::now());
        match self.ready.recv_timeout(timeout) {
            Ok(ok) => ok,
            Err(_) => {
                self.notes.push(
                    "the audio devices did not respond in time, so the recording has no sound",
                );
                self.stop.store(true, Ordering::SeqCst);
                self.handle = None; // detach; the thread exits on its own once unstuck
                false
            }
        }
    }

    /// Stop capture and join the thread.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---- formats ----------------------------------------------------------------------------------

/// Parsed essentials of a `WAVEFORMATEX`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fmt {
    rate: u32,
    channels: u16,
    float: bool,
    /// Container size of one sample, in bits (24-in-32 reports 32).
    bits: u16,
    /// Bytes per frame (all channels).
    block_align: usize,
    /// Speaker positions (`dwChannelMask`), 0 if the format did not say.
    mask: u32,
}

impl Fmt {
    fn describe(&self) -> String {
        format!(
            "{} Hz, {} ch, {}-bit {}",
            self.rate,
            self.channels,
            self.bits,
            if self.float { "float" } else { "PCM" }
        )
    }
}

/// Sample encodings [`decode_sample`] understands.
fn supported(float: bool, bits: u16) -> bool {
    matches!((float, bits), (false, 8 | 16 | 24 | 32) | (true, 32 | 64))
}

/// Read a format WASAPI handed us. `Err` describes a format we can't decode.
///
/// # Safety
/// `p` must point at a valid `WAVEFORMATEX` (or `WAVEFORMATEXTENSIBLE` when the tag says so).
unsafe fn parse_format(p: *const WAVEFORMATEX) -> Result<Fmt, String> {
    // SAFETY: the caller guarantees `p` is a valid format; fields are copied out by value
    // because the struct is packed.
    let wf = unsafe { std::ptr::read_unaligned(p) };
    let tag = wf.wFormatTag;
    let bits = wf.wBitsPerSample;
    let (float, mask) = match tag {
        1 => (false, 0), // WAVE_FORMAT_PCM
        3 => (true, 0),  // WAVE_FORMAT_IEEE_FLOAT
        0xFFFE => {
            // WAVE_FORMAT_EXTENSIBLE: the sub-format GUID says PCM or float.
            // SAFETY: the tag guarantees the extensible layout.
            let ext = unsafe { std::ptr::read_unaligned(p as *const WAVEFORMATEXTENSIBLE) };
            let sub = ext.SubFormat;
            let mask = ext.dwChannelMask;
            if sub == SUBTYPE_FLOAT {
                (true, mask)
            } else if sub == SUBTYPE_PCM {
                (false, mask)
            } else {
                return Err(format!("unsupported sub-format {sub:?}"));
            }
        }
        other => return Err(format!("unsupported format tag {other:#x}")),
    };
    let channels = wf.nChannels;
    let block_align = wf.nBlockAlign as usize;
    if !supported(float, bits)
        || channels == 0
        || block_align < (bits as usize / 8) * channels as usize
    {
        return Err(format!(
            "unsupported sample format ({}-bit {}, {channels} ch, block {block_align})",
            bits,
            if float { "float" } else { "PCM" }
        ));
    }
    Ok(Fmt {
        rate: wf.nSamplesPerSec,
        channels,
        float,
        bits,
        block_align,
        mask,
    })
}

/// Decode one little-endian sample to `-1.0..=1.0`. Integer samples are MSB-aligned in their
/// container, so 24-in-32 decodes correctly as 32-bit.
fn decode_sample(b: &[u8], float: bool) -> f32 {
    match (float, b) {
        (false, [x]) => (*x as f32 - 128.0) / 128.0,
        (false, [a, b]) => i16::from_le_bytes([*a, *b]) as f32 / 32_768.0,
        (false, [a, b, c]) => i32::from_le_bytes([0, *a, *b, *c]) as f32 / 2_147_483_648.0,
        (false, [a, b, c, d]) => i32::from_le_bytes([*a, *b, *c, *d]) as f32 / 2_147_483_648.0,
        (true, [a, b, c, d]) => f32::from_le_bytes([*a, *b, *c, *d]),
        (true, [a, b, c, d, e, f, g, h]) => {
            f64::from_le_bytes([*a, *b, *c, *d, *e, *f, *g, *h]) as f32
        }
        _ => 0.0,
    }
}

// Speaker-position bits of `dwChannelMask` (ksmedia.h).
const SPEAKER_FRONT_LEFT: u32 = 0x1;
const SPEAKER_FRONT_RIGHT: u32 = 0x2;
const SPEAKER_LOW_FREQUENCY: u32 = 0x8;
const LEFT_SIDE: u32 = 0x10 | 0x40 | 0x200 | 0x1000 | 0x8000; // BL, FLC, SL, TFL, TBL
const RIGHT_SIDE: u32 = 0x20 | 0x80 | 0x400 | 0x4000 | 0x20000; // BR, FRC, SR, TFR, TBR
const MINUS_3DB: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Per-channel (left, right) gains that fold a device's channels down to stereo. Centre and
/// rear channels are kept at -3 dB (dropping the centre would lose the dialogue in a 5.1 mix);
/// the LFE channel is dropped.
fn downmix_gains(channels: usize, mask: u32) -> Vec<[f32; 2]> {
    if channels == 1 {
        return vec![[1.0, 1.0]];
    }
    let mask = if mask.count_ones() as usize == channels {
        mask
    } else {
        // The format did not describe its layout: assume the Windows default for the count.
        match channels {
            2 => 0x3,
            4 => 0x33,  // quad
            6 => 0x3F,  // 5.1
            8 => 0x63F, // 7.1
            _ => 0,
        }
    };
    if mask == 0 {
        // Unknown layout: first two channels are left/right, the rest is folded in at -3 dB.
        return (0..channels)
            .map(|c| match c {
                0 => [1.0, 0.0],
                1 => [0.0, 1.0],
                _ if c % 2 == 0 => [MINUS_3DB, 0.0],
                _ => [0.0, MINUS_3DB],
            })
            .collect();
    }
    // Channels appear in the stream in ascending bit order of the mask.
    (0..32)
        .map(|b| 1u32 << b)
        .filter(|bit| mask & bit != 0)
        .map(|bit| match bit {
            SPEAKER_FRONT_LEFT => [1.0, 0.0],
            SPEAKER_FRONT_RIGHT => [0.0, 1.0],
            SPEAKER_LOW_FREQUENCY => [0.0, 0.0],
            b if b & LEFT_SIDE != 0 => [MINUS_3DB, 0.0],
            b if b & RIGHT_SIDE != 0 => [0.0, MINUS_3DB],
            _ => [MINUS_3DB, MINUS_3DB], // centres
        })
        .collect()
}

/// Decode `frames` frames of a device buffer to stereo f32, appending to `out`.
fn decode_frames(
    raw: &[u8],
    frames: usize,
    fmt: &Fmt,
    gains: &[[f32; 2]],
    out: &mut Vec<[f32; 2]>,
) {
    let bytes = (fmt.bits / 8) as usize;
    for f in 0..frames {
        let base = f * fmt.block_align;
        let Some(frame) = raw.get(base..base + bytes * gains.len()) else {
            break;
        };
        let mut lr = [0.0f32; 2];
        for (c, g) in gains.iter().enumerate() {
            let s = decode_sample(&frame[c * bytes..(c + 1) * bytes], fmt.float);
            lr[0] += s * g[0];
            lr[1] += s * g[1];
        }
        out.push(lr);
    }
}

/// Linear resampler from a device rate to [`RATE`] that interpolates across buffer boundaries.
#[derive(Debug)]
struct Resampler {
    /// Input frames advanced per output frame.
    step: f64,
    /// Read position relative to `prev` (0.0 = `prev`, 1.0 = the first new frame).
    pos: f64,
    prev: Option<[f32; 2]>,
}

impl Resampler {
    fn new(from_rate: u32) -> Self {
        Resampler {
            step: from_rate as f64 / RATE as f64,
            pos: 0.0,
            prev: None,
        }
    }

    fn push(&mut self, src: &[[f32; 2]], out: &mut VecDeque<[f32; 2]>) {
        if self.step == 1.0 {
            out.extend(src.iter().copied());
            return;
        }
        let (prev, src) = match (self.prev, src) {
            (Some(p), s) => (p, s),
            (None, [first, rest @ ..]) => (*first, rest),
            (None, []) => return,
        };
        let n = src.len();
        // The virtual buffer is [prev, src...]; positions up to (but excluding) `n` have both
        // neighbours available.
        while self.pos < n as f64 {
            let i = self.pos.floor() as usize;
            let frac = (self.pos - i as f64) as f32;
            let a = if i == 0 { prev } else { src[i - 1] };
            let b = src[i];
            out.push_back([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]);
            self.pos += self.step;
        }
        self.pos -= n as f64;
        self.prev = Some(src.last().copied().unwrap_or(prev));
    }
}

// ---- mixing ------------------------------------------------------------------------------------

/// After this many empty mixer calls, a starved source is treated as silent so it doesn't stall
/// the other source (≈ STARVE_LIMIT × 5 ms). Keeps A/V aligned through a device hiccup.
const STARVE_LIMIT: u32 = 10;
/// A source more than this far ahead of the mixer (its clock runs fast, or the other source
/// stalled for long) has its oldest audio dropped so it can't drift out of sync indefinitely.
const MAX_BACKLOG: usize = RATE as usize / 2;

/// One source's queue of stereo frames at [`RATE`], plus its mixer bookkeeping.
#[derive(Debug, Default)]
struct Lane {
    queue: VecDeque<[f32; 2]>,
    /// Consecutive mixer calls this lane has been short of a chunk.
    starved: u32,
    /// Frames the mixer already filled with silence for this lane; its late audio for that span
    /// is dropped when it arrives, so the lane stays aligned with the others.
    owed: usize,
    /// The device failed; the lane is silence from now on.
    dead: bool,
}

/// Pure mixer step: emit one chunk if possible, summing all lanes and substituting silence for
/// any lane that has starved past [`STARVE_LIMIT`]. Returns `None` when it should wait for more.
fn mix_step(lanes: &mut [Lane], chunk_frames: usize) -> Option<Vec<i16>> {
    if lanes.is_empty() {
        return None;
    }
    for lane in lanes.iter_mut() {
        let late = lane.owed.min(lane.queue.len());
        lane.queue.drain(..late);
        lane.owed -= late;
        let excess = lane.queue.len().saturating_sub(MAX_BACKLOG);
        lane.queue.drain(..excess);
    }
    // A lane with too little buffered blocks the emit — but only until it starves out.
    let mut any_ready = false;
    for lane in lanes.iter_mut() {
        if lane.queue.len() >= chunk_frames {
            any_ready = true;
            lane.starved = 0;
        } else {
            lane.starved = lane.starved.saturating_add(1);
        }
    }
    if !any_ready {
        return None;
    }
    if lanes
        .iter()
        .any(|l| !l.dead && l.queue.len() < chunk_frames && l.starved < STARVE_LIMIT)
    {
        return None; // give the lagging lane a little time to catch up
    }

    let mut chunk = vec![0i16; chunk_frames * CHANNELS];
    let mut sums = vec![[0.0f32; 2]; chunk_frames];
    for lane in lanes.iter_mut() {
        let take = lane.queue.len().min(chunk_frames);
        for (sum, [l, r]) in sums.iter_mut().zip(lane.queue.drain(..take)) {
            sum[0] += l;
            sum[1] += r;
        }
        if !lane.dead {
            lane.owed += chunk_frames - take;
        }
    }
    for (slot, s) in chunk.as_chunks_mut::<CHANNELS>().0.iter_mut().zip(&sums) {
        slot[0] = to_i16(s[0]);
        slot[1] = to_i16(s[1]);
    }
    Some(chunk)
}

fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
}

// ---- devices -----------------------------------------------------------------------------------

/// A `GetMixFormat` result, freed with `CoTaskMemFree` when dropped (WASAPI copies the format
/// in `Initialize`, so freeing afterwards is fine).
struct MixFormat(*mut WAVEFORMATEX);

impl MixFormat {
    /// # Safety
    /// `client` must be an activated audio client.
    unsafe fn of(client: &IAudioClient) -> windows::core::Result<MixFormat> {
        // SAFETY: per the caller; the pointer is owned by us from here on.
        unsafe { client.GetMixFormat().map(MixFormat) }
    }
}

impl Drop for MixFormat {
    fn drop(&mut self) {
        // SAFETY: allocated by WASAPI with the COM task allocator; freed exactly once.
        unsafe { CoTaskMemFree(Some(self.0 as *const _)) };
    }
}

/// Take ownership of a COM-allocated wide string.
///
/// # Safety
/// `p` must come from the COM task allocator (or be null).
unsafe fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: per the caller; the string is NUL-terminated and freed exactly once.
    unsafe {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Endpoint id and friendly name of an audio device.
///
/// # Safety
/// `dev` must be a valid endpoint.
unsafe fn device_identity(dev: &IMMDevice) -> (String, String) {
    // SAFETY: standard IMMDevice property reads; every allocation is freed.
    unsafe {
        let id = dev.GetId().map(|p| take_pwstr(p)).unwrap_or_default();
        let name = dev
            .OpenPropertyStore(STGM_READ)
            .and_then(|store| store.GetValue(&PKEY_Device_FriendlyName))
            .map(|mut v| {
                let s = PropVariantToStringAlloc(&v)
                    .map(|p| take_pwstr(p))
                    .unwrap_or_default();
                let _ = PropVariantClear(&mut v);
                s
            })
            .unwrap_or_default();
        (id, name)
    }
}

/// An active audio input (microphone) the user can choose in `microphone_device`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInput {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// List the active audio inputs. Requires COM on the calling thread.
pub fn list_inputs() -> windows::core::Result<Vec<AudioInput>> {
    // SAFETY: COM enumeration of capture endpoints.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let default_id = enumerator
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .map(|d| device_identity(&d).0)
            .unwrap_or_default();
        let all = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
        let mut out = Vec::new();
        for i in 0..all.GetCount()? {
            let dev = all.Item(i)?;
            let (id, name) = device_identity(&dev);
            out.push(AudioInput {
                is_default: id == default_id,
                id,
                name,
            });
        }
        Ok(out)
    }
}

/// Pick the input matching `wanted`: the exact endpoint id, else an exact (case-insensitive)
/// name, else the first name containing it.
fn match_input(inputs: &[(String, String)], wanted: &str) -> Option<usize> {
    let w = wanted.trim().to_lowercase();
    inputs
        .iter()
        .position(|(id, _)| id.eq_ignore_ascii_case(wanted.trim()))
        .or_else(|| inputs.iter().position(|(_, n)| n.to_lowercase() == w))
        .or_else(|| {
            inputs
                .iter()
                .position(|(_, n)| n.to_lowercase().contains(&w))
        })
}

/// One capture client plus its conversion state.
struct Source {
    label: &'static str,
    client: IAudioClient,
    capture: IAudioCaptureClient,
    fmt: Fmt,
    gains: Vec<[f32; 2]>,
    resampler: Resampler,
}

type Render = (IAudioClient, Fmt, IAudioRenderClient);

/// Open a shared-mode capture client on `dev` (loopback when `flags` says so).
///
/// # Safety
/// COM must be initialised on this thread.
unsafe fn open_source(label: &'static str, dev: &IMMDevice, flags: u32) -> Result<Source, String> {
    // SAFETY: standard WASAPI client setup; the mix format is freed on every path.
    unsafe {
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        let mix = MixFormat::of(&client).map_err(|e| e.to_string())?;
        let fmt = parse_format(mix.0)?;
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, mix.0, None)
            .map_err(|e| e.to_string())?;
        let capture: IAudioCaptureClient = client.GetService().map_err(|e| e.to_string())?;
        let (_, name) = device_identity(dev);
        tracing::info!("audio source {label}: \"{name}\", {}", fmt.describe());
        Ok(Source {
            label,
            client,
            capture,
            gains: downmix_gains(fmt.channels as usize, fmt.mask),
            resampler: Resampler::new(fmt.rate),
            fmt,
        })
    }
}

/// Open the render keep-alive on the default output.
///
/// # Safety
/// COM must be initialised on this thread.
unsafe fn open_render(dev: &IMMDevice) -> Result<Render, String> {
    // SAFETY: standard WASAPI render-client setup; the mix format is freed on every path.
    unsafe {
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        let mix = MixFormat::of(&client).map_err(|e| e.to_string())?;
        let fmt = parse_format(mix.0)?;
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 2_000_000, 0, mix.0, None)
            .map_err(|e| e.to_string())?;
        let render: IAudioRenderClient = client.GetService().map_err(|e| e.to_string())?;
        Ok((client, fmt, render))
    }
}

/// Open every requested source; failures become notes and are skipped.
///
/// # Safety
/// COM must be initialised on this thread.
unsafe fn setup(req: &AudioRequest, notes: &Notes) -> (Vec<Source>, Option<Render>) {
    let mut sources = Vec::new();
    let mut render = None;
    // SAFETY: COM object creation and endpoint lookups for WASAPI.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(e) => {
                    notes.push(format!("audio is unavailable on this PC ({e})"));
                    return (sources, render);
                }
            };

        // System audio = loopback on the default render endpoint, plus a render client
        // (silence keep-alive / tone) so loopback doesn't starve.
        if req.system {
            match enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                Ok(dev) => {
                    match open_source("system audio", &dev, AUDCLNT_STREAMFLAGS_LOOPBACK) {
                        Ok(s) => sources.push(s),
                        Err(e) => notes.push(format!("system audio could not be recorded ({e})")),
                    }
                    match open_render(&dev) {
                        Ok(r) => render = Some(r),
                        // Loopback still works while something is playing; the silent gaps are
                        // filled by the pipeline's clock check.
                        Err(e) => tracing::warn!("audio keep-alive unavailable ({e})"),
                    }
                }
                Err(e) => notes.push(format!(
                    "system audio could not be recorded: no output device ({e})"
                )),
            }
        }

        if req.mic {
            match pick_microphone(&enumerator, req.mic_device.as_deref(), notes) {
                Some(dev) => match open_source("microphone", &dev, 0) {
                    Ok(s) => sources.push(s),
                    Err(e) => notes.push(format!("the microphone could not be recorded ({e})")),
                },
                None => notes.push("no microphone is connected, so none was recorded"),
            }
        }
    }
    (sources, render)
}

/// The microphone to open: the configured one if it is connected, else the default.
///
/// # Safety
/// COM must be initialised on this thread.
unsafe fn pick_microphone(
    enumerator: &IMMDeviceEnumerator,
    wanted: Option<&str>,
    notes: &Notes,
) -> Option<IMMDevice> {
    // SAFETY: endpoint enumeration on a live enumerator.
    unsafe {
        if let Some(wanted) = wanted.filter(|w| !w.trim().is_empty()) {
            let devices: Vec<IMMDevice> = enumerator
                .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
                .ok()
                .map(|all| {
                    (0..all.GetCount().unwrap_or(0))
                        .filter_map(|i| all.Item(i).ok())
                        .collect()
                })
                .unwrap_or_default();
            let ids: Vec<(String, String)> = devices.iter().map(|d| device_identity(d)).collect();
            if let Some(i) = match_input(&ids, wanted) {
                return devices.into_iter().nth(i);
            }
            notes.push(format!(
                "the microphone \"{}\" was not found, so the default microphone was used",
                wanted.trim()
            ));
        }
        enumerator.GetDefaultAudioEndpoint(eCapture, eConsole).ok()
    }
}

fn audio_thread(
    req: AudioRequest,
    chunk_tx: Sender<Vec<i16>>,
    ready_tx: Sender<bool>,
    stop: Arc<AtomicBool>,
    notes: Notes,
) {
    // SAFETY: MTA init for this thread; tolerate an already-initialised apartment.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    // SAFETY: COM is initialised above.
    let (mut sources, render) = unsafe { setup(&req, &notes) };
    let have_audio = !sources.is_empty();
    let _ = ready_tx.send(have_audio);
    if !have_audio || stop.load(Ordering::SeqCst) {
        return;
    }

    // SAFETY: clients are initialised; Start begins streaming.
    unsafe {
        for s in &sources {
            let _ = s.client.Start();
        }
        if let Some((rclient, _, _)) = &render {
            let _ = rclient.Start();
        }
    }

    let mut tone_phase: f32 = 0.0;
    let chunk_frames = RATE as usize / 100; // 10 ms
    let mut lanes: Vec<Lane> = sources.iter().map(|_| Lane::default()).collect();
    let mut scratch = Vec::new();

    while !stop.load(Ordering::SeqCst) {
        if let Some((rclient, rfmt, rrender)) = &render {
            // SAFETY: feed the render buffer to avoid loopback starvation.
            unsafe { feed_render(rclient, rrender, *rfmt, req.tone_hz, &mut tone_phase) };
        }

        for (s, lane) in sources.iter_mut().zip(lanes.iter_mut()) {
            if lane.dead {
                continue;
            }
            // SAFETY: drains all available packets from the capture client.
            if let Err(e) = unsafe { drain_source(s, &mut lane.queue, &mut scratch) } {
                lane.dead = true;
                lane.queue.clear();
                notes.push(format!(
                    "the {} stopped partway through the recording ({e}); the rest has no {} sound",
                    s.label, s.label
                ));
            }
        }

        while let Some(chunk) = mix_step(&mut lanes, chunk_frames) {
            if chunk_tx.send(chunk).is_err() {
                return;
            }
        }

        std::thread::sleep(Duration::from_millis(5));
    }

    // SAFETY: stop all clients on the way out.
    unsafe {
        for s in &sources {
            let _ = s.client.Stop();
        }
        if let Some((rclient, _, _)) = &render {
            let _ = rclient.Stop();
        }
    }
}

/// Drain every available packet into `queue` (at [`RATE`]). An error means the device is gone.
///
/// # Safety
/// `s` must hold a started capture client.
unsafe fn drain_source(
    s: &mut Source,
    queue: &mut VecDeque<[f32; 2]>,
    scratch: &mut Vec<[f32; 2]>,
) -> windows::core::Result<()> {
    // SAFETY: standard capture-client packet drain.
    unsafe {
        loop {
            if s.capture.GetNextPacketSize()? == 0 {
                return Ok(());
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            s.capture
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            let frames = frames as usize;
            scratch.clear();
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                // Zeros keep the sample clock advancing.
                scratch.resize(frames, [0.0, 0.0]);
            } else {
                let raw = std::slice::from_raw_parts(data, frames * s.fmt.block_align);
                decode_frames(raw, frames, &s.fmt, &s.gains, scratch);
            }
            s.capture.ReleaseBuffer(frames as u32)?;
            s.resampler.push(scratch, queue);
        }
    }
}

unsafe fn feed_render(
    client: &IAudioClient,
    render: &IAudioRenderClient,
    fmt: Fmt,
    tone_hz: Option<f32>,
    phase: &mut f32,
) {
    // SAFETY: fill the available render space with silence or a sine tone.
    unsafe {
        let buffer_frames = client.GetBufferSize().unwrap_or(0);
        let padding = client.GetCurrentPadding().unwrap_or(0);
        let avail = buffer_frames.saturating_sub(padding);
        if avail == 0 {
            return;
        }
        let Ok(ptr) = render.GetBuffer(avail) else {
            return;
        };
        match tone_hz {
            None => {
                let _ = render.ReleaseBuffer(avail, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
            }
            Some(hz) => {
                let ch = fmt.channels.max(1) as usize;
                let step = 2.0 * std::f32::consts::PI * hz / fmt.rate as f32;
                if fmt.float && fmt.bits == 32 {
                    let out = std::slice::from_raw_parts_mut(ptr as *mut f32, avail as usize * ch);
                    for f in 0..avail as usize {
                        let v = (*phase).sin() * 0.3;
                        *phase += step;
                        for c in 0..ch {
                            out[f * ch + c] = v;
                        }
                    }
                } else if !fmt.float && fmt.bits == 16 {
                    let out = std::slice::from_raw_parts_mut(ptr as *mut i16, avail as usize * ch);
                    for f in 0..avail as usize {
                        let v = ((*phase).sin() * 0.3 * 32767.0) as i16;
                        *phase += step;
                        for c in 0..ch {
                            out[f * ch + c] = v;
                        }
                    }
                }
                if *phase > std::f32::consts::TAU {
                    *phase -= std::f32::consts::TAU;
                }
                let _ = render.ReleaseBuffer(avail, 0);
            }
        }
    }
}

// ---- probe (dev) -------------------------------------------------------------------------------

/// What an in-memory capture run saw (`rustshot dev audio-probe`). Nothing is written to disk.
#[derive(Debug, Clone, Default)]
pub struct AudioProbe {
    pub capturing: bool,
    pub frames: u64,
    pub rms: f32,
    pub peak: f32,
    pub notes: Vec<String>,
}

/// Run the capture engine for `seconds` and report levels.
pub fn probe_capture(req: AudioRequest, seconds: f32) -> AudioProbe {
    let notes = Notes::default();
    let started = Instant::now();
    let (mut engine, rx) = AudioEngine::start(req, notes.clone());
    let capturing = engine.wait_ready(started + READY_TIMEOUT);
    let mut report = AudioProbe {
        capturing,
        ..AudioProbe::default()
    };
    if capturing {
        let end = Instant::now() + Duration::from_secs_f32(seconds);
        let mut sum_sq = 0.0f64;
        let mut n = 0u64;
        while Instant::now() < end {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(100)) {
                for s in &chunk {
                    let v = *s as f32 / 32768.0;
                    sum_sq += (v as f64) * (v as f64);
                    report.peak = report.peak.max(v.abs());
                }
                n += chunk.len() as u64;
            }
        }
        report.frames = n / CHANNELS as u64;
        report.rms = if n > 0 {
            (sum_sq / n as f64).sqrt() as f32
        } else {
            0.0
        };
    }
    engine.stop();
    report.notes = notes.take();
    report
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn lanes(n: usize) -> Vec<Lane> {
        (0..n).map(|_| Lane::default()).collect()
    }

    #[test]
    fn mix_step_sums_two_ready_sources() {
        let mut l = lanes(2);
        for _ in 0..480 {
            l[0].queue.push_back([0.25, -0.25]);
            l[1].queue.push_back([0.25, 0.25]);
        }
        let chunk = mix_step(&mut l, 480).expect("a chunk");
        assert_eq!(chunk.len(), 480 * 2);
        // 0.25 + 0.25 = 0.5 → ~16383 on the left channel.
        assert!((chunk[0] as i32 - 16383).abs() < 4, "left {}", chunk[0]);
        // -0.25 + 0.25 = 0 on the right.
        assert_eq!(chunk[1], 0);
        assert!(l[0].queue.is_empty() && l[1].queue.is_empty());
    }

    #[test]
    fn mix_step_waits_then_pads_a_starved_source() {
        let mut l = lanes(2);
        for _ in 0..480 {
            l[0].queue.push_back([0.5, 0.5]);
        }
        // l[1] is empty. While under the starve limit, we wait (no emit). The emit happens on
        // the call where l[1]'s starvation counter reaches STARVE_LIMIT.
        for _ in 0..STARVE_LIMIT - 1 {
            assert!(mix_step(&mut l, 480).is_none());
        }
        let chunk = mix_step(&mut l, 480).expect("chunk after starve");
        assert_eq!(chunk.len(), 480 * 2);
        assert!((chunk[0] as i32 - 16383).abs() < 4);
        assert_eq!(l[1].owed, 480, "the padded span is owed");
    }

    #[test]
    fn late_audio_for_a_padded_span_is_dropped() {
        let mut l = lanes(2);
        l[1].owed = 480;
        for _ in 0..480 {
            l[0].queue.push_back([0.1, 0.1]);
        }
        // l[1]'s late 10 ms (already covered by silence), then its on-time 10 ms.
        for _ in 0..480 {
            l[1].queue.push_back([0.9, 0.9]);
        }
        for _ in 0..480 {
            l[1].queue.push_back([0.2, 0.2]);
        }
        let chunk = mix_step(&mut l, 480).expect("a chunk");
        // 0.1 + 0.2 — the late 0.9 span was discarded, keeping l[1] aligned with l[0].
        assert!(
            (chunk[0] as f32 / 32767.0 - 0.3).abs() < 0.01,
            "{}",
            chunk[0]
        );
        assert_eq!(l[1].owed, 0);
    }

    #[test]
    fn dead_lane_does_not_block_or_accrue_debt() {
        let mut l = lanes(2);
        l[1].dead = true;
        for _ in 0..480 {
            l[0].queue.push_back([0.5, 0.5]);
        }
        assert!(
            mix_step(&mut l, 480).is_some(),
            "no starve wait for a dead lane"
        );
        assert_eq!(l[1].owed, 0);
    }

    #[test]
    fn backlog_is_capped() {
        let mut l = lanes(1);
        l[0].queue
            .extend(std::iter::repeat_n([0.0, 0.0], MAX_BACKLOG + 4800));
        mix_step(&mut l, 480).expect("a chunk");
        assert_eq!(l[0].queue.len(), MAX_BACKLOG - 480);
    }

    #[test]
    fn mix_step_none_when_nothing_ready() {
        assert!(mix_step(&mut lanes(1), 480).is_none());
        assert!(mix_step(&mut [], 480).is_none());
    }

    #[test]
    fn decodes_every_supported_width() {
        assert_eq!(decode_sample(&[0x80], false), 0.0);
        assert_eq!(decode_sample(&[0x00], false), -1.0);
        assert_eq!(decode_sample(&0x4000i16.to_le_bytes(), false), 0.5);
        // 24-bit: 0x400000 = half scale; 0xC00000 = minus half (sign-extended).
        assert_eq!(decode_sample(&[0x00, 0x00, 0x40], false), 0.5);
        assert_eq!(decode_sample(&[0x00, 0x00, 0xC0], false), -0.5);
        assert_eq!(decode_sample(&0x4000_0000i32.to_le_bytes(), false), 0.5);
        assert_eq!(decode_sample(&(-0.25f32).to_le_bytes(), true), -0.25);
        assert_eq!(decode_sample(&0.75f64.to_le_bytes(), true), 0.75);
        assert_eq!(
            decode_sample(&[1, 2, 3], true),
            0.0,
            "24-bit float is not a thing"
        );
        for (float, bits) in [
            (false, 8),
            (false, 16),
            (false, 24),
            (false, 32),
            (true, 32),
            (true, 64),
        ] {
            assert!(supported(float, bits));
        }
        assert!(!supported(true, 16) && !supported(false, 12));
    }

    fn fmt(channels: u16, bits: u16, float: bool, mask: u32) -> Fmt {
        Fmt {
            rate: RATE,
            channels,
            float,
            bits,
            block_align: (bits / 8 * channels) as usize,
            mask,
        }
    }

    #[test]
    fn decode_frames_24bit_stereo() {
        let f = fmt(2, 24, false, 0x3);
        // Frame 0: L = +0.5, R = -0.5.
        let raw = [0x00, 0x00, 0x40, 0x00, 0x00, 0xC0];
        let mut out = Vec::new();
        decode_frames(&raw, 1, &f, &downmix_gains(2, 0x3), &mut out);
        assert_eq!(out, vec![[0.5, -0.5]]);
    }

    #[test]
    fn decode_frames_respects_block_align_padding() {
        // 16-bit mono in a 4-byte block (2 bytes padding per frame).
        let mut f = fmt(1, 16, false, 0);
        f.block_align = 4;
        let raw = [0x00, 0x40, 0xAA, 0xAA, 0x00, 0xC0, 0xAA, 0xAA];
        let mut out = Vec::new();
        decode_frames(&raw, 2, &f, &downmix_gains(1, 0), &mut out);
        assert_eq!(out, vec![[0.5, 0.5], [-0.5, -0.5]]);
    }

    #[test]
    fn downmix_keeps_centre_and_drops_lfe() {
        let g = downmix_gains(6, 0x3F); // FL FR FC LFE BL BR
        assert_eq!(g.len(), 6);
        assert_eq!(g[0], [1.0, 0.0]);
        assert_eq!(g[1], [0.0, 1.0]);
        assert_eq!(g[2], [MINUS_3DB, MINUS_3DB]);
        assert_eq!(g[3], [0.0, 0.0]);
        assert_eq!(g[4], [MINUS_3DB, 0.0]);
        assert_eq!(g[5], [0.0, MINUS_3DB]);
        // A mask that doesn't match the count falls back to the default 5.1 layout.
        assert_eq!(downmix_gains(6, 0x3), g);
        assert_eq!(downmix_gains(1, 0x4), vec![[1.0, 1.0]]);
        assert_eq!(downmix_gains(3, 0).len(), 3);
    }

    #[test]
    fn resampler_is_continuous_across_buffers_and_keeps_rate() {
        let mut r = Resampler::new(44_100);
        let mut out = VecDeque::new();
        // A ramp split across uneven buffers resamples to a monotonic ramp (no seam jumps).
        let ramp: Vec<[f32; 2]> = (0..44_100).map(|i| [i as f32, i as f32]).collect();
        for part in ramp.chunks(441 + 7) {
            r.push(part, &mut out);
        }
        let n = out.len() as i64;
        assert!((n - 48_000).abs() <= 2, "{n} frames");
        let step = 44_100.0 / 48_000.0;
        for (k, f) in out.iter().enumerate() {
            assert!(
                (f[0] - k as f32 * step as f32).abs() < 0.05,
                "frame {k}: {}",
                f[0]
            );
        }
    }

    #[test]
    fn resampler_passes_through_at_canonical_rate() {
        let mut r = Resampler::new(RATE);
        let mut out = VecDeque::new();
        r.push(&[[0.1, 0.2], [0.3, 0.4]], &mut out);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn microphone_matching_prefers_id_then_name_then_substring() {
        let inputs = vec![
            (
                "{0.0.1}.{aaa}".to_string(),
                "Microphone (USB Cam)".to_string(),
            ),
            ("{0.0.1}.{bbb}".to_string(), "Microphone Array".to_string()),
        ];
        assert_eq!(match_input(&inputs, "{0.0.1}.{BBB}"), Some(1));
        assert_eq!(match_input(&inputs, "microphone array"), Some(1));
        assert_eq!(match_input(&inputs, "usb cam"), Some(0));
        assert_eq!(match_input(&inputs, "  Array "), Some(1));
        assert_eq!(match_input(&inputs, "headset"), None);
    }
}
