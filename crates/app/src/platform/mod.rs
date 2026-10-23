//! Small Win32 building blocks shared by the windows the app draws itself: DPI scaling and
//! owned GDI handles, plus the conversion from our geometry type to a Win32 `RECT`.

pub(crate) mod dpi;
pub(crate) mod gdi;

use capture_core::Rect;
use windows::Win32::Foundation::RECT;

/// `r` as a Win32 `RECT` (left/top/right/bottom).
pub(crate) fn win_rect(r: Rect) -> RECT {
    RECT {
        left: r.x,
        top: r.y,
        right: r.right(),
        bottom: r.bottom(),
    }
}
