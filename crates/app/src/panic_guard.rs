//! Panic containment. A panic that unwinds out of an `extern "system"` window procedure aborts
//! the process — for a tray app that means the icon silently vanishes, nothing reaches the
//! (asynchronous) log, and an in-progress recording is never finalized.
//!
//! Two layers:
//! - [`install_hook`]: a process-wide panic hook that records every panic **synchronously** to
//!   `panic.log` (the tracing file writer is a background thread and may not flush before an
//!   abort) and to the normal log.
//! - [`guard`]: wraps each Win32 callback body in `catch_unwind`, so a panic becomes a logged,
//!   one-time error dialog and a neutral return value instead of an abort. The dialog is shown
//!   *after* unwinding, never inside the hook, because at hook time the panicking frame may
//!   still hold the `AppState` borrow and pumping messages there could panic again (= abort).

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
};

static DIALOG_SHOWN: AtomicBool = AtomicBool::new(false);

/// Install the process-wide panic hook (call once, early in `main`).
pub fn install_hook() {
    panic::set_hook(Box::new(|info| {
        let line = format!("panic: {info}");
        // Synchronous, so it survives an abort elsewhere; then the normal (async) log and stderr
        // for `dev` runs in a terminal.
        crate::logging::write_panic_line(&line);
        tracing::error!("{line}");
        eprintln!("{line}");
    }));
}

/// Run `f`, converting a panic into a logged error, a one-time dialog, and `fallback()`.
///
/// `what` names the callback for the log. `fallback` is lazy so an `unsafe` default (e.g.
/// `DefWindowProcW`) is only evaluated when actually needed.
pub fn guard<R>(what: &str, fallback: impl FnOnce() -> R, f: impl FnOnce() -> R) -> R {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => {
            tracing::error!("{what}: panic caught; the app keeps running in a degraded state");
            notify_once();
            fallback()
        }
    }
}

/// Tell the user once per process; later panics are only logged (avoids a dialog storm if a
/// repeating message such as WM_PAINT keeps failing).
fn notify_once() {
    if DIALOG_SHOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    let dir = crate::logging::logs_dir_display();
    let text: Vec<u16> = format!(
        "RustShot hit an internal error and recovered, but may not work correctly until it is \
         restarted.\n\nDetails were written to:\n{dir}"
    )
    .encode_utf16()
    .chain([0])
    .collect();
    let title: Vec<u16> = "RustShot".encode_utf16().chain([0]).collect();
    // SAFETY: a plain, unowned, top-most message box; both strings are NUL-terminated.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn guard_returns_the_result_when_nothing_panics() {
        assert_eq!(guard("t", || -1, || 42), 42);
    }

    #[test]
    fn guard_swallows_a_panic_and_returns_the_fallback() {
        // Mark the dialog as already shown so the test stays headless.
        DIALOG_SHOWN.store(true, Ordering::SeqCst);
        let r = guard("t", || "fallback", || -> &str { panic!("boom") });
        assert_eq!(r, "fallback");
    }

    #[test]
    fn fallback_is_not_evaluated_on_success() {
        let mut evaluated = false;
        let r = guard(
            "t",
            || {
                evaluated = true;
                0
            },
            || 7,
        );
        assert_eq!(r, 7);
        assert!(!evaluated);
    }
}
