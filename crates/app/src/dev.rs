//! Developer/diagnostic CLI commands. These never start the tray; they let GPU/display paths
//! be exercised on a real desktop and let config/paths be inspected on any machine (incl. CI).

use std::path::PathBuf;
use std::process::ExitCode;

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
        DevCmd::ListMonitors | DevCmd::GrabMonitor { .. } => {
            eprintln!("This dev command is implemented in milestone M1 (capture engine).");
            ExitCode::from(2)
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
