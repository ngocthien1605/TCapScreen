//! System tray icon + context menu and the Win32 message loop that drives them.
//!
//! M0 scope: the menu structure is in place (Screenshot/Record submenus are stubs that log
//! "not yet implemented"); Open-output-folder, Settings, and Quit are wired. Hotkeys, the
//! region overlay, capture, and the flyout arrive in M1/M2.

use std::path::PathBuf;

use anyhow::{Context, Result};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIconBuilder};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostQuitMessage, TranslateMessage, MSG,
};

use crate::icon::app_icon_rgba;

/// IDs of the menu items we react to (kept so the event handler can match them).
struct MenuIds {
    shot_region: MenuId,
    shot_window: MenuId,
    shot_monitor: MenuId,
    shot_all: MenuId,
    rec_region: MenuId,
    open_folder: MenuId,
    settings: MenuId,
    quit: MenuId,
}

/// Build the tray, install the menu-event handler, and run the message loop until Quit.
pub fn run(output_folder: PathBuf) -> Result<()> {
    let menu = Menu::new();

    let shot_region = MenuItem::new("Screenshot — Region", true, None);
    let shot_window = MenuItem::new("Screenshot — Window", true, None);
    let shot_monitor = MenuItem::new("Screenshot — Monitor", true, None);
    let shot_all = MenuItem::new("Screenshot — All monitors", true, None);
    let screenshot_menu = Submenu::new("Screenshot", true);
    screenshot_menu
        .append_items(&[&shot_region, &shot_window, &shot_monitor, &shot_all])
        .context("building Screenshot submenu")?;

    let rec_region = MenuItem::new("Record — Region", true, None);
    let record_menu = Submenu::new("Record", true);
    record_menu
        .append_items(&[&rec_region])
        .context("building Record submenu")?;

    let open_folder = MenuItem::new("Open output folder", true, None);
    let settings = MenuItem::new("Settings…", true, None);
    let quit = MenuItem::new("Quit RustShot", true, None);

    menu.append_items(&[
        &screenshot_menu,
        &record_menu,
        &PredefinedMenuItem::separator(),
        &open_folder,
        &settings,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .context("building tray menu")?;

    let ids = MenuIds {
        shot_region: shot_region.id().clone(),
        shot_window: shot_window.id().clone(),
        shot_monitor: shot_monitor.id().clone(),
        shot_all: shot_all.id().clone(),
        rec_region: rec_region.id().clone(),
        open_folder: open_folder.id().clone(),
        settings: settings.id().clone(),
        quit: quit.id().clone(),
    };

    let (rgba, w, h) = app_icon_rgba();
    let tray_icon = Icon::from_rgba(rgba, w, h).context("building tray icon")?;

    // The tray icon must stay alive for the whole loop.
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("RustShot — idle")
        .with_icon(tray_icon)
        .build()
        .context("creating tray icon")?;

    install_menu_handler(ids, output_folder);

    tracing::info!("RustShot is running in the tray");
    run_message_loop();
    tracing::info!("RustShot tray loop exited");
    Ok(())
}

fn install_menu_handler(ids: MenuIds, output_folder: PathBuf) {
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let id = event.id;
        if id == ids.quit {
            tracing::info!("Quit selected");
            // SAFETY: called on the message-loop thread; posts WM_QUIT to this thread's queue.
            unsafe { PostQuitMessage(0) };
        } else if id == ids.open_folder {
            open_folder(&output_folder);
        } else if id == ids.settings {
            tracing::info!("Settings selected (settings UI lands in M6)");
        } else if id == ids.shot_region
            || id == ids.shot_window
            || id == ids.shot_monitor
            || id == ids.shot_all
        {
            tracing::info!("Screenshot action selected (capture lands in M1/M2)");
        } else if id == ids.rec_region {
            tracing::info!("Record action selected (recording lands in M3)");
        }
    }));
}

fn open_folder(path: &PathBuf) {
    if let Err(e) = std::fs::create_dir_all(path) {
        tracing::warn!("could not create output folder {}: {e}", path.display());
    }
    if let Err(e) = std::process::Command::new("explorer").arg(path).spawn() {
        tracing::warn!("could not open output folder: {e}");
    }
}

/// Standard blocking Win32 message loop; returns when WM_QUIT is received.
fn run_message_loop() {
    // SAFETY: standard message pump; `msg` is a valid, writable MSG for the duration.
    unsafe {
        let mut msg = MSG::default();
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                // 0 = WM_QUIT, -1 = error: either way, stop pumping.
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
