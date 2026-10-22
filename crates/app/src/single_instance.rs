//! Single-instance enforcement via a named Win32 mutex.
//!
//! The mutex is process-global (`Global\`-less, i.e. per-session, which is what we want for a
//! per-user tray app). Holding the returned guard keeps the instance "owned"; dropping it
//! releases the mutex so a later launch succeeds.

use anyhow::{Context, Result};
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

/// An owned single-instance lock. Drop releases it.
pub struct SingleInstance {
    handle: HANDLE,
}

impl SingleInstance {
    /// Try to become the single instance identified by `name`.
    ///
    /// Returns `Ok(Some(guard))` if we are the first/only instance, `Ok(None)` if another
    /// instance already holds the mutex, or `Err` on an unexpected OS failure.
    pub fn acquire(name: &str) -> Result<Option<Self>> {
        let wide = HSTRING::from(name);
        // SAFETY: `wide` outlives the call; CreateMutexW copies the name. A null security
        // attribute pointer requests the default descriptor.
        let handle = unsafe { CreateMutexW(None, true, &wide) }
            .context("CreateMutexW failed while enforcing single-instance")?;

        // SAFETY: GetLastError reads thread-local error state set by the call above.
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already {
            // Another instance owns it. Close our handle to the existing mutex and report.
            // SAFETY: `handle` is a valid handle returned by CreateMutexW.
            let _ = unsafe { CloseHandle(handle) };
            Ok(None)
        } else {
            Ok(Some(Self { handle }))
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: `handle` is valid and owned by this guard for its whole lifetime.
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

impl std::fmt::Debug for SingleInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SingleInstance").finish_non_exhaustive()
    }
}

// SAFETY: a Win32 mutex HANDLE is safe to move between threads; we only close it on drop.
unsafe impl Send for SingleInstance {}
