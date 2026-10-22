//! Top-level window enumeration, filtering invisible/cloaked/tool windows.

use core::ffi::c_void;
use std::mem::size_of;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

use crate::{Rect, Result, WindowId, WindowInfo};

/// Enumerate capturable top-level windows (visible, titled, not cloaked, not tool windows).
pub fn enumerate() -> Result<Vec<WindowInfo>> {
    let mut out: Vec<WindowInfo> = Vec::new();
    // SAFETY: the callback only pushes into the Vec addressed by `lparam`, valid for the call.
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut out as *mut Vec<WindowInfo> as isize),
        );
    }
    Ok(out)
}

/// Resolve a [`WindowId`] back to its `HWND`.
pub fn hwnd_of(id: WindowId) -> HWND {
    HWND(id.0 as *mut c_void)
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` carries the &mut Vec<WindowInfo> set up in `enumerate`.
    let out = unsafe { &mut *(lparam.0 as *mut Vec<WindowInfo>) };
    if let Some(info) = describe(hwnd) {
        out.push(info);
    }
    BOOL(1)
}

fn describe(hwnd: HWND) -> Option<WindowInfo> {
    // SAFETY: `hwnd` is provided by EnumWindows and valid for these queries.
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return None;
    }

    let mut cloaked: u32 = 0;
    // SAFETY: writes a u32 via a correctly-sized out buffer.
    let _ = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    if cloaked != 0 {
        return None;
    }

    // SAFETY: reads the extended window style.
    let exstyle = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    if exstyle & WS_EX_TOOLWINDOW.0 != 0 {
        return None;
    }

    // SAFETY: returns the title length in characters.
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize + 1];
    // SAFETY: `buf` has room for `len + 1` UTF-16 code units incl. NUL.
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if n <= 0 {
        return None;
    }
    let title = String::from_utf16_lossy(&buf[..n as usize]);
    if title.trim().is_empty() {
        return None;
    }

    let mut rc = RECT::default();
    // SAFETY: writes a RECT via a correctly-sized out buffer.
    let got = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rc as *mut RECT as *mut c_void,
            size_of::<RECT>() as u32,
        )
    };
    if got.is_err() {
        // SAFETY: fallback; writes the (possibly border-inflated) window rect.
        unsafe {
            let _ = GetWindowRect(hwnd, &mut rc);
        }
    }
    let bounds = Rect::new(rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top);
    if bounds.is_empty() {
        return None;
    }

    Some(WindowInfo {
        id: WindowId(hwnd.0 as isize),
        title,
        bounds,
    })
}
