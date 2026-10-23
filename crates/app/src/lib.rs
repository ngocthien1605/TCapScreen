//! RustShot — a lightweight Windows screenshot + screen-recording tray app.
//!
//! The library holds the whole application; two thin binaries drive it:
//! `rustshot` (the GUI-subsystem tray app, [`run_tray`]) and `rustshot-dev` (a console program
//! for developer/diagnostic commands, [`run_dev`]). Everything else is crate-private.

mod annotate;
mod capture_flow;
mod clipboard;
mod dev;
mod dispatch;
mod flyout;
mod hotkeys;
mod icon;
mod logging;
mod notify;
mod overlay;
mod panic_guard;
mod platform;
mod record_flow;
mod recording;
mod settings_ui;
mod single_instance;
mod startup;
mod tray;

use std::process::ExitCode;

use config::{Recovery, Settings};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, TranslateMessage, MSG,
};

pub use dev::DevCmd;

use crate::single_instance::SingleInstance;

const SINGLE_INSTANCE_NAME: &str = "RustShot.SingleInstance.Mutex";

/// Run a developer/diagnostic command (the `rustshot-dev` binary). Never starts the tray.
pub fn run_dev(cmd: DevCmd) -> ExitCode {
    dev::run(cmd)
}

/// Tell someone following old instructions (`rustshot dev …`) where the commands went. The tray
/// binary has no console, so this is a message box rather than text.
pub fn dev_commands_moved() -> ExitCode {
    notify::error(
        "Developer commands have moved.",
        "Run them with rustshot-dev.exe instead of \"rustshot dev\", for example:\n\n\
         rustshot-dev list-monitors",
    );
    ExitCode::FAILURE
}

/// Start the tray application.
pub fn run_tray() -> ExitCode {
    // Logging first so everything after is captured; then the panic hook, which relies on it.
    let logs_dir = config::paths::logs_dir().ok();
    let _log_guard = logs_dir.as_deref().and_then(logging::init);
    panic_guard::install_hook();

    // Enforce a single instance.
    match SingleInstance::acquire(SINGLE_INSTANCE_NAME) {
        Ok(Some(guard)) => {
            let _guard = guard;
            if let Err(e) = start() {
                // Launched from Explorer or the Run key there is no console, so stderr alone
                // would make a failed start look like the app simply never appeared.
                eprintln!("RustShot failed to start: {e:#}");
                notify::fatal("RustShot could not start.", e);
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
            eprintln!("RustShot failed to start: {e:#}");
            notify::fatal(
                "RustShot could not start (single-instance check failed).",
                e,
            );
            ExitCode::FAILURE
        }
    }
}

fn start() -> anyhow::Result<()> {
    // Initialise COM as STA on the UI thread for shell verbs (and M6 file dialogs). The capture
    // backend later attempts RoInitialize(MTA) on this thread; that returns RPC_E_CHANGED_MODE
    // and is harmlessly tolerated — WGC's free-threaded frame pool is apartment-agnostic.
    init_com_sta();

    // Load + repair config; create it on first run. A file that cannot be parsed is moved aside
    // and defaults take over, so a hand-edit typo never leaves the user without hotkeys. The
    // `exists()` check must come *before* the load: a quarantined file is gone afterwards.
    let config_path = config::paths::config_file()?;
    let first_run = !config_path.exists();
    let (mut settings, recovery) = Settings::load_or_recover(&config_path);
    let repaired = settings.sanitize();
    if repaired {
        tracing::warn!("config had out-of-range values; repaired and saving");
    }
    let quarantined = matches!(recovery, Some(Recovery::Quarantined { moved: Ok(_), .. }));
    if let Some(r) = &recovery {
        tracing::error!("config recovery: {r:?}");
    }
    // Persist when there is nothing usable on disk (first run / moved aside) or we changed
    // something; never overwrite a file we merely could not read.
    if first_run || quarantined || repaired {
        if let Err(e) = settings.save_to(&config_path) {
            tracing::warn!("could not persist config to {}: {e}", config_path.display());
        } else if first_run {
            tracing::info!(
                "first run: wrote default config to {}",
                config_path.display()
            );
        } else if quarantined {
            tracing::info!("wrote a fresh default config to {}", config_path.display());
        }
    }

    // Resolve the output folder; if it cannot be created, fall back to the default rather than
    // start with a folder every capture will fail to write to.
    let (output_folder, folder_notice) = usable_output_folder(&settings)?;

    // Apply the run-at-startup preference (re-writes the path in case the exe moved).
    if let Err(e) = startup::apply(settings.run_at_startup) {
        tracing::warn!("could not apply run-at-startup: {e:#}");
    }

    // Create the dispatch window (owns app state), the tray, and the global hotkeys. Each must
    // stay alive for the whole message loop.
    let dispatch_hwnd = dispatch::create(settings.clone(), output_folder)?;
    let _tray = tray::build(dispatch_hwnd)?;
    let hotkeys = hotkeys::register(&settings.hotkeys, dispatch_hwnd);
    if let Some(hk) = &hotkeys {
        // A key held by another app (Snipping Tool, another capture tool) is a permanent
        // condition on that machine, so at startup it goes to the tooltip and the log rather
        // than a dialog on every login; Settings → OK reports it in a dialog.
        let _ = _tray.set_tooltip(Some(hk.tooltip()));
    }
    dispatch::with_app(|app| app.hotkeys = hotkeys);

    // Startup notices go last, once the tray icon is up, so the user can see the app is
    // running while the (message-pumping) box is open. The gate is held meanwhile so a hotkey
    // pressed at that moment is refused rather than starting a capture nested inside the box.
    if recovery.is_some() || folder_notice.is_some() {
        dispatch::with_app(|app| app.busy = true);
        if let Some(r) = &recovery {
            notify::notice(&r.message());
        }
        if let Some(n) = folder_notice {
            notify::warn("The configured output folder is not usable.", n);
        }
        dispatch::with_app(|app| app.busy = false);
    }

    tracing::info!("RustShot is running in the tray");
    run_message_loop();
    // Tear the state down here, on the UI thread, whatever posted the WM_QUIT: this finalizes
    // any recording still running and destroys the remaining windows.
    drop(dispatch::take_state());
    tracing::info!("RustShot tray loop exited");
    Ok(())
}

/// The folder captures will be written to: the configured one if it exists or can be created,
/// else the default. The second value is a user-facing explanation when the fallback is used.
pub(crate) fn usable_output_folder(
    settings: &Settings,
) -> anyhow::Result<(std::path::PathBuf, Option<String>)> {
    let wanted = settings.effective_output_folder()?;
    match std::fs::create_dir_all(&wanted) {
        Ok(()) => Ok((wanted, None)),
        Err(e) => {
            let fallback = config::paths::default_output_folder()?;
            if let Err(e2) = std::fs::create_dir_all(&fallback) {
                tracing::warn!(
                    "could not create default output folder {}: {e2}",
                    fallback.display()
                );
            }
            Ok((
                fallback.clone(),
                Some(format!(
                    "{}\n{e}\n\nCaptures will be saved to:\n{}\n\nChoose another folder in \
                     Settings.",
                    wanted.display(),
                    fallback.display()
                )),
            ))
        }
    }
}

pub(crate) fn init_com_sta() {
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
