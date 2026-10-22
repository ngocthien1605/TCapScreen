//! Process-wide platform setup that must happen early (before any window is created).

/// Opt the process into Per-Monitor-V2 DPI awareness so monitor/window rectangles and the
/// capture pipeline work in true physical pixels on mixed-DPI setups.
///
/// Must be called before anything creates an HWND (e.g. the tray's hidden window). Safe to
/// call more than once; later calls are no-ops. A manifest-based declaration (M6/M7) is the
/// more robust form; this programmatic call covers dev builds and early milestones.
#[cfg(windows)]
pub fn ensure_process_dpi_aware() {
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    // SAFETY: no arguments borrowed; failure (already set via manifest, or unsupported OS)
    // is expected and ignored.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// No-op on non-Windows targets (only Windows is shipped; this keeps the crate portable).
#[cfg(not(windows))]
pub fn ensure_process_dpi_aware() {}
