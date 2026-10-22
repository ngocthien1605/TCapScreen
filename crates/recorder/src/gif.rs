//! GIF recording: capture frames via WGC, downscale to the configured max width, and encode a
//! looping GIF with the `image` crate. Bounded by the max-duration and max-fps caps so the
//! in-memory frame buffer stays small.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use capture_core::{CaptureStream, Frame, StreamEvent};
use imaging::Rgba;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::{RecordCfg, RecordOutput, RecorderError, Result};

#[derive(Debug)]
pub struct GifSession {
    paused: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<PathBuf>>>,
}

impl GifSession {
    pub fn start(cfg: RecordCfg) -> Result<GifSession> {
        let paused = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        let p = paused.clone();
        let s = stopping.clone();
        let thread = std::thread::Builder::new()
            .name("rustshot-gif".into())
            .spawn(move || gif_loop(cfg, p, s))
            .map_err(|e| RecorderError::Platform(format!("spawning gif thread: {e}")))?;
        Ok(GifSession {
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

    /// True once the GIF thread has exited on its own (source closed, device lost, length cap).
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    pub fn stop(mut self) -> Result<PathBuf> {
        self.stopping.store(true, Ordering::SeqCst);
        match self.thread.take() {
            Some(t) => t
                .join()
                .map_err(|_| RecorderError::Platform("gif thread panicked".into()))?,
            None => Err(RecorderError::Platform(
                "gif recording already stopped".into(),
            )),
        }
    }
}

fn cap_err(e: capture_core::CaptureError) -> RecorderError {
    RecorderError::Capture(e.to_string())
}

fn gif_loop(cfg: RecordCfg, paused: Arc<AtomicBool>, stopping: Arc<AtomicBool>) -> Result<PathBuf> {
    // SAFETY: MTA for WGC on this thread; tolerated if already initialised.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let (fps, cursor, max_width, max_seconds) = match cfg.output {
        RecordOutput::Gif {
            fps,
            cursor,
            max_width,
            max_seconds,
        } => (
            fps.clamp(1, 50),
            cursor,
            max_width.max(16),
            max_seconds.max(1),
        ),
        RecordOutput::Mp4 { .. } => return Err(RecorderError::NotImplemented),
    };

    let (tx, rx) = mpsc::channel::<StreamEvent>();
    let stream = CaptureStream::start(&cfg.target, cursor, tx).map_err(cap_err)?;

    let max_frames = (fps * max_seconds) as usize;
    let tick = Duration::from_nanos(1_000_000_000u64 / fps as u64);
    let mut frames: Vec<Rgba> = Vec::new();
    let mut latest: Option<Frame> = None;
    let mut ended_early: Option<String> = None;
    let mut last_health_check = Instant::now();
    let start = Instant::now();

    while !stopping.load(Ordering::SeqCst) && ended_early.is_none() && frames.len() < max_frames {
        let now = Instant::now();
        if now.saturating_duration_since(last_health_check) >= Duration::from_secs(1) {
            last_health_check = now;
            if let Err(e) = stream.check_health() {
                ended_early = Some(e.to_string());
                break;
            }
        }
        loop {
            match rx.try_recv() {
                Ok(StreamEvent::Frame(f)) => latest = Some(f),
                Ok(StreamEvent::Closed) => {
                    ended_early = Some(crate::pipeline::SOURCE_CLOSED.into());
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

        if !paused.load(Ordering::SeqCst) {
            if let Some(f) = &latest {
                if let Ok(rgba) = imaging::frame_to_rgba(f, true) {
                    frames.push(imaging::thumbnail(&rgba, max_width));
                }
            }
        }
        std::thread::sleep(tick);
    }

    stream.stop();

    tracing::info!(
        "gif capture finished: {} frames at {fps} fps ({:.1}s wall){}",
        frames.len(),
        start.elapsed().as_secs_f32(),
        match &ended_early {
            Some(r) => format!(" (ended early: {r})"),
            None => String::new(),
        }
    );

    if frames.is_empty() {
        return Err(RecorderError::Capture(match ended_early {
            Some(r) => format!("no frames captured for GIF ({r})"),
            None => "no frames captured for GIF".into(),
        }));
    }
    imaging::save_gif(&frames, fps, &cfg.out_path)
        .map_err(|e| RecorderError::Platform(format!("gif encode: {e}")))?;
    match ended_early {
        Some(reason) => Err(RecorderError::EndedEarly {
            path: cfg.out_path,
            reason,
        }),
        None => Ok(cfg.out_path),
    }
}
