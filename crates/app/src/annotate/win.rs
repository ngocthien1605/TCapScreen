//! The annotation editor window: a normal, resizable top-level window that shows a screenshot
//! with a small toolbar (Rectangle, Text, Undo, Save, Copy). It runs on the main UI thread's
//! message loop (like the flyout), so it needs no thread of its own.
//!
//! One GDI routine ([`paint_items`]) draws the annotations both on screen and into the exported
//! image, so what you see is exactly what gets saved.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::Once;

use anyhow::{anyhow, Context, Result};
use capture_core::{Point, Rect};
use imaging::Rgba;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GdiFlush,
    GetStockObject, IntersectClipRect, InvalidateRect, Rectangle, ReleaseDC, RestoreDC, SaveDC,
    SelectObject, SetBkMode, SetStretchBltMode, SetTextColor, StretchBlt, BITMAPINFO,
    BITMAPINFOHEADER, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_GUI_FONT,
    DIB_RGB_COLORS, DT_NOPREFIX, DT_WORDBREAK, HALFTONE, HBITMAP, HDC, HGDIOBJ, HOLLOW_BRUSH,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_INSIDEFRAME, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::CheckRadioButton;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN,
    VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
    GetSystemMetrics, GetWindowLongPtrW, LoadCursorW, MessageBoxW, RegisterClassW, SendMessageW,
    SetWindowLongPtrW, SetWindowTextW, ShowWindow, CW_USEDEFAULT, GWLP_USERDATA, HMENU, IDC_CROSS,
    IDNO, IDYES, MB_ICONQUESTION, MB_YESNOCANCEL, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW,
    WINDOW_EX_STYLE, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SETFONT, WM_SIZE,
    WM_SYSKEYDOWN, WNDCLASSW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use super::model::{
    enter_action, font_px, Editor, EnterAction, EscapeResult, Item, Tool, Typing, View,
};

const CLASS_NAME: PCWSTR = w!("RustShotAnnotate");
static REGISTER: Once = Once::new();

const TOOLBAR_H: i32 = 40;
const MARGIN: i32 = 8;
/// Outline thickness in image pixels.
const LINE_PX: i32 = 3;
const RED: COLORREF = COLORREF(0x0000_00FF); // 0x00BBGGRR

const ID_RECT: i32 = 201;
const ID_TEXT: i32 = 202;
const ID_UNDO: i32 = 203;
const ID_SAVE: i32 = 204;
const ID_COPY: i32 = 205;

// Button styles (kept local, like the Settings dialog does).
const BS_PUSHLIKE: u32 = 0x1000;
const BS_RADIOBUTTON: u32 = 0x4;
const WS_GROUP: u32 = 0x0002_0000;

/// Per-window state, owned by the window (freed in `WM_NCDESTROY`).
struct Ctx {
    editor: Editor,
    image: Rgba,
    /// The screenshot as a DIB section selected into `src_dc`, for fast scaled blits.
    src_dc: HDC,
    src_bmp: HBITMAP,
    src_old: HGDIOBJ,
    source: PathBuf,
    /// Last view mapping computed by layout.
    view: View,
    /// True when there are annotations not yet saved.
    dirty: bool,
    /// Pending UTF-16 high surrogate from a `WM_CHAR` pair.
    high_surrogate: Option<u16>,
}

impl Drop for Ctx {
    fn drop(&mut self) {
        // SAFETY: restore the DC's original object, then free the bitmap and the DC we created.
        unsafe {
            SelectObject(self.src_dc, self.src_old);
            let _ = DeleteObject(HGDIOBJ(self.src_bmp.0));
            let _ = DeleteDC(self.src_dc);
        }
    }
}

thread_local! {
    /// The state of a window being created, until its `WM_NCCREATE` adopts it. Whatever is
    /// still here after `CreateWindowExW` returns was never adopted and is dropped normally, so
    /// the state is freed exactly once however creation fails.
    static PENDING: RefCell<Option<Box<Ctx>>> = const { RefCell::new(None) };
}

/// Open the editor on the screenshot at `path`. Returns immediately; the window lives on the UI
/// thread's message loop until the user closes it.
pub fn open(path: &Path) {
    if let Err(e) = try_open(path) {
        crate::notify::error("The annotation editor could not be opened.", e);
    }
}

fn try_open(path: &Path) -> Result<()> {
    ensure_class()?;
    let image = imaging::load_rgba(path).context("loading the screenshot")?;
    let (iw, ih) = (image.width() as i32, image.height() as i32);

    let (src_dc, src_bmp, src_old) = make_source_dc(&image)?;
    let ctx = Box::new(Ctx {
        editor: Editor::new(iw, ih),
        image,
        src_dc,
        src_bmp,
        src_old,
        source: path.to_path_buf(),
        view: View::IDENTITY,
        dirty: false,
        high_surrogate: None,
    });

    // Size the window to the image where it fits, bounded by the screen.
    // SAFETY: plain metric queries and window creation. The window adopts `ctx` in
    // WM_NCCREATE (via PENDING) and frees it in WM_NCDESTROY.
    let created = unsafe {
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let cw = (iw + 2 * MARGIN).clamp(640, (sw * 9 / 10).max(640));
        let ch = (ih + TOOLBAR_H + 2 * MARGIN).clamp(420, (sh * 85 / 100).max(420));
        let mut r = RECT {
            left: 0,
            top: 0,
            right: cw,
            bottom: ch,
        };
        let _ = AdjustWindowRect(&mut r, WS_OVERLAPPEDWINDOW, false);
        let hinstance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let title = HSTRING::from(title_for(path, None));
        PENDING.with(|p| *p.borrow_mut() = Some(ctx));
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS_NAME,
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            r.right - r.left,
            r.bottom - r.top,
            None,
            Some(HMENU(null_mut())),
            Some(hinstance.into()),
            None,
        )
    };
    // Not adopted (creation failed before WM_NCCREATE): free it here.
    drop(PENDING.with(|p| p.borrow_mut().take()));
    let hwnd = created.context("creating the annotation window")?;

    if let Err(e) = create_toolbar(hwnd) {
        // SAFETY: our own window, never shown; WM_NCDESTROY frees its state.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return Err(e);
    }
    // SAFETY: show and focus our own window so typed characters reach it.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetFocus(Some(hwnd));
    }
    Ok(())
}

fn title_for(path: &Path, status: Option<&str>) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match status {
        Some(s) => format!("RustShot — Annotate — {name} — {s}"),
        None => format!("RustShot — Annotate — {name}"),
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
                // The red crosshair; the system cross if it cannot be built.
                hCursor: super::cursor::create()
                    .or_else(|| LoadCursorW(None, IDC_CROSS).ok())
                    .unwrap_or_default(),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                err = Some(windows::core::Error::from_thread());
            }
        }
    });
    match err {
        Some(e) => Err(e).context("registering the annotation window class"),
        None => Ok(()),
    }
}

fn create_toolbar(parent: HWND) -> Result<()> {
    let items: [(i32, PCWSTR, i32, u32); 5] = [
        (
            ID_RECT,
            w!("Rectangle"),
            8,
            BS_PUSHLIKE | BS_RADIOBUTTON | WS_GROUP,
        ),
        (ID_TEXT, w!("Text"), 92, BS_PUSHLIKE | BS_RADIOBUTTON),
        (ID_UNDO, w!("Undo"), 188, 0),
        (ID_SAVE, w!("Save"), 256, 0),
        (ID_COPY, w!("Copy"), 324, 0),
    ];
    // SAFETY: standard BUTTON child creation, each given the stock GUI font.
    unsafe {
        let hinstance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let font = GetStockObject(DEFAULT_GUI_FONT);
        for (id, label, x, extra) in items {
            let width = if id == ID_RECT || id == ID_TEXT {
                80
            } else {
                60
            };
            let h = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                label,
                windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | extra,
                ),
                x,
                6,
                width,
                28,
                Some(parent),
                Some(HMENU(id as isize as *mut c_void)),
                Some(hinstance.into()),
                None,
            )
            .context("creating a toolbar button")?;
            SendMessageW(
                h,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        let _ = CheckRadioButton(parent, ID_RECT, ID_TEXT, ID_RECT);
    }
    Ok(())
}

fn ctx_of<'a>(hwnd: HWND) -> Option<&'a mut Ctx> {
    // SAFETY: userdata is either null or the Box<Ctx> pointer installed at creation and freed in
    // WM_NCDESTROY (after which no more messages reach us).
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Ctx;
    if p.is_null() {
        None
    } else {
        Some(unsafe { &mut *p })
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // A panic must not unwind across this FFI boundary (that aborts the process): run the real
    // handler under the panic guard and fall back to default processing.
    crate::panic_guard::guard(
        "annotate editor wndproc",
        || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        || unsafe { wndproc_inner(hwnd, msg, wparam, lparam) },
    )
}

unsafe fn wndproc_inner(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // Adopt the pending state: from here the window owns it until WM_NCDESTROY.
        if let Some(ctx) = PENDING.with(|p| p.borrow_mut().take()) {
            // SAFETY: store the owned pointer in our own window's userdata.
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(ctx) as isize) };
        }
        // SAFETY: default processing of WM_NCCREATE.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let Some(ctx) = ctx_of(hwnd) else {
        // SAFETY: default handling before/after our state exists.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    };

    match msg {
        WM_SIZE => {
            relayout(hwnd, ctx);
            redraw(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1), // we paint everything (double-buffered)
        WM_PAINT => {
            paint(hwnd, ctx);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            // SAFETY: capture the mouse while dragging.
            unsafe {
                let _ = SetCapture(hwnd);
                let _ = SetFocus(Some(hwnd));
            }
            let p = to_image(ctx, lparam);
            ctx.editor.on_down(p);
            redraw(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if ctx.editor.drag_rect().is_some() {
                let p = to_image(ctx, lparam);
                ctx.editor.on_move(p);
                redraw(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            // SAFETY: release our capture.
            unsafe {
                let _ = ReleaseCapture();
            }
            let before = ctx.editor.items().len();
            let p = to_image(ctx, lparam);
            ctx.editor.on_up(p);
            if ctx.editor.items().len() != before {
                ctx.dirty = true;
            }
            redraw(hwnd);
            LRESULT(0)
        }
        WM_CHAR => {
            handle_char(hwnd, ctx, wparam.0 as u16);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            handle_key(hwnd, ctx, wparam.0 as u16);
            LRESULT(0)
        }
        // Alt+Enter arrives as a *system* key message (and produces no WM_CHAR).
        WM_SYSKEYDOWN if wparam.0 as u16 == VK_RETURN.0 => {
            handle_key(hwnd, ctx, wparam.0 as u16);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            match id {
                ID_RECT | ID_TEXT => {
                    let before = ctx.editor.items().len();
                    ctx.editor.set_tool(if id == ID_RECT {
                        Tool::Rect
                    } else {
                        Tool::Text
                    });
                    if ctx.editor.items().len() != before {
                        ctx.dirty = true;
                    }
                    // SAFETY: keep the two tool buttons mutually exclusive.
                    unsafe {
                        let _ = CheckRadioButton(hwnd, ID_RECT, ID_TEXT, id);
                    }
                }
                ID_UNDO => {
                    ctx.editor.undo();
                    ctx.dirty = !ctx.editor.items().is_empty();
                }
                ID_SAVE => save(hwnd, ctx),
                ID_COPY => copy(hwnd, ctx),
                _ => {}
            }
            // Buttons grab focus on click; typed characters must keep reaching the canvas.
            // SAFETY: refocus our own window.
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
            redraw(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            finish_typing(ctx);
            let dirty = ctx.dirty;
            // `ctx` is not used past this point: the message box below pumps messages, and
            // WM_PAINT & co. re-enter this procedure with their own reference to the state.
            if confirm_close(hwnd, dirty) {
                // SAFETY: destroy our own window; WM_NCDESTROY frees the ctx.
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // SAFETY: last message: take back ownership and free GDI + the Box exactly once.
            unsafe {
                let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Ctx;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if !p.is_null() {
                    drop(Box::from_raw(p)); // Ctx's Drop releases its GDI objects
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        // SAFETY: default handling.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn handle_char(hwnd: HWND, ctx: &mut Ctx, unit: u16) {
    match unit {
        0x08 => ctx.editor.on_backspace(),
        // Enter (plain, Shift, Ctrl) is handled in `handle_key`, where Shift/Alt can be told
        // apart; the character message must not also act on it.
        0x0D | 0x0A => {}
        u if (0xD800..0xDC00).contains(&u) => {
            ctx.high_surrogate = Some(u);
            return;
        }
        u => {
            let ch = match (ctx.high_surrogate.take(), u) {
                (Some(hi), lo) if (0xDC00..0xE000).contains(&lo) => {
                    char::decode_utf16([hi, lo]).next().and_then(|r| r.ok())
                }
                (_, u) => char::from_u32(u as u32),
            };
            if let Some(c) = ch {
                ctx.editor.on_char(c);
            }
        }
    }
    redraw(hwnd);
}

fn handle_key(hwnd: HWND, ctx: &mut Ctx, vk: u16) {
    // SAFETY: reads a modifier's state for the message being processed.
    let down = |k: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe {
        GetKeyState(k.0 as i32) < 0
    };
    let ctrl = down(VK_CONTROL);
    if vk == VK_RETURN.0 {
        match enter_action(down(VK_SHIFT), down(VK_MENU)) {
            EnterAction::NewLine => ctx.editor.on_newline(),
            EnterAction::Commit => {
                let before = ctx.editor.items().len();
                ctx.editor.on_enter();
                if ctx.editor.items().len() != before {
                    ctx.dirty = true;
                }
            }
        }
    } else if vk == VK_ESCAPE.0 {
        if ctx.editor.on_escape() == EscapeResult::Nothing {
            // SAFETY: ask ourselves to close (runs the unsaved-changes prompt).
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(hwnd),
                    WM_CLOSE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        }
    } else if ctrl && vk == b'Z' as u16 {
        ctx.editor.undo();
        ctx.dirty = !ctx.editor.items().is_empty();
    } else if ctrl && vk == b'S' as u16 {
        save(hwnd, ctx);
    } else if ctrl && vk == b'C' as u16 && !ctx.editor.is_typing() {
        copy(hwnd, ctx);
    }
    redraw(hwnd);
}

/// Window (client) coordinates from a mouse message → image pixels.
fn to_image(ctx: &Ctx, lparam: LPARAM) -> Point {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    ctx.view.to_image(x, y)
}

fn relayout(hwnd: HWND, ctx: &mut Ctx) {
    let mut rc = RECT::default();
    // SAFETY: reads our client rect.
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
    }
    let area = Rect::new(
        MARGIN,
        TOOLBAR_H + MARGIN,
        (rc.right - 2 * MARGIN).max(1),
        (rc.bottom - TOOLBAR_H - 2 * MARGIN).max(1),
    );
    ctx.view = View::fit(area, ctx.image.width() as i32, ctx.image.height() as i32);
}

fn redraw(hwnd: HWND) {
    // SAFETY: request a repaint of the whole window.
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn paint(hwnd: HWND, ctx: &mut Ctx) {
    if ctx.view == View::IDENTITY {
        relayout(hwnd, ctx);
    }
    // SAFETY: standard double-buffered paint into a compatible bitmap, then one BitBlt.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (cw, ch) = (rc.right.max(1), rc.bottom.max(1));

        let back = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, cw, ch);
        let old = SelectObject(back, HGDIOBJ(bmp.0));

        let bg = CreateSolidBrush(COLORREF(0x0030_2C2A));
        FillRect(back, &rc, bg);
        let _ = DeleteObject(HGDIOBJ(bg.0));

        let v = ctx.view;
        let (iw, ih) = (ctx.image.width() as i32, ctx.image.height() as i32);
        SetStretchBltMode(back, HALFTONE);
        let _ = StretchBlt(
            back,
            v.ox,
            v.oy,
            v.px(iw),
            v.px(ih),
            Some(ctx.src_dc),
            0,
            0,
            iw,
            ih,
            SRCCOPY,
        );

        // Keep annotations inside the picture while previewing.
        let saved = SaveDC(back);
        IntersectClipRect(back, v.ox, v.oy, v.ox + v.px(iw), v.oy + v.px(ih));
        paint_items(
            back,
            ctx.editor.items(),
            ctx.editor.drag_rect(),
            ctx.editor.typing(),
            v,
        );
        let _ = RestoreDC(back, saved);

        let _ = BitBlt(hdc, 0, 0, cw, ch, Some(back), 0, 0, SRCCOPY);
        SelectObject(back, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(back);
        let _ = EndPaint(hwnd, &ps);
    }
}

/// Draw all annotations with `view` mapping image px → `hdc` px. Used for the live preview and,
/// with [`View::IDENTITY`], for the exported image.
fn paint_items(
    hdc: HDC,
    items: &[Item],
    preview: Option<Rect>,
    typing: Option<&Typing>,
    view: View,
) {
    // SAFETY: GDI pen/brush selection and drawing on a DC we are given; objects are restored and
    // deleted before returning.
    unsafe {
        let pen = CreatePen(PS_INSIDEFRAME, view.px(LINE_PX).max(1), RED);
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let old_brush = SelectObject(hdc, GetStockObject(HOLLOW_BRUSH));

        let outline = |r: Rect| {
            let vr = view_rect(view, r);
            let _ = Rectangle(hdc, vr.left, vr.top, vr.right, vr.bottom);
        };

        for item in items {
            match item {
                Item::Rect(r) => outline(*r),
                Item::Text { rect, text } => {
                    outline(*rect);
                    draw_text(hdc, *rect, text, view);
                }
            }
        }
        if let Some(r) = preview {
            outline(r);
        }
        if let Some(t) = typing {
            outline(t.rect);
            // A trailing bar stands in for the caret while typing.
            draw_text(hdc, t.rect, &format!("{}|", t.text), view);
        }

        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
}

fn view_rect(view: View, r: Rect) -> RECT {
    RECT {
        left: view.ox + view.px(r.x),
        top: view.oy + view.px(r.y),
        right: view.ox + view.px(r.right()),
        bottom: view.oy + view.px(r.bottom()),
    }
}

/// Word-wrapped red text inside `rect`, clipped to it.
fn draw_text(hdc: HDC, rect: Rect, text: &str, view: View) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() {
        return;
    }
    let outer = view_rect(view, rect);
    let pad = view.px(5);
    let mut inner = RECT {
        left: outer.left + pad,
        top: outer.top + pad,
        right: outer.right - pad,
        bottom: outer.bottom - pad,
    };
    // SAFETY: GDI font creation, clipped text drawing, and cleanup of what we created.
    unsafe {
        let font = CreateFontW(
            -view.px(font_px(rect)).max(6),
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
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        let saved = SaveDC(hdc);
        IntersectClipRect(hdc, outer.left, outer.top, outer.right, outer.bottom);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, RED);
        DrawTextW(hdc, &mut wide, &mut inner, DT_WORDBREAK | DT_NOPREFIX);
        let _ = RestoreDC(hdc, saved);
        SelectObject(hdc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
    }
}

// ----------------------------------------------------------------------------------------------
// DIBs and export
// ----------------------------------------------------------------------------------------------

/// Create a top-down 32-bit DIB section the size of `img`, filled with its pixels, selected into a
/// new memory DC. Returns `(dc, bitmap, previously-selected object)`; the caller cleans up.
fn make_source_dc(img: &Rgba) -> Result<(HDC, HBITMAP, HGDIOBJ)> {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = 0; // BI_RGB

    // SAFETY: DIB section creation; `bits` points at w*h*4 bytes owned by the bitmap, which we
    // fill before use. Failures clean up what was created.
    unsafe {
        let screen = windows::Win32::Graphics::Gdi::GetDC(None);
        let mut bits: *mut c_void = null_mut();
        let bmp = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
        let bmp = match bmp {
            Ok(b) if !bits.is_null() => b,
            Ok(b) => {
                ReleaseDC(None, screen);
                let _ = DeleteObject(HGDIOBJ(b.0));
                return Err(anyhow!("CreateDIBSection returned no pixel buffer"));
            }
            Err(e) => {
                ReleaseDC(None, screen);
                return Err(anyhow!("CreateDIBSection failed: {e}"));
            }
        };
        let dc = CreateCompatibleDC(Some(screen));
        ReleaseDC(None, screen);
        let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w as usize) * (h as usize) * 4);
        let (dst_px, _) = dst.as_chunks_mut::<4>();
        let (src_px, _) = img.as_raw().as_chunks::<4>();
        for (d, s) in dst_px.iter_mut().zip(src_px) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = 255;
        }
        let old = SelectObject(dc, HGDIOBJ(bmp.0));
        Ok((dc, bmp, old))
    }
}

/// Render `items` onto a copy of `img` at full resolution (what Save/Copy produce).
pub fn render(img: &Rgba, items: &[Item]) -> Result<Rgba> {
    let (dc, bmp, old) = make_source_dc(img)?;
    let (w, h) = (img.width(), img.height());
    // SAFETY: draw into our DIB-backed DC, flush GDI, then copy the pixels back out before the
    // objects are released.
    let out = unsafe {
        paint_items(dc, items, None, None, View::IDENTITY);
        let _ = GdiFlush();
        let mut bits_bmp = windows::Win32::Graphics::Gdi::BITMAP::default();
        windows::Win32::Graphics::Gdi::GetObjectW(
            HGDIOBJ(bmp.0),
            std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAP>() as i32,
            Some(&mut bits_bmp as *mut _ as *mut c_void),
        );
        let src = std::slice::from_raw_parts(
            bits_bmp.bmBits as *const u8,
            (w as usize) * (h as usize) * 4,
        );
        let mut rgba = Vec::with_capacity(src.len());
        for p in src.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        rgba
    };
    // SAFETY: release the GDI objects we created above.
    unsafe {
        SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(dc);
    }
    Rgba::from_raw(w, h, out).ok_or_else(|| anyhow!("rendered buffer had the wrong size"))
}

/// Where an annotated copy goes: the original's folder and `<stem>_annotated.<ext>`, which
/// `save_image_unique` numbers (`_annotated_2`, …) rather than overwrite.
fn annotated_target(source: &Path) -> (PathBuf, String) {
    let dir = source.parent().unwrap_or_else(|| Path::new("."));
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "screenshot".into());
    let ext = source
        .extension()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "png".into());
    (dir.to_path_buf(), format!("{stem}_annotated.{ext}"))
}

fn set_status(hwnd: HWND, ctx: &Ctx, status: &str) {
    let title = HSTRING::from(title_for(&ctx.source, Some(status)));
    // SAFETY: sets our own window's title.
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
    }
}

/// Commit any text being typed so Save/Copy include it.
fn finish_typing(ctx: &mut Ctx) {
    if ctx.editor.on_enter() {
        ctx.dirty = true;
    }
}

fn save(hwnd: HWND, ctx: &mut Ctx) {
    finish_typing(ctx);
    if ctx.editor.items().is_empty() {
        set_status(hwnd, ctx, "nothing to save yet");
        return;
    }
    let quality = crate::dispatch::with_app(|a| a.settings.screenshot.jpeg_quality).unwrap_or(90);
    let result = render(&ctx.image, ctx.editor.items()).and_then(|img| {
        let (dir, name) = annotated_target(&ctx.source);
        imaging::save_image_unique(&img, &dir, &name, quality).map_err(|e| anyhow!("{e}"))
    });
    match result {
        Ok(path) => {
            ctx.dirty = false;
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            tracing::info!("saved annotated screenshot to {}", path.display());
            set_status(hwnd, ctx, &format!("saved as {name}"));
        }
        Err(e) => {
            tracing::warn!("could not save the annotated screenshot: {e:#}");
            set_status(hwnd, ctx, "save failed");
        }
    }
}

fn copy(hwnd: HWND, ctx: &mut Ctx) {
    finish_typing(ctx);
    let result =
        render(&ctx.image, ctx.editor.items()).and_then(|img| crate::clipboard::copy_image(&img));
    match result {
        Ok(()) => set_status(hwnd, ctx, "copied to clipboard"),
        Err(e) => {
            tracing::warn!("could not copy the annotated screenshot: {e:#}");
            set_status(hwnd, ctx, "copy failed");
        }
    }
}

/// On close with unsaved annotations, ask what to do. Returns true if the window may close.
/// Whether the editor may close; offers to save unsaved work first. Takes no state reference
/// across the message box (it pumps messages); the state is fetched afresh to save.
fn confirm_close(hwnd: HWND, dirty: bool) -> bool {
    if !dirty {
        return true;
    }
    let text = HSTRING::from("Save the annotated screenshot before closing?");
    let title = HSTRING::from("RustShot");
    // SAFETY: standard modal message box owned by this window.
    let answer = unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        )
    };
    if answer == IDYES {
        let Some(ctx) = ctx_of(hwnd) else {
            return true;
        };
        save(hwnd, ctx);
        !ctx.dirty // closes only if the save worked
    } else {
        answer == IDNO
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn white(w: u32, h: u32) -> Rgba {
        Rgba::from_raw(w, h, vec![255; (w * h * 4) as usize]).unwrap()
    }

    fn px(img: &Rgba, x: u32, y: u32) -> [u8; 4] {
        img.get_pixel(x, y).0
    }

    #[test]
    fn rectangle_is_drawn_in_pure_red_inside_its_bounds() {
        let out = render(&white(120, 80), &[Item::Rect(Rect::new(10, 10, 60, 30))]).unwrap();
        // Border pixels (3 px wide, inside the frame) are pure red...
        assert_eq!(px(&out, 11, 25), [255, 0, 0, 255], "left edge");
        assert_eq!(px(&out, 40, 11), [255, 0, 0, 255], "top edge");
        // ...the interior and everything outside is untouched.
        assert_eq!(px(&out, 40, 25), [255, 255, 255, 255], "interior");
        assert_eq!(px(&out, 100, 70), [255, 255, 255, 255], "outside");
        assert_eq!(px(&out, 5, 5), [255, 255, 255, 255], "outside corner");
    }

    #[test]
    fn text_box_draws_red_text_inside_a_red_border() {
        let rect = Rect::new(5, 5, 110, 50);
        let out = render(
            &white(120, 80),
            &[Item::Text {
                rect,
                text: "Hello".into(),
            }],
        )
        .unwrap();
        assert_eq!(px(&out, 6, 30), [255, 0, 0, 255], "border");
        // Count text pixels strictly inside the box (clear of the 3 px border + 5 px padding).
        let mut red = 0;
        for y in 12..50 {
            for x in 12..110 {
                let p = px(&out, x, y);
                if p[0] > 200 && p[1] < 120 && p[2] < 120 {
                    red += 1;
                }
            }
        }
        assert!(red > 30, "expected red glyph pixels, found {red}");
    }

    /// Count text-coloured pixels in the rows `y0..y1` of the box interior.
    fn red_in_rows(img: &Rgba, y0: u32, y1: u32) -> usize {
        let mut n = 0;
        for y in y0..y1 {
            for x in 12..190 {
                let p = px(img, x, y);
                if p[0] > 200 && p[1] < 120 && p[2] < 120 {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn a_line_break_puts_the_text_on_a_second_line() {
        let rect = Rect::new(5, 5, 190, 130); // tall box -> 32 px font, ~43 px per line
        let one = render(
            &white(200, 140),
            &[Item::Text {
                rect,
                text: "A".into(),
            }],
        )
        .unwrap();
        let two = render(
            &white(200, 140),
            &[Item::Text {
                rect,
                text: "A\nA".into(),
            }],
        )
        .unwrap();
        // Rows 55..95 are only reached by a second line of text.
        assert_eq!(
            red_in_rows(&one, 55, 95),
            0,
            "single line stays on the first row"
        );
        assert!(
            red_in_rows(&two, 55, 95) > 30,
            "second line is drawn lower down"
        );
        // The first line is identical in both.
        assert!(red_in_rows(&two, 10, 50) > 30);
    }

    #[test]
    fn export_keeps_size_and_is_opaque_and_leaves_source_pixels() {
        let mut src = white(64, 48);
        let i = ((24 * 64 + 30) * 4) as usize;
        src.as_mut()[i..i + 4].copy_from_slice(&[10, 20, 30, 255]);
        let out = render(&src, &[Item::Rect(Rect::new(2, 2, 10, 10))]).unwrap();
        assert_eq!((out.width(), out.height()), (64, 48));
        assert_eq!(px(&out, 30, 24), [10, 20, 30, 255], "untouched pixel kept");
        assert!(out.pixels().all(|p| p.0[3] == 255), "all pixels opaque");
    }

    #[test]
    fn no_items_renders_an_identical_image() {
        let src = white(32, 32);
        assert_eq!(render(&src, &[]).unwrap(), src);
    }

    #[test]
    fn annotated_copy_sits_next_to_the_original_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("rustshot-annot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("shot.png");
        let (tdir, name) = annotated_target(&src);
        assert_eq!(
            (tdir.as_path(), name.as_str()),
            (dir.as_path(), "shot_annotated.png")
        );
        let img = white(4, 4);
        let first = imaging::save_image_unique(&img, &tdir, &name, 90).unwrap();
        let second = imaging::save_image_unique(&img, &tdir, &name, 90).unwrap();
        assert_eq!(first, dir.join("shot_annotated.png"));
        assert_eq!(second, dir.join("shot_annotated_2.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
