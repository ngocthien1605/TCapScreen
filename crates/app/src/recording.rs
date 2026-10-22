//! The compact, always-on-top recording toolbar: a small Win32 window showing elapsed time
//! with Pause/Resume and Stop buttons. Buttons post actions to the dispatch window; the toolbar
//! holds no recorder state itself (that lives in [`crate::dispatch::AppState`]).

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::Once;
use std::time::Instant;

use anyhow::{Context, Result};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, SetBkColor, SetTextColor,
    TextOutW, HGDIOBJ, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, GetWindowLongPtrW, KillTimer,
    RegisterClassW, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow, CREATESTRUCTW,
    GWLP_USERDATA, HMENU, SM_CXSCREEN, SW_SHOWNA, WINDOW_EX_STYLE, WM_COMMAND, WM_NCCREATE,
    WM_PAINT, WM_TIMER, WNDCLASSW, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use crate::dispatch::{self, Action};

/// The in-progress recording: the encoder handle plus its toolbar window.
pub struct Recording {
    pub handle: recorder::RecorderHandle,
    pub toolbar: Toolbar,
}

const CLASS_NAME: PCWSTR = w!("RustShotRecToolbar");
static REGISTER: Once = Once::new();

const WIN_W: i32 = 240;
const WIN_H: i32 = 56;
const TIMER_ID: usize = 1;
const ID_PAUSE: usize = 201;
const ID_STOP: usize = 202;

struct ToolbarInner {
    dispatch_hwnd: HWND,
    start: Instant,
}

/// The toolbar window. Dropping it destroys the window.
pub struct Toolbar {
    hwnd: HWND,
    pause_btn: HWND,
    _inner: Box<ToolbarInner>,
}

impl Toolbar {
    /// Create and show the toolbar, posting its button actions to `dispatch_hwnd`.
    pub fn show(dispatch_hwnd: HWND) -> Result<Toolbar> {
        ensure_class()?;
        let mut inner = Box::new(ToolbarInner {
            dispatch_hwnd,
            start: Instant::now(),
        });

        // SAFETY: standard top-most tool-window creation; CREATESTRUCT carries the inner ptr.
        let hwnd = unsafe {
            let hinstance = GetModuleHandleW(None).context("GetModuleHandleW")?;
            let sw = GetSystemMetrics(SM_CXSCREEN);
            let x = (sw - WIN_W) / 2;
            let ptr: *mut ToolbarInner = &mut *inner;
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("RustShot Recording"),
                WS_POPUP,
                x.max(0),
                24,
                WIN_W,
                WIN_H,
                None,
                Some(HMENU(null_mut())),
                Some(hinstance.into()),
                Some(ptr as *const c_void),
            )
            .context("creating toolbar window")?
        };

        let pause_btn = create_button(hwnd, ID_PAUSE, w!("Pause"), 92)?;
        let _stop = create_button(hwnd, ID_STOP, w!("Stop"), 166)?;

        // SAFETY: show without stealing focus and tick the elapsed clock twice a second.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
            let _ = SetTimer(Some(hwnd), TIMER_ID, 500, None);
        }

        Ok(Toolbar {
            hwnd,
            pause_btn,
            _inner: inner,
        })
    }

    /// Reflect pause state on the Pause/Resume button.
    pub fn set_paused(&self, paused: bool) {
        let label = if paused { w!("Resume") } else { w!("Pause") };
        // SAFETY: updates our own button's text.
        unsafe {
            let _ = SetWindowTextW(self.pause_btn, label);
        }
    }
}

impl Drop for Toolbar {
    fn drop(&mut self) {
        // SAFETY: destroys the toolbar window (and its buttons); the timer dies with it.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_ID);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn create_button(parent: HWND, id: usize, label: PCWSTR, x: i32) -> Result<HWND> {
    // SAFETY: standard BUTTON child-window creation.
    unsafe {
        let hinstance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            label,
            WS_CHILD | WS_VISIBLE,
            x,
            16,
            70,
            26,
            Some(parent),
            Some(HMENU(id as *mut c_void)),
            Some(hinstance.into()),
            None,
        )
        .context("creating toolbar button")
    }
}

fn ensure_class() -> Result<()> {
    let mut err: Option<windows::core::Error> = None;
    REGISTER.call_once(|| {
        // SAFETY: one-time class registration.
        unsafe {
            let hinstance = match GetModuleHandleW(None) {
                Ok(h) => h,
                Err(e) => {
                    err = Some(e);
                    return;
                }
            };
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                err = Some(windows::core::Error::from_thread());
            }
        }
    });
    match err {
        Some(e) => Err(e).context("registering toolbar window class"),
        None => Ok(()),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // A panic must not unwind across this FFI boundary (that aborts the process): run the real
    // handler under the panic guard and fall back to default processing.
    crate::panic_guard::guard(
        "recording toolbar wndproc",
        || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        || unsafe { wndproc_inner(hwnd, msg, wparam, lparam) },
    )
}

unsafe fn wndproc_inner(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: stash the inner pointer from the CREATESTRUCT.
        unsafe {
            let cs = lparam.0 as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }
    // SAFETY: userdata holds the inner pointer after WM_NCCREATE.
    let inner_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const ToolbarInner;
    if inner_ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: the inner lives as long as its Toolbar, which outlives the window.
    let inner = unsafe { &*inner_ptr };

    match msg {
        WM_COMMAND => {
            let id = wparam.0 & 0xFFFF;
            let action = match id {
                ID_PAUSE => Some(Action::RecordPauseToggle),
                ID_STOP => Some(Action::StopRecording),
                _ => None,
            };
            if let Some(a) = action {
                dispatch::post(inner.dispatch_hwnd, a);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            // SAFETY: repaint the elapsed clock.
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
            }
            // The same tick asks the dispatcher whether the recording ended on its own (source
            // window closed, graphics device lost) so this toolbar does not keep counting over
            // a recording that already finalized.
            dispatch::post(inner.dispatch_hwnd, Action::RecordPoll);
            LRESULT(0)
        }
        WM_PAINT => {
            paint(hwnd, inner);
            LRESULT(0)
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn paint(hwnd: HWND, inner: &ToolbarInner) {
    let secs = inner.start.elapsed().as_secs();
    let text = format!("● REC   {:02}:{:02}", secs / 60, secs % 60);
    let wide: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: standard paint of the background and the elapsed label.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let bg = CreateSolidBrush(COLORREF(0x0014_1210));
        let rect = RECT {
            left: 0,
            top: 0,
            right: WIN_W,
            bottom: WIN_H,
        };
        FillRect(hdc, &rect, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));
        SetBkColor(hdc, COLORREF(0x0014_1210));
        SetTextColor(hdc, COLORREF(0x0040_40F0)); // reddish (BGR)
        let _ = TextOutW(hdc, 12, 18, &wide);
        let _ = EndPaint(hwnd, &ps);
    }
}
