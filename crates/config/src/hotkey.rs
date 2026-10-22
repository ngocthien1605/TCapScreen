//! A toolkit-agnostic hotkey model.
//!
//! Hotkeys are stored as plain data (`mods` + `key`) so parsing, normalization, and
//! conflict detection are pure and unit-testable, independent of any hotkey library. The
//! `app` crate maps these to `global_hotkey` codes at registration time.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A keyboard modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Win,
}

impl Modifier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ctrl => "Ctrl",
            Self::Alt => "Alt",
            Self::Shift => "Shift",
            Self::Win => "Win",
        }
    }

    fn parse(token: &str) -> Option<Self> {
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "ctl" => Some(Self::Ctrl),
            "alt" => Some(Self::Alt),
            "shift" => Some(Self::Shift),
            "win" | "super" | "meta" | "cmd" => Some(Self::Win),
            _ => None,
        }
    }
}

/// Error parsing a hotkey from a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyParseError(pub String);

impl fmt::Display for HotkeyParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid hotkey: {}", self.0)
    }
}

impl std::error::Error for HotkeyParseError {}

/// A hotkey: zero or more modifiers plus a single main key (e.g. `Ctrl+Shift+R`,
/// `PrintScreen`). Modifiers are normalized (deduplicated and sorted) so that logically
/// equal hotkeys compare equal regardless of how they were written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: Vec<Modifier>,
    /// The main key, normalized to a canonical spelling (see [`Hotkey::normalize_key`]).
    pub key: String,
}

impl Hotkey {
    /// Build a hotkey from parts, normalizing modifiers and the key.
    pub fn new(mut mods: Vec<Modifier>, key: impl Into<String>) -> Self {
        mods.sort();
        mods.dedup();
        Self {
            mods,
            key: Self::normalize_key(&key.into()),
        }
    }

    /// Canonicalize a main-key spelling so `printscreen`, `PrtSc`, `prt_scr` etc. unify.
    pub fn normalize_key(raw: &str) -> String {
        let cleaned = raw.trim();
        match cleaned
            .to_ascii_lowercase()
            .replace([' ', '_', '-'], "")
            .as_str()
        {
            "printscreen" | "prtsc" | "prtscr" | "prtscn" | "print" => "PrintScreen".to_string(),
            "esc" | "escape" => "Escape".to_string(),
            "enter" | "return" => "Enter".to_string(),
            "space" | "spacebar" => "Space".to_string(),
            "del" | "delete" => "Delete".to_string(),
            "ins" | "insert" => "Insert".to_string(),
            "pgup" | "pageup" => "PageUp".to_string(),
            "pgdn" | "pagedown" => "PageDown".to_string(),
            // Single letters -> uppercase; function keys -> F<N>; otherwise title-case-ish.
            other if other.len() == 1 => other.to_ascii_uppercase(),
            other
                if other.starts_with('f')
                    && other[1..].chars().all(|c| c.is_ascii_digit())
                    && other.len() > 1 =>
            {
                format!("F{}", &other[1..])
            }
            _ => {
                // Title-case the first char, keep the rest as-is lowercased.
                let mut chars = cleaned.chars();
                match chars.next() {
                    Some(first) => {
                        first.to_ascii_uppercase().to_string()
                            + &chars.as_str().to_ascii_lowercase()
                    }
                    None => String::new(),
                }
            }
        }
    }

    /// True if this hotkey has no main key (unbound).
    pub fn is_empty(&self) -> bool {
        self.key.is_empty()
    }

    /// A key used for conflict comparison: normalized mods + normalized key, case-insensitive.
    fn conflict_key(&self) -> (Vec<Modifier>, String) {
        (self.mods.clone(), self.key.to_ascii_lowercase())
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for m in &self.mods {
            write!(f, "{}+", m.as_str())?;
        }
        write!(f, "{}", self.key)
    }
}

impl FromStr for Hotkey {
    type Err = HotkeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Ok(Hotkey {
                mods: Vec::new(),
                key: String::new(),
            });
        }
        let parts: Vec<&str> = trimmed
            .split('+')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            return Err(HotkeyParseError(s.to_string()));
        }
        let mut mods = Vec::new();
        let mut key: Option<String> = None;
        for part in parts {
            if let Some(m) = Modifier::parse(part) {
                mods.push(m);
            } else if key.is_none() {
                key = Some(part.to_string());
            } else {
                // Two non-modifier tokens: ambiguous.
                return Err(HotkeyParseError(s.to_string()));
            }
        }
        match key {
            Some(k) => Ok(Hotkey::new(mods, k)),
            None => Err(HotkeyParseError(s.to_string())),
        }
    }
}

impl Serialize for Hotkey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Hotkey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Hotkey::from_str(&s).map_err(serde::de::Error::custom)
    }
}

/// The full set of configurable hotkeys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyMap {
    pub region_screenshot: Hotkey,
    pub window_screenshot: Hotkey,
    pub monitor_screenshot: Hotkey,
    pub all_monitors_screenshot: Hotkey,
    pub region_recording: Hotkey,
    pub stop_recording: Hotkey,
}

impl Default for HotkeyMap {
    fn default() -> Self {
        Self {
            region_screenshot: Hotkey::new(vec![], "PrintScreen"),
            window_screenshot: Hotkey::new(vec![Modifier::Ctrl], "PrintScreen"),
            monitor_screenshot: Hotkey::new(vec![Modifier::Alt], "PrintScreen"),
            all_monitors_screenshot: Hotkey::new(vec![Modifier::Shift], "PrintScreen"),
            region_recording: Hotkey::new(vec![Modifier::Ctrl, Modifier::Shift], "R"),
            stop_recording: Hotkey::new(vec![Modifier::Ctrl, Modifier::Shift], "S"),
        }
    }
}

impl HotkeyMap {
    /// All hotkeys as (name, hotkey) pairs, in a stable order.
    pub fn entries(&self) -> Vec<(&'static str, &Hotkey)> {
        vec![
            ("region_screenshot", &self.region_screenshot),
            ("window_screenshot", &self.window_screenshot),
            ("monitor_screenshot", &self.monitor_screenshot),
            ("all_monitors_screenshot", &self.all_monitors_screenshot),
            ("region_recording", &self.region_recording),
            ("stop_recording", &self.stop_recording),
        ]
    }

    /// Detect conflicts: pairs of actions bound to the same (non-empty) key combination.
    /// Returns pairs of action names, each unordered pair reported once.
    pub fn conflicts(&self) -> Vec<(&'static str, &'static str)> {
        let entries = self.entries();
        let mut out = Vec::new();
        for i in 0..entries.len() {
            let (name_a, hk_a) = entries[i];
            if hk_a.is_empty() {
                continue;
            }
            for &(name_b, hk_b) in entries.iter().skip(i + 1) {
                if hk_b.is_empty() {
                    continue;
                }
                if hk_a.conflict_key() == hk_b.conflict_key() {
                    out.push((name_a, name_b));
                }
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_and_modified() {
        let h: Hotkey = "PrintScreen".parse().unwrap();
        assert!(h.mods.is_empty());
        assert_eq!(h.key, "PrintScreen");

        let h: Hotkey = "Ctrl+Shift+R".parse().unwrap();
        assert_eq!(h.mods, vec![Modifier::Ctrl, Modifier::Shift]);
        assert_eq!(h.key, "R");
    }

    #[test]
    fn modifier_order_and_dups_normalized() {
        let a: Hotkey = "Shift+Ctrl+R".parse().unwrap();
        let b: Hotkey = "Ctrl+Shift+R".parse().unwrap();
        assert_eq!(a, b);
        let c: Hotkey = "Ctrl+Ctrl+R".parse().unwrap();
        assert_eq!(c.mods, vec![Modifier::Ctrl]);
    }

    #[test]
    fn key_spelling_aliases_unify() {
        for s in ["PrintScreen", "prtsc", "Print Screen", "prt_scr", "PRTSCN"] {
            let h: Hotkey = s.parse().unwrap();
            assert_eq!(h.key, "PrintScreen", "input {s}");
        }
        assert_eq!("escape".parse::<Hotkey>().unwrap().key, "Escape");
        assert_eq!("f5".parse::<Hotkey>().unwrap().key, "F5");
        assert_eq!("f12".parse::<Hotkey>().unwrap().key, "F12");
    }

    #[test]
    fn display_round_trips_through_parse() {
        let h = Hotkey::new(vec![Modifier::Ctrl, Modifier::Shift], "R");
        let s = h.to_string();
        assert_eq!(s, "Ctrl+Shift+R");
        assert_eq!(s.parse::<Hotkey>().unwrap(), h);
    }

    #[test]
    fn empty_string_is_unbound() {
        let h: Hotkey = "".parse().unwrap();
        assert!(h.is_empty());
        assert_eq!(h.to_string(), "");
    }

    #[test]
    fn ambiguous_two_keys_is_error() {
        assert!("A+B".parse::<Hotkey>().is_err());
    }

    #[test]
    fn serde_is_a_plain_string() {
        let h = Hotkey::new(vec![Modifier::Ctrl], "PrintScreen");
        let json = serde_json_like(&h);
        assert_eq!(json, "\"Ctrl+PrintScreen\"");
    }

    // Tiny helper: TOML needs a table context, so round-trip through a 1-field wrapper.
    fn serde_json_like(h: &Hotkey) -> String {
        #[derive(Serialize)]
        struct W<'a> {
            h: &'a Hotkey,
        }
        let t = toml::to_string(&W { h }).unwrap();
        // t is: h = "Ctrl+PrintScreen"\n
        t.trim().trim_start_matches("h = ").to_string()
    }

    #[test]
    fn default_map_has_no_conflicts() {
        assert!(HotkeyMap::default().conflicts().is_empty());
    }

    #[test]
    fn detects_conflict_when_two_actions_share_a_combo() {
        let mut m = HotkeyMap::default();
        m.window_screenshot = Hotkey::new(vec![], "PrintScreen"); // same as region_screenshot
        let c = m.conflicts();
        assert_eq!(c.len(), 1);
        let pair = c[0];
        assert!(
            (pair.0 == "region_screenshot" && pair.1 == "window_screenshot")
                || (pair.0 == "window_screenshot" && pair.1 == "region_screenshot")
        );
    }

    #[test]
    fn unbound_hotkeys_never_conflict() {
        let mut m = HotkeyMap::default();
        m.region_screenshot = Hotkey::new(vec![], "");
        m.window_screenshot = Hotkey::new(vec![], "");
        assert!(m.conflicts().is_empty());
    }
}
