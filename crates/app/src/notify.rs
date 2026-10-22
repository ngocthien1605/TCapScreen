//! User-facing notices. Every failure the user caused or would otherwise never learn about
//! (a capture that did not save, a recording that did not start, settings that did not stick)
//! goes through here as well as to the log, so a problem never ends as a silent no-op.
//!
//! The implementation is a plain top-most message box: the tray crate has no balloon API and
//! a custom toast is more than the seam needs today. The functions deliberately do not promise
//! non-modality, so a toast can replace the box later without touching call sites. Callers on
//! the UI thread should keep the `busy` gate held while they notify (the box pumps messages, so
//! a hotkey pressed meanwhile would otherwise start a nested capture).

use std::fmt::Display;

use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    MESSAGEBOX_STYLE,
};

const TITLE: &str = "RustShot";

/// Something the user asked for did not happen. `what` is a short headline ("Screenshot
/// failed"); `detail` is the underlying error.
pub fn error(what: &str, detail: impl Display) {
    tracing::error!("{what}: {detail:#}");
    show(&format!("{what}\n\n{detail:#}"), MB_ICONERROR);
}

/// Something happened, but not quite as configured (a fallback is in effect).
pub fn warn(what: &str, detail: impl Display) {
    tracing::warn!("{what}: {detail:#}");
    show(&format!("{what}\n\n{detail:#}"), MB_ICONWARNING);
}

/// Show an already-composed notice verbatim (used for multi-paragraph startup messages).
pub fn notice(text: &str) {
    show(text, MB_ICONWARNING);
}

/// The app cannot continue. Logged, shown, and then it is up to the caller to exit.
pub fn fatal(what: &str, detail: impl Display) {
    tracing::error!("fatal: {what}: {detail:#}");
    show(&format!("{what}\n\n{detail:#}"), MB_ICONERROR);
}

fn show(text: &str, icon: MESSAGEBOX_STYLE) {
    let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let title: Vec<u16> = TITLE.encode_utf16().chain([0]).collect();
    // SAFETY: an unowned, top-most message box; both strings are NUL-terminated and outlive
    // the call.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | icon | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}
