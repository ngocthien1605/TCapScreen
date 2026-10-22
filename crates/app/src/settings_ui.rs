//! The Settings dialog: a modal Win32 dialog built from an in-memory `DLGTEMPLATE` (so tab
//! order, the default button, and Enter/Esc come for free). The template builder is pure and
//! unit-tested; on OK the edited settings are persisted and applied live (config saved, output
//! folder updated, hotkeys re-registered, run-at-startup toggled).

use std::ffi::c_void;
use std::path::PathBuf;
use std::str::FromStr;

use config::{HotkeyMap, ImageFormat, Settings};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    CheckDlgButton, IsDlgButtonChecked, BST_CHECKED, DLG_BUTTON_CHECK_STATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DialogBoxIndirectParamW, EndDialog, GetDlgItemInt, GetDlgItemTextW, GetWindowLongPtrW,
    MessageBoxW, SetDlgItemInt, SetDlgItemTextW, SetWindowLongPtrW, DLGTEMPLATE, IDCANCEL, IDOK,
    MB_ICONWARNING, MB_OK, WINDOW_LONG_PTR_INDEX, WM_CLOSE, WM_COMMAND, WM_INITDIALOG,
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
// Six editable shortcut fields (in a fixed order) + a restore-to-defaults button.
const EDIT_HK_BASE: i32 = 1020;
const BTN_RESTORE_HK: i32 = 1030;

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
            140,
            12,
            (EDIT_HK_BASE + i as i32) as u16,
            ATOM_EDIT,
            "",
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
        return;
    }

    if ret == IDOK.0 as isize {
        if let Some(new) = draft.result.take() {
            apply(new);
        }
    }
}

fn apply(new: Settings) {
    if let Ok(path) = config::paths::config_file() {
        if let Err(e) = new.save_to(&path) {
            tracing::warn!("could not save settings: {e}");
        }
    }
    let hwnd_raw = dispatch::with_app(|a| a.dispatch_hwnd.0 as isize).unwrap_or(0);

    // Unregister the old hotkeys first so re-registering the same keys doesn't self-conflict.
    dispatch::with_app(|a| a.hotkeys = None);
    let hk = crate::hotkeys::register(&new.hotkeys, HWND(hwnd_raw as *mut c_void));
    if let Err(e) = crate::startup::apply(new.run_at_startup) {
        tracing::warn!("could not apply run-at-startup: {e:#}");
    }
    let folder = new.effective_output_folder().ok();
    dispatch::with_app(|a| {
        a.settings = new;
        if let Some(f) = folder {
            let _ = std::fs::create_dir_all(&f);
            a.output_folder = f;
        }
        a.hotkeys = hk;
    });
    tracing::info!("settings updated");
}

unsafe extern "system" fn dlgproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
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
    s.output_folder = if folder.trim().is_empty() {
        PathBuf::new()
    } else {
        PathBuf::from(folder.trim())
    };

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
        // section: 1 header + 6 labels + 6 edits + Restore = 14 -> 29.
        assert_eq!(u16::from_le_bytes([t[8], t[9]]), 29);
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
    fn blank_shortcut_is_allowed_as_unbound() {
        let mut strs = map_to_strings(&HotkeyMap::default());
        strs[0] = String::new();
        let m = strings_to_map(&strs).unwrap();
        assert!(m.region_screenshot.is_empty());
    }
}
