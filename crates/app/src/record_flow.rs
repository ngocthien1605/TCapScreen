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
    // Reserve the `busy` gate in one check-and-set, so nothing else (a screenshot hotkey, a
    // second Record, Settings) can open an overlay nested inside ours or capture the dimmed
    // overlay itself. It stays reserved through target selection *and* the countdown, and is
    // released on every exit path below or by `execute_start`.
    let reserved = dispatch::with_app(|a| {
        if a.recording.is_some() || a.busy {
            false
        } else {
            a.busy = true;
            true
        }
    })
    .unwrap_or(false);
    if !reserved {
        tracing::info!("a capture or recording is already in progress; ignoring");
        return;
    }

    let target = match select_target() {
        Ok(Some(t)) => t,
        Ok(None) => {
            tracing::info!("recording cancelled");
            clear_busy();
            return;
        }
        Err(e) => {
            // Notify while the gate is still held (the box pumps messages).
            crate::notify::error("Recording could not start.", e);
            clear_busy();
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
        clear_busy();
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

/// Start the queued recording (immediately, or when the countdown timer fires). Whatever
/// happens, the `busy` gate reserved by [`begin`] is released: from here on an in-progress
/// recording is tracked by `AppState::recording`, not by `busy`.
pub fn execute_start() {
    let Some(cfg) = dispatch::with_app(|a| a.pending_record.take()).flatten() else {
        // The countdown was cancelled (or there never was a request).
        clear_busy();
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
                let _ = handle.stop();
                crate::notify::error("Recording stopped: its control bar could not be shown.", e);
            }
        },
        // Still inside the gate reserved by `begin`, so the box cannot be re-entered by a hotkey.
        Err(e) => crate::notify::error("Recording could not start.", e),
    }
    clear_busy();
}

fn clear_busy() {
    dispatch::with_app(|a| a.busy = false);
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

/// Stop and finalize the active recording. A recording still in its countdown is cancelled
/// instead (the countdown timer then finds nothing queued and releases the gate).
pub fn stop() {
    let cancelled = dispatch::with_app(|a| a.pending_record.take().is_some()).unwrap_or(false);
    if cancelled {
        tracing::info!("recording countdown cancelled");
        clear_busy();
    }
    let Some(rec) = dispatch::with_app(|a| a.recording.take()).flatten() else {
        if !cancelled {
            tracing::debug!("no active recording to stop");
        }
        return;
    };
    // Destroy the toolbar first (drops its window), then finalize the file.
    let Recording { handle, toolbar } = rec;
    drop(toolbar);
    let (result, notes) = handle.finish();
    let sound = sound_notes(&notes);
    match result {
        Ok(path) if notes.is_empty() => tracing::info!("recording saved to {}", path.display()),
        // Saved, but without the sound the user asked for (a missing or busy microphone, a
        // device that stopped mid-way): say so now rather than let them find a silent file.
        Ok(path) => crate::notify::warn(
            "The recording was saved, but its sound was not recorded as set up.",
            format!("{sound}\n\nThe file is saved at:\n{}", path.display()),
        ),
        // The file exists and is playable; the user only needs to know it is shorter than
        // they expected and why (a closed window is informational, not a failure).
        Err(recorder::RecorderError::EndedEarly { path, reason }) => crate::notify::warn(
            "The recording ended early.",
            format!(
                "{}.{}\n\nWhat was captured up to that point is saved at:\n{}",
                capitalize(&reason),
                if sound.is_empty() {
                    String::new()
                } else {
                    format!("\n\n{sound}")
                },
                path.display()
            ),
        ),
        // No gate is held here (the recording was tracked by `recording`, not `busy`), so a
        // hotkey pressed while this box is up can start a new capture inside its message pump.
        // That is harmless — the failed recording is already torn down — and accepted for now.
        Err(e) => crate::notify::error("The recording could not be saved.", e),
    }
}

/// Toolbar tick: if the recording ended on its own (the recorder thread exited because the
/// source closed, the graphics device was lost, or a write failed), collect it now — the file
/// is already finalized, and the toolbar must not keep counting over it.
pub fn poll() {
    let finished = dispatch::with_app(|a| a.recording.as_ref().map(|r| r.handle.is_finished()))
        .flatten()
        .unwrap_or(false);
    if finished {
        tracing::info!("recording ended on its own; collecting it");
        stop();
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

    match overlay::select(&windows, &frozen, bbox)? {
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

/// The recorder's notes as a bulleted list, one sentence each (empty when there are none).
fn sound_notes(notes: &[String]) -> String {
    notes
        .iter()
        .map(|n| format!("• {}.", capitalize(n)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_become_capitalized_bullets() {
        assert_eq!(sound_notes(&[]), "");
        let notes = vec![
            "the microphone could not be recorded (busy)".to_string(),
            "system audio could not be recorded".to_string(),
        ];
        assert_eq!(
            sound_notes(&notes),
            "• The microphone could not be recorded (busy).\n• System audio could not be recorded."
        );
        assert_eq!(capitalize("éclair"), "Éclair");
    }
}
