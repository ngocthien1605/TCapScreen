//! The action bus. Menu clicks, global hotkeys, the delay timer, and flyout buttons all run in
//! `Fn + Send + Sync + 'static` callbacks that cannot touch application state directly, so they
//! instead `PostMessage` an [`Action`] to a hidden message-only window. Its window procedure
//! runs on the UI thread and owns all the state via a thread-local [`AppState`].

use std::cell::RefCell;
use std::path::PathBuf;

use anyhow::{Context, Result};
use config::Settings;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, PostMessageW, PostQuitMessage, RegisterClassW, CW_USEDEFAULT,
    HMENU, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WNDCLASSW,
};

use crate::flyout::Flyout;
use crate::{capture_flow, flyout};

/// Custom message carrying an [`Action`] in its `wParam`.
pub const WM_RUSTSHOT_ACTION: u32 = WM_APP + 1;

/// One user-facing action. `Copy` + `repr(u32)` so it round-trips through a `WPARAM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Action {
    ShotRegion = 1,
    ShotWindow,
    ShotMonitor,
    ShotAllMonitors,
    RecordRegion,
    StopRecording,
    ExecuteDelayedShot,
    OpenFolder,
    Settings,
    Quit,
    FlyoutOpen,
    FlyoutOpenFolder,
    FlyoutCopy,
    FlyoutCopyPath,
    FlyoutDelete,
    FlyoutDismiss,
    ExecuteRecordStart,
    RecordPauseToggle,
    RecordGif,
}

impl Action {
    fn from_u32(v: u32) -> Option<Action> {
        use Action::*;
        Some(match v {
            1 => ShotRegion,
            2 => ShotWindow,
            3 => ShotMonitor,
            4 => ShotAllMonitors,
            5 => RecordRegion,
            6 => StopRecording,
            7 => ExecuteDelayedShot,
            8 => OpenFolder,
            9 => Settings,
            10 => Quit,
            11 => FlyoutOpen,
            12 => FlyoutOpenFolder,
            13 => FlyoutCopy,
            14 => FlyoutCopyPath,
            15 => FlyoutDelete,
            16 => FlyoutDismiss,
            17 => ExecuteRecordStart,
            18 => RecordPauseToggle,
            19 => RecordGif,
            _ => return None,
        })
    }
}

/// Post an action to the dispatch window from any thread/callback.
pub fn post(hwnd: HWND, action: Action) {
    // SAFETY: PostMessageW is thread-safe; the window outlives all senders (it is destroyed only
    // on Quit, after which no more actions are posted).
    unsafe {
        let _ = PostMessageW(
            Some(hwnd),
            WM_RUSTSHOT_ACTION,
            WPARAM(action as u32 as usize),
            LPARAM(0),
        );
    }
}

/// All UI-thread-owned application state.
pub struct AppState {
    pub settings: Settings,
    pub output_folder: PathBuf,
    pub dispatch_hwnd: HWND,
    /// True while a capture/overlay is in progress; a second trigger is dropped.
    pub busy: bool,
    /// The most recent post-capture flyout, kept alive until dismissed/replaced.
    pub flyout: Option<Flyout>,
    /// The shot kind queued behind the delay timer.
    pub pending_shot: Option<capture_flow::ShotKind>,
    /// The in-progress recording (handle + toolbar), if any.
    pub recording: Option<crate::recording::Recording>,
    /// A recording request queued behind the countdown.
    pub pending_record: Option<recorder::RecordCfg>,
}

thread_local! {
    static APP: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

/// Run `f` with mutable access to the app state, if initialized.
pub fn with_app<R>(f: impl FnOnce(&mut AppState) -> R) -> Option<R> {
    APP.with(|cell| cell.borrow_mut().as_mut().map(f))
}

/// Create the hidden dispatch window and install `state` (which it takes ownership of). Returns
/// the window handle used as the action target.
pub fn create(settings: Settings, output_folder: PathBuf) -> Result<HWND> {
    const CLASS: PCWSTR = w!("RustShotDispatch");
    // SAFETY: standard window-class registration + message-only window creation.
    let hwnd = unsafe {
        let hinstance = GetModuleHandleW(None).context("GetModuleHandleW")?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        // Ignore a duplicate-class error (harmless if already registered).
        let _ = RegisterClassW(&wc);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("RustShot"),
            WINDOW_STYLE(0),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(HWND_MESSAGE),
            Some(HMENU(std::ptr::null_mut())),
            Some(hinstance.into()),
            None,
        )
        .context("creating dispatch window")?
    };

    APP.with(|cell| {
        *cell.borrow_mut() = Some(AppState {
            settings,
            output_folder,
            dispatch_hwnd: hwnd,
            busy: false,
            flyout: None,
            pending_shot: None,
            recording: None,
            pending_record: None,
        });
    });

    Ok(hwnd)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_RUSTSHOT_ACTION {
        if let Some(action) = Action::from_u32(wparam.0 as u32) {
            handle(action);
        }
        return LRESULT(0);
    }
    // Let the flyout's child-button notifications flow through if routed here in future.
    // SAFETY: standard default handling.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Route an [`Action`] to its handler on the UI thread.
fn handle(action: Action) {
    use Action::*;
    match action {
        ShotRegion => capture_flow::begin(capture_flow::ShotKind::Region),
        ShotWindow => capture_flow::begin(capture_flow::ShotKind::Window),
        ShotMonitor => capture_flow::begin(capture_flow::ShotKind::Monitor),
        ShotAllMonitors => capture_flow::begin(capture_flow::ShotKind::AllMonitors),
        ExecuteDelayedShot => capture_flow::execute_pending(),
        RecordRegion => crate::record_flow::begin(false),
        RecordGif => crate::record_flow::begin(true),
        ExecuteRecordStart => crate::record_flow::execute_start(),
        RecordPauseToggle => crate::record_flow::toggle_pause(),
        StopRecording => crate::record_flow::stop(),
        OpenFolder => {
            // Clone the path out, then shell out *outside* the borrow (explorer can pump).
            if let Some(folder) = with_app(|app| app.output_folder.clone()) {
                flyout::open_in_explorer(&folder);
            }
        }
        Settings => {
            tracing::info!("Settings selected (settings UI lands in M6)");
        }
        Quit => {
            tracing::info!("Quit selected");
            // SAFETY: posts WM_QUIT to this (UI) thread's queue.
            unsafe { PostQuitMessage(0) };
        }
        // For every flyout action we copy the path out under a short borrow, release it, and
        // only then run shell/clipboard work — which can pump messages and re-enter `handle`,
        // and must not find the `RefCell` still borrowed (that would panic in a wndproc).
        FlyoutOpen => {
            if let Some(p) = flyout_path() {
                flyout::shell_open(&p);
            }
        }
        FlyoutOpenFolder => {
            if let Some(p) = flyout_path() {
                flyout::select_in_explorer(&p);
            }
        }
        FlyoutCopy => {
            if let Some(p) = flyout_path() {
                match imaging::load_rgba(&p) {
                    Ok(img) => {
                        if let Err(e) = crate::clipboard::copy_image(&img) {
                            tracing::warn!("flyout copy failed: {e:#}");
                        }
                    }
                    Err(e) => tracing::warn!("could not reload image for copy: {e:#}"),
                }
            }
        }
        FlyoutCopyPath => {
            if let Some(p) = flyout_path() {
                if let Err(e) = crate::clipboard::copy_text(&p.to_string_lossy()) {
                    tracing::warn!("flyout copy-path failed: {e:#}");
                }
            }
        }
        FlyoutDelete => {
            // Take the flyout out (dropping it destroys the window), then delete the file.
            if let Some(f) = with_app(|app| app.flyout.take()).flatten() {
                let path = f.path().to_owned();
                drop(f);
                match std::fs::remove_file(&path) {
                    Ok(()) => tracing::info!("deleted {}", path.display()),
                    Err(e) => tracing::warn!("could not delete {}: {e}", path.display()),
                }
            }
        }
        FlyoutDismiss => {
            with_app(|app| {
                app.flyout = None;
            });
        }
    }
}

/// The current flyout's saved path, copied out under a short borrow (released before use).
fn flyout_path() -> Option<PathBuf> {
    with_app(|app| app.flyout.as_ref().map(|f| f.path().to_owned())).flatten()
}
