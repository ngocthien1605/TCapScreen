//! Structured logging to both stderr and a rolling file under `%APPDATA%\RustShot\logs`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Daily log files kept (two weeks); older ones are deleted at rotation.
const KEEP_LOG_FILES: usize = 14;
/// Size at which `panic.log` is rolled over to `panic.log.old`.
const MAX_PANIC_LOG_BYTES: u64 = 1024 * 1024;

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
    // Bounded too: past the cap the old file is kept once as `panic.log.old`.
    let path = dir.join("panic.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_PANIC_LOG_BYTES) {
        let _ = std::fs::rename(&path, dir.join("panic.log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
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
            // One file per day (`rustshot.log.YYYY-MM-DD`), keeping the newest
            // [`KEEP_LOG_FILES`]: a tray app runs for months, and the folder must not grow
            // forever. Falls back to stderr-only if the appender can't be built.
            let file_appender = match tracing_appender::rolling::RollingFileAppender::builder()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("rustshot.log")
                .max_log_files(KEEP_LOG_FILES)
                .build(logs_dir)
            {
                Ok(a) => a,
                Err(e) => {
                    let _ = tracing_subscriber::registry()
                        .with(filter)
                        .with(stderr_layer)
                        .try_init();
                    tracing::warn!("could not open the log file in {}: {e}", logs_dir.display());
                    return None;
                }
            };
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
