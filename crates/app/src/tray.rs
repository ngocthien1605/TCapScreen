//! System tray icon + context menu. Menu clicks are translated into dispatch [`Action`]s and
//! posted to the dispatch window; the tray holds no application state itself. The blocking
//! Win32 message loop lives in `main` and drives both this tray and the hotkey/overlay windows.

use anyhow::{Context, Result};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::Foundation::HWND;

use crate::dispatch::{self, Action};
use crate::icon::app_icon_rgba;

/// Menu-item ids we react to, paired with the action each posts.
struct Routes {
    entries: Vec<(MenuId, Action)>,
}

/// Build the tray icon + menu and install the menu-event handler. The returned [`TrayIcon`]
/// must be kept alive for the whole run.
pub fn build(dispatch_hwnd: HWND) -> Result<TrayIcon> {
    let menu = Menu::new();

    let shot_region = MenuItem::new("Screenshot — Region", true, None);
    let shot_window = MenuItem::new("Screenshot — Window", true, None);
    let shot_monitor = MenuItem::new("Screenshot — Monitor", true, None);
    let shot_all = MenuItem::new("Screenshot — All monitors", true, None);
    let screenshot_menu = Submenu::new("Screenshot", true);
    screenshot_menu
        .append_items(&[&shot_region, &shot_window, &shot_monitor, &shot_all])
        .context("building Screenshot submenu")?;

    let rec_region = MenuItem::new("Record — Region (M3)", true, None);
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

    let routes = Routes {
        entries: vec![
            (shot_region.id().clone(), Action::ShotRegion),
            (shot_window.id().clone(), Action::ShotWindow),
            (shot_monitor.id().clone(), Action::ShotMonitor),
            (shot_all.id().clone(), Action::ShotAllMonitors),
            (rec_region.id().clone(), Action::RecordRegion),
            (open_folder.id().clone(), Action::OpenFolder),
            (settings.id().clone(), Action::Settings),
            (quit.id().clone(), Action::Quit),
        ],
    };

    let (rgba, w, h) = app_icon_rgba();
    let tray_icon = Icon::from_rgba(rgba, w, h).context("building tray icon")?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("RustShot — idle")
        .with_icon(tray_icon)
        .build()
        .context("creating tray icon")?;

    install_menu_handler(routes, dispatch_hwnd);

    Ok(tray)
}

fn install_menu_handler(routes: Routes, dispatch_hwnd: HWND) {
    // HWND is not Send; carry it as isize and reconstruct on the UI thread where the handler runs.
    let hwnd_raw = dispatch_hwnd.0 as isize;
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some((_, action)) = routes.entries.iter().find(|(id, _)| *id == event.id) {
            // SAFETY: PostMessageW is thread-safe; the dispatch window outlives the tray.
            dispatch::post(HWND(hwnd_raw as *mut std::ffi::c_void), *action);
        }
    }));
}
