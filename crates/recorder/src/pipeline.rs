//! The recording pipeline (Windows): a dedicated encoder thread owns the Media Foundation sink
//! and the WGC capture stream, re-emitting the latest captured frame at a constant rate.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use capture_core::{CaptureStream, Frame, StreamEvent};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::audio::{AudioEngine, AudioRequest};
use crate::clock::FrameClock;
use crate::mf::VideoSink;
use crate::{Notes, RecordCfg, RecordOutput, RecorderError, Result};

/// Controls and the join handle for a running encoder thread.
#[derive(Debug)]
pub struct Session {
    paused: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<std::path::PathBuf>>>,
}

impl Session {
    pub fn start(cfg: RecordCfg, notes: Notes) -> Result<Session> {
        let paused = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        let p = paused.clone();
        let s = stopping.clone();
        let thread = std::thread::Builder::new()
            .name("rustshot-encoder".into())
            .spawn(move || encode_loop(cfg, p, s, notes))
            .map_err(|e| RecorderError::Platform(format!("spawning encoder thread: {e}")))?;
        Ok(Session {
            paused,
            stopping,
            thread: Some(thread),
        })
    }

    pub fn set_paused(&self, v: bool) {
        self.paused.store(v, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// True once the encoder thread has exited on its own.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    pub fn stop(mut self) -> Result<std::path::PathBuf> {
        self.stopping.store(true, Ordering::SeqCst);
        match self.thread.take() {
            Some(t) => t
                .join()
                .map_err(|_| RecorderError::Platform("encoder thread panicked".into()))?,
            None => Err(RecorderError::Platform("recording already stopped".into())),
        }
    }
}

fn cap_err(e: capture_core::CaptureError) -> RecorderError {
    RecorderError::Capture(e.to_string())
}

fn encode_loop(
    cfg: RecordCfg,
    paused: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    notes: Notes,
) -> Result<std::path::PathBuf> {
    // MTA for this thread: satisfies both Media Foundation and WinRT (WGC).
    // SAFETY: initialises COM for the encoder thread; result is tolerated.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let (fps, bitrate, cursor, audio_cfg) = match cfg.output {
        RecordOutput::Mp4 {
            fps,
            bitrate,
            cursor,
            audio,
        } => (fps, bitrate, cursor, audio),
        RecordOutput::Gif { .. } => return Err(RecorderError::NotImplemented),
    };

    // Optional audio capture, started before the video so device setup overlaps the wait for
    // the first frame. A dev/test env var injects a sine tone into the render keep-alive so
    // loopback has a known signal to prove the path end-to-end.
    let want_audio = audio_cfg.microphone || audio_cfg.system_audio;
    let audio_started = Instant::now();
    let mut audio = want_audio.then(|| {
        AudioEngine::start(
            AudioRequest {
                system: audio_cfg.system_audio,
                mic: audio_cfg.microphone,
                mic_device: audio_cfg.microphone_device.clone(),
                tone_hz: std::env::var("RUSTSHOT_TEST_TONE_HZ")
                    .ok()
                    .and_then(|s| s.parse::<f32>().ok()),
            },
            notes.clone(),
        )
    });

    let (tx, rx) = mpsc::channel::<StreamEvent>();
    let stream = CaptureStream::start(&cfg.target, cursor, fps, tx).map_err(cap_err)?;

    // Wait for the first frame to learn the real (content) size.
    let first = match wait_first(&rx) {
        Ok(f) => f,
        Err(e) => {
            stream.stop();
            return Err(e);
        }
    };
    let (w, h) = crate::even_dims(first.width, first.height);
    if w == 0 || h == 0 {
        stream.stop();
        return Err(RecorderError::Platform("captured frame is empty".into()));
    }

    // The sink's track layout is fixed at creation, so settle whether there is audio now. A
    // source that failed has already left a note saying why.
    if let Some((engine, _)) = audio.as_mut() {
        if !engine.wait_ready(audio_started + crate::audio::READY_TIMEOUT) {
            audio = None;
        }
    }
    let (audio_engine, audio_rx) = match audio {
        Some((engine, rx)) => (Some(engine), Some(rx)),
        None => (None, None),
    };

    let path_str = cfg.out_path.to_string_lossy().to_string();
    let sink = match VideoSink::new(&path_str, w, h, fps, bitrate, audio_rx.is_some()) {
        Ok(s) => s,
        Err(e) => {
            stream.stop();
            return Err(e);
        }
    };
    let tick = Duration::from_nanos(1_000_000_000 / fps.max(1) as u64);

    let mut latest = first;
    let mut idx: i64 = 0;
    let captured = AtomicU64::new(1);
    let mut emitted: u64 = 0;
    // Why the loop ended before `stopping` was set, if it did.
    let mut ended_early: Option<String> = None;
    let mut last_health_check = Instant::now();
    // Audio uses a sample-count clock (can't drift against the video index clock).
    let mut audio_frames: i64 = 0;

    // CFR driven by the monotonic clock (see `clock`): we re-emit the latest frame to catch up,
    // so the output's duration tracks real (unpaused) wall-clock time rather than drifting with
    // sleep latency.
    let mut clock = FrameClock::new(fps, Instant::now());
    let max_catch_up = (fps as i64).max(1) * 2;

    // Drop audio captured during video startup so both timelines begin at t = 0 (A/V sync).
    if let Some(arx) = &audio_rx {
        while arx.try_recv().is_ok() {}
    }

    let result = (|| -> Result<()> {
        while !stopping.load(Ordering::SeqCst) && ended_early.is_none() {
            let now = Instant::now();

            // A removed/reset graphics device may simply stop delivering frames, and WGC sends
            // nothing on an idle desktop anyway, so poll the device rather than a frame clock.
            if now.saturating_duration_since(last_health_check) >= HEALTH_CHECK_EVERY {
                last_health_check = now;
                if let Err(e) = stream.check_health() {
                    ended_early = Some(e.to_string());
                    break;
                }
            }
            let is_paused = paused.load(Ordering::SeqCst);
            clock.set_paused(now, is_paused);

            // Let the stream deliver a frame it held back to stay within `fps` copies a second.
            stream.pump();
            // Drain to the newest available frame (CFR: we only keep the latest).
            loop {
                match rx.try_recv() {
                    Ok(StreamEvent::Frame(f)) => {
                        latest = f;
                        captured.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(StreamEvent::Closed) => {
                        ended_early = Some(SOURCE_CLOSED.into());
                        break;
                    }
                    Ok(StreamEvent::Error(e)) => {
                        ended_early = Some(e.to_string());
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        ended_early = Some("the capture stream went away".into());
                        break;
                    }
                }
            }
            if ended_early.is_some() {
                break;
            }

            if !is_paused {
                let target = clock.frames_due(now) as i64;
                let mut n = 0;
                while idx < target && n < max_catch_up {
                    sink.write_frame(&latest, idx)?;
                    idx += 1;
                    emitted += 1;
                    n += 1;
                }
            }

            // Audio: write while recording, discard while paused (keeps A and V aligned). The
            // track is timed by its own sample count, held to the video clock: a gap (every
            // device stalled, or one was unplugged) is filled with silence, and a device clock
            // running fast has its excess trimmed, so the two never drift apart audibly.
            if let Some(arx) = &audio_rx {
                let expected = if is_paused {
                    i64::MAX
                } else {
                    expected_audio_frames(clock.active(now))
                };
                while let Ok(chunk) = arx.try_recv() {
                    if is_paused {
                        continue;
                    }
                    let frames = (chunk.len() / AUDIO_CHANNELS) as i64;
                    let skip = audio_overrun(expected, audio_frames, frames);
                    if skip > 0 {
                        tracing::info!("audio ahead of video; trimmed {} ms", skip * 1000 / RATE);
                    }
                    let keep = &chunk[skip as usize * AUDIO_CHANNELS..];
                    write_audio(&sink, keep, &mut audio_frames);
                }
                if !is_paused {
                    let gap = audio_catch_up(expected, audio_frames);
                    if gap > 0 {
                        tracing::info!(
                            "audio behind video; filled {} ms of silence",
                            gap * 1000 / RATE
                        );
                        let mut left = gap;
                        while left > 0 {
                            let n = left.min(RATE);
                            write_audio(
                                &sink,
                                &vec![0i16; n as usize * AUDIO_CHANNELS],
                                &mut audio_frames,
                            );
                            left -= n;
                        }
                    }
                }
            }

            std::thread::sleep(tick);
        }
        Ok(())
    })();

    stream.stop();
    if let Some(eng) = audio_engine {
        eng.stop();
    }

    let cap = captured.load(Ordering::Relaxed);
    let dropped = cap.saturating_sub(emitted);
    let audio_secs = audio_frames as f64 / crate::mf::AUDIO_RATE as f64;
    tracing::info!(
        "recording finished: {emitted} frames emitted, {cap} captured (~{dropped} coalesced), \
         {w}x{h} @ {fps}fps, audio {audio_secs:.1}s{}",
        match &ended_early {
            Some(r) => format!(" (ended early: {r})"),
            None => String::new(),
        }
    );

    // Always finalize so the file is playable, even if a write errored mid-stream; then report
    // an early end *with the path*, so the caller can tell the user where the partial file is.
    let finalize = sink.finalize();
    if let Err(e) = result {
        ended_early.get_or_insert_with(|| format!("write failed: {e}"));
    }
    finalize?;
    match ended_early {
        Some(reason) => Err(RecorderError::EndedEarly {
            path: cfg.out_path,
            reason,
        }),
        None => Ok(cfg.out_path),
    }
}

/// How often the encoder loop polls the capture device's health.
const HEALTH_CHECK_EVERY: Duration = Duration::from_secs(1);
/// Reason text for a source that disappeared mid-recording.
pub(crate) const SOURCE_CLOSED: &str = "the captured window or display was closed";

/// Audio sample rate as the clock arithmetic below uses it.
const RATE: i64 = crate::mf::AUDIO_RATE as i64;
const AUDIO_CHANNELS: usize = crate::mf::AUDIO_CHANNELS as usize;
/// How far the audio track may drift from the video clock before it is corrected (200 ms):
/// beyond normal capture latency and scheduling jitter, well short of an audible lip-sync slip.
const AUDIO_SLACK: i64 = RATE / 5;
/// Lag left after filling a gap (50 ms), so audio still in flight lands on time, not early.
const AUDIO_MARGIN: i64 = RATE / 20;

/// Audio frames that belong in the track after `active` (unpaused) recording time.
fn expected_audio_frames(active: Duration) -> i64 {
    (active.as_nanos() * RATE as u128 / 1_000_000_000) as i64
}

/// Frames of silence to insert when the track has fallen more than [`AUDIO_SLACK`] behind.
fn audio_catch_up(expected: i64, written: i64) -> i64 {
    let lag = expected - written;
    if lag > AUDIO_SLACK {
        lag - AUDIO_MARGIN
    } else {
        0
    }
}

/// Frames to drop from the front of a `chunk`-frame chunk that would put the track more than
/// [`AUDIO_SLACK`] ahead of the video clock (a device clock running fast).
fn audio_overrun(expected: i64, written: i64, chunk: i64) -> i64 {
    let lead = written.saturating_add(chunk).saturating_sub(expected);
    if lead > AUDIO_SLACK {
        lead.min(chunk)
    } else {
        0
    }
}

/// Write PCM at the track's running sample position and advance it (also on a failed write,
/// so one bad sample can't shift everything after it).
fn write_audio(sink: &VideoSink, pcm: &[i16], written: &mut i64) {
    if pcm.is_empty() {
        return;
    }
    if let Err(e) = sink.write_audio(pcm, *written * 10_000_000 / RATE) {
        tracing::warn!("audio write failed: {e}");
    }
    *written += (pcm.len() / AUDIO_CHANNELS) as i64;
}

/// Block up to 3s for the first frame; map a close/timeout to an error.
fn wait_first(rx: &mpsc::Receiver<StreamEvent>) -> Result<Frame> {
    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(StreamEvent::Frame(f)) => Ok(f),
        Ok(StreamEvent::Closed) => Err(RecorderError::Capture(
            "capture item closed before any frame".into(),
        )),
        Ok(StreamEvent::Error(e)) => Err(RecorderError::Capture(e.to_string())),
        Err(_) => Err(RecorderError::Capture(
            "no frame arrived within 3s of starting capture".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_frames_follow_active_time() {
        assert_eq!(expected_audio_frames(Duration::ZERO), 0);
        assert_eq!(expected_audio_frames(Duration::from_millis(1500)), 72_000);
    }

    #[test]
    fn small_lag_is_left_alone_and_a_gap_is_filled_to_the_margin() {
        assert_eq!(audio_catch_up(48_000, 48_000), 0);
        assert_eq!(
            audio_catch_up(48_000, 48_000 - AUDIO_SLACK),
            0,
            "at the limit"
        );
        // Every device stalled for a second: fill all but the margin.
        assert_eq!(audio_catch_up(96_000, 48_000), 48_000 - AUDIO_MARGIN);
        // Ahead of the clock is never "behind".
        assert_eq!(audio_catch_up(0, 4_800), 0);
    }

    #[test]
    fn audio_running_ahead_is_trimmed_back_to_the_clock() {
        // Normal: a 10 ms chunk with the track exactly on time.
        assert_eq!(audio_overrun(48_000, 47_520, 480), 0);
        // Track already 250 ms ahead: trim the whole lead, capped at the chunk.
        assert_eq!(audio_overrun(48_000, 48_000 + 12_000 - 480, 480), 480);
        assert_eq!(audio_overrun(48_000, 48_000, 12_000), 12_000);
        // 220 ms ahead after a 10 ms chunk: trim the excess over the clock.
        assert_eq!(audio_overrun(48_000, 48_000 + 10_080, 480), 480);
        // Paused (expected = MAX) never trims.
        assert_eq!(audio_overrun(i64::MAX, 10, 480), 0);
    }
}
