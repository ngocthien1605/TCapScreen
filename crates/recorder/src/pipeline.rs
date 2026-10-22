//! The recording pipeline (Windows): a dedicated encoder thread owns the Media Foundation sink
//! and the WGC capture stream, re-emitting the latest captured frame at a constant rate.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use capture_core::{CaptureStream, Frame, StreamEvent};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::mf::VideoSink;
use crate::{RecordCfg, RecordOutput, RecorderError, Result};

/// Controls and the join handle for a running encoder thread.
#[derive(Debug)]
pub struct Session {
    paused: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<std::path::PathBuf>>>,
}

impl Session {
    pub fn start(cfg: RecordCfg) -> Result<Session> {
        let paused = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        let p = paused.clone();
        let s = stopping.clone();
        let thread = std::thread::Builder::new()
            .name("rustshot-encoder".into())
            .spawn(move || encode_loop(cfg, p, s))
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

    // Optional audio capture. A dev/test env var injects a sine tone into the render keep-alive
    // so loopback has a known signal to prove the path end-to-end.
    let want_audio = audio_cfg.microphone || audio_cfg.system_audio;
    let tone_hz = std::env::var("RUSTSHOT_TEST_TONE_HZ")
        .ok()
        .and_then(|s| s.parse::<f32>().ok());
    let (audio_engine, audio_rx, audio_available) = if want_audio {
        let (eng, rx, avail) =
            crate::audio::AudioEngine::start(audio_cfg.system_audio, audio_cfg.microphone, tone_hz);
        (Some(eng), Some(rx), avail)
    } else {
        (None, None, false)
    };

    let (tx, rx) = mpsc::channel::<StreamEvent>();
    let stream = CaptureStream::start(&cfg.target, cursor, tx).map_err(cap_err)?;

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

    let path_str = cfg.out_path.to_string_lossy().to_string();
    let sink = match VideoSink::new(&path_str, w, h, fps, bitrate, audio_available) {
        Ok(s) => s,
        Err(e) => {
            stream.stop();
            return Err(e);
        }
    };
    let frame_dur = sink.frame_duration();
    let tick = Duration::from_nanos((frame_dur as u64) * 100);

    let mut latest = first;
    let mut idx: i64 = 0;
    let captured = AtomicU64::new(1);
    let mut emitted: u64 = 0;
    let mut closed = false;
    // Audio uses a sample-count clock (can't drift against the video index clock).
    let mut audio_frames: i64 = 0;

    // CFR driven by the monotonic clock: the number of frames that should exist by now is
    // `active_elapsed / frame_dur`. We re-emit the latest frame to catch up, so the output's
    // duration tracks real (unpaused) wall-clock time rather than drifting with sleep latency.
    let start = Instant::now();
    let mut paused_accum = Duration::ZERO;
    let mut pause_started: Option<Instant> = None;
    let mut was_paused = false;
    let tick_nanos = tick.as_nanos().max(1);
    let max_catch_up = (fps as i64).max(1) * 2;

    // Drop audio captured during video startup so both timelines begin at t = 0 (A/V sync).
    if let Some(arx) = &audio_rx {
        while arx.try_recv().is_ok() {}
    }

    let result = (|| -> Result<()> {
        while !stopping.load(Ordering::SeqCst) && !closed {
            let now = Instant::now();
            let is_paused = paused.load(Ordering::SeqCst);
            // Accumulate whole pause spans at their edges so `active` time is exact.
            if is_paused && !was_paused {
                pause_started = Some(now);
            } else if !is_paused && was_paused {
                if let Some(ps) = pause_started.take() {
                    paused_accum += now.saturating_duration_since(ps);
                }
            }
            was_paused = is_paused;

            // Drain to the newest available frame (CFR: we only keep the latest).
            loop {
                match rx.try_recv() {
                    Ok(StreamEvent::Frame(f)) => {
                        latest = f;
                        captured.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(StreamEvent::Closed) => {
                        closed = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        closed = true;
                        break;
                    }
                }
            }

            if !is_paused {
                let active = now
                    .saturating_duration_since(start)
                    .saturating_sub(paused_accum);
                let target = (active.as_nanos() / tick_nanos) as i64 + 1;
                let mut n = 0;
                while idx < target && n < max_catch_up {
                    sink.write_frame(&latest, idx * frame_dur)?;
                    idx += 1;
                    emitted += 1;
                    n += 1;
                }
            }

            // Audio: write while recording, discard while paused (keeps A and V aligned).
            if let Some(arx) = &audio_rx {
                while let Ok(chunk) = arx.try_recv() {
                    if is_paused {
                        continue;
                    }
                    let ts = audio_frames * 10_000_000 / crate::mf::AUDIO_RATE as i64;
                    if let Err(e) = sink.write_audio(&chunk, ts) {
                        tracing::warn!("audio write failed: {e}");
                    }
                    audio_frames += (chunk.len() / crate::mf::AUDIO_CHANNELS as usize) as i64;
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
        if closed { " (source closed)" } else { "" }
    );

    // Always finalize so the file is playable, even if a write errored mid-stream.
    let finalize = sink.finalize();
    result?;
    finalize?;
    Ok(cfg.out_path)
}

/// Block up to 3s for the first frame; map a close/timeout to an error.
fn wait_first(rx: &mpsc::Receiver<StreamEvent>) -> Result<Frame> {
    match rx.recv_timeout(Duration::from_secs(3)) {
        Ok(StreamEvent::Frame(f)) => Ok(f),
        Ok(StreamEvent::Closed) => Err(RecorderError::Capture(
            "capture item closed before any frame".into(),
        )),
        Err(_) => Err(RecorderError::Capture(
            "no frame arrived within 3s of starting capture".into(),
        )),
    }
}
