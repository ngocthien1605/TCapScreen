//! RustShot — a lightweight Windows screenshot + screen-recording tray app.
//!
//! Running with no subcommand starts the tray application. The `dev` subcommand exposes
//! diagnostic commands that do not start the tray (see [`dev`]).

// NOTE: `#![windows_subsystem = "windows"]` (hide the console) is deliberately deferred to M6
// so the dev CLI and early manual testing can print to a console. See DECISIONS.md.

mod dev;
mod icon;
mod logging;
mod single_instance;
mod tray;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use config::Settings;

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
            // Keep the guard alive for the whole run.
            let _guard = guard;
            if let Err(e) = start(&_guard) {
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

fn start(_instance: &SingleInstance) -> anyhow::Result<()> {
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

    tray::run(output_folder)
}
