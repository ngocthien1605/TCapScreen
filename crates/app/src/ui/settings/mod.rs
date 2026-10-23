//! The Settings dialog: a modal Win32 dialog built from an in-memory `DLGTEMPLATE` (so tab
//! order, the default button, and Enter/Esc come for free). The template builder is pure and
//! unit-tested; on OK the edited settings are persisted and applied live (config saved, output
//! folder updated, hotkeys re-registered, run-at-startup toggled).
//!
//! Layout: `template` builds the dialog templates, `dialog` runs the Settings dialog and applies
//! its result, `capture` is the "Modify" key-capture sub-dialog; this module holds the control
//! ids and the shortcut-string conversions they share.

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

mod capture;
mod dialog;
mod template;

use capture::*;
use dialog::*;
use template::*;

pub use capture::capture_selftest;
pub use dialog::{open, selftest};

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
        // `from_str` rejects unknown key names; the second check guards the registration
        // mapping, so a shortcut accepted here is one that can actually be registered.
        match config::Hotkey::from_str(raw) {
            Ok(h) if h.is_empty() || crate::hotkeys::key_to_code(&h.key).is_some() => {
                parsed.push(h)
            }
            _ => {
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
            0x09, 0x25, 0x26, 0x27, 0x28, 0x13, 0x91, 0x60, 0x69, 0x6A, 0x6B, 0x6D, 0x6E, 0x6F,
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
    fn every_key_config_accepts_can_be_registered() {
        // config decides which names parse; hotkeys::key_to_code decides which register. They
        // must agree, or Settings would accept a shortcut that silently never fires.
        let mut keys: Vec<String> = config::NAMED_KEYS.iter().map(|k| k.to_string()).collect();
        keys.extend(('A'..='Z').chain('0'..='9').map(|c| c.to_string()));
        keys.extend((1..=24).map(|n| format!("F{n}")));
        for key in keys {
            assert!(config::Hotkey::is_known_key(&key), "not known: {key}");
            assert!(
                crate::hotkeys::key_to_code(&key).is_some(),
                "unmappable: {key}"
            );
        }
    }

    #[test]
    fn strings_to_map_rejects_keys_that_cannot_register() {
        let mut strs = map_to_strings(&HotkeyMap::default());
        strs[0] = "Ctrl+F99".into();
        assert!(strs_err(&strs).contains("Region"), "{}", strs_err(&strs));
        strs[0] = "Ctrl+ArrowUp".into();
        assert!(strings_to_map(&strs).is_ok());
    }

    fn strs_err(strs: &[String; 6]) -> String {
        strings_to_map(strs).unwrap_err()
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
