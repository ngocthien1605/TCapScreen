//! RustShot configuration: settings schema, persistence, path resolution, and a
//! toolkit-agnostic hotkey model.
//!
//! The schema is forward-compatible: every field has a `#[serde(default)]` so a config
//! file written by an older build (missing newer keys) still loads. Load/save take explicit
//! paths; the [`paths`] module resolves the real `%APPDATA%\RustShot` locations.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod hotkey;
pub mod paths;

pub use hotkey::{Hotkey, HotkeyMap, Modifier};

/// Errors from loading or saving configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read or write config file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("config file {path} is not valid TOML: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("could not determine a standard directory for RustShot data")]
    NoBaseDir,
}

/// Image output format for screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
}

impl ImageFormat {
    /// File extension (without the dot) for this format.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }
}

/// Screenshot-related settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenshotSettings {
    /// Default output image format.
    pub format: ImageFormat,
    /// JPEG quality 1..=100 (ignored for PNG).
    pub jpeg_quality: u8,
    /// Include the mouse cursor in captures.
    pub include_cursor: bool,
    /// Also copy the captured image to the clipboard.
    pub copy_to_clipboard: bool,
    /// Optional delay before capture, in seconds (0 = immediate).
    pub delay_seconds: u8,
}

impl Default for ScreenshotSettings {
    fn default() -> Self {
        Self {
            format: ImageFormat::Png,
            jpeg_quality: 90,
            include_cursor: false,
            copy_to_clipboard: true,
            delay_seconds: 0,
        }
    }
}

/// Recording-related settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingSettings {
    /// Target frames per second (typically 15/30/60).
    pub fps: u32,
    /// Target video bitrate in bits per second.
    pub bitrate: u32,
    /// Include the mouse cursor in recordings.
    pub include_cursor: bool,
    /// Countdown length before recording starts, in seconds.
    pub countdown_seconds: u8,
    /// Capture the microphone.
    pub capture_microphone: bool,
    /// Capture system (loopback) audio.
    pub capture_system_audio: bool,
    /// Preferred microphone device name (None = system default).
    pub microphone_device: Option<String>,
    /// Maximum GIF width in pixels (longer edge is scaled to fit).
    pub gif_max_width: u32,
    /// Maximum GIF duration in seconds.
    pub gif_max_seconds: u32,
    /// GIF frame rate (usually lower than video).
    pub gif_fps: u32,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            fps: 30,
            bitrate: 8_000_000,
            include_cursor: true,
            countdown_seconds: 3,
            capture_microphone: false,
            capture_system_audio: false,
            microphone_device: None,
            gif_max_width: 800,
            gif_max_seconds: 30,
            gif_fps: 15,
        }
    }
}

/// Top-level settings persisted to `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where captures are saved. Empty string means "resolve the default at runtime".
    pub output_folder: PathBuf,
    /// Launch RustShot at Windows startup (opt-in `Run` key).
    pub run_at_startup: bool,
    pub screenshot: ScreenshotSettings,
    pub recording: RecordingSettings,
    pub hotkeys: HotkeyMap,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_folder: PathBuf::new(),
            run_at_startup: false,
            screenshot: ScreenshotSettings::default(),
            recording: RecordingSettings::default(),
            hotkeys: HotkeyMap::default(),
        }
    }
}

impl Settings {
    /// Resolve the effective output folder: the configured one, or the default
    /// (`<Pictures>\RustShot`, falling back to `%APPDATA%\RustShot\captures`).
    pub fn effective_output_folder(&self) -> Result<PathBuf, ConfigError> {
        if self.output_folder.as_os_str().is_empty() {
            paths::default_output_folder()
        } else {
            Ok(self.output_folder.clone())
        }
    }

    /// Clamp/repair any out-of-range values to safe defaults. Returns `true` if anything
    /// was changed (so the caller can persist the repaired config).
    pub fn sanitize(&mut self) -> bool {
        let mut changed = false;
        if !(1..=100).contains(&self.screenshot.jpeg_quality) {
            self.screenshot.jpeg_quality = 90;
            changed = true;
        }
        if !matches!(self.recording.fps, 1..=240) {
            self.recording.fps = 30;
            changed = true;
        }
        if self.recording.gif_fps == 0 || self.recording.gif_fps > 50 {
            self.recording.gif_fps = 15;
            changed = true;
        }
        if self.recording.bitrate == 0 {
            self.recording.bitrate = 8_000_000;
            changed = true;
        }
        changed
    }

    /// Parse settings from a TOML string.
    pub fn from_toml(s: &str, path: &Path) -> Result<Self, ConfigError> {
        toml::from_str(s).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Serialize settings to a TOML string.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Load settings from `path`. If the file does not exist, returns defaults (and does not
    /// create the file — the caller decides when to persist).
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml(&text, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Save settings to `path`, creating parent directories as needed. Writes atomically
    /// (to a temp file in the same directory, then rename) to avoid a half-written config.
    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        let text = self.to_toml()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text.as_bytes()).map_err(|source| ConfigError::Io {
            path: tmp.clone(),
            source,
        })?;
        std::fs::rename(&tmp, path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }

    /// Convenience: load from the standard `%APPDATA%\RustShot\config.toml`.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&paths::config_file()?)
    }

    /// Convenience: save to the standard `%APPDATA%\RustShot\config.toml`.
    pub fn save(&self) -> Result<(), ConfigError> {
        self.save_to(&paths::config_file()?)
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
    fn defaults_round_trip_through_toml() {
        let s = Settings::default();
        let toml_text = s.to_toml().unwrap();
        let back = Settings::from_toml(&toml_text, Path::new("test.toml")).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn partial_config_fills_missing_fields_with_defaults() {
        // Only output_folder specified; everything else must fall back to defaults.
        let toml_text = r#"output_folder = "C:\\shots""#;
        let s = Settings::from_toml(toml_text, Path::new("test.toml")).unwrap();
        assert_eq!(s.output_folder, PathBuf::from("C:\\shots"));
        assert_eq!(s.screenshot.format, ImageFormat::Png);
        assert_eq!(s.recording.fps, 30);
        assert!(
            !s.hotkeys.region_screenshot.mods.is_empty()
                || !s.hotkeys.region_screenshot.key.is_empty()
        );
    }

    #[test]
    fn unknown_keys_are_ignored_not_fatal() {
        // Forward-compat: a key from a newer build must not break an older parser.
        let toml_text = "run_at_startup = true\nsome_future_key = 42\n";
        let s = Settings::from_toml(toml_text, Path::new("t.toml")).unwrap();
        assert!(s.run_at_startup);
    }

    #[test]
    fn save_then_load_is_identity() {
        let dir = std::env::temp_dir().join(format!("rustshot_test_{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut s = Settings::default();
        s.recording.fps = 60;
        s.screenshot.format = ImageFormat::Jpeg;
        s.save_to(&path).unwrap();
        let back = Settings::load_from(&path).unwrap();
        assert_eq!(s, back);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let path = std::env::temp_dir().join("rustshot_definitely_missing_abc123.toml");
        let _ = std::fs::remove_file(&path);
        let s = Settings::load_from(&path).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn sanitize_repairs_out_of_range() {
        let mut s = Settings::default();
        s.screenshot.jpeg_quality = 200;
        s.recording.fps = 0;
        s.recording.gif_fps = 999;
        s.recording.bitrate = 0;
        assert!(s.sanitize());
        assert_eq!(s.screenshot.jpeg_quality, 90);
        assert_eq!(s.recording.fps, 30);
        assert_eq!(s.recording.gif_fps, 15);
        assert_eq!(s.recording.bitrate, 8_000_000);
        // Second pass is a no-op.
        assert!(!s.sanitize());
    }

    #[test]
    fn jpeg_quality_in_range_is_preserved() {
        let mut s = Settings::default();
        s.screenshot.jpeg_quality = 75;
        assert!(!s.sanitize());
        assert_eq!(s.screenshot.jpeg_quality, 75);
    }

    #[test]
    fn image_format_extensions() {
        assert_eq!(ImageFormat::Png.extension(), "png");
        assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
    }
}
