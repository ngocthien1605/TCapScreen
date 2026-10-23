//! Standard filesystem locations for RustShot data.
//!
//! Layout (Windows):
//! - base:   `%APPDATA%\RustShot`
//! - config: `%APPDATA%\RustShot\config.toml`
//! - logs:   `%APPDATA%\RustShot\logs`
//! - default output: `<Pictures>\RustShot`, falling back to `%APPDATA%\RustShot\captures`

use std::path::PathBuf;

use directories::{BaseDirs, UserDirs};

use crate::ConfigError;

const APP_DIR_NAME: &str = "RustShot";

/// `%APPDATA%\RustShot` — the base directory for all RustShot data.
pub fn app_dir() -> Result<PathBuf, ConfigError> {
    let base = BaseDirs::new().ok_or(ConfigError::NoBaseDir)?;
    // On Windows, `config_dir()` is `%APPDATA%` (Roaming).
    Ok(base.config_dir().join(APP_DIR_NAME))
}

/// `%APPDATA%\RustShot\config.toml`.
pub fn config_file() -> Result<PathBuf, ConfigError> {
    Ok(app_dir()?.join("config.toml"))
}

/// `%APPDATA%\RustShot\logs`.
pub fn logs_dir() -> Result<PathBuf, ConfigError> {
    Ok(app_dir()?.join("logs"))
}

/// The default output folder when the user has not chosen one:
/// `<Pictures>\RustShot`, else `%APPDATA%\RustShot\captures`.
pub fn default_output_folder() -> Result<PathBuf, ConfigError> {
    if let Some(user) = UserDirs::new() {
        if let Some(pics) = user.picture_dir() {
            return Ok(pics.join(APP_DIR_NAME));
        }
    }
    Ok(app_dir()?.join("captures"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn app_dir_ends_with_rustshot() {
        // Does not touch the filesystem; just resolves a path.
        let dir = app_dir().expect("APPDATA resolves on Windows");
        assert_eq!(dir.file_name().and_then(|s| s.to_str()), Some("RustShot"));
    }

    #[test]
    fn config_file_is_under_app_dir() {
        let (cfg, dir) = (
            config_file().expect("config path"),
            app_dir().expect("app dir"),
        );
        assert!(cfg.starts_with(&dir));
        assert_eq!(
            cfg.file_name().and_then(|s| s.to_str()),
            Some("config.toml")
        );
    }
}
