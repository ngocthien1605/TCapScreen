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

use crate::clock::FrameClock;
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
    let stream = CaptureStream::start(&cfg.target, cursor, fps, tx).map_err(cap_err)?;

    // Frame slots at `fps` over unpaused time, capped at the length limit. Each stored image
    // carries how many consecutive slots it fills: a frame that is still current at the next
    // tick (static screen, or conversion running slower than `fps`) is extended rather than
    // converted again, so playback length matches the time recorded.
    let max_slots = fps as u64 * max_seconds as u64;
    let tick = Duration::from_nanos(1_000_000_000u64 / fps as u64);
    let mut frames: Vec<(Rgba, u64)> = Vec::new();
    let mut slots: u64 = 0;
    let mut latest: Option<Frame> = None;
    let mut fresh = false;
    let mut ended_early: Option<String> = None;
    let mut last_health_check = Instant::now();
    let start = Instant::now();
    let mut clock = FrameClock::new(fps, start);

    while !stopping.load(Ordering::SeqCst) && ended_early.is_none() && slots < max_slots {
        let now = Instant::now();
        let is_paused = paused.load(Ordering::SeqCst);
        clock.set_paused(now, is_paused);
        if now.saturating_duration_since(last_health_check) >= Duration::from_secs(1) {
            last_health_check = now;
            if let Err(e) = stream.check_health() {
                ended_early = Some(e.to_string());
                break;
            }
        }
        // Let the stream deliver a frame it held back to stay within `fps` copies a second.
        stream.pump();
        loop {
            match rx.try_recv() {
                Ok(StreamEvent::Frame(f)) => {
                    latest = Some(f);
                    fresh = true;
                }
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

        if !is_paused {
            if let Some(f) = &latest {
                let due = clock.frames_due(now).min(max_slots);
                while slots < due {
                    match frames.last_mut() {
                        Some((_, n)) if !fresh => *n += 1,
                        _ => match imaging::frame_to_rgba(f, true) {
                            Ok(rgba) => {
                                frames.push((imaging::thumbnail(&rgba, max_width), 1));
                                fresh = false;
                            }
                            Err(e) => {
                                tracing::warn!("gif frame conversion failed: {e}");
                                break;
                            }
                        },
                    }
                    slots += 1;
                }
            }
        }
        std::thread::sleep(tick);
    }

    // Hitting the length cap ends the recording on its own; say so rather than just stopping.
    if ended_early.is_none() && slots >= max_slots && !stopping.load(Ordering::SeqCst) {
        ended_early = Some(format!(
            "it reached the {max_seconds}-second limit for GIF recordings (gif_max_seconds)"
        ));
    }
    stream.stop();

    tracing::info!(
        "gif capture finished: {} distinct frames over {slots} slots at {fps} fps ({:.1}s wall){}",
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
    let timed: Vec<(Rgba, Duration)> = frames
        .into_iter()
        .map(|(img, n)| (img, Duration::from_nanos(n * 1_000_000_000 / fps as u64)))
        .collect();
    imaging::save_gif(&timed, &cfg.out_path).map_err(|e| match &e {
        imaging::ImagingError::Io { source, .. }
            if source
                .raw_os_error()
                .is_some_and(|c| crate::is_disk_full_code(c as u32)) =>
        {
            RecorderError::DiskFull
        }
        _ => RecorderError::Platform(format!("gif encode: {e}")),
    })?;
    match ended_early {
        Some(reason) => Err(RecorderError::EndedEarly {
            path: cfg.out_path,
            reason,
        }),
        None => Ok(cfg.out_path),
    }
}
