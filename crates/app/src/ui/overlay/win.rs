//! Win32 plumbing for the selection overlay. A `WS_POPUP | WS_EX_TOPMOST` window covers the
//! virtual-desktop bounding box and paints a frozen capture of the desktop, dimmed, with the
//! live selection (region rubber-band or hovered-window highlight) shown undimmed plus a
//! dimension label. A nested modal message loop runs until the user confirms or cancels.
//!
//! The overlay never captures itself: the caller freezes the desktop *before* calling [`select`]
//! and passes that frame in, so the result is cropped from the frozen image (regions) or
//! recaptured cleanly (windows) — the overlay pixels can't leak into either path.

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::Once;

use anyhow::{Context, Result};
use capture_core::{Frame, Point, Rect, WindowInfo};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AlphaBlend, BeginPaint, BitBlt, CreateFontW, CreatePen, CreateSolidBrush, EndPaint, FillRect,
    GetStockObject, GetTextExtentPoint32W, InvalidateRect, Rectangle, SelectObject, SetBkColor,
    SetBkMode, SetTextColor, TextOutW, AC_SRC_OVER, BLENDFUNCTION, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, HBRUSH, HOLLOW_BRUSH, OPAQUE, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, PS_SOLID, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
    GetWindowLongPtrW, LoadCursorW, PostQuitMessage, RegisterClassW, SetWindowLongPtrW, ShowWindow,
    TranslateMessage, CREATESTRUCTW, GWLP_USERDATA, HICON, HMENU, IDC_CROSS, MSG, SW_SHOW, WM_CHAR,
    WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_PAINT,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::state::{crosshair_bars, label_position, Outcome, SelectionState};
use crate::platform::gdi::{copy_frame, GdiObj, ScreenDc, Surface};

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
        crate::platform::bring_to_front(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    // Nested modal loop: runs until the window proc sets `done` (and destroys the window).
    // SAFETY: standard message pump; `msg` is valid for the duration.
    unsafe {
        // `ctx.done` is set from the window proc during `DispatchMessageW` (via the raw pointer),
        // which clippy can't see — hence a `loop` with the check after dispatch, not a `while`.
        let mut msg = MSG::default();
        let mut got_quit = false;
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 == 0 {
                got_quit = true; // WM_QUIT: the whole app is shutting down
                break;
            }
            if r.0 == -1 {
                break; // error
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            if ctx.done {
                break;
            }
        }
        // If a quit/error interrupted us, the window is still alive with a GWLP_USERDATA pointer
        // into `ctx` (about to drop). Destroy it now, then re-post the quit for the outer loop.
        if !ctx.done {
            let _ = DestroyWindow(hwnd);
        }
        if got_quit {
            PostQuitMessage(0);
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

    /// Record the result. The caller then calls [`close`] with no live reference to the ctx:
    /// destroying the window re-enters the window procedure (WM_DESTROY), which makes its own.
    fn conclude(&mut self, outcome: Outcome) {
        self.outcome = Some(outcome);
        self.done = true;
    }
}

/// Destroy the overlay window, which ends the nested loop.
fn close(hwnd: HWND) {
    // SAFETY: `hwnd` is this overlay's window.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn loword(l: LPARAM) -> i32 {
    (l.0 & 0xFFFF) as u16 as i16 as i32
}
fn hiword(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xFFFF) as u16 as i16 as i32
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // A panic must not unwind across this FFI boundary (that aborts the process): run the real
    // handler under the panic guard and fall back to default processing.
    crate::panic_guard::guard(
        "overlay wndproc",
        || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        || unsafe { wndproc_inner(hwnd, msg, wparam, lparam) },
    )
}

unsafe fn wndproc_inner(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
                ctx.conclude(outcome);
                close(hwnd);
            } else {
                invalidate(hwnd);
            }
            LRESULT(0)
        }
        // Swallow the press and cancel on release, so the button-up isn't delivered to the window
        // underneath (which would pop up its context menu after the overlay closes).
        WM_RBUTTONDOWN => LRESULT(0),
        WM_RBUTTONUP => {
            let outcome = ctx.state.on_cancel();
            ctx.conclude(outcome);
            close(hwnd);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if let Some(outcome) = ctx.state.on_key(wparam.0 as u32) {
                ctx.conclude(outcome);
                close(hwnd);
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
            g.back.dc,
            0,
            0,
            ctx.width,
            ctx.height,
            Some(g.screen.dc),
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
            g.back.dc, 0, 0, ctx.width, ctx.height, g.dim.dc, 0, 0, 1, 1, blend,
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
                    g.back.dc,
                    cr.x,
                    cr.y,
                    cr.width,
                    cr.height,
                    Some(g.screen.dc),
                    cr.x,
                    cr.y,
                    SRCCOPY,
                );
                draw_border(g, cr);
            }
        }

        // 4. crosshair through the cursor, under the dimension label.
        draw_crosshair(ctx, g);
        if let Some(vrect) = highlight {
            draw_label(ctx, g, vrect);
        }

        // 5. present.
        let _ = BitBlt(
            hdc,
            0,
            0,
            ctx.width,
            ctx.height,
            Some(g.back.dc),
            0,
            0,
            SRCCOPY,
        );
        let _ = EndPaint(hwnd, &ps);
    }
}

/// Full-width / full-height crosshair through the cursor: a 3 px black edge with a 1 px white
/// line on top, so it stays visible on both light and dark content.
fn draw_crosshair(ctx: &OverlayCtx, g: &Gdi) {
    let c = Point::new(
        ctx.state.cursor().x - ctx.origin.x,
        ctx.state.cursor().y - ctx.origin.y,
    );
    for (grow, brush) in [
        (1, HBRUSH(g.edge_brush.0 .0)),
        (0, HBRUSH(g.line_brush.0 .0)),
    ] {
        for bar in crosshair_bars(c, ctx.width, ctx.height, grow) {
            let r = crate::platform::win_rect(bar);
            // SAFETY: fills a rectangle in our back-buffer DC with a brush we own; GDI clips it.
            unsafe {
                FillRect(g.back.dc, &r, brush);
            }
        }
    }
}

fn draw_border(g: &Gdi, cr: Rect) {
    // SAFETY: selects our pen + a hollow brush and strokes a rectangle outline.
    unsafe {
        let old_pen = SelectObject(g.back.dc, g.pen.0);
        let old_brush = SelectObject(g.back.dc, GetStockObject(HOLLOW_BRUSH));
        let _ = Rectangle(g.back.dc, cr.x, cr.y, cr.right(), cr.bottom());
        SelectObject(g.back.dc, old_pen);
        SelectObject(g.back.dc, old_brush);
    }
}

fn draw_label(ctx: &OverlayCtx, g: &Gdi, vrect: Rect) {
    let text = format!("{} × {}", vrect.width, vrect.height);
    let wide: Vec<u16> = text.encode_utf16().collect();
    // SAFETY: GDI text drawing into the back-buffer with our font/colours.
    unsafe {
        let old_font = SelectObject(g.back.dc, g.font.0);
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(g.back.dc, &wide, &mut sz);
        let bounds = Rect::new(ctx.origin.x, ctx.origin.y, ctx.width, ctx.height);
        let pos_v = label_position(ctx.state.cursor(), sz.cx + 10, sz.cy + 6, bounds);
        let cx = pos_v.x - ctx.origin.x;
        let cy = pos_v.y - ctx.origin.y;
        SetBkMode(g.back.dc, OPAQUE);
        SetBkColor(g.back.dc, rgb(20, 22, 26));
        SetTextColor(g.back.dc, rgb(240, 240, 245));
        let _ = TextOutW(g.back.dc, cx + 5, cy + 3, &wide);
        SelectObject(g.back.dc, old_font);
    }
}

fn rgb(r: u8, gc: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((gc as u32) << 8) | ((b as u32) << 16))
}

/// GDI resources backing the overlay: a memory DC with the frozen desktop, a 1×1 black DC for
/// dimming, a back-buffer, a border pen, and a label font. Each piece owns its handles, so a
/// failure part-way through [`Gdi::new`] releases whatever was already created.
struct Gdi {
    screen: Surface,
    dim: Surface,
    back: Surface,
    pen: GdiObj,
    line_brush: GdiObj,
    edge_brush: GdiObj,
    font: GdiObj,
}

impl Gdi {
    fn new(frozen: &Frame, w: i32, h: i32) -> Result<Gdi> {
        let screen_dc = ScreenDc::get()?;
        // A display change between the freeze and now may leave the frame smaller than the
        // overlay: `copy_frame` clips, and the uncovered part stays black.
        let mut screen = Surface::dib(screen_dc.0, w, h)?;
        if let Some(px) = screen.pixels_mut() {
            copy_frame(frozen, px, w.max(0) as usize * 4);
        }
        let dim = Surface::dib(screen_dc.0, 1, 1)?;
        let back = Surface::compatible(screen_dc.0, w, h)?;
        drop(screen_dc);

        // SAFETY: plain GDI object creation; each handle is owned by a `GdiObj`.
        let (pen, line_brush, edge_brush, font) = unsafe {
            (
                GdiObj::new(CreatePen(PS_SOLID, 2, rgb(255, 0, 0)).0)?,
                GdiObj::new(CreateSolidBrush(rgb(255, 255, 255)).0)?,
                GdiObj::new(CreateSolidBrush(rgb(0, 0, 0)).0)?,
                GdiObj::new(
                    CreateFontW(
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
                    )
                    .0,
                )?,
            )
        };
        Ok(Gdi {
            screen,
            dim,
            back,
            pen,
            line_brush,
            edge_brush,
            font,
        })
    }
}
