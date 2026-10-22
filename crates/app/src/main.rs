//! RustShot — a lightweight Windows screenshot + screen-recording tray app.
//!
//! Running with no subcommand starts the tray application. The `dev` subcommand exposes
//! diagnostic commands that do not start the tray (see [`dev`]).

// NOTE: `#![windows_subsystem = "windows"]` (hide the console) is deliberately deferred to M6
// so the dev CLI and early manual testing can print to a console. See DECISIONS.md.

mod capture_flow;
mod clipboard;
mod dev;
mod dispatch;
mod flyout;
mod hotkeys;
mod icon;
mod logging;
mod overlay;
mod single_instance;
mod tray;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use config::Settings;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, TranslateMessage, MSG,
};

use crate::single_instance::SingleInstance;

const SINGLE_INSTANCE_NAME: &str = "RustShot.SingleInstance.Mutex";

#[derive(Debug, Parser)]
#[command(
    name = "rustshot",
    version,
    about = "Screenshots and screen recording for Windows"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Developer/diagnostic commands (these do not start the tray).
    Dev {
        #[command(subcommand)]
        cmd: dev::DevCmd,
    },
}

fn main() -> ExitCode {
    // Must happen before any window (incl. the tray's hidden one) so all coordinates are in
    // true physical pixels on mixed-DPI setups.
    capture_core::ensure_process_dpi_aware();

    let cli = Cli::parse();
    match cli.command {
        Some(Command::Dev { cmd }) => dev::run(cmd),
        None => run_app(),
    }
}

/// Start the tray application.
fn run_app() -> ExitCode {
    // Logging first so everything after is captured.
    let logs_dir = config::paths::logs_dir().ok();
    let _log_guard = logs_dir.as_deref().and_then(logging::init);

    // Enforce a single instance.
    match SingleInstance::acquire(SINGLE_INSTANCE_NAME) {
        Ok(Some(guard)) => {
            let _guard = guard;
            if let Err(e) = start() {
                tracing::error!("fatal: {e:#}");
                eprintln!("RustShot failed to start: {e:#}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Ok(None) => {
            tracing::info!("another RustShot instance is already running; exiting");
            eprintln!("RustShot is already running.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("single-instance check failed: {e:#}");
            eprintln!("RustShot failed to start: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn start() -> anyhow::Result<()> {
    // Initialise COM as STA on the UI thread for shell verbs (and M6 file dialogs). The capture
    // backend later attempts RoInitialize(MTA) on this thread; that returns RPC_E_CHANGED_MODE
    // and is harmlessly tolerated — WGC's free-threaded frame pool is apartment-agnostic.
    init_com_sta();

    // Load + repair config; create it on first run.
    let config_path = config::paths::config_file()?;
    let mut settings = Settings::load_from(&config_path)?;
    let first_run = !config_path.exists();
    let repaired = settings.sanitize();
    if first_run || repaired {
        if let Err(e) = settings.save_to(&config_path) {
            tracing::warn!("could not persist config to {}: {e}", config_path.display());
        } else if first_run {
            tracing::info!(
                "first run: wrote default config to {}",
                config_path.display()
            );
        }
    }

    // Ensure the output folder exists.
    let output_folder = settings.effective_output_folder()?;
    if let Err(e) = std::fs::create_dir_all(&output_folder) {
        tracing::warn!(
            "could not create output folder {}: {e}",
            output_folder.display()
        );
    }

    // Create the dispatch window (owns app state), the tray, and the global hotkeys. Each must
    // stay alive for the whole message loop.
    let dispatch_hwnd = dispatch::create(settings.clone(), output_folder)?;
    let _tray = tray::build(dispatch_hwnd)?;
    let _hotkeys = hotkeys::register(&settings.hotkeys, dispatch_hwnd);

    tracing::info!("RustShot is running in the tray");
    run_message_loop();
    tracing::info!("RustShot tray loop exited");
    Ok(())
}

fn init_com_sta() {
    // SAFETY: initialises COM for this thread; tolerate an already-initialised apartment.
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() && hr != RPC_E_CHANGED_MODE {
            tracing::warn!("CoInitializeEx(STA) returned {hr:?}");
        }
    }
}

/// Standard blocking Win32 message loop; returns when WM_QUIT is received.
fn run_message_loop() {
    // SAFETY: standard message pump; `msg` is a valid, writable MSG for the duration.
    unsafe {
        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                // 0 = WM_QUIT, -1 = error: either way, stop pumping.
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
