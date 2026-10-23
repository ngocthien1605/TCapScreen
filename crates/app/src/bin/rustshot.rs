//! The RustShot tray app. GUI subsystem: no console window; all output goes to the log.

#![windows_subsystem = "windows"]

use std::process::ExitCode;

fn main() -> ExitCode {
    // Must happen before any window (incl. the tray's hidden one) so all coordinates are in
    // true physical pixels on mixed-DPI setups.
    capture_core::ensure_process_dpi_aware();
    if std::env::args().nth(1).as_deref() == Some("dev") {
        return rustshot_app::dev_commands_moved();
    }
    rustshot_app::run_tray()
}
