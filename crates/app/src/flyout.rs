//! The post-capture flyout: a small, non-modal, top-most window in the bottom-right corner
//! showing a thumbnail of the shot and six actions (Annotate, Open, Folder, Copy, Path, Delete). It
//! auto-dismisses after a few seconds. Button clicks post [`Action`]s to the dispatch window,
//! which calls back into the [`Flyout`] methods — keeping all state on the UI thread.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::Once;

use anyhow::{Context, Result};
use imaging::Rgba;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, SetBkColor, SetTextColor,
    StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, GetWindowLongPtrW, KillTimer,
    RegisterClassW, SetTimer, SetWindowLongPtrW, ShowWindow, CREATESTRUCTW, GWLP_USERDATA, HMENU,
    SM_CXSCREEN, SM_CYSCREEN, SW_SHOWNA, SW_SHOWNORMAL, WINDOW_EX_STYLE, WM_COMMAND, WM_DESTROY,
    WM_NCCREATE, WM_PAINT, WM_TIMER, WNDCLASSW, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP, WS_VISIBLE,
};

use crate::dispatch::{self, Action};

const CLASS_NAME: PCWSTR = w!("RustShotFlyout");
static REGISTER: Once = Once::new();

const WIN_W: i32 = 500;
const WIN_H: i32 = 170;
const DISMISS_MS: u32 = 8000;
const TIMER_ID: usize = 1;

// Child-button control ids.
const ID_OPEN: usize = 101;
const ID_FOLDER: usize = 102;
const ID_COPY: usize = 103;
const ID_PATH: usize = 104;
const ID_DELETE: usize = 105;
const ID_ANNOTATE: usize = 106;

/// Heap-stable inner state, pointed to by the window's `GWLP_USERDATA` and owned by [`Flyout`].
struct FlyoutInner {
    dispatch_hwnd: HWND,
    path: PathBuf,
    /// Thumbnail pixels in top-down BGRA for `StretchDIBits`.
    thumb_bgra: Vec<u8>,
    thumb_w: i32,
    thumb_h: i32,
}

/// A live flyout window. Dropping it destroys the window.
pub struct Flyout {
    hwnd: HWND,
    // Boxed so its address is stable while the window proc holds a pointer to it.
    _inner: Box<FlyoutInner>,
}

impl Flyout {
    /// Create and show a flyout for the just-saved screenshot `path` with preview `img`.
    pub fn show(dispatch_hwnd: HWND, path: &Path, img: &Rgba) -> Result<Flyout> {
        ensure_class()?;

        let thumb = imaging::thumbnail(img, 300);
        let (tw, th) = (thumb.width() as i32, thumb.height() as i32);
        let thumb_bgra = rgba_to_bgra(thumb.as_raw());

        let mut inner = Box::new(FlyoutInner {
            dispatch_hwnd,
            path: path.to_path_buf(),
            thumb_bgra,
            thumb_w: tw,
            thumb_h: th,
        });

        // SAFETY: standard top-most tool window creation; CREATESTRUCT carries the inner ptr.
        let hwnd = unsafe {
            let hinstance = GetModuleHandleW(None).context("GetModuleHandleW")?;
            let sw = GetSystemMetrics(SM_CXSCREEN);
            let sh = GetSystemMetrics(SM_CYSCREEN);
            let x = (sw - WIN_W - 24).max(0);
            let y = (sh - WIN_H - 64).max(0);
            let ptr: *mut FlyoutInner = &mut *inner;
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!("RustShot"),
                WS_POPUP,
                x,
                y,
                WIN_W,
                WIN_H,
                None,
                Some(HMENU(null_mut())),
                Some(hinstance.into()),
                Some(ptr as *const c_void),
            )
            .context("creating flyout window")?
        };

        create_buttons(hwnd)?;

        // SAFETY: show without activating (don't steal focus), and arm the auto-dismiss timer.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
            let _ = SetTimer(Some(hwnd), TIMER_ID, DISMISS_MS, None);
        }

        Ok(Flyout {
            hwnd,
            _inner: inner,
        })
    }

    /// The saved file this flyout represents.
    pub fn path(&self) -> &Path {
        &self._inner.path
    }
}

/// Open an image file with its default viewer. Free function so it runs *outside* any
/// `AppState` borrow (`ShellExecuteW` can pump messages and re-enter the dispatch loop).
pub fn shell_open(file: &Path) {
    let wide = HSTRING::from(file.as_os_str());
    // SAFETY: ShellExecuteW with valid wide strings; the returned handle is advisory only.
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

impl Drop for Flyout {
    fn drop(&mut self) {
        // SAFETY: destroys the window (and its child buttons); the timer dies with it.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_ID);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Open a folder (or a file's folder) in Explorer — used by the tray "Open output folder".
pub fn open_in_explorer(path: &Path) {
    if let Err(e) = std::fs::create_dir_all(path) {
        tracing::warn!("could not create {}: {e}", path.display());
    }
    if let Err(e) = std::process::Command::new("explorer").arg(path).spawn() {
        tracing::warn!("could not open folder: {e}");
    }
}

/// Open Explorer with `file` selected (used by the flyout's Folder button).
pub fn select_in_explorer(file: &Path) {
    // `explorer /select,<path>` opens the folder with the file highlighted.
    if let Err(e) = std::process::Command::new("explorer")
        .arg(format!("/select,{}", file.display()))
        .spawn()
    {
        tracing::warn!("could not open containing folder: {e}");
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
        Some(e) => Err(e).context("registering flyout window class"),
        None => Ok(()),
    }
}

fn create_buttons(parent: HWND) -> Result<()> {
    let labels = [
        (ID_ANNOTATE, w!("Annotate")),
        (ID_OPEN, w!("Open")),
        (ID_FOLDER, w!("Folder")),
        (ID_COPY, w!("Copy")),
        (ID_PATH, w!("Path")),
        (ID_DELETE, w!("Delete")),
    ];
    let (bw, bh, gap, x0, y) = (76i32, 28i32, 4i32, 12i32, WIN_H - 40);
    // SAFETY: standard BUTTON child-window creation.
    unsafe {
        let hinstance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        for (i, (id, label)) in labels.iter().enumerate() {
            let x = x0 + i as i32 * (bw + gap);
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                *label,
                // BS_PUSHBUTTON is 0 (the default button style), so WS_CHILD | WS_VISIBLE suffices.
                WS_CHILD | WS_VISIBLE,
                x,
                y,
                bw,
                bh,
                Some(parent),
                Some(HMENU(*id as *mut c_void)),
                Some(hinstance.into()),
                None,
            )
            .context("creating flyout button")?;
        }
    }
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: stash the inner pointer from the CREATESTRUCT.
        unsafe {
            let cs = lparam.0 as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }

    // SAFETY: userdata holds the inner pointer after WM_NCCREATE.
    let inner_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const FlyoutInner;
    if inner_ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: the inner lives as long as its Flyout, which outlives the window.
    let inner = unsafe { &*inner_ptr };

    match msg {
        WM_COMMAND => {
            let id = wparam.0 & 0xFFFF;
            let action = match id {
                ID_ANNOTATE => Some(Action::FlyoutAnnotate),
                ID_OPEN => Some(Action::FlyoutOpen),
                ID_FOLDER => Some(Action::FlyoutOpenFolder),
                ID_COPY => Some(Action::FlyoutCopy),
                ID_PATH => Some(Action::FlyoutCopyPath),
                ID_DELETE => Some(Action::FlyoutDelete),
                _ => None,
            };
            if let Some(a) = action {
                dispatch::post(inner.dispatch_hwnd, a);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            // SAFETY: one-shot dismissal.
            unsafe {
                let _ = KillTimer(Some(hwnd), TIMER_ID);
            }
            dispatch::post(inner.dispatch_hwnd, Action::FlyoutDismiss);
            LRESULT(0)
        }
        WM_PAINT => {
            paint(hwnd, inner);
            LRESULT(0)
        }
        WM_DESTROY => LRESULT(0),
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn paint(hwnd: HWND, inner: &FlyoutInner) {
    // SAFETY: standard paint; draws background + the thumbnail via StretchDIBits.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);

        let bg = CreateSolidBrush(COLORREF(0x0014_1210));
        let client = RECT {
            left: 0,
            top: 0,
            right: WIN_W,
            bottom: WIN_H,
        };
        FillRect(hdc, &client, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));

        // Fit the thumbnail into the preview area, preserving aspect ratio.
        let (area_w, area_h) = (WIN_W - 24, WIN_H - 60);
        let (tw, th) = (inner.thumb_w.max(1), inner.thumb_h.max(1));
        let scale = (area_w as f64 / tw as f64)
            .min(area_h as f64 / th as f64)
            .min(1.0);
        let dw = (tw as f64 * scale).round() as i32;
        let dh = (th as f64 * scale).round() as i32;
        let dx = 12 + (area_w - dw) / 2;
        let dy = 10 + (area_h - dh) / 2;

        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = tw;
        bmi.bmiHeader.biHeight = -th; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = 0; // BI_RGB

        SetBkColor(hdc, COLORREF(0x0014_1210));
        SetTextColor(hdc, COLORREF(0x00F0_F0F0));
        StretchDIBits(
            hdc,
            dx,
            dy,
            dw,
            dh,
            0,
            0,
            tw,
            th,
            Some(inner.thumb_bgra.as_ptr() as *const c_void),
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );

        let _ = EndPaint(hwnd, &ps);
    }
}

/// Convert tightly-packed RGBA to BGRA (what a BI_RGB 32bpp DIB expects).
fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; rgba.len()];
    let (out_px, _) = out.as_chunks_mut::<4>();
    let (in_px, _) = rgba.as_chunks::<4>();
    for (dst, src) in out_px.iter_mut().zip(in_px) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = src[3];
    }
    out
}
