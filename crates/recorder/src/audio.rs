//! WASAPI audio capture for recording: system (render-endpoint loopback) and/or microphone,
//! mixed to a canonical 48 kHz stereo 16-bit PCM stream delivered in ~10 ms chunks.
//!
//! cpal does not expose WASAPI loopback, so this uses raw WASAPI. A render client on the default
//! output endpoint is always opened alongside loopback: it feeds silence (a "keep-alive") so the
//! loopback capture doesn't starve when nothing else is playing — or, for the functional test, a
//! sine tone so the captured audio has a known signal.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::GUID;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IAudioRenderClient,
    IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};

/// Canonical capture rate and channel count.
pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

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

/// A running audio capture. Dropping it stops the thread.
pub struct AudioEngine {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine").finish_non_exhaustive()
    }
}

impl AudioEngine {
    /// Start capture. Returns the engine, a receiver of interleaved-stereo i16 chunks, and a
    /// flag that is true if at least one source initialised (else the caller records video-only).
    pub fn start(
        system: bool,
        mic: bool,
        tone_hz: Option<f32>,
    ) -> (AudioEngine, Receiver<Vec<i16>>, bool) {
        let (chunk_tx, chunk_rx) = mpsc::channel::<Vec<i16>>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<bool>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();

        let handle = std::thread::Builder::new()
            .name("rustshot-audio".into())
            .spawn(move || audio_thread(system, mic, tone_hz, chunk_tx, ready_tx, stop_thread))
            .ok();

        let available = handle.is_some()
            && ready_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap_or(false);

        (AudioEngine { stop, handle }, chunk_rx, available)
    }

    /// Stop capture and join the thread.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Parsed essentials of a `WAVEFORMATEX`.
#[derive(Clone, Copy)]
struct Fmt {
    rate: u32,
    channels: u16,
    float: bool,
    bits: u16,
}

unsafe fn parse_format(p: *const WAVEFORMATEX) -> Option<Fmt> {
    // SAFETY: `p` points at a valid WAVEFORMATEX (from GetMixFormat).
    let wf = unsafe { &*p };
    let (float, bits) = match wf.wFormatTag {
        1 => (false, wf.wBitsPerSample), // WAVE_FORMAT_PCM
        3 => (true, wf.wBitsPerSample),  // WAVE_FORMAT_IEEE_FLOAT
        0xFFFE => {
            // WAVE_FORMAT_EXTENSIBLE: inspect the sub-format GUID.
            let ext = unsafe { &*(p as *const WAVEFORMATEXTENSIBLE) };
            let sub = ext.SubFormat;
            if sub == SUBTYPE_FLOAT {
                (true, wf.wBitsPerSample)
            } else if sub == SUBTYPE_PCM {
                (false, wf.wBitsPerSample)
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(Fmt {
        rate: wf.nSamplesPerSec,
        channels: wf.nChannels,
        float,
        bits,
    })
}

/// Convert one device buffer to canonical-rate stereo f32 frames, appending to `out`.
/// `resample_pos` carries the fractional read position across calls for linear resampling.
fn convert_append(
    raw: &[u8],
    frames: usize,
    fmt: Fmt,
    out: &mut VecDeque<[f32; 2]>,
    resample_pos: &mut f64,
) {
    let ch = fmt.channels.max(1) as usize;
    let mut src: Vec<[f32; 2]> = Vec::with_capacity(frames);
    for i in 0..frames {
        let (l, r) = read_stereo(raw, i, ch, fmt);
        src.push([l, r]);
    }
    if src.is_empty() {
        return;
    }
    if fmt.rate == RATE {
        out.extend(src);
        return;
    }
    // Linear resample src.rate → RATE.
    let ratio = fmt.rate as f64 / RATE as f64;
    let mut pos = *resample_pos;
    while pos < src.len() as f64 {
        let i = pos.floor() as usize;
        let frac = (pos - i as f64) as f32;
        let a = src[i];
        let b = src[(i + 1).min(src.len() - 1)];
        out.push_back([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]);
        pos += ratio;
    }
    *resample_pos = pos - src.len() as f64;
}

fn read_stereo(raw: &[u8], frame: usize, ch: usize, fmt: Fmt) -> (f32, f32) {
    let bytes_per_sample = (fmt.bits / 8).max(1) as usize;
    let frame_bytes = bytes_per_sample * ch;
    let base = frame * frame_bytes;
    let sample = |c: usize| -> f32 {
        let off = base + c * bytes_per_sample;
        if off + bytes_per_sample > raw.len() {
            return 0.0;
        }
        if fmt.float && bytes_per_sample == 4 {
            f32::from_le_bytes([raw[off], raw[off + 1], raw[off + 2], raw[off + 3]])
        } else if !fmt.float && bytes_per_sample == 2 {
            i16::from_le_bytes([raw[off], raw[off + 1]]) as f32 / 32768.0
        } else {
            0.0
        }
    };
    if ch == 1 {
        let m = sample(0);
        (m, m)
    } else {
        (sample(0), sample(1))
    }
}

/// One capture client plus its conversion state (its frames go into a parallel queue).
struct Source {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    fmt: Fmt,
    resample_pos: f64,
}

/// After this many empty iterations, a starved source is treated as silent so it doesn't stall
/// the other source (≈ STARVE_LIMIT × 5 ms). Keeps A/V aligned through a device hiccup.
const STARVE_LIMIT: u32 = 10;

/// Pure mixer step: emit one chunk if possible, summing all sources and substituting silence for
/// any source that has starved past [`STARVE_LIMIT`]. Returns `None` when it should wait for more.
fn mix_step(
    queues: &mut [VecDeque<[f32; 2]>],
    starved: &mut [u32],
    chunk_frames: usize,
) -> Option<Vec<i16>> {
    if queues.is_empty() {
        return None;
    }
    // A source with too little buffered blocks the emit — but only until it starves out.
    let mut any_ready = false;
    for (q, st) in queues.iter().zip(starved.iter_mut()) {
        if q.len() >= chunk_frames {
            any_ready = true;
            *st = 0;
        } else {
            *st += 1;
        }
    }
    if !any_ready {
        return None;
    }
    for (q, st) in queues.iter().zip(starved.iter()) {
        if q.len() < chunk_frames && *st < STARVE_LIMIT {
            return None; // give the lagging source a little time to catch up
        }
    }

    let mut chunk = vec![0i16; chunk_frames * CHANNELS];
    let (pairs, _) = chunk.as_chunks_mut::<2>();
    for slot in pairs.iter_mut() {
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for q in queues.iter_mut() {
            if let Some([sl, sr]) = q.pop_front() {
                l += sl;
                r += sr;
            }
        }
        slot[0] = to_i16(l);
        slot[1] = to_i16(r);
    }
    Some(chunk)
}

fn audio_thread(
    system: bool,
    mic: bool,
    tone_hz: Option<f32>,
    chunk_tx: std::sync::mpsc::Sender<Vec<i16>>,
    ready_tx: SyncSender<bool>,
    stop: Arc<AtomicBool>,
) {
    // SAFETY: MTA init for this thread; tolerate an already-initialised apartment.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let result = unsafe { setup(system, mic, tone_hz) };
    let (mut sources, render) = match result {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("audio setup failed ({e}); recording video-only");
            let _ = ready_tx.send(false);
            return;
        }
    };
    let have_audio = !sources.is_empty();
    let _ = ready_tx.send(have_audio);
    if !have_audio {
        return;
    }

    // Start all clients.
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
    let mut queues: Vec<VecDeque<[f32; 2]>> = (0..sources.len()).map(|_| VecDeque::new()).collect();
    let mut starved = vec![0u32; sources.len()];

    while !stop.load(Ordering::SeqCst) {
        // Service the render endpoint (silence keep-alive or test tone).
        if let Some((rclient, rfmt, rrender)) = &render {
            // SAFETY: feed the render buffer to avoid loopback starvation.
            unsafe {
                feed_render(rclient, rrender, *rfmt, tone_hz, &mut tone_phase);
            }
        }

        // Drain each capture source into its parallel queue.
        for (i, s) in sources.iter_mut().enumerate() {
            // SAFETY: drains all available packets from the capture client.
            unsafe { drain_source(s, &mut queues[i]) };
        }

        // Emit as many aligned chunks as the sources can supply.
        while let Some(chunk) = mix_step(&mut queues, &mut starved, chunk_frames) {
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

fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
}

type Render = (IAudioClient, Fmt, IAudioRenderClient);

unsafe fn setup(
    system: bool,
    mic: bool,
    tone_hz: Option<f32>,
) -> Result<(Vec<Source>, Option<Render>), windows::core::Error> {
    // SAFETY: COM object creation for WASAPI.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;

        let mut sources = Vec::new();
        let mut render = None;

        // System audio = loopback on the default render endpoint. Always open a render client
        // too (silence keep-alive / tone), so loopback doesn't starve.
        if system {
            let dev = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
            let mix = client.GetMixFormat()?;
            if let Some(fmt) = parse_format(mix) {
                client.Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_LOOPBACK,
                    2_000_000,
                    0,
                    mix,
                    None,
                )?;
                let capture: IAudioCaptureClient = client.GetService()?;
                sources.push(Source {
                    client,
                    capture,
                    fmt,
                    resample_pos: 0.0,
                });
            }

            // Render client (keep-alive / tone).
            let rdev = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let rclient: IAudioClient = rdev.Activate(CLSCTX_ALL, None)?;
            let rmix = rclient.GetMixFormat()?;
            if let Some(rfmt) = parse_format(rmix) {
                rclient.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 2_000_000, 0, rmix, None)?;
                let rrender: IAudioRenderClient = rclient.GetService()?;
                let _ = tone_hz;
                render = Some((rclient, rfmt, rrender));
            }
        }

        // Microphone = default capture endpoint.
        if mic {
            match enumerator.GetDefaultAudioEndpoint(eCapture, eConsole) {
                Ok(dev) => {
                    let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
                    let mix = client.GetMixFormat()?;
                    if let Some(fmt) = parse_format(mix) {
                        client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 2_000_000, 0, mix, None)?;
                        let capture: IAudioCaptureClient = client.GetService()?;
                        sources.push(Source {
                            client,
                            capture,
                            fmt,
                            resample_pos: 0.0,
                        });
                    }
                }
                Err(e) => tracing::warn!("no microphone endpoint ({e}); skipping mic"),
            }
        }

        Ok((sources, render))
    }
}

unsafe fn drain_source(s: &mut Source, queue: &mut VecDeque<[f32; 2]>) {
    // SAFETY: standard capture-client packet drain.
    unsafe {
        loop {
            let packet = match s.capture.GetNextPacketSize() {
                Ok(p) => p,
                Err(_) => break,
            };
            if packet == 0 {
                break;
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            if s.capture
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                .is_err()
            {
                break;
            }
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            let frame_bytes = (s.fmt.bits / 8).max(1) as usize * s.fmt.channels.max(1) as usize;
            if silent || data.is_null() {
                // Push zeros so the sample clock keeps advancing.
                for _ in 0..frames as usize {
                    queue.push_back([0.0, 0.0]);
                }
            } else {
                let raw = std::slice::from_raw_parts(data, frames as usize * frame_bytes);
                convert_append(raw, frames as usize, s.fmt, queue, &mut s.resample_pos);
            }
            let _ = s.capture.ReleaseBuffer(frames);
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn mix_step_sums_two_ready_sources() {
        let mut q = vec![VecDeque::new(), VecDeque::new()];
        for _ in 0..480 {
            q[0].push_back([0.25, -0.25]);
            q[1].push_back([0.25, 0.25]);
        }
        let mut starved = vec![0u32; 2];
        let chunk = mix_step(&mut q, &mut starved, 480).expect("a chunk");
        assert_eq!(chunk.len(), 480 * 2);
        // 0.25 + 0.25 = 0.5 → ~16383 on the left channel.
        assert!((chunk[0] as i32 - 16383).abs() < 4, "left {}", chunk[0]);
        // -0.25 + 0.25 = 0 on the right.
        assert_eq!(chunk[1], 0);
        assert!(q[0].is_empty() && q[1].is_empty());
    }

    #[test]
    fn mix_step_waits_then_pads_a_starved_source() {
        let mut q = vec![VecDeque::new(), VecDeque::new()];
        for _ in 0..480 {
            q[0].push_back([0.5, 0.5]);
        }
        // q[1] is empty. While under the starve limit, we wait (no emit). The emit happens on
        // the call where q[1]'s starvation counter reaches STARVE_LIMIT.
        let mut starved = vec![0u32; 2];
        for _ in 0..STARVE_LIMIT - 1 {
            assert!(mix_step(&mut q, &mut starved, 480).is_none());
        }
        // Once q[1] has starved out, emit using q[0] with silence for q[1].
        let chunk = mix_step(&mut q, &mut starved, 480).expect("chunk after starve");
        assert_eq!(chunk.len(), 480 * 2);
        assert!((chunk[0] as i32 - 16383).abs() < 4);
    }

    #[test]
    fn mix_step_none_when_nothing_ready() {
        let mut q = vec![VecDeque::new()];
        let mut starved = vec![0u32; 1];
        assert!(mix_step(&mut q, &mut starved, 480).is_none());
    }
}
