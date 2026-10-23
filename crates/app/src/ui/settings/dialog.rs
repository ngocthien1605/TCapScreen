//! The Settings dialog itself: opening it, its dialog procedure, moving values between the
//! controls and `Settings`, the Browse picker, applying the result, and the render self-tests.

use super::*;

/// Settings passed into the dialog and the edited result passed back out.
struct Draft {
    initial: Settings,
    result: Option<Settings>,
    /// The microphone drop-down's entries, in order (filled when the dialog opens).
    mics: Vec<MicChoice>,
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
        mics: Vec::new(),
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
        dispatch::refresh_tray_tooltip();
    }

    // Release the gate only now: `apply` may show notices (a hotkey Windows refused, a folder
    // that fell back), and while such a box pumps messages a hotkey press or a second
    // "Settings…" must still be refused rather than nest inside this call.
    dispatch::with_app(|a| a.busy = false);
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
    let report = hk.as_ref().and_then(|h| h.unregistered_report());
    dispatch::with_app(|a| a.hotkeys = hk);
    dispatch::refresh_tray_tooltip();
    if let Some(report) = report {
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
        mics: Vec::new(),
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
                let draft = &mut *(lparam.0 as *mut Draft);
                populate(hwnd, &draft.initial);
                draft.mics = populate_mics(hwnd, None);
                sync_mic_enabled(hwnd);
                tracing::info!("settings selftest: {} microphone entries", draft.mics.len());
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

pub(super) unsafe extern "system" fn dlgproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
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
                let draft = &mut *(lparam.0 as *mut Draft);
                populate(hwnd, &draft.initial);
                draft.mics =
                    populate_mics(hwnd, draft.initial.recording.microphone_device.as_deref());
                sync_mic_enabled(hwnd);
            }
            show_in_taskbar(hwnd);
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
                    match try_gather(hwnd, &d.initial, &d.mics) {
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
            } else if id == CHK_MIC {
                sync_mic_enabled(hwnd);
                0
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

pub(super) fn is_checked(hwnd: HWND, id: i32) -> bool {
    // SAFETY: reads a child button's check state.
    unsafe { IsDlgButtonChecked(hwnd, id) == BST_CHECKED.0 }
}

/// The value to store for the output-folder field. The dialog displays the *resolved* folder, so
/// an untouched default must be saved back as empty ("use the default") rather than pinned as an
/// explicit path.
pub(super) fn folder_setting(text: &str, default: Option<PathBuf>) -> PathBuf {
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
pub(super) fn validate_folder(text: &str) -> Result<(), String> {
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
fn try_gather(hwnd: HWND, initial: &Settings, mics: &[MicChoice]) -> Result<Settings, String> {
    let mut s = initial.clone();
    s.screenshot.include_cursor = is_checked(hwnd, CHK_CURSOR);
    s.screenshot.copy_to_clipboard = is_checked(hwnd, CHK_CLIP);
    s.recording.capture_microphone = is_checked(hwnd, CHK_MIC);
    s.recording.capture_system_audio = is_checked(hwnd, CHK_SYS);
    s.recording.microphone_device = selected_mic(hwnd, mics);
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

pub(super) fn get_text(hwnd: HWND, id: i32) -> String {
    let mut buf = [0u16; 1024];
    // SAFETY: fills `buf` with up to its length; returns the copied length.
    let n = unsafe { GetDlgItemTextW(hwnd, id, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn browse(hwnd: HWND) {
    // Owned by the Settings dialog, so the picker is modal to it: OK/Cancel and a second
    // Browse can't run underneath it, and the result can't land in a dialog that has ended.
    if let Some(folder) = rfd::FileDialog::new()
        .set_title("Choose the RustShot output folder")
        .set_parent(&Owner(hwnd))
        .pick_folder()
    {
        let wide: Vec<u16> = folder.to_string_lossy().encode_utf16().chain([0]).collect();
        // SAFETY: sets the folder edit control's text.
        unsafe {
            let _ = SetDlgItemTextW(hwnd, EDIT_FOLDER, PCWSTR(wide.as_ptr()));
        }
    }
}

/// A dialog window as a parent for `rfd`'s pickers.
struct Owner(HWND);

impl raw_window_handle::HasWindowHandle for Owner {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let hwnd = std::num::NonZeroIsize::new(self.0 .0 as isize)
            .ok_or(raw_window_handle::HandleError::Unavailable)?;
        let raw = raw_window_handle::RawWindowHandle::Win32(
            raw_window_handle::Win32WindowHandle::new(hwnd),
        );
        // SAFETY: the dialog outlives the modal picker that borrows its handle.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(raw) })
    }
}

impl raw_window_handle::HasDisplayHandle for Owner {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(raw_window_handle::DisplayHandle::windows())
    }
}

/// Give the dialog a taskbar button with the app icon, and bring it to the front. Its owner is
/// the hidden dispatch window, so without this it has no taskbar entry and is easily lost
/// behind other windows (a tray app has no main window to find it under). Called before the
/// dialog is first shown, which is when Windows picks up `WS_EX_APPWINDOW`.
fn show_in_taskbar(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        LoadIconW, SendMessageW, GWL_EXSTYLE, ICON_BIG, ICON_SMALL, WM_SETICON, WS_EX_APPWINDOW,
    };
    // SAFETY: style and icon changes on our own dialog; the icon is a shared resource icon
    // owned by the module, so it is never destroyed by us.
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_APPWINDOW.0 as isize);
        if let Ok(module) = GetModuleHandleW(None) {
            // Resource id 1 is the app icon (app.rc).
            if let Ok(icon) =
                LoadIconW(Some(module.into()), PCWSTR(std::ptr::without_provenance(1)))
            {
                for size in [ICON_BIG, ICON_SMALL] {
                    SendMessageW(
                        hwnd,
                        WM_SETICON,
                        Some(WPARAM(size as usize)),
                        Some(LPARAM(icon.0 as isize)),
                    );
                }
            }
        }
    }
    crate::platform::bring_to_front(hwnd);
}
