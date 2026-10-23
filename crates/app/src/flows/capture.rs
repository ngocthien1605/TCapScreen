//! Screenshot orchestration: delay timer → freeze/overlay (region & window) or direct grab
//! (monitor & all-monitors) → encode → save (auto-named, non-colliding) → clipboard → flyout.
//!
//! The flow never holds the app-state borrow across the (nested-modal) overlay loop: it clones
//! the few needed settings out, runs the capture with a fresh backend, then re-enters the state
//! only to stash the flyout and clear the busy flag. That keeps re-entrant actions (other
//! hotkeys, menu clicks) safe while an overlay is open — the `busy` flag drops duplicate shots.

use std::ffi::c_void;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Result};
use capture_core::{
    bounding_box, CaptureBackend, CaptureTarget, Frame, GrabOpts, MonitorId, Point, WindowsBackend,
};
use config::Settings;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::dispatch::{self, Action};
use crate::ui::flyout::Flyout;
use crate::ui::overlay::{self, Outcome};

/// Which screenshot the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotKind {
    /// Interactive: drag a region (or click a window) on a frozen desktop.
    Region,
    /// Interactive, same overlay but intent is to click a window.
    Window,
    /// The monitor currently under the cursor.
    Monitor,
    /// The whole virtual desktop (all monitors).
    AllMonitors,
}

/// Entry point for a screenshot action. Applies the configured delay, then captures.
pub fn begin(kind: ShotKind) {
    let delay = dispatch::with_app(|app| {
        if app.busy {
            return None;
        }
        app.busy = true;
        app.flyout = None; // a stale flyout must not appear in an interactive shot
        app.pending_shot = Some(kind);
        Some(app.settings.screenshot.delay_seconds)
    })
    .flatten();

    let Some(delay) = delay else {
        tracing::info!("capture already in progress; ignoring trigger");
        return;
    };

    if delay == 0 {
        execute_pending();
    } else {
        schedule_delay(delay);
    }
}

/// Run the capture queued by [`begin`] (called immediately, or by the delay timer).
pub fn execute_pending() {
    let inputs = dispatch::with_app(|app| {
        let kind = app.pending_shot.take()?;
        Some((
            kind,
            app.settings.clone(),
            app.output_folder.clone(),
            app.dispatch_hwnd.0 as isize,
        ))
    })
    .flatten();

    let Some((kind, settings, out_dir, hwnd_raw)) = inputs else {
        clear_busy();
        return;
    };
    // SAFETY: the dispatch window outlives any in-flight capture.
    let hwnd = HWND(hwnd_raw as *mut c_void);

    match run_capture(kind, &settings, &out_dir, hwnd) {
        Ok(Some(flyout)) => {
            dispatch::with_app(|app| app.flyout = Some(flyout));
        }
        Ok(None) => tracing::info!("capture cancelled"),
        // Notify *before* releasing the gate: the box pumps messages, and a hotkey pressed
        // while it is up must not start a second capture nested inside it.
        Err(e) => crate::notify::error("Screenshot failed.", e),
    }
    clear_busy();
}

fn clear_busy() {
    dispatch::with_app(|app| app.busy = false);
}

fn schedule_delay(delay: u8) {
    let Some(hwnd_raw) = dispatch::with_app(|app| app.dispatch_hwnd.0 as isize) else {
        return;
    };
    tracing::info!("capturing in {delay}s…");
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(delay as u64));
        // SAFETY: PostMessageW is thread-safe; the handle refers to the live dispatch window.
        dispatch::post(HWND(hwnd_raw as *mut c_void), Action::ExecuteDelayedShot);
    });
}

fn run_capture(
    kind: ShotKind,
    settings: &Settings,
    out_dir: &Path,
    hwnd: HWND,
) -> Result<Option<Flyout>> {
    let backend = WindowsBackend::new()?;
    let opts = GrabOpts {
        include_cursor: settings.screenshot.include_cursor,
    };

    let frame = match kind {
        ShotKind::Monitor => {
            let id = monitor_under_cursor(&backend)?;
            backend.grab(&CaptureTarget::Monitor(id), opts)?
        }
        ShotKind::AllMonitors => backend.grab(&CaptureTarget::VirtualDesktop, opts)?,
        ShotKind::Region | ShotKind::Window => match interactive(&backend, opts)? {
            Some(f) => f,
            None => return Ok(None),
        },
    };

    let img = imaging::frame_to_rgba(&frame, true)?;
    if imaging::is_near_black(&img, 8, 16) {
        tracing::warn!("capture is almost entirely black (protected/DRM content?)");
    }

    let ext = settings.screenshot.format.extension();
    let name = imaging::auto_name(imaging::Timestamp::now_local(), ext);
    let path = imaging::save_image_unique(&img, out_dir, &name, settings.screenshot.jpeg_quality)?;
    tracing::info!("saved screenshot to {}", path.display());

    if settings.screenshot.copy_to_clipboard {
        if let Err(e) = crate::clipboard::copy_image(&img) {
            // The file is saved; only the clipboard half failed (typically another app holding
            // the clipboard open). The gate is still held here.
            crate::notify::warn(
                "The screenshot was saved but could not be copied to the clipboard.",
                e,
            );
        }
    }

    let flyout = Flyout::show(hwnd, &path, &img)?;
    Ok(Some(flyout))
}

/// Freeze the virtual desktop and run the selection overlay; resolve the outcome into a frame.
fn interactive(backend: &WindowsBackend, opts: GrabOpts) -> Result<Option<Frame>> {
    let mons = backend.monitors()?;
    let bbox = bounding_box(&mons.iter().map(|m| m.bounds).collect::<Vec<_>>());
    if bbox.is_empty() {
        return Err(anyhow!("no monitors available for capture"));
    }
    let windows = backend.windows().unwrap_or_default();
    let frozen = backend.grab(&CaptureTarget::VirtualDesktop, opts)?;

    let origin = Point::new(bbox.x, bbox.y);
    match overlay::select(&windows, &frozen, bbox)? {
        Outcome::Cancel => Ok(None),
        Outcome::Region(rect) => Ok(Some(frozen.crop(rect.to_local(origin)))),
        Outcome::Window(id) => {
            // Prefer a clean, unoccluded recapture of the window itself.
            match backend.grab(&CaptureTarget::Window(id), opts) {
                Ok(f) => Ok(Some(f)),
                Err(e) => {
                    tracing::warn!("window recapture failed ({e}); cropping frozen frame");
                    match windows.iter().find(|w| w.id == id) {
                        Some(w) => Ok(Some(frozen.crop(w.bounds.to_local(origin)))),
                        None => Ok(None),
                    }
                }
            }
        }
    }
}

fn monitor_under_cursor(backend: &WindowsBackend) -> Result<MonitorId> {
    let mons = backend.monitors()?;
    let mut pt = POINT::default();
    // SAFETY: writes the cursor position into `pt`.
    let _ = unsafe { GetCursorPos(&mut pt) };
    let p = Point::new(pt.x, pt.y);
    mons.iter()
        .find(|m| m.bounds.contains(p))
        .or_else(|| mons.iter().find(|m| m.is_primary))
        .or_else(|| mons.first())
        .map(|m| m.id)
        .ok_or_else(|| anyhow!("no monitors available for capture"))
}
