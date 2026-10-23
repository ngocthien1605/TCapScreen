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

/// Make `hwnd` the foreground window with keyboard focus. `SetForegroundWindow` alone is
/// refused when this process did not receive the last input — a delayed screenshot firing from
/// a timer, or an action posted by another program — and the window would then show up
/// topmost without the keyboard, so Esc/Enter would go to whatever app had focus. Attaching to
/// the foreground thread's input queue for the call is the documented way around that.
pub(crate) fn bring_to_front(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    };
    // SAFETY: focus/activation calls on our own window; the input attachment is undone before
    // returning.
    unsafe {
        if SetForegroundWindow(hwnd).as_bool() && GetForegroundWindow() == hwnd {
            let _ = SetFocus(Some(hwnd));
            return;
        }
        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let me = GetCurrentThreadId();
        let attached = fg_thread != 0
            && fg_thread != me
            && windows::Win32::System::Threading::AttachThreadInput(me, fg_thread, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        if attached {
            let _ = windows::Win32::System::Threading::AttachThreadInput(me, fg_thread, false);
        }
    }
}
