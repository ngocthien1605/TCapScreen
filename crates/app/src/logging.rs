//! Structured logging to both stderr and a rolling file under `%APPDATA%\RustShot\logs`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Where the log files live, once [`init`] has run (used by the panic hook).
static LOGS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The log directory as text for user-facing messages ("(not available)" before `init`).
pub fn logs_dir_display() -> String {
    LOGS_DIR
        .get()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(log directory not available)".into())
}

/// Append one line to `panic.log` synchronously. Used by the panic hook: the normal file writer
/// is a background thread that may never get to flush if the process aborts.
pub fn write_panic_line(line: &str) {
    let Some(dir) = LOGS_DIR.get() else {
        return;
    };
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("panic.log"))
    {
        let _ = writeln!(f, "[unix {secs}] {line}");
        let _ = f.flush();
    }
}

/// Stderr-only logging for the `dev` commands, so pipeline diagnostics (frame-pool recreates,
/// early ends) are visible in the terminal. Level comes from `RUSTSHOT_LOG` (default `info`).
pub fn init_stderr() {
    let filter = EnvFilter::try_from_env("RUSTSHOT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr).with_target(false))
        .try_init();
}

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
            let _ = LOGS_DIR.set(logs_dir.to_path_buf());
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
