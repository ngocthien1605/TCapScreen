//! DPI helpers for the hand-placed windows (flyout, recording toolbar, editor toolbar).
//!
//! The process is Per-Monitor-V2 aware, so Windows does not stretch these windows: sizes
//! written as 96-DPI pixels must be scaled to the DPI of the monitor the window lands on, or
//! they come out small and cramped at 125 % / 150 % / 200 %. Dialogs built from templates
//! (Settings) are scaled by the dialog manager and don't need this.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetMonitorInfoW, MonitorFromPoint, HFONT, HGDIOBJ,
    MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, GetDpiForWindow, SystemParametersInfoForDpi, MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS,
};

/// The DPI that layout constants are written for (100 %).
pub const BASE: u32 = 96;

/// Scale a length in 96-DPI pixels to `dpi`, rounding to nearest.
pub fn scale(px: i32, dpi: u32) -> i32 {
    ((px as i64 * dpi as i64 + BASE as i64 / 2) / BASE as i64) as i32
}

/// The DPI of the monitor `hwnd` is on (96 if Windows can't say).
pub fn of_window(hwnd: HWND) -> u32 {
    // SAFETY: plain query; returns 0 for an invalid window.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        BASE
    } else {
        dpi
    }
}

/// The work area (screen minus taskbar) and DPI of the monitor under the mouse pointer — where
/// a notification-style window should appear.
pub fn cursor_monitor() -> (RECT, u32) {
    let mut pt = POINT::default();
    // SAFETY: plain queries into locals.
    unsafe {
        let _ = GetCursorPos(&mut pt);
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut x, mut y) = (BASE, BASE);
        if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut x, &mut y).is_err() || x == 0 {
            x = BASE;
        }
        (info.rcWork, x)
    }
}

/// The system message font (what dialogs use) at `dpi`, deleted on drop.
pub struct UiFont(pub HFONT);

impl UiFont {
    pub fn new(dpi: u32) -> Option<UiFont> {
        let mut m = NONCLIENTMETRICSW {
            cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        // SAFETY: `m` is a properly sized NONCLIENTMETRICSW; the font is owned by `UiFont`.
        unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                m.cbSize,
                Some(&mut m as *mut _ as *mut _),
                0,
                dpi,
            )
            .ok()?;
            let font = CreateFontIndirectW(&m.lfMessageFont);
            (!font.is_invalid()).then_some(UiFont(font))
        }
    }
}

impl Drop for UiFont {
    fn drop(&mut self) {
        // SAFETY: we created this font; windows using it are destroyed first by their owners.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0 .0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_from_96_dpi() {
        assert_eq!(scale(100, 96), 100);
        assert_eq!(scale(500, 120), 625);
        assert_eq!(scale(28, 144), 42);
        assert_eq!(scale(170, 192), 340);
        assert_eq!(scale(3, 120), 4, "rounds to nearest");
    }
}
