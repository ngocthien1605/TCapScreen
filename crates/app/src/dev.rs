//! Developer/diagnostic CLI commands. These never start the tray; they let GPU/display paths
//! be exercised on a real desktop and let config/paths be inspected on any machine (incl. CI).

use std::path::PathBuf;
use std::process::ExitCode;

use capture_core::{
    bounding_box, CaptureBackend, CaptureTarget, GrabOpts, Point, Rect, WindowsBackend,
};
use clap::Subcommand;
use config::Settings;

#[derive(Debug, Subcommand)]
pub enum DevCmd {
    /// Load the config (repairing out-of-range values), print it, and show its file path.
    Config,
    /// Print all resolved RustShot paths.
    Paths,
    /// List connected monitors.
    ListMonitors,
    /// List capturable top-level windows.
    ListWindows,
    /// Grab a full monitor to a PNG file.
    GrabMonitor {
        /// Monitor index as reported by `list-monitors`.
        index: usize,
        /// Output PNG path.
        out: PathBuf,
    },
    /// Grab the first window whose title contains `query` (case-insensitive) to a file.
    GrabWindow {
        /// Case-insensitive substring of the window title.
        query: String,
        /// Output image path (extension picks PNG/JPEG).
        out: PathBuf,
    },
    /// Grab a region of the virtual desktop (physical px, virtual-desktop coords) to a file.
    GrabRegion {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        out: PathBuf,
    },
    /// Grab the whole virtual desktop (all monitors) to a file.
    GrabAll {
        /// Output image path.
        out: PathBuf,
    },
    /// Load an image file and copy it to the clipboard (round-trip check).
    ClipboardImage {
        /// Image file to copy.
        file: PathBuf,
    },
}

pub fn run(cmd: DevCmd) -> ExitCode {
    match cmd {
        DevCmd::Config => cmd_config(),
        DevCmd::Paths => cmd_paths(),
        DevCmd::ListMonitors => cmd_list_monitors(),
        DevCmd::ListWindows => cmd_list_windows(),
        DevCmd::GrabMonitor { index, out } => cmd_grab_monitor(index, out),
        DevCmd::GrabWindow { query, out } => cmd_grab_window(&query, out),
        DevCmd::GrabRegion {
            x,
            y,
            width,
            height,
            out,
        } => cmd_grab_region(Rect::new(x, y, width, height), out),
        DevCmd::GrabAll { out } => cmd_grab_all(out),
        DevCmd::ClipboardImage { file } => cmd_clipboard_image(file),
    }
}

fn save_frame(frame: &capture_core::Frame, out: &std::path::Path) -> ExitCode {
    println!(
        "captured frame: {}x{}  stride={} (padding={})",
        frame.width,
        frame.height,
        frame.stride,
        frame.stride.saturating_sub(frame.width as usize * 4),
    );
    let img = match imaging::frame_to_rgba(frame, true) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("conversion failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    if imaging::is_near_black(&img, 8, 16) {
        eprintln!("warning: capture is almost entirely black (protected/DRM content?)");
    }
    match imaging::save_image(&img, out, 90) {
        Ok(()) => {
            println!(
                "saved {}x{} to {}",
                img.width(),
                img.height(),
                out.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("save failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_list_windows() -> ExitCode {
    let backend = match WindowsBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    match backend.windows() {
        Ok(wins) => {
            println!("{} capturable window(s):", wins.len());
            for (i, w) in wins.iter().enumerate() {
                println!(
                    "  [{i}] {}x{} @ ({},{})  {}",
                    w.bounds.width, w.bounds.height, w.bounds.x, w.bounds.y, w.title
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error enumerating windows: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_grab_window(query: &str, out: PathBuf) -> ExitCode {
    let backend = match WindowsBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let wins = match backend.windows() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("error enumerating windows: {e}");
            return ExitCode::FAILURE;
        }
    };
    let needle = query.to_lowercase();
    let Some(win) = wins
        .iter()
        .find(|w| w.title.to_lowercase().contains(&needle))
    else {
        eprintln!("no window title contains {query:?}");
        return ExitCode::FAILURE;
    };
    println!("matched window: {}", win.title);
    match backend.grab(
        &CaptureTarget::Window(win.id),
        GrabOpts {
            include_cursor: false,
        },
    ) {
        Ok(f) => save_frame(&f, &out),
        Err(e) => {
            eprintln!("capture failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_grab_region(rect: Rect, out: PathBuf) -> ExitCode {
    let backend = match WindowsBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Grab the whole virtual desktop, then crop — exercises the same path the overlay uses.
    let mons = match backend.monitors() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error enumerating monitors: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bbox = bounding_box(&mons.iter().map(|m| m.bounds).collect::<Vec<_>>());
    let frame = match backend.grab(
        &CaptureTarget::VirtualDesktop,
        GrabOpts {
            include_cursor: false,
        },
    ) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("capture failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let local = rect.to_local(Point::new(bbox.x, bbox.y));
    save_frame(&frame.crop(local), &out)
}

fn cmd_grab_all(out: PathBuf) -> ExitCode {
    let backend = match WindowsBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    match backend.grab(
        &CaptureTarget::VirtualDesktop,
        GrabOpts {
            include_cursor: false,
        },
    ) {
        Ok(f) => save_frame(&f, &out),
        Err(e) => {
            eprintln!("capture failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_clipboard_image(file: PathBuf) -> ExitCode {
    // STA is friendliest for the clipboard; the backend's apartment choice doesn't matter here.
    let img = match imaging::load_rgba(&file) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("could not load {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    match crate::clipboard::copy_image(&img) {
        Ok(()) => {
            println!(
                "copied {}x{} image from {} to the clipboard",
                img.width(),
                img.height(),
                file.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("clipboard copy failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_list_monitors() -> ExitCode {
    let backend = match WindowsBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    match backend.monitors() {
        Ok(mons) => {
            println!("{} monitor(s):", mons.len());
            for (i, m) in mons.iter().enumerate() {
                println!(
                    "  [{i}] {:<14} {}x{} @ ({},{})  scale {:.0}%{}",
                    m.name,
                    m.bounds.width,
                    m.bounds.height,
                    m.bounds.x,
                    m.bounds.y,
                    m.scale_factor * 100.0,
                    if m.is_primary { "  (primary)" } else { "" }
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error enumerating monitors: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_grab_monitor(index: usize, out: PathBuf) -> ExitCode {
    let backend = match WindowsBackend::new() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mons = match backend.monitors() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error enumerating monitors: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Some(mon) = mons.get(index) else {
        eprintln!("no monitor at index {index} (found {})", mons.len());
        return ExitCode::FAILURE;
    };

    let frame = match backend.grab(
        &CaptureTarget::Monitor(mon.id),
        GrabOpts {
            include_cursor: false,
        },
    ) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("capture failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "captured frame: {}x{}  stride={} (width*4={}, padding={})",
        frame.width,
        frame.height,
        frame.stride,
        frame.width as usize * 4,
        frame.stride.saturating_sub(frame.width as usize * 4),
    );

    let img = match imaging::frame_to_rgba(&frame, true) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("conversion failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    if imaging::is_near_black(&img, 8, 16) {
        eprintln!("warning: capture is almost entirely black (protected/DRM content?)");
    }
    match imaging::save_image(&img, &out, 90) {
        Ok(()) => {
            println!(
                "saved {}x{} to {}",
                img.width(),
                img.height(),
                out.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("save failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_config() -> ExitCode {
    let path = match config::paths::config_file() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut settings = match Settings::load_from(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error loading config: {e}");
            return ExitCode::FAILURE;
        }
    };
    let repaired = settings.sanitize();
    match settings.to_toml() {
        Ok(toml_text) => {
            println!("# config file: {}", path.display());
            println!("# exists: {}", path.exists());
            if repaired {
                println!("# (note: some out-of-range values were repaired in memory)");
            }
            match settings.effective_output_folder() {
                Ok(folder) => println!("# effective output folder: {}", folder.display()),
                Err(e) => println!("# effective output folder: <error: {e}>"),
            }
            println!("# default hotkeys:");
            for (name, hk) in settings.hotkeys.entries() {
                println!("#   {name:<24} {hk}");
            }
            let conflicts = settings.hotkeys.conflicts();
            if conflicts.is_empty() {
                println!("# hotkey conflicts: none");
            } else {
                println!("# hotkey conflicts: {conflicts:?}");
            }
            println!("\n{toml_text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error serializing config: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_paths() -> ExitCode {
    macro_rules! show {
        ($label:expr, $expr:expr) => {
            match $expr {
                Ok(p) => println!("{:<22} {}", $label, p.display()),
                Err(e) => println!("{:<22} <error: {e}>", $label),
            }
        };
    }
    show!("app_dir", config::paths::app_dir());
    show!("config_file", config::paths::config_file());
    show!("logs_dir", config::paths::logs_dir());
    show!("default_output", config::paths::default_output_folder());
    ExitCode::SUCCESS
}
