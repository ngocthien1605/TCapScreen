//! Developer/diagnostic CLI commands. These never start the tray; they let GPU/display paths
//! be exercised on a real desktop and let config/paths be inspected on any machine (incl. CI).

use std::path::PathBuf;
use std::process::ExitCode;

use capture_core::{CaptureBackend, CaptureTarget, GrabOpts, WindowsBackend};
use clap::Subcommand;
use config::Settings;

#[derive(Debug, Subcommand)]
pub enum DevCmd {
    /// Load the config (repairing out-of-range values), print it, and show its file path.
    Config,
    /// Print all resolved RustShot paths.
    Paths,
    /// List connected monitors (implemented in M1).
    ListMonitors,
    /// Grab a full monitor to a PNG file (implemented in M1).
    GrabMonitor {
        /// Monitor index as reported by `list-monitors`.
        index: usize,
        /// Output PNG path.
        out: PathBuf,
    },
}

pub fn run(cmd: DevCmd) -> ExitCode {
    match cmd {
        DevCmd::Config => cmd_config(),
        DevCmd::Paths => cmd_paths(),
        DevCmd::ListMonitors => cmd_list_monitors(),
        DevCmd::GrabMonitor { index, out } => cmd_grab_monitor(index, out),
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
