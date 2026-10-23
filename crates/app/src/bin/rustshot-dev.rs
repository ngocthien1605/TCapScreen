//! `rustshot-dev`: developer and diagnostic commands (capture, record, probe, inspect config).
//! A console program, so output simply goes to the terminal. It never starts the tray.

use std::process::ExitCode;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "rustshot-dev",
    version,
    about = "RustShot developer and diagnostic commands"
)]
struct Cli {
    #[command(subcommand)]
    cmd: rustshot_app::DevCmd,
}

fn main() -> ExitCode {
    capture_core::ensure_process_dpi_aware();
    rustshot_app::run_dev(Cli::parse().cmd)
}
