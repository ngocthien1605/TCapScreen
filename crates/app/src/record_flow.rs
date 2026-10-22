//! Recording orchestration: select a region/window (same overlay as screenshots) → optional
//! countdown → start the recorder and show the toolbar → pause/resume → stop and finalize.
//!
//! Like the screenshot flow, no `AppState` borrow is held across the overlay's nested loop or
//! across recorder start; data is copied out, acted on, then stashed back.

use std::ffi::c_void;
use std::time::Duration;

use anyhow::{anyhow, Result};
use capture_core::{bounding_box, CaptureBackend, CaptureTarget, GrabOpts, Point, WindowsBackend};
use recorder::{AudioCfg, RecordCfg, RecordOutput, Recorder};
use windows::Win32::Foundation::HWND;

use crate::dispatch::{self, Action};
use crate::overlay::{self, Outcome};
use crate::recording::{Recording, Toolbar};

/// Begin a recording: pick a target, queue it behind the countdown, then start. `gif` selects
/// animated-GIF output instead of MP4.
pub fn begin(gif: bool) {
    let already = dispatch::with_app(|a| a.recording.is_some() || a.busy).unwrap_or(true);
    if already {
        tracing::info!("a capture or recording is already in progress; ignoring");
        return;
    }

    let target = match select_target() {
        Ok(Some(t)) => t,
        Ok(None) => {
            tracing::info!("recording cancelled");
            return;
        }
        Err(e) => {
            tracing::error!("could not select a recording target: {e:#}");
            return;
        }
    };

    let Some((settings, out_dir, hwnd_raw)) = dispatch::with_app(|a| {
        (
            a.settings.clone(),
            a.output_folder.clone(),
            a.dispatch_hwnd.0 as isize,
        )
    }) else {
        return;
    };

    let ext = if gif { "gif" } else { "mp4" };
    let name = imaging::auto_name(imaging::Timestamp::now_local(), ext);
    let path = imaging::unique_path(&out_dir, &name);
    let output = if gif {
        RecordOutput::Gif {
            fps: settings.recording.gif_fps,
            cursor: settings.recording.include_cursor,
            max_width: settings.recording.gif_max_width,
            max_seconds: settings.recording.gif_max_seconds,
        }
    } else {
        RecordOutput::Mp4 {
            fps: settings.recording.fps,
            bitrate: settings.recording.bitrate,
            audio: AudioCfg {
                microphone: settings.recording.capture_microphone,
                system_audio: settings.recording.capture_system_audio,
                microphone_device: settings.recording.microphone_device.clone(),
            },
            cursor: settings.recording.include_cursor,
        }
    };
    let cfg = RecordCfg {
        target,
        output,
        out_path: path,
    };

    dispatch::with_app(|a| a.pending_record = Some(cfg));

    let countdown = settings.recording.countdown_seconds;
    if countdown == 0 {
        execute_start();
    } else {
        tracing::info!("recording starts in {countdown}s…");
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(countdown as u64));
            // HWND isn't Send; rebuild it from the raw pointer on this thread.
            dispatch::post(HWND(hwnd_raw as *mut c_void), Action::ExecuteRecordStart);
        });
    }
}

/// Start the queued recording (immediately, or when the countdown timer fires).
pub fn execute_start() {
    let Some(cfg) = dispatch::with_app(|a| a.pending_record.take()).flatten() else {
        return;
    };
    let hwnd_raw = dispatch::with_app(|a| a.dispatch_hwnd.0 as isize).unwrap_or(0);
    let hwnd = HWND(hwnd_raw as *mut c_void);

    let recorder = Recorder::new();
    match recorder.start(cfg) {
        Ok(handle) => match Toolbar::show(hwnd) {
            Ok(toolbar) => {
                dispatch::with_app(|a| a.recording = Some(Recording { handle, toolbar }));
                tracing::info!("recording started");
            }
            Err(e) => {
                tracing::error!("could not show recording toolbar: {e:#}; stopping");
                let _ = handle.stop();
            }
        },
        Err(e) => tracing::error!("could not start recording: {e:#}"),
    }
}

/// Toggle pause/resume on the active recording.
pub fn toggle_pause() {
    dispatch::with_app(|a| {
        if let Some(rec) = a.recording.as_mut() {
            let pause = !rec.handle.is_paused();
            let _ = if pause {
                rec.handle.pause()
            } else {
                rec.handle.resume()
            };
            rec.toolbar.set_paused(pause);
            tracing::info!("recording {}", if pause { "paused" } else { "resumed" });
        }
    });
}

/// Stop and finalize the active recording.
pub fn stop() {
    let Some(rec) = dispatch::with_app(|a| a.recording.take()).flatten() else {
        tracing::info!("no active recording to stop");
        return;
    };
    // Destroy the toolbar first (drops its window), then finalize the file.
    let Recording { handle, toolbar } = rec;
    drop(toolbar);
    match handle.stop() {
        Ok(path) => tracing::info!("recording saved to {}", path.display()),
        Err(e) => tracing::error!("stopping recording failed: {e:#}"),
    }
}

/// Freeze the desktop and run the overlay to choose a region or a window to record.
fn select_target() -> Result<Option<CaptureTarget>> {
    let backend = WindowsBackend::new()?;
    let mons = backend.monitors()?;
    let bbox = bounding_box(&mons.iter().map(|m| m.bounds).collect::<Vec<_>>());
    if bbox.is_empty() {
        return Err(anyhow!("no monitors available"));
    }
    let windows = backend.windows().unwrap_or_default();
    let frozen = backend.grab(
        &CaptureTarget::VirtualDesktop,
        GrabOpts {
            include_cursor: false,
        },
    )?;

    match overlay::select(&windows, &frozen, bbox, overlay::Style::Crosshair)? {
        Outcome::Cancel => Ok(None),
        Outcome::Window(id) => Ok(Some(CaptureTarget::Window(id))),
        Outcome::Region(rect) => {
            let center = Point::new(rect.x + rect.width / 2, rect.y + rect.height / 2);
            let monitor = mons
                .iter()
                .find(|m| m.bounds.contains(center))
                .or_else(|| mons.iter().find(|m| m.is_primary))
                .or_else(|| mons.first())
                .map(|m| m.id);
            match monitor {
                Some(monitor) => Ok(Some(CaptureTarget::Region { rect, monitor })),
                None => Ok(None),
            }
        }
    }
}
