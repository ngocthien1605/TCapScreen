//! The in-memory `DLGTEMPLATE`s for the Settings dialog and the hotkey capture sub-dialog:
//! a small pure builder (unit-tested) plus the two layouts.

use super::*;

// Standard control class atoms (prefixed with 0xFFFF in a DLGITEMTEMPLATE).
pub(super) const ATOM_BUTTON: u16 = 0x0080;
const ATOM_EDIT: u16 = 0x0081;
pub(super) const ATOM_STATIC: u16 = 0x0082;
const ATOM_COMBOBOX: u16 = 0x0085;

// Window/dialog/control style bits (avoids pulling the typed constants into the template).
pub(super) const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_BORDER: u32 = 0x0080_0000;
pub(super) const WS_POPUP: u32 = 0x8000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
pub(super) const DS_SETFONT: u32 = 0x40;
const DS_MODALFRAME: u32 = 0x80;
const BS_AUTOCHECKBOX: u32 = 0x3;
const BS_DEFPUSHBUTTON: u32 = 0x1;
const ES_AUTOHSCROLL: u32 = 0x80;
const WS_VSCROLL: u32 = 0x0020_0000;
const CBS_DROPDOWNLIST: u32 = 0x3;

/// Builds an in-memory classic `DLGTEMPLATE` (+ items). Pure and unit-testable.
pub(super) struct DlgBuilder {
    buf: Vec<u8>,
    items: u16,
    cdit_pos: usize,
}

impl DlgBuilder {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
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
    pub(super) fn item(
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

    pub(super) fn build(mut self) -> Vec<u8> {
        let c = self.items.to_le_bytes();
        self.buf[self.cdit_pos] = c[0];
        self.buf[self.cdit_pos + 1] = c[1];
        self.buf
    }

    pub(super) fn w16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(super) fn wi16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(super) fn w32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub(super) fn wstr(&mut self, s: &str) {
        for u in s.encode_utf16() {
            self.w16(u);
        }
        self.w16(0);
    }
    pub(super) fn align(&mut self) {
        while !self.buf.len().is_multiple_of(4) {
            self.buf.push(0);
        }
    }
}

/// Convert the byte template into a DWORD-aligned buffer suitable for `DialogBoxIndirectParamW`.
pub(super) fn to_aligned(bytes: &[u8]) -> Vec<u32> {
    let mut v = bytes.to_vec();
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
    let (chunks, _) = v.as_chunks::<4>();
    chunks.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

pub(super) fn build_template() -> Vec<u8> {
    let chk = WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX;
    let edit = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL;
    let stat = WS_CHILD | WS_VISIBLE;
    let btn = WS_CHILD | WS_VISIBLE | WS_TABSTOP;

    let mut b = DlgBuilder::new(
        WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME,
        40,
        40,
        250,
        312,
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

    // Which microphone "Record microphone" uses (a drop-down list; its height is the open list).
    b.item(
        stat,
        10,
        96,
        55,
        10,
        LABEL_MIC as u16,
        ATOM_STATIC,
        "Microphone:",
    );
    b.item(
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST,
        65,
        94,
        175,
        120,
        COMBO_MIC as u16,
        ATOM_COMBOBOX,
        "",
    );

    b.item(
        stat,
        10,
        112,
        90,
        10,
        0xFFFF,
        ATOM_STATIC,
        "JPEG quality (1-100):",
    );
    b.item(edit, 105, 110, 40, 12, EDIT_QUALITY as u16, ATOM_EDIT, "");
    b.item(
        stat,
        10,
        128,
        90,
        10,
        0xFFFF,
        ATOM_STATIC,
        "Video frame rate (fps):",
    );
    b.item(edit, 105, 126, 40, 12, EDIT_FPS as u16, ATOM_EDIT, "");

    b.item(
        stat,
        10,
        146,
        190,
        10,
        0xFFFF,
        ATOM_STATIC,
        "Output folder:",
    );
    b.item(edit, 10, 158, 140, 13, EDIT_FOLDER as u16, ATOM_EDIT, "");
    b.item(
        btn,
        155,
        157,
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
        178,
        230,
        10,
        0xFFFF,
        ATOM_STATIC,
        "Global hotkeys (e.g. Ctrl+Shift+R; blank = off):",
    );
    for (i, label) in HK_LABELS.iter().enumerate() {
        let y = 192 + (i as i16) * 15;
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
        288,
        110,
        14,
        BTN_RESTORE_HK as u16,
        ATOM_BUTTON,
        "Restore default hotkeys",
    );
    b.item(
        btn | BS_DEFPUSHBUTTON,
        148,
        288,
        45,
        15,
        IDOK.0 as u16,
        ATOM_BUTTON,
        "OK",
    );
    b.item(
        btn,
        200,
        288,
        45,
        15,
        IDCANCEL.0 as u16,
        ATOM_BUTTON,
        "Cancel",
    );

    b.build()
}

/// The capture sub-dialog template: a prompt, one edit (subclassed), and OK/Cancel.
pub(super) fn build_capture_template() -> Vec<u8> {
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
