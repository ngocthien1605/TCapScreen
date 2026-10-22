//! The Settings dialog: a modal Win32 dialog built from an in-memory `DLGTEMPLATE` (so tab
//! order, the default button, and Enter/Esc come for free). The template builder is pure and
//! unit-tested; on OK the edited settings are persisted and applied live (config saved, output
//! folder updated, hotkeys re-registered, run-at-startup toggled).

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use config::{HotkeyMap, ImageFormat, Settings};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    CheckDlgButton, IsDlgButtonChecked, BST_CHECKED, DLG_BUTTON_CHECK_STATE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, DefWindowProcW, DialogBoxIndirectParamW, EndDialog, GetDlgItem, GetDlgItemInt,
    GetDlgItemTextW, GetParent, GetWindowLongPtrW, MessageBoxW, PostMessageW, SetDlgItemInt,
    SetDlgItemTextW, SetTimer, SetWindowLongPtrW, SetWindowTextW, DLGTEMPLATE, GWLP_USERDATA,
    GWLP_WNDPROC, IDCANCEL, IDOK, MB_ICONWARNING, MB_OK, WINDOW_LONG_PTR_INDEX, WM_CHAR, WM_CLOSE,
    WM_COMMAND, WM_GETDLGCODE, WM_INITDIALOG, WM_KEYDOWN, WM_KEYUP, WM_SYSCHAR, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_TIMER, WNDPROC,
};

use crate::dispatch;

/// `DWLP_USER` dialog-extra-bytes offset (x64: DWLP_DLGPROC + sizeof(DLGPROC) = 16). Not
/// exported by the `windows` crate, so defined here for the dialog's user pointer.
const DWLP_USER: i32 = 16;

// Control ids.
const CHK_CURSOR: i32 = 1001;
const CHK_CLIP: i32 = 1002;
const CHK_MIC: i32 = 1003;
const CHK_SYS: i32 = 1004;
const CHK_STARTUP: i32 = 1005;
const CHK_JPEG: i32 = 1006;
const EDIT_QUALITY: i32 = 1010;
const EDIT_FPS: i32 = 1011;
const EDIT_FOLDER: i32 = 1012;
const BTN_BROWSE: i32 = 1013;
// Six editable shortcut fields (in a fixed order) + per-row "Modify" buttons + restore button.
const EDIT_HK_BASE: i32 = 1020;
const BTN_RESTORE_HK: i32 = 1030;
const BTN_MODIFY_BASE: i32 = 1040;
// The capture sub-dialog's single edit control.
const EDIT_CAP: i32 = 1101;

// `DLGC_WANTALLKEYS`: tell the dialog manager to route every key (incl. Tab/Enter/Esc/arrows)
// to the control instead of using them for navigation. Not re-exported conveniently, so local.
const DLGC_WANTALLKEYS: isize = 0x0004;

// Virtual-key codes we special-case during capture (avoids pulling the typed VK constants in).
const VK_BACK: u16 = 0x08;
const VK_RETURN: u16 = 0x0D;
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_ESCAPE: u16 = 0x1B;
const VK_SNAPSHOT: u16 = 0x2C;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;

/// Friendly labels for the six hotkey rows, in the fixed order shared by `HotkeyMap`'s fields,
/// [`map_to_strings`], and [`strings_to_map`].
const HK_LABELS: [&str; 6] = [
    "Region screenshot",
    "Window screenshot",
    "Monitor screenshot",
    "All monitors",
    "Start recording",
    "Stop recording",
];

// Standard control class atoms (prefixed with 0xFFFF in a DLGITEMTEMPLATE).
const ATOM_BUTTON: u16 = 0x0080;
const ATOM_EDIT: u16 = 0x0081;
const ATOM_STATIC: u16 = 0x0082;

// Window/dialog/control style bits (avoids pulling the typed constants into the template).
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const DS_SETFONT: u32 = 0x40;
const DS_MODALFRAME: u32 = 0x80;
const BS_AUTOCHECKBOX: u32 = 0x3;
const BS_DEFPUSHBUTTON: u32 = 0x1;
const ES_AUTOHSCROLL: u32 = 0x80;

/// Builds an in-memory classic `DLGTEMPLATE` (+ items). Pure and unit-testable.
struct DlgBuilder {
    buf: Vec<u8>,
    items: u16,
    cdit_pos: usize,
}

impl DlgBuilder {
    #[allow(clippy::too_many_arguments)]
    fn new(
        style: u32,
        x: i16,
        y: i16,
        cx: i16,
        cy: i16,
        title: &str,
        font_pt: u16,
        font: &str,
    ) -> Self {
        let mut b = DlgBuilder {
            buf: Vec::new(),
            items: 0,
            cdit_pos: 0,
        };
        b.w32(style | DS_SETFONT);
        b.w32(0); // dwExtendedStyle
        b.cdit_pos = b.buf.len();
        b.w16(0); // cdit (patched in build)
        b.wi16(x);
        b.wi16(y);
        b.wi16(cx);
        b.wi16(cy);
        b.w16(0); // menu: none
        b.w16(0); // window class: default
        b.wstr(title);
        b.w16(font_pt);
        b.wstr(font);
        b
    }

    #[allow(clippy::too_many_arguments)]
    fn item(
        &mut self,
        style: u32,
        x: i16,
        y: i16,
        cx: i16,
        cy: i16,
        id: u16,
        atom: u16,
        text: &str,
    ) {
        self.align();
        self.w32(style);
        self.w32(0); // dwExtendedStyle
        self.wi16(x);
        self.wi16(y);
        self.wi16(cx);
        self.wi16(cy);
        self.w16(id);
        self.w16(0xFFFF); // class is an ordinal…
        self.w16(atom); // …of this standard control
        self.wstr(text);
        self.w16(0); // creation-data byte count
        self.items += 1;
    }

    fn build(mut self) -> Vec<u8> {
        let c = self.items.to_le_bytes();
        self.buf[self.cdit_pos] = c[0];
        self.buf[self.cdit_pos + 1] = c[1];
        self.buf
    }

    fn w16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn wi16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn w32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn wstr(&mut self, s: &str) {
        for u in s.encode_utf16() {
            self.w16(u);
        }
        self.w16(0);
    }
    fn align(&mut self) {
        while !self.buf.len().is_multiple_of(4) {
            self.buf.push(0);
        }
    }
}

/// Convert the byte template into a DWORD-aligned buffer suitable for `DialogBoxIndirectParamW`.
fn to_aligned(bytes: &[u8]) -> Vec<u32> {
    let mut v = bytes.to_vec();
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
    let (chunks, _) = v.as_chunks::<4>();
    chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

fn build_template() -> Vec<u8> {
    let chk = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX;
    let edit = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL;
    let stat = WS_CHILD | WS_VISIBLE;
    let btn = WS_CHILD | WS_VISIBLE | WS_TABSTOP;

    let mut b = DlgBuilder::new(
        WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME,
        40,
        40,
        250,
        296,
        "RustShot Settings",
        9,
        "Segoe UI",
    );

    b.item(
        chk,
        10,
        8,
        190,
        11,
        CHK_CURSOR as u16,
        ATOM_BUTTON,
        "Include cursor in screenshots",
    );
    b.item(
        chk,
        10,
        22,
        190,
        11,
        CHK_CLIP as u16,
        ATOM_BUTTON,
        "Copy screenshots to clipboard",
    );
    b.item(
        chk,
        10,
        36,
        190,
        11,
        CHK_MIC as u16,
        ATOM_BUTTON,
        "Record microphone",
    );
    b.item(
        chk,
        10,
        50,
        190,
        11,
        CHK_SYS as u16,
        ATOM_BUTTON,
        "Record system audio",
    );
    b.item(
        chk,
        10,
        64,
        190,
        11,
        CHK_STARTUP as u16,
        ATOM_BUTTON,
        "Run RustShot at startup",
    );
    b.item(
        chk,
        10,
        78,
        190,
        11,
        CHK_JPEG as u16,
        ATOM_BUTTON,
        "Save screenshots as JPEG (else PNG)",
    );

    b.item(
        stat,
        10,
        96,
        90,
        10,
        0xFFFF,
        ATOM_STATIC,
        "JPEG quality (1-100):",
    );
    b.item(edit, 105, 94, 40, 12, EDIT_QUALITY as u16, ATOM_EDIT, "");
    b.item(
        stat,
        10,
        112,
        90,
        10,
        0xFFFF,
        ATOM_STATIC,
        "Video frame rate (fps):",
    );
    b.item(edit, 105, 110, 40, 12, EDIT_FPS as u16, ATOM_EDIT, "");

    b.item(
        stat,
        10,
        130,
        190,
        10,
        0xFFFF,
        ATOM_STATIC,
        "Output folder:",
    );
    b.item(edit, 10, 142, 140, 13, EDIT_FOLDER as u16, ATOM_EDIT, "");
    b.item(
        btn,
        155,
        141,
        45,
        14,
        BTN_BROWSE as u16,
        ATOM_BUTTON,
        "Browse…",
    );

    // Global-hotkeys section: a header, then one labelled, editable shortcut field per action.
    b.item(
        stat,
        10,
        162,
        230,
        10,
        0xFFFF,
        ATOM_STATIC,
        "Global hotkeys (e.g. Ctrl+Shift+R; blank = off):",
    );
    for (i, label) in HK_LABELS.iter().enumerate() {
        let y = 176 + (i as i16) * 15;
        b.item(stat, 10, y, 85, 10, 0xFFFF, ATOM_STATIC, label);
        b.item(
            edit,
            100,
            y - 2,
            95,
            12,
            (EDIT_HK_BASE + i as i32) as u16,
            ATOM_EDIT,
            "",
        );
        b.item(
            btn,
            198,
            y - 2,
            42,
            12,
            (BTN_MODIFY_BASE + i as i32) as u16,
            ATOM_BUTTON,
            "Modify",
        );
    }

    b.item(
        btn,
        10,
        272,
        110,
        14,
        BTN_RESTORE_HK as u16,
        ATOM_BUTTON,
        "Restore default hotkeys",
    );
    b.item(
        btn | BS_DEFPUSHBUTTON,
        148,
        272,
        45,
        15,
        IDOK.0 as u16,
        ATOM_BUTTON,
        "OK",
    );
    b.item(
        btn,
        200,
        272,
        45,
        15,
        IDCANCEL.0 as u16,
        ATOM_BUTTON,
        "Cancel",
    );

    b.build()
}

/// The six shortcuts as display strings, in the fixed row order.
fn map_to_strings(m: &HotkeyMap) -> [String; 6] {
    [
        m.region_screenshot.to_string(),
        m.window_screenshot.to_string(),
        m.monitor_screenshot.to_string(),
        m.all_monitors_screenshot.to_string(),
        m.region_recording.to_string(),
        m.stop_recording.to_string(),
    ]
}

/// Parse six shortcut strings (row order) back into a [`HotkeyMap`], rejecting unparseable
/// entries and any duplicate binding. Returns a user-facing message on error.
fn strings_to_map(strs: &[String; 6]) -> Result<HotkeyMap, String> {
    let mut parsed = Vec::with_capacity(6);
    for (i, raw) in strs.iter().enumerate() {
        match config::Hotkey::from_str(raw) {
            Ok(h) => parsed.push(h),
            Err(_) => {
                return Err(format!(
                    "\"{}\" is not a valid shortcut for {}.",
                    raw.trim(),
                    HK_LABELS[i]
                ))
            }
        }
    }
    let map = HotkeyMap {
        region_screenshot: parsed[0].clone(),
        window_screenshot: parsed[1].clone(),
        monitor_screenshot: parsed[2].clone(),
        all_monitors_screenshot: parsed[3].clone(),
        region_recording: parsed[4].clone(),
        stop_recording: parsed[5].clone(),
    };
    if let Some((a, b)) = map.conflicts().first() {
        let la = label_for(a);
        let lb = label_for(b);
        return Err(format!(
            "{la} and {lb} are assigned the same shortcut. Please make them different."
        ));
    }
    Ok(map)
}

/// Map a `HotkeyMap::entries()` machine name to its friendly label.
fn label_for(name: &str) -> &'static str {
    match name {
        "region_screenshot" => HK_LABELS[0],
        "window_screenshot" => HK_LABELS[1],
        "monitor_screenshot" => HK_LABELS[2],
        "all_monitors_screenshot" => HK_LABELS[3],
        "region_recording" => HK_LABELS[4],
        "stop_recording" => HK_LABELS[5],
        _ => "an action",
    }
}

/// Settings passed into the dialog and the edited result passed back out.
struct Draft {
    initial: Settings,
    result: Option<Settings>,
}

/// Open the modal Settings dialog and, on OK, persist + apply the changes.
pub fn open() {
    // Reserve `busy` (same gate as captures/recording) so a hotkey can't open an overlay nested
    // inside this modal dialog. Released once the dialog closes.
    let Some((settings, parent_raw)) = dispatch::with_app(|a| {
        if a.busy || a.recording.is_some() {
            None
        } else {
            a.busy = true;
            Some((a.settings.clone(), a.dispatch_hwnd.0 as isize))
        }
    })
    .flatten() else {
        tracing::info!("busy; settings dialog not opened");
        return;
    };

    // Drop our global hotkeys for the whole dialog session so a capture field can actually read
    // keys we'd otherwise swallow via RegisterHotKey (notably PrintScreen). Restored on close
    // (apply() re-registers the new set on OK; the cancel path re-registers `initial` below).
    dispatch::with_app(|a| a.hotkeys = None);

    let aligned = to_aligned(&build_template());
    let mut draft = Draft {
        initial: settings,
        result: None,
    };

    // SAFETY: standard modal dialog from an in-memory template; `draft` outlives the call.
    let ret = unsafe {
        let hinst = GetModuleHandleW(None).ok().map(|h| HINSTANCE(h.0));
        DialogBoxIndirectParamW(
            hinst,
            aligned.as_ptr() as *const DLGTEMPLATE,
            Some(HWND(parent_raw as *mut c_void)),
            Some(dlgproc),
            LPARAM(&mut draft as *mut Draft as isize),
        )
    };

    // Release the gate before applying (apply re-enters `with_app` on its own).
    dispatch::with_app(|a| a.busy = false);

    if ret == -1 {
        // SAFETY: immediately reads the thread's last-error after the failed call.
        let err = unsafe { GetLastError() };
        tracing::error!("settings dialog failed to open (DialogBoxIndirectParamW = -1, {err:?})");
    }

    let applied = ret == IDOK.0 as isize
        && match draft.result.take() {
            Some(new) => {
                apply(new);
                true
            }
            None => false,
        };

    if !applied {
        // Cancelled/closed/failed: re-register the hotkeys we dropped for the session.
        let hwnd = HWND(parent_raw as *mut c_void);
        let hk = crate::hotkeys::register(&draft.initial.hotkeys, hwnd);
        dispatch::with_app(|a| a.hotkeys = hk);
    }
}

fn apply(new: Settings) {
    // Everything the user asked for is attempted; whatever did not take effect is reported
    // right away (the user is at the dialog, so a box is the natural place) instead of being
    // discovered later as a missing hotkey or a capture that went to the wrong folder.
    match config::paths::config_file() {
        Ok(path) => {
            if let Err(e) = new.save_to(&path) {
                crate::notify::warn(
                    "Settings are in effect for this session but could not be saved.",
                    e,
                );
            }
        }
        Err(e) => crate::notify::warn(
            "Settings are in effect for this session but could not be saved.",
            e,
        ),
    }
    let hwnd_raw = dispatch::with_app(|a| a.dispatch_hwnd.0 as isize).unwrap_or(0);

    // Unregister the old hotkeys first so re-registering the same keys doesn't self-conflict.
    dispatch::with_app(|a| a.hotkeys = None);
    let hk = crate::hotkeys::register(&new.hotkeys, HWND(hwnd_raw as *mut c_void));
    if let Some(report) = hk.as_ref().and_then(|h| h.unregistered_report()) {
        crate::notify::warn("Some hotkeys could not be registered.", report);
    }
    if let Err(e) = crate::startup::apply(new.run_at_startup) {
        crate::notify::warn("The \"run at startup\" setting could not be applied.", e);
    }
    let folder = match crate::usable_output_folder(&new) {
        Ok((f, notice)) => {
            if let Some(n) = notice {
                crate::notify::warn("The output folder is not usable.", n);
            }
            Some(f)
        }
        Err(e) => {
            crate::notify::warn("The output folder could not be resolved.", e);
            None
        }
    };
    dispatch::with_app(|a| {
        a.settings = new;
        if let Some(f) = folder {
            a.output_folder = f;
        }
        a.hotkeys = hk;
    });
    tracing::info!("settings updated");
}

/// Developer smoke-test: render the real Settings dialog with default settings and auto-close it
/// after a moment, returning the dialog manager's result. A result of `-1` means the in-memory
/// `DLGTEMPLATE` is malformed (nothing would show in the product); any other value means it
/// rendered. Exercises `build_template` + `populate` without touching live app state.
pub fn selftest() -> isize {
    let aligned = to_aligned(&build_template());
    let mut draft = Draft {
        initial: Settings::default(),
        result: None,
    };
    // SAFETY: parentless modal dialog from an in-memory template; `draft` outlives the call.
    let ret = unsafe {
        let hinst = GetModuleHandleW(None).ok().map(|h| HINSTANCE(h.0));
        DialogBoxIndirectParamW(
            hinst,
            aligned.as_ptr() as *const DLGTEMPLATE,
            None,
            Some(selftest_dlgproc),
            LPARAM(&mut draft as *mut Draft as isize),
        )
    };
    if ret == -1 {
        // SAFETY: reads the last error immediately after the failed call.
        let err = unsafe { GetLastError() };
        tracing::error!("selftest: dialog failed to render ({err:?})");
    }
    ret
}

/// Developer smoke-test for the capture sub-dialog: renders it (installing the edit subclass)
/// and auto-closes. Returns the dialog result; `-1` means its template is malformed.
pub fn capture_selftest() -> isize {
    let aligned = to_aligned(&build_capture_template());
    let mut d = CaptureDraft {
        initial: empty_hotkey(),
        captured: empty_hotkey(),
        result: None,
        old_proc: 0,
    };
    // SAFETY: parentless modal dialog from an in-memory template; `d` outlives the call.
    let ret = unsafe {
        let hinst = GetModuleHandleW(None).ok().map(|h| HINSTANCE(h.0));
        DialogBoxIndirectParamW(
            hinst,
            aligned.as_ptr() as *const DLGTEMPLATE,
            None,
            Some(capture_selftest_dlgproc),
            LPARAM(&mut d as *mut CaptureDraft as isize),
        )
    };
    if ret == -1 {
        // SAFETY: reads the last error immediately after the failed call.
        let err = unsafe { GetLastError() };
        tracing::error!("capture selftest: dialog failed to render ({err:?})");
    }
    ret
}

unsafe extern "system" fn capture_selftest_dlgproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    crate::panic_guard::guard(
        "capture selftest dlgproc",
        || 0,
        || unsafe { capture_selftest_dlgproc_inner(hwnd, msg, wparam, lparam) },
    )
}

unsafe fn capture_selftest_dlgproc_inner(
    hwnd: HWND,
    msg: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            // SAFETY: install the edit subclass (as the real dialog does), then auto-close.
            unsafe {
                SetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(DWLP_USER), lparam.0);
                let d = &mut *(lparam.0 as *mut CaptureDraft);
                if let Ok(edit) = GetDlgItem(Some(hwnd), EDIT_CAP) {
                    let newproc = std::mem::transmute::<WNDPROC, isize>(Some(edit_subclass));
                    d.old_proc = SetWindowLongPtrW(edit, GWLP_WNDPROC, newproc);
                    SetWindowLongPtrW(edit, GWLP_USERDATA, lparam.0);
                }
                SetTimer(Some(hwnd), 1, 400, None);
            }
            1
        }
        WM_TIMER | WM_CLOSE => {
            // SAFETY: close the self-test dialog.
            unsafe {
                let _ = EndDialog(hwnd, IDCANCEL.0 as isize);
            }
            1
        }
        _ => 0,
    }
}

unsafe extern "system" fn selftest_dlgproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    crate::panic_guard::guard(
        "settings selftest dlgproc",
        || 0,
        || unsafe { selftest_dlgproc_inner(hwnd, msg, wparam, lparam) },
    )
}

unsafe fn selftest_dlgproc_inner(hwnd: HWND, msg: u32, _wparam: WPARAM, lparam: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            // SAFETY: populate controls, then arm a one-shot timer to auto-cancel.
            unsafe {
                let draft = &*(lparam.0 as *const Draft);
                populate(hwnd, &draft.initial);
                SetTimer(Some(hwnd), 1, 600, None);
            }
            1
        }
        WM_TIMER => {
            // SAFETY: close the self-test dialog.
            unsafe {
                let _ = EndDialog(hwnd, IDCANCEL.0 as isize);
            }
            1
        }
        WM_CLOSE => {
            // SAFETY: close on the X button too.
            unsafe {
                let _ = EndDialog(hwnd, IDCANCEL.0 as isize);
            }
            1
        }
        _ => 0,
    }
}

unsafe extern "system" fn dlgproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    // A panic must not unwind across this FFI boundary (that aborts the process).
    crate::panic_guard::guard(
        "settings dlgproc",
        || 0,
        || unsafe { dlgproc_inner(hwnd, msg, wparam, lparam) },
    )
}

unsafe fn dlgproc_inner(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            // SAFETY: stash the Draft pointer for later messages, then populate controls.
            unsafe {
                SetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(DWLP_USER), lparam.0);
                let draft = &*(lparam.0 as *const Draft);
                populate(hwnd, &draft.initial);
            }
            1
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            if id == IDOK.0 {
                // SAFETY: recover the Draft, validate, and record the gathered settings. On a
                // validation error we warn and keep the dialog open instead of closing.
                unsafe {
                    let ptr =
                        GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(DWLP_USER)) as *mut Draft;
                    if ptr.is_null() {
                        let _ = EndDialog(hwnd, IDCANCEL.0 as isize);
                        return 1;
                    }
                    let d = &mut *ptr;
                    match try_gather(hwnd, &d.initial) {
                        Ok(s) => {
                            d.result = Some(s);
                            let _ = EndDialog(hwnd, IDOK.0 as isize);
                        }
                        Err(msg) => warn_box(hwnd, &msg),
                    }
                }
                1
            } else if id == BTN_RESTORE_HK {
                restore_hotkeys(hwnd);
                1
            } else if (BTN_MODIFY_BASE..BTN_MODIFY_BASE + 6).contains(&id) {
                modify_hotkey(hwnd, id - BTN_MODIFY_BASE);
                1
            } else if id == IDCANCEL.0 {
                // SAFETY: close with a cancel result.
                unsafe {
                    let _ = EndDialog(hwnd, IDCANCEL.0 as isize);
                }
                1
            } else if id == BTN_BROWSE {
                browse(hwnd);
                1
            } else {
                0
            }
        }
        WM_CLOSE => {
            // SAFETY: treat the X button as Cancel.
            unsafe {
                let _ = EndDialog(hwnd, IDCANCEL.0 as isize);
            }
            1
        }
        _ => 0,
    }
}

fn check_state(on: bool) -> DLG_BUTTON_CHECK_STATE {
    if on {
        BST_CHECKED
    } else {
        DLG_BUTTON_CHECK_STATE(0)
    }
}

fn populate(hwnd: HWND, s: &Settings) {
    // SAFETY: setting control states on our dialog's children.
    unsafe {
        let _ = CheckDlgButton(hwnd, CHK_CURSOR, check_state(s.screenshot.include_cursor));
        let _ = CheckDlgButton(hwnd, CHK_CLIP, check_state(s.screenshot.copy_to_clipboard));
        let _ = CheckDlgButton(hwnd, CHK_MIC, check_state(s.recording.capture_microphone));
        let _ = CheckDlgButton(hwnd, CHK_SYS, check_state(s.recording.capture_system_audio));
        let _ = CheckDlgButton(hwnd, CHK_STARTUP, check_state(s.run_at_startup));
        let _ = CheckDlgButton(
            hwnd,
            CHK_JPEG,
            check_state(s.screenshot.format == ImageFormat::Jpeg),
        );
        let _ = SetDlgItemInt(hwnd, EDIT_QUALITY, s.screenshot.jpeg_quality as u32, false);
        let _ = SetDlgItemInt(hwnd, EDIT_FPS, s.recording.fps, false);
        let folder = s
            .effective_output_folder()
            .unwrap_or_default()
            .display()
            .to_string();
        let wide: Vec<u16> = folder.encode_utf16().chain([0]).collect();
        let _ = SetDlgItemTextW(hwnd, EDIT_FOLDER, PCWSTR(wide.as_ptr()));

        for (i, text) in map_to_strings(&s.hotkeys).iter().enumerate() {
            let w: Vec<u16> = text.encode_utf16().chain([0]).collect();
            let _ = SetDlgItemTextW(hwnd, EDIT_HK_BASE + i as i32, PCWSTR(w.as_ptr()));
        }
    }
}

/// Reset the six shortcut fields to the built-in defaults (does not close the dialog).
fn restore_hotkeys(hwnd: HWND) {
    for (i, text) in map_to_strings(&HotkeyMap::default()).iter().enumerate() {
        let w: Vec<u16> = text.encode_utf16().chain([0]).collect();
        // SAFETY: sets our own child edit controls' text.
        unsafe {
            let _ = SetDlgItemTextW(hwnd, EDIT_HK_BASE + i as i32, PCWSTR(w.as_ptr()));
        }
    }
}

/// Show a modal warning over the dialog (used for validation failures on OK).
fn warn_box(hwnd: HWND, msg: &str) {
    let text: Vec<u16> = msg.encode_utf16().chain([0]).collect();
    let title: Vec<u16> = "RustShot Settings".encode_utf16().chain([0]).collect();
    // SAFETY: standard modal message box owned by the settings dialog.
    unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONWARNING,
        );
    }
}

fn is_checked(hwnd: HWND, id: i32) -> bool {
    // SAFETY: reads a child button's check state.
    unsafe { IsDlgButtonChecked(hwnd, id) == BST_CHECKED.0 }
}

/// The value to store for the output-folder field. The dialog displays the *resolved* folder, so
/// an untouched default must be saved back as empty ("use the default") rather than pinned as an
/// explicit path.
fn folder_setting(text: &str, default: Option<PathBuf>) -> PathBuf {
    let typed = PathBuf::from(text);
    if text.is_empty() || default.as_ref() == Some(&typed) {
        PathBuf::new()
    } else {
        typed
    }
}

/// Reject a typed output folder that could never work: a relative path resolves against the
/// process's working directory (`System32` when launched from the `Run` key). Browse can only
/// produce absolute paths, so this guards hand-typed input. Returns the user-facing message.
fn validate_folder(text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() || Path::new(text).is_absolute() {
        Ok(())
    } else {
        Err(format!(
            "The output folder must be a full path such as C:\\Users\\you\\Pictures\\RustShot, \
             not \"{text}\"."
        ))
    }
}

/// Read every control into a `Settings`, validating the hotkeys. Returns a user-facing message
/// (shown in a warning box, dialog left open) if a shortcut is unparseable or two conflict.
fn try_gather(hwnd: HWND, initial: &Settings) -> Result<Settings, String> {
    let mut s = initial.clone();
    s.screenshot.include_cursor = is_checked(hwnd, CHK_CURSOR);
    s.screenshot.copy_to_clipboard = is_checked(hwnd, CHK_CLIP);
    s.recording.capture_microphone = is_checked(hwnd, CHK_MIC);
    s.recording.capture_system_audio = is_checked(hwnd, CHK_SYS);
    s.run_at_startup = is_checked(hwnd, CHK_STARTUP);
    s.screenshot.format = if is_checked(hwnd, CHK_JPEG) {
        ImageFormat::Jpeg
    } else {
        ImageFormat::Png
    };

    // SAFETY: read integer fields; invalid/empty leaves the previous value.
    unsafe {
        let q = GetDlgItemInt(hwnd, EDIT_QUALITY, None, false);
        if q > 0 {
            s.screenshot.jpeg_quality = q.min(100) as u8;
        }
        let fps = GetDlgItemInt(hwnd, EDIT_FPS, None, false);
        if fps > 0 {
            s.recording.fps = fps;
        }
    }

    let folder = get_text(hwnd, EDIT_FOLDER);
    validate_folder(&folder)?;
    s.output_folder = folder_setting(folder.trim(), config::paths::default_output_folder().ok());
    if !s.output_folder.as_os_str().is_empty() {
        // Create it now so a typo in a drive letter or a read-only location is reported here,
        // with the dialog still open, rather than on the first capture.
        if let Err(e) = std::fs::create_dir_all(&s.output_folder) {
            return Err(format!(
                "The output folder could not be created:\n{}\n\n{e}",
                s.output_folder.display()
            ));
        }
    }

    let hk_strs: [String; 6] = std::array::from_fn(|i| get_text(hwnd, EDIT_HK_BASE + i as i32));
    s.hotkeys = strings_to_map(&hk_strs)?;

    s.sanitize();
    Ok(s)
}

fn get_text(hwnd: HWND, id: i32) -> String {
    let mut buf = [0u16; 1024];
    // SAFETY: fills `buf` with up to its length; returns the copied length.
    let n = unsafe { GetDlgItemTextW(hwnd, id, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn browse(hwnd: HWND) {
    if let Some(folder) = rfd::FileDialog::new()
        .set_title("Choose the RustShot output folder")
        .pick_folder()
    {
        let wide: Vec<u16> = folder.to_string_lossy().encode_utf16().chain([0]).collect();
        // SAFETY: sets the folder edit control's text.
        unsafe {
            let _ = SetDlgItemTextW(hwnd, EDIT_FOLDER, PCWSTR(wide.as_ptr()));
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Hotkey capture sub-dialog ("Modify" button): a small modal that reads the next key combination
// the user presses — including PrintScreen and Alt/Win combos the dialog manager would otherwise
// eat — and writes it back as a shortcut string. A classic dialog won't deliver raw keys to its
// proc, so we subclass the edit control, ask for DLGC_WANTALLKEYS, and translate VK codes here.
// ---------------------------------------------------------------------------------------------

/// State shared between the capture dialog proc and the subclassed edit control.
struct CaptureDraft {
    /// The shortcut the field held when "Modify" was pressed (prefilled, used on nothing-captured).
    initial: config::Hotkey,
    /// The working value as the user presses keys.
    captured: config::Hotkey,
    /// Set to the accepted value on OK (`None` while the dialog is open / on cancel).
    result: Option<config::Hotkey>,
    /// The edit control's original window procedure (restored implicitly when it's destroyed).
    old_proc: isize,
}

/// Map a Win32 virtual-key code to a canonical key name. Only emits spellings that survive both
/// `config::Hotkey::normalize_key` and `hotkeys::key_to_code` (see the round-trip unit test), so
/// a captured shortcut is always re-parseable and registerable.
fn vk_to_key(vk: u16) -> Option<String> {
    match vk {
        0x41..=0x5A | 0x30..=0x39 => Some(((vk as u8) as char).to_string()), // A-Z, 0-9
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),                      // F1-F24
        0x2C => Some("PrintScreen".to_string()),
        0x20 => Some("Space".to_string()),
        0x2E => Some("Delete".to_string()),
        0x2D => Some("Insert".to_string()),
        0x21 => Some("PageUp".to_string()),
        0x22 => Some("PageDown".to_string()),
        0x24 => Some("Home".to_string()),
        0x23 => Some("End".to_string()),
        0x09 => Some("Tab".to_string()),
        _ => None,
    }
}

fn is_modifier_vk(vk: u16) -> bool {
    matches!(
        vk,
        VK_SHIFT | VK_CONTROL | VK_MENU | VK_LWIN | VK_RWIN | 0xA0..=0xA5
    )
}

fn empty_hotkey() -> config::Hotkey {
    config::Hotkey::new(Vec::new(), "")
}

/// The capture sub-dialog template: a prompt, one edit (subclassed), and OK/Cancel.
fn build_capture_template() -> Vec<u8> {
    let stat = WS_CHILD | WS_VISIBLE;
    let edit = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL;
    let btn = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
    let mut b = DlgBuilder::new(
        WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME,
        60,
        60,
        210,
        76,
        "Set shortcut",
        9,
        "Segoe UI",
    );
    b.item(
        stat,
        10,
        8,
        190,
        26,
        0xFFFF,
        ATOM_STATIC,
        "Press the shortcut keys. Backspace clears it. Enter = OK, Esc = Cancel.",
    );
    b.item(edit, 10, 38, 190, 12, EDIT_CAP as u16, ATOM_EDIT, "");
    b.item(
        btn | BS_DEFPUSHBUTTON,
        58,
        56,
        45,
        14,
        IDOK.0 as u16,
        ATOM_BUTTON,
        "OK",
    );
    b.item(
        btn,
        110,
        56,
        45,
        14,
        IDCANCEL.0 as u16,
        ATOM_BUTTON,
        "Cancel",
    );
    b.build()
}

/// Open the capture dialog for one row. Returns the accepted shortcut (possibly empty = unbound),
/// or `None` if the user cancelled.
fn open_capture(parent: HWND, current: config::Hotkey) -> Option<config::Hotkey> {
    let aligned = to_aligned(&build_capture_template());
    let mut d = CaptureDraft {
        captured: current.clone(),
        initial: current,
        result: None,
        old_proc: 0,
    };
    // SAFETY: modal dialog from an in-memory template; `d` outlives the call.
    let ret = unsafe {
        let hinst = GetModuleHandleW(None).ok().map(|h| HINSTANCE(h.0));
        DialogBoxIndirectParamW(
            hinst,
            aligned.as_ptr() as *const DLGTEMPLATE,
            Some(parent),
            Some(capture_dlgproc),
            LPARAM(&mut d as *mut CaptureDraft as isize),
        )
    };
    if ret == IDOK.0 as isize {
        d.result.take()
    } else {
        None
    }
}

/// Handle the "Modify" button for hotkey row `row`: capture a combination and write it back.
fn modify_hotkey(parent: HWND, row: i32) {
    let current = config::Hotkey::from_str(&get_text(parent, EDIT_HK_BASE + row))
        .unwrap_or_else(|_| empty_hotkey());
    if let Some(hk) = open_capture(parent, current) {
        let w: Vec<u16> = hk.to_string().encode_utf16().chain([0]).collect();
        // SAFETY: sets our own child edit control's text.
        unsafe {
            let _ = SetDlgItemTextW(parent, EDIT_HK_BASE + row, PCWSTR(w.as_ptr()));
        }
    }
}

unsafe extern "system" fn capture_dlgproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    crate::panic_guard::guard(
        "hotkey capture dlgproc",
        || 0,
        || unsafe { capture_dlgproc_inner(hwnd, msg, wparam, lparam) },
    )
}

unsafe fn capture_dlgproc_inner(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            // SAFETY: stash the draft pointer, prefill the field, and subclass the edit so it
            // receives raw key input.
            unsafe {
                SetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(DWLP_USER), lparam.0);
                let d = &mut *(lparam.0 as *mut CaptureDraft);
                let text: Vec<u16> = d.initial.to_string().encode_utf16().chain([0]).collect();
                let _ = SetDlgItemTextW(hwnd, EDIT_CAP, PCWSTR(text.as_ptr()));
                if let Ok(edit) = GetDlgItem(Some(hwnd), EDIT_CAP) {
                    let newproc = std::mem::transmute::<WNDPROC, isize>(Some(edit_subclass));
                    d.old_proc = SetWindowLongPtrW(edit, GWLP_WNDPROC, newproc);
                    SetWindowLongPtrW(edit, GWLP_USERDATA, lparam.0);
                    let _ = SetFocus(Some(edit));
                }
            }
            0 // we set focus ourselves
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            // SAFETY: recover the draft to record the result before closing.
            unsafe {
                let ptr =
                    GetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(DWLP_USER)) as *mut CaptureDraft;
                if id == IDOK.0 {
                    if !ptr.is_null() {
                        (*ptr).result = Some((*ptr).captured.clone());
                    }
                    let _ = EndDialog(hwnd, IDOK.0 as isize);
                    1
                } else if id == IDCANCEL.0 {
                    let _ = EndDialog(hwnd, IDCANCEL.0 as isize);
                    1
                } else {
                    0
                }
            }
        }
        WM_CLOSE => {
            // SAFETY: treat the X button as Cancel.
            unsafe {
                let _ = EndDialog(hwnd, IDCANCEL.0 as isize);
            }
            1
        }
        _ => 0,
    }
}

/// Subclass procedure for the capture edit: consumes keys and translates them into a shortcut.
unsafe extern "system" fn edit_subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    crate::panic_guard::guard(
        "hotkey capture edit subclass",
        || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        || unsafe { edit_subclass_inner(hwnd, msg, wparam, lparam) },
    )
}

unsafe fn edit_subclass_inner(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: userdata holds the CaptureDraft pointer set in WM_INITDIALOG.
    let dptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut CaptureDraft;
    if dptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let d = unsafe { &mut *dptr };
    let old: WNDPROC = unsafe { std::mem::transmute::<isize, WNDPROC>(d.old_proc) };
    let pass = || unsafe { CallWindowProcW(old, hwnd, msg, wparam, lparam) };

    match msg {
        // Ask the dialog manager to hand us every key, and swallow char messages so the edit
        // never shows raw typed text (we display the formatted combo instead).
        WM_GETDLGCODE => LRESULT(DLGC_WANTALLKEYS),
        WM_CHAR | WM_SYSCHAR => LRESULT(0),
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = (wparam.0 & 0xFFFF) as u16;
            if handle_capture_key(d, vk, hwnd) {
                LRESULT(0)
            } else {
                pass()
            }
        }
        WM_KEYUP | WM_SYSKEYUP => {
            let vk = (wparam.0 & 0xFFFF) as u16;
            // PrintScreen only reports on key-up. Also swallow modifier key-ups so releasing
            // Alt alone doesn't fall through to DefWindowProc and open the system menu.
            if vk == VK_SNAPSHOT {
                set_captured_key(d, "PrintScreen", hwnd);
                LRESULT(0)
            } else if is_modifier_vk(vk) {
                LRESULT(0)
            } else {
                pass()
            }
        }
        _ => pass(),
    }
}

/// Process one key-down in the capture edit. Returns true if the key was consumed.
fn handle_capture_key(d: &mut CaptureDraft, vk: u16, edit: HWND) -> bool {
    match vk {
        VK_RETURN => post_cmd(edit, IDOK.0),
        VK_ESCAPE => post_cmd(edit, IDCANCEL.0),
        VK_BACK => {
            d.captured = empty_hotkey();
            set_edit_text(edit, "");
        }
        _ if is_modifier_vk(vk) => {} // ignore modifier-only presses, but consume
        _ => {
            if let Some(key) = vk_to_key(vk) {
                set_captured_key(d, &key, edit);
            }
        }
    }
    true
}

fn set_captured_key(d: &mut CaptureDraft, key: &str, edit: HWND) {
    let hk = config::Hotkey::new(current_mods(), key);
    set_edit_text(edit, &hk.to_string());
    d.captured = hk;
}

/// The modifiers currently held down, read at the instant of the key event.
fn current_mods() -> Vec<config::Modifier> {
    // SAFETY: GetKeyState reads the calling thread's key state for the message being processed.
    let down = |vk: u16| (unsafe { GetKeyState(vk as i32) } as u16 & 0x8000) != 0;
    let mut m = Vec::new();
    if down(VK_CONTROL) {
        m.push(config::Modifier::Ctrl);
    }
    if down(VK_MENU) {
        m.push(config::Modifier::Alt);
    }
    if down(VK_SHIFT) {
        m.push(config::Modifier::Shift);
    }
    if down(VK_LWIN) || down(VK_RWIN) {
        m.push(config::Modifier::Win);
    }
    m
}

fn post_cmd(edit: HWND, id: i32) {
    // SAFETY: route accept/cancel to the capture dialog's own WM_COMMAND handler.
    unsafe {
        if let Ok(parent) = GetParent(edit) {
            let _ = PostMessageW(Some(parent), WM_COMMAND, WPARAM(id as usize), LPARAM(0));
        }
    }
}

fn set_edit_text(edit: HWND, s: &str) {
    let w: Vec<u16> = s.encode_utf16().chain([0]).collect();
    // SAFETY: sets the capture edit's text directly by handle.
    unsafe {
        let _ = SetWindowTextW(edit, PCWSTR(w.as_ptr()));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn template_header_and_item_count() {
        let mut b = DlgBuilder::new(WS_POPUP, 0, 0, 100, 100, "T", 9, "Segoe UI");
        b.item(WS_CHILD, 1, 2, 3, 4, 42, ATOM_STATIC, "x");
        b.item(WS_CHILD, 1, 2, 3, 4, 43, ATOM_BUTTON, "y");
        let t = b.build();

        // style has DS_SETFONT OR'd in.
        let style = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
        assert_eq!(style, WS_POPUP | DS_SETFONT);
        // cdit (item count) is at offset 8.
        assert_eq!(u16::from_le_bytes([t[8], t[9]]), 2);
        // The aligned buffer is non-empty (and, being Vec<u32>, DWORD-aligned by construction).
        assert!(!to_aligned(&t).is_empty());
    }

    #[test]
    fn real_template_builds_with_expected_item_count() {
        let t = build_template();
        // Base 15 (6 checkboxes + 3 labels + 3 edits + Browse + OK + Cancel) plus the hotkeys
        // section: 1 header + 6 labels + 6 edits + 6 Modify buttons + Restore = 20 -> 35.
        assert_eq!(u16::from_le_bytes([t[8], t[9]]), 35);
    }

    #[test]
    fn capture_template_builds() {
        let t = build_capture_template();
        // prompt + edit + OK + Cancel = 4.
        assert_eq!(u16::from_le_bytes([t[8], t[9]]), 4);
        assert!(!to_aligned(&t).is_empty());
    }

    #[test]
    fn vk_to_key_outputs_are_canonical_and_registerable() {
        // Every spelling vk_to_key can emit must be stable through normalize_key and map to a
        // real Code, or a captured shortcut would silently fail to re-parse / register.
        let samples = [
            0x41, 0x5A, 0x30, 0x39, 0x70, 0x87, 0x2C, 0x20, 0x2E, 0x2D, 0x21, 0x22, 0x24, 0x23,
            0x09,
        ];
        for vk in samples {
            let key = vk_to_key(vk).unwrap();
            assert_eq!(config::Hotkey::normalize_key(&key), key, "unstable: {key}");
            assert!(
                crate::hotkeys::key_to_code(&key).is_some(),
                "unmappable: {key}"
            );
        }
        assert!(vk_to_key(VK_SHIFT).is_none());
        assert!(is_modifier_vk(VK_CONTROL) && !is_modifier_vk(0x41));
    }

    #[test]
    fn hotkey_map_strings_round_trip() {
        let m = HotkeyMap::default();
        let back = strings_to_map(&map_to_strings(&m)).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn restore_strings_match_defaults() {
        assert_eq!(
            map_to_strings(&HotkeyMap::default()),
            [
                "PrintScreen",
                "Ctrl+PrintScreen",
                "Alt+PrintScreen",
                "Shift+PrintScreen",
                "Ctrl+Shift+R",
                "Ctrl+Shift+S",
            ]
            .map(String::from)
        );
    }

    #[test]
    fn strings_to_map_rejects_bad_shortcut() {
        let mut strs = map_to_strings(&HotkeyMap::default());
        strs[0] = "Ctrl+Alt+".to_string(); // modifier with no main key
        assert!(strings_to_map(&strs).is_err());
    }

    #[test]
    fn strings_to_map_detects_conflict() {
        let mut strs = map_to_strings(&HotkeyMap::default());
        // Bind two different actions to the same combination.
        strs[0] = "Ctrl+Shift+R".to_string();
        assert_eq!(strs[4], "Ctrl+Shift+R");
        assert!(strings_to_map(&strs).is_err());
    }

    #[test]
    fn untouched_default_folder_is_stored_as_empty() {
        let default = PathBuf::from(r"C:\Pictures\RustShot");
        assert_eq!(
            folder_setting(r"C:\Pictures\RustShot", Some(default.clone())),
            PathBuf::new()
        );
        assert_eq!(folder_setting("", Some(default.clone())), PathBuf::new());
        assert_eq!(
            folder_setting(r"D:\Shots", Some(default)),
            PathBuf::from(r"D:\Shots")
        );
        assert_eq!(
            folder_setting(r"D:\Shots", None),
            PathBuf::from(r"D:\Shots")
        );
    }

    #[test]
    fn relative_folder_is_rejected_with_a_message() {
        assert_eq!(validate_folder(""), Ok(()));
        assert_eq!(validate_folder("   "), Ok(()));
        assert_eq!(validate_folder(r"D:\Shots"), Ok(()));
        assert_eq!(validate_folder(r"\\server\share\shots"), Ok(()));
        let err = validate_folder("shots").unwrap_err();
        assert!(err.contains("full path"), "{err}");
        assert!(err.contains("\"shots\""), "{err}");
        assert!(validate_folder("C:shots").is_err());
    }

    #[test]
    fn blank_shortcut_is_allowed_as_unbound() {
        let mut strs = map_to_strings(&HotkeyMap::default());
        strs[0] = String::new();
        let m = strings_to_map(&strs).unwrap();
        assert!(m.region_screenshot.is_empty());
    }
}
