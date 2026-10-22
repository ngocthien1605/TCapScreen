//! Win32 plumbing for the selection overlay. A `WS_POPUP | WS_EX_TOPMOST` window covers the
//! virtual-desktop bounding box and paints a frozen capture of the desktop, dimmed, with the
//! live selection (region rubber-band or hovered-window highlight) shown undimmed plus a
//! dimension label. A nested modal message loop runs until the user confirms or cancels.
//!
//! The overlay never captures itself: the caller freezes the desktop *before* calling [`select`]
//! and passes that frame in, so the result is cropped from the frozen image (regions) or
//! recaptured cleanly (windows) — the overlay pixels can't leak into either path.

use std::ffi::c_void;
use std::ptr::{copy_nonoverlapping, null_mut, write_bytes};
use std::sync::Once;

use anyhow::{Context, Result};
use capture_core::{Frame, Point, Rect, WindowInfo};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AlphaBlend, BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection,
    CreateFontW, CreatePen, DeleteDC, DeleteObject, EndPaint, GetStockObject,
    GetTextExtentPoint32W, InvalidateRect, Rectangle, ReleaseDC, SelectObject, SetBkColor,
    SetBkMode, SetTextColor, TextOutW, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, HBITMAP, HDC, HFONT,
    HGDIOBJ, HOLLOW_BRUSH, OPAQUE, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    GetWindowLongPtrW, LoadCursorW, RegisterClassW, SetForegroundWindow, SetWindowLongPtrW,
    ShowWindow, TranslateMessage, CREATESTRUCTW, GWLP_USERDATA, HICON, HMENU, IDC_CROSS, MSG,
    SW_SHOW, WM_CHAR, WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_NCCREATE, WM_PAINT, WM_RBUTTONDOWN, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::state::{label_position, Outcome, SelectionState};

const CLASS_NAME: PCWSTR = w!("RustShotOverlay");
static REGISTER: Once = Once::new();

/// Dim strength applied to the non-selected area (0 = untouched, 255 = black).
const DIM_ALPHA: u8 = 120;

/// Run the selection overlay over the frozen desktop `frozen` (which spans `bbox`, in
/// virtual-desktop physical px). Blocks until the user confirms or cancels.
pub fn select(windows: &[WindowInfo], frozen: &Frame, bbox: Rect) -> Result<Outcome> {
    ensure_class()?;

    let mut ctx = Box::new(OverlayCtx {
        state: SelectionState::new(windows.to_vec()),
        origin: Point::new(bbox.x, bbox.y),
        width: bbox.width,
        height: bbox.height,
        gdi: Gdi::new(frozen, bbox.width, bbox.height)?,
        outcome: None,
        done: false,
    });

    // SAFETY: standard window creation; the CREATESTRUCT carries a pointer to `ctx`, which
    // outlives the window (we destroy the window before `ctx` drops at the end of this fn).
    let hwnd = unsafe {
        let hinstance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let ptr: *mut OverlayCtx = &mut *ctx;
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("RustShot Selection"),
            WS_POPUP,
            bbox.x,
            bbox.y,
            bbox.width,
            bbox.height,
            None,
            Some(HMENU(null_mut())),
            Some(hinstance.into()),
            Some(ptr as *const c_void),
        )
        .context("creating overlay window")?
    };

    // Seed the cursor position so a window highlights immediately on show.
    // SAFETY: writes the cursor position into `pt`.
    let mut pt = POINT::default();
    let _ = unsafe { GetCursorPos(&mut pt) };
    ctx.state.on_move(Point::new(pt.x, pt.y));

    // SAFETY: standard show/focus sequence on a just-created window.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    // Nested modal loop: runs until the window proc sets `done` (and destroys the window).
    // SAFETY: standard message pump; `msg` is valid for the duration.
    unsafe {
        // `ctx.done` is set from the window proc during `DispatchMessageW` (via the raw pointer),
        // which clippy can't see — hence a `loop` with the check after dispatch, not a `while`.
        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            if ctx.done {
                break;
            }
        }
    }

    Ok(ctx.outcome.take().unwrap_or(Outcome::Cancel))
}

fn ensure_class() -> Result<()> {
    let mut err: Option<windows::core::Error> = None;
    REGISTER.call_once(|| {
        // SAFETY: one-time window-class registration.
        unsafe {
            let hinstance = match GetModuleHandleW(None) {
                Ok(h) => h,
                Err(e) => {
                    err = Some(e);
                    return;
                }
            };
            let cursor = LoadCursorW(None, IDC_CROSS).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: CLASS_NAME,
                hCursor: cursor,
                hIcon: HICON(null_mut()),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                err = Some(windows::core::Error::from_thread());
            }
        }
    });
    match err {
        Some(e) => Err(e).context("registering overlay window class"),
        None => Ok(()),
    }
}

/// Overlay state owned for the duration of [`select`], reachable from the window proc via
/// `GWLP_USERDATA`.
struct OverlayCtx {
    state: SelectionState,
    origin: Point,
    width: i32,
    height: i32,
    gdi: Gdi,
    outcome: Option<Outcome>,
    done: bool,
}

impl OverlayCtx {
    /// Convert a virtual-desktop rect to a client rect clamped to the window.
    fn to_client(&self, r: Rect) -> Rect {
        let local = Rect::new(r.x - self.origin.x, r.y - self.origin.y, r.width, r.height);
        local
            .intersect(&Rect::new(0, 0, self.width, self.height))
            .unwrap_or(Rect::new(0, 0, 0, 0))
    }

    fn finish(&mut self, outcome: Outcome, hwnd: HWND) {
        self.outcome = Some(outcome);
        self.done = true;
        // SAFETY: `hwnd` is this overlay's window; destroying it ends the nested loop.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn loword(l: LPARAM) -> i32 {
    (l.0 & 0xFFFF) as u16 as i16 as i32
}
fn hiword(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xFFFF) as u16 as i16 as i32
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: WM_NCCREATE's lParam is a CREATESTRUCTW with our ctx pointer.
        unsafe {
            let cs = lparam.0 as *const CREATESTRUCTW;
            let ptr = (*cs).lpCreateParams as isize;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr);
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }

    // SAFETY: userdata holds the ctx pointer installed in WM_NCCREATE (null before then).
    let ctx_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut OverlayCtx;
    if ctx_ptr.is_null() {
        // SAFETY: default handling before our state is attached.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: the pointer is valid for the window's whole lifetime (owned by `select`).
    let ctx = unsafe { &mut *ctx_ptr };

    match msg {
        WM_LBUTTONDOWN => {
            // SAFETY: capturing the mouse on our own window.
            unsafe {
                SetCapture(hwnd);
            }
            let p = Point::new(loword(lparam) + ctx.origin.x, hiword(lparam) + ctx.origin.y);
            ctx.state.on_down(p);
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let p = Point::new(loword(lparam) + ctx.origin.x, hiword(lparam) + ctx.origin.y);
            ctx.state.on_move(p);
            invalidate(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            // SAFETY: releasing our own capture.
            unsafe {
                let _ = ReleaseCapture();
            }
            let p = Point::new(loword(lparam) + ctx.origin.x, hiword(lparam) + ctx.origin.y);
            if let Some(outcome) = ctx.state.on_up(p) {
                ctx.finish(outcome, hwnd);
            } else {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let outcome = ctx.state.on_cancel();
            ctx.finish(outcome, hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if let Some(outcome) = ctx.state.on_key(wparam.0 as u32) {
                ctx.finish(outcome, hwnd);
            }
            LRESULT(0)
        }
        WM_CHAR => LRESULT(0),
        WM_PAINT => {
            paint(hwnd, ctx);
            LRESULT(0)
        }
        WM_DESTROY => {
            ctx.done = true;
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn invalidate(hwnd: HWND) {
    // SAFETY: requests a repaint of the whole client area.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn paint(hwnd: HWND, ctx: &mut OverlayCtx) {
    let g = &ctx.gdi;
    // SAFETY: standard BeginPaint/EndPaint around GDI blits into our owned back-buffer.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);

        // 1. bright frozen desktop into the back-buffer.
        let _ = BitBlt(
            g.back_dc,
            0,
            0,
            ctx.width,
            ctx.height,
            Some(g.screen_dc),
            0,
            0,
            SRCCOPY,
        );
        // 2. dim everything by alpha-blending a 1×1 black source stretched over the buffer.
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: DIM_ALPHA,
            AlphaFormat: 0,
        };
        let _ = AlphaBlend(
            g.back_dc, 0, 0, ctx.width, ctx.height, g.dim_dc, 0, 0, 1, 1, blend,
        );

        // 3. restore brightness + outline the current selection or hovered window.
        let highlight = ctx
            .state
            .selection_rect()
            .or_else(|| ctx.state.hovered_window().map(|w| w.bounds));
        if let Some(vrect) = highlight {
            let cr = ctx.to_client(vrect);
            if !cr.is_empty() {
                let _ = BitBlt(
                    g.back_dc,
                    cr.x,
                    cr.y,
                    cr.width,
                    cr.height,
                    Some(g.screen_dc),
                    cr.x,
                    cr.y,
                    SRCCOPY,
                );
                draw_border(g, cr);
            }
            draw_label(ctx, g, vrect);
        }

        // 4. present.
        let _ = BitBlt(
            hdc,
            0,
            0,
            ctx.width,
            ctx.height,
            Some(g.back_dc),
            0,
            0,
            SRCCOPY,
        );
        let _ = EndPaint(hwnd, &ps);
    }
}

fn draw_border(g: &Gdi, cr: Rect) {
    // SAFETY: selects our pen + a hollow brush and strokes a rectangle outline.
    unsafe {
        let old_pen = SelectObject(g.back_dc, HGDIOBJ(g.pen.0));
        let old_brush = SelectObject(g.back_dc, GetStockObject(HOLLOW_BRUSH));
        let _ = Rectangle(g.back_dc, cr.x, cr.y, cr.right(), cr.bottom());
        SelectObject(g.back_dc, old_pen);
        SelectObject(g.back_dc, old_brush);
    }
}

fn draw_label(ctx: &OverlayCtx, g: &Gdi, vrect: Rect) {
    let text = format!("{} × {}", vrect.width, vrect.height);
    let wide: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: GDI text drawing into the back-buffer with our font/colours.
    unsafe {
        let old_font = SelectObject(g.back_dc, HGDIOBJ(g.font.0));
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(g.back_dc, &wide, &mut sz);
        let bounds = Rect::new(ctx.origin.x, ctx.origin.y, ctx.width, ctx.height);
        let pos_v = label_position(ctx.state.cursor(), sz.cx + 10, sz.cy + 6, bounds);
        let cx = pos_v.x - ctx.origin.x;
        let cy = pos_v.y - ctx.origin.y;
        SetBkMode(g.back_dc, OPAQUE);
        SetBkColor(g.back_dc, rgb(20, 22, 26));
        SetTextColor(g.back_dc, rgb(240, 240, 245));
        let _ = TextOutW(g.back_dc, cx + 5, cy + 3, &wide);
        SelectObject(g.back_dc, old_font);
    }
}

fn rgb(r: u8, gc: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((gc as u32) << 8) | ((b as u32) << 16))
}

/// GDI resources backing the overlay: a memory DC with the frozen desktop, a 1×1 black DC for
/// dimming, a back-buffer, a border pen, and a label font.
struct Gdi {
    screen_dc: HDC,
    screen_bmp: HBITMAP,
    screen_old: HGDIOBJ,
    dim_dc: HDC,
    dim_bmp: HBITMAP,
    dim_old: HGDIOBJ,
    back_dc: HDC,
    back_bmp: HBITMAP,
    back_old: HGDIOBJ,
    pen: windows::Win32::Graphics::Gdi::HPEN,
    font: HFONT,
}

impl Gdi {
    fn new(frozen: &Frame, w: i32, h: i32) -> Result<Gdi> {
        // SAFETY: GDI object creation; failures are checked and the Drop impl cleans up.
        unsafe {
            let screen = windows::Win32::Graphics::Gdi::GetDC(None);
            let (screen_dc, screen_bmp, screen_old) = make_dib(screen, w, h, Some(frozen))?;
            let (dim_dc, dim_bmp, dim_old) = make_dib(screen, 1, 1, None)?;

            let back_dc = CreateCompatibleDC(Some(screen));
            let back_bmp = CreateCompatibleBitmap(screen, w, h);
            let back_old = SelectObject(back_dc, HGDIOBJ(back_bmp.0));
            ReleaseDC(None, screen);

            let pen = CreatePen(PS_SOLID, 2, rgb(0, 180, 255));
            let font = CreateFontW(
                -20,
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                w!("Segoe UI"),
            );

            Ok(Gdi {
                screen_dc,
                screen_bmp,
                screen_old,
                dim_dc,
                dim_bmp,
                dim_old,
                back_dc,
                back_bmp,
                back_old,
                pen,
                font,
            })
        }
    }
}

impl Drop for Gdi {
    fn drop(&mut self) {
        // SAFETY: restore selected objects then delete everything we created.
        unsafe {
            SelectObject(self.screen_dc, self.screen_old);
            let _ = DeleteObject(HGDIOBJ(self.screen_bmp.0));
            let _ = DeleteDC(self.screen_dc);
            SelectObject(self.dim_dc, self.dim_old);
            let _ = DeleteObject(HGDIOBJ(self.dim_bmp.0));
            let _ = DeleteDC(self.dim_dc);
            SelectObject(self.back_dc, self.back_old);
            let _ = DeleteObject(HGDIOBJ(self.back_bmp.0));
            let _ = DeleteDC(self.back_dc);
            let _ = DeleteObject(HGDIOBJ(self.pen.0));
            let _ = DeleteObject(HGDIOBJ(self.font.0));
        }
    }
}

/// Create a top-down 32-bpp DIB section in a fresh memory DC, optionally filling it with a
/// frozen frame's pixels (BGRA, stride-aware). Returns the DC, bitmap, and the previously
/// selected object (to restore on drop).
unsafe fn make_dib(
    ref_dc: HDC,
    w: i32,
    h: i32,
    src: Option<&Frame>,
) -> Result<(HDC, HBITMAP, HGDIOBJ)> {
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // negative = top-down, matching our frame layout
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = 0; // BI_RGB

    let mut bits: *mut c_void = null_mut();
    // SAFETY: valid BITMAPINFO; `bits` receives the pixel pointer.
    let bmp = unsafe {
        CreateDIBSection(Some(ref_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
            .context("CreateDIBSection")?
    };
    // SAFETY: a memory DC compatible with the reference DC.
    let dc = unsafe { CreateCompatibleDC(Some(ref_dc)) };
    // SAFETY: select the DIB into its DC.
    let old = unsafe { SelectObject(dc, HGDIOBJ(bmp.0)) };

    let ptr = bits as *mut u8;
    if !ptr.is_null() {
        match src {
            Some(f) => {
                let row = (w as usize) * 4;
                for y in 0..h as usize {
                    // SAFETY: both buffers hold at least `row` bytes at these offsets.
                    unsafe {
                        copy_nonoverlapping(
                            f.data.as_ptr().add(y * f.stride),
                            ptr.add(y * row),
                            row,
                        );
                    }
                }
            }
            None => {
                // SAFETY: zero the (tiny) 1×1 buffer.
                unsafe {
                    write_bytes(ptr, 0, (w as usize) * (h as usize) * 4);
                }
            }
        }
    }
    Ok((dc, bmp, old))
}
