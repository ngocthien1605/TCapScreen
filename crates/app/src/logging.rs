//! Structured logging to both stderr and a rolling file under `%APPDATA%\RustShot\logs`.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialise logging. Returns a guard that must be kept alive for the lifetime of the
/// process (dropping it flushes and stops the background file writer).
///
/// `logs_dir` may fail to be writable; in that case we log to stderr only and return `None`
/// for the guard rather than failing startup.
pub fn init(logs_dir: &Path) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_env("RUSTSHOT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let stderr_layer = fmt::layer().with_writer(std::io::stderr).with_target(false);

    match std::fs::create_dir_all(logs_dir) {
        Ok(()) => {
            let file_appender = tracing_appender::rolling::daily(logs_dir, "rustshot.log");
            let (nb, guard) = tracing_appender::non_blocking(file_appender);
            let file_layer = fmt::layer()
                .with_writer(nb)
                .with_ansi(false)
                .with_target(false);
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(stderr_layer)
                .with(file_layer)
                .try_init();
            Some(guard)
        }
        Err(e) => {
            let _ = tracing_subscriber::registry()
                .with(filter)
                .with(stderr_layer)
                .try_init();
            tracing::warn!("could not create log directory {}: {e}", logs_dir.display());
            None
        }
    }
}
