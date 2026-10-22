//! Global hotkey registration, bridging the toolkit-agnostic [`config::Hotkey`] model to the
//! `global-hotkey` crate and routing presses to dispatch [`Action`]s.
//!
//! The string→`Code` mapping and modifier translation are pure and unit-tested. Actual
//! registration (which needs a live message loop) is best-effort: a key Windows has already
//! claimed (e.g. PrintScreen → Snipping Tool) logs a warning instead of failing startup.

use std::collections::{BTreeMap, HashSet};
use std::ffi::c_void;
use std::str::FromStr;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Mutex, Once};

use config::{Hotkey, HotkeyMap, Modifier};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use windows::Win32::Foundation::HWND;

use crate::dispatch::{self, Action};

/// Translate our modifier set into `global-hotkey` modifiers.
pub fn mods_to_modifiers(mods: &[Modifier]) -> Modifiers {
    let mut m = Modifiers::empty();
    for modifier in mods {
        m |= match modifier {
            Modifier::Ctrl => Modifiers::CONTROL,
            Modifier::Alt => Modifiers::ALT,
            Modifier::Shift => Modifiers::SHIFT,
            Modifier::Win => Modifiers::META,
        };
    }
    m
}

/// Translate a normalized [`config::Hotkey`] main key into a W3C [`Code`].
///
/// `config::Hotkey::normalize_key` yields spellings like `A`, `5`, `F5`, `PrintScreen`,
/// `Escape`, `Enter`, `Space`, `Delete`, `Insert`, `PageUp`, `PageDown`. We convert single
/// letters/digits to their `KeyX`/`DigitN` code names and parse the rest directly.
pub fn key_to_code(key: &str) -> Option<Code> {
    if key.is_empty() {
        return None;
    }
    let w3c: String = if key.len() == 1 {
        let c = key.chars().next()?;
        if c.is_ascii_alphabetic() {
            format!("Key{}", c.to_ascii_uppercase())
        } else if c.is_ascii_digit() {
            format!("Digit{c}")
        } else {
            return None;
        }
    } else {
        key.to_string()
    };
    Code::from_str(&w3c).ok()
}

/// Build a `global-hotkey` [`HotKey`] from our model, or `None` if unbound/unmappable.
pub fn to_hotkey(hk: &Hotkey) -> Option<HotKey> {
    if hk.is_empty() {
        return None;
    }
    let code = key_to_code(&hk.key)?;
    Some(HotKey::new(Some(mods_to_modifiers(&hk.mods)), code))
}

/// Hotkey id -> action table consulted by the single global event handler. `global-hotkey` keeps
/// only the *first* handler ever installed (a `OnceCell`), so re-registering after Settings must
/// swap this table rather than install a new handler — otherwise rebound keys never fire.
static ROUTES: Mutex<BTreeMap<u32, Action>> = Mutex::new(BTreeMap::new());
/// The dispatch window the handler posts to (an `HWND` stored as an integer; it is not `Send`).
static DISPATCH_HWND: AtomicIsize = AtomicIsize::new(0);
static INSTALL_HANDLER: Once = Once::new();

/// Replace the active hotkey routes.
fn set_routes(routes: BTreeMap<u32, Action>) {
    *ROUTES.lock().unwrap_or_else(|e| e.into_inner()) = routes;
}

/// The action bound to a registered hotkey id, if any.
fn route_for(id: u32) -> Option<Action> {
    ROUTES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .copied()
}

/// Owns the registered hotkeys. Kept alive for the whole run (dropping it unregisters them).
pub struct Hotkeys {
    _manager: GlobalHotKeyManager,
}

/// Register all bound, non-conflicting hotkeys and install an event handler that posts the
/// bound [`Action`] to the dispatch window on each press. Returns `None` if the manager itself
/// cannot be created; individual key failures are logged and skipped so the app still runs.
pub fn register(map: &HotkeyMap, dispatch_hwnd: HWND) -> Option<Hotkeys> {
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("global hotkeys unavailable ({e}); continuing without them");
            return None;
        }
    };

    // Don't register either side of a conflicting pair — ambiguous bindings help no one.
    let conflicts = map.conflicts();
    let conflicting: HashSet<&'static str> = conflicts.iter().flat_map(|(a, b)| [*a, *b]).collect();

    let bindings: [(&'static str, &Hotkey, Action); 6] = [
        (
            "region_screenshot",
            &map.region_screenshot,
            Action::ShotRegion,
        ),
        (
            "window_screenshot",
            &map.window_screenshot,
            Action::ShotWindow,
        ),
        (
            "monitor_screenshot",
            &map.monitor_screenshot,
            Action::ShotMonitor,
        ),
        (
            "all_monitors_screenshot",
            &map.all_monitors_screenshot,
            Action::ShotAllMonitors,
        ),
        (
            "region_recording",
            &map.region_recording,
            Action::RecordRegion,
        ),
        ("stop_recording", &map.stop_recording, Action::StopRecording),
    ];

    let mut routes = BTreeMap::new();
    for (name, hk, action) in bindings {
        if conflicting.contains(name) {
            tracing::warn!("hotkey '{name}' ({hk}) skipped: conflicts with another binding");
            continue;
        }
        let Some(ghk) = to_hotkey(hk) else {
            if !hk.is_empty() {
                tracing::warn!("hotkey '{name}' ({hk}) could not be mapped; skipping");
            }
            continue;
        };
        match manager.register(ghk) {
            Ok(()) => {
                routes.insert(ghk.id(), action);
                tracing::info!("registered hotkey '{name}' = {hk}");
            }
            Err(e) => {
                tracing::warn!(
                    "could not register hotkey '{name}' ({hk}): {e}; \
                     it may be claimed by another app or Windows"
                );
            }
        }
    }

    // Publish the new routes, then make sure the (one and only) handler is installed. It fires on
    // the thread that created the manager — the UI thread — and looks routes up at press time.
    set_routes(routes);
    DISPATCH_HWND.store(dispatch_hwnd.0 as isize, Ordering::SeqCst);
    INSTALL_HANDLER.call_once(|| {
        GlobalHotKeyEvent::set_event_handler(Some(|ev: GlobalHotKeyEvent| {
            // Called from the hotkey crate's own window procedure: contain panics.
            crate::panic_guard::guard(
                "hotkey handler",
                || (),
                || {
                    if ev.state != HotKeyState::Pressed {
                        return;
                    }
                    if let Some(action) = route_for(ev.id) {
                        let hwnd = HWND(DISPATCH_HWND.load(Ordering::SeqCst) as *mut c_void);
                        dispatch::post(hwnd, action);
                    }
                },
            );
        }));
    });

    Some(Hotkeys { _manager: manager })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_digits_map_to_codes() {
        assert_eq!(key_to_code("A"), Some(Code::KeyA));
        assert_eq!(key_to_code("R"), Some(Code::KeyR));
        assert_eq!(key_to_code("5"), Some(Code::Digit5));
        assert_eq!(key_to_code("0"), Some(Code::Digit0));
    }

    #[test]
    fn named_keys_map_to_codes() {
        assert_eq!(key_to_code("PrintScreen"), Some(Code::PrintScreen));
        assert_eq!(key_to_code("Escape"), Some(Code::Escape));
        assert_eq!(key_to_code("Enter"), Some(Code::Enter));
        assert_eq!(key_to_code("F5"), Some(Code::F5));
        assert_eq!(key_to_code("F12"), Some(Code::F12));
    }

    #[test]
    fn empty_or_garbage_is_none() {
        assert_eq!(key_to_code(""), None);
        assert_eq!(key_to_code("Nonsense"), None);
    }

    #[test]
    fn modifiers_translate() {
        let m = mods_to_modifiers(&[Modifier::Ctrl, Modifier::Shift]);
        assert!(m.contains(Modifiers::CONTROL));
        assert!(m.contains(Modifiers::SHIFT));
        assert!(!m.contains(Modifiers::ALT));
    }

    #[test]
    fn default_map_all_map_to_hotkeys() {
        // Every default binding must be representable as a global hotkey.
        for (name, hk) in HotkeyMap::default().entries() {
            assert!(to_hotkey(hk).is_some(), "{name} ({hk}) did not map");
        }
    }

    #[test]
    fn rebinding_replaces_routes_instead_of_keeping_stale_ones() {
        // Regression: the handler used to capture the *first* routes forever, so a hotkey
        // rebound from Settings registered with Windows but never triggered an action.
        set_routes(BTreeMap::from([(900_001, Action::ShotRegion)]));
        assert_eq!(route_for(900_001), Some(Action::ShotRegion));

        set_routes(BTreeMap::from([(900_002, Action::ShotMonitor)]));
        assert_eq!(
            route_for(900_002),
            Some(Action::ShotMonitor),
            "new binding must route"
        );
        assert_eq!(route_for(900_001), None, "old binding must be gone");
    }

    #[test]
    fn unbound_is_none() {
        let hk = Hotkey::new(vec![], "");
        assert!(to_hotkey(&hk).is_none());
    }
}
