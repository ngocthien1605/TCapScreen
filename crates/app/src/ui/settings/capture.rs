//! Hotkey capture sub-dialog ("Modify" button): a small modal that reads the next key combination
//! the user presses — including PrintScreen and Alt/Win combos the dialog manager would otherwise
//! eat — and writes it back as a shortcut string. A classic dialog won't deliver raw keys to its
// proc, so we subclass the edit control, ask for DLGC_WANTALLKEYS, and translate VK codes here.

use super::*;

// `DLGC_WANTALLKEYS`: tell the dialog manager to route every key (incl. Tab/Enter/Esc/arrows)
// to the control instead of using them for navigation. Not re-exported conveniently, so local.
const DLGC_WANTALLKEYS: isize = 0x0004;

// Virtual-key codes we special-case during capture (avoids pulling the typed VK constants in).
const VK_BACK: u16 = 0x08;
const VK_RETURN: u16 = 0x0D;
pub(super) const VK_SHIFT: u16 = 0x10;
pub(super) const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;
const VK_ESCAPE: u16 = 0x1B;
const VK_SNAPSHOT: u16 = 0x2C;
const VK_LWIN: u16 = 0x5B;
const VK_RWIN: u16 = 0x5C;

/// State shared between the capture dialog proc and the subclassed edit control.
struct CaptureDraft {
    /// The shortcut the field held when "Modify" was pressed (prefilled, used on nothing-captured).
    pub(super) initial: config::Hotkey,
    /// The working value as the user presses keys.
    pub(super) captured: config::Hotkey,
    /// Set to the accepted value on OK (`None` while the dialog is open / on cancel).
    pub(super) result: Option<config::Hotkey>,
    /// The edit control's original window procedure (restored implicitly when it's destroyed).
    pub(super) old_proc: isize,
}

/// Map a Win32 virtual-key code to a canonical key name. Only emits spellings that survive both
/// `config::Hotkey::normalize_key` and `hotkeys::key_to_code` (see the round-trip unit test), so
/// a captured shortcut is always re-parseable and registerable.
pub(super) fn vk_to_key(vk: u16) -> Option<String> {
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
        0x25 => Some("ArrowLeft".to_string()),
        0x26 => Some("ArrowUp".to_string()),
        0x27 => Some("ArrowRight".to_string()),
        0x28 => Some("ArrowDown".to_string()),
        0x13 => Some("Pause".to_string()),
        0x91 => Some("ScrollLock".to_string()),
        0x60..=0x69 => Some(format!("Numpad{}", vk - 0x60)),
        0x6A => Some("NumpadMultiply".to_string()),
        0x6B => Some("NumpadAdd".to_string()),
        0x6D => Some("NumpadSubtract".to_string()),
        0x6E => Some("NumpadDecimal".to_string()),
        0x6F => Some("NumpadDivide".to_string()),
        _ => None,
    }
}

pub(super) fn is_modifier_vk(vk: u16) -> bool {
    matches!(
        vk,
        VK_SHIFT | VK_CONTROL | VK_MENU | VK_LWIN | VK_RWIN | 0xA0..=0xA5
    )
}

fn empty_hotkey() -> config::Hotkey {
    config::Hotkey::new(Vec::new(), "")
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
pub(super) fn modify_hotkey(parent: HWND, row: i32) {
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
