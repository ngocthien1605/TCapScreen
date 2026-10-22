//! Run-at-startup via the per-user `Run` registry key. Enabling always (re)writes the value
//! with the current executable path, so a moved/portable build repairs a stale entry.

use anyhow::{Context, Result};
use windows::core::w;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

const RUN_KEY: windows::core::PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: windows::core::PCWSTR = w!("RustShot");

/// Add or remove the startup entry to match `enabled`.
pub fn apply(enabled: bool) -> Result<()> {
    // SAFETY: standard registry open/set/delete on HKCU; handle is closed before returning.
    unsafe {
        let mut hkey = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            None,
            windows::core::PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            None,
            &mut hkey,
            None,
        )
        .ok()
        .context("opening HKCU Run key")?;

        let result = if enabled {
            let exe = std::env::current_exe().context("current_exe")?;
            let quoted = format!("\"{}\"", exe.display());
            let wide: Vec<u16> = quoted.encode_utf16().chain([0]).collect();
            // REG_SZ data is the UTF-16 string including its NUL terminator, as bytes.
            let bytes = std::slice::from_raw_parts(
                wide.as_ptr() as *const u8,
                std::mem::size_of_val(&wide[..]),
            );
            RegSetValueExW(hkey, VALUE_NAME, None, REG_SZ, Some(bytes))
                .ok()
                .context("writing Run value")
        } else {
            // Deleting a missing value is fine.
            let _ = RegDeleteValueW(hkey, VALUE_NAME);
            Ok(())
        };

        let _ = RegCloseKey(hkey);
        result
    }
}
