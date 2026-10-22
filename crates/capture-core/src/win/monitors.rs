//! Monitor enumeration via GDI + Per-Monitor DPI query.

use core::ffi::c_void;
use std::mem::size_of;

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

use crate::{dpi_to_scale, CaptureError, MonitorId, MonitorInfo, Rect, Result};

/// `MONITORINFOF_PRIMARY` — the primary-monitor flag in `MONITORINFO::dwFlags`.
const MONITORINFOF_PRIMARY: u32 = 0x0000_0001;

/// Enumerate all monitors with physical-pixel bounds, DPI scale, and primary flag.
pub fn enumerate() -> Result<Vec<MonitorInfo>> {
    let mut handles: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only pushes into the Vec addressed by `lparam`, which outlives the call.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut handles as *mut Vec<HMONITOR> as isize),
        );
    }

    let mut out = Vec::with_capacity(handles.len());
    for hmon in handles {
        if let Some(info) = describe(hmon) {
            out.push(info);
        }
    }
    if out.is_empty() {
        return Err(CaptureError::MonitorNotFound);
    }
    Ok(out)
}

/// Resolve a [`MonitorId`] back to its `HMONITOR`.
pub fn hmonitor_of(id: MonitorId) -> HMONITOR {
    HMONITOR(id.0 as *mut c_void)
}

fn describe(hmon: HMONITOR) -> Option<MonitorInfo> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: MONITORINFOEXW begins with its MONITORINFO field; cbSize tells the API to fill
    // the extended `szDevice` too.
    let ok = unsafe { GetMonitorInfoW(hmon, &mut info.monitorInfo as *mut MONITORINFO) };
    if !ok.as_bool() {
        return None;
    }
    let r = info.monitorInfo.rcMonitor;
    let bounds = Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top);

    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: valid monitor handle; both out-params are writable.
    let _ = unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };

    let name = String::from_utf16_lossy(&info.szDevice)
        .trim_end_matches('\0')
        .to_string();
    let is_primary = (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0;

    Some(MonitorInfo {
        id: MonitorId(hmon.0 as isize),
        name,
        bounds,
        scale_factor: dpi_to_scale(dpi_x),
        is_primary,
    })
}

unsafe extern "system" fn enum_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rc: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    // SAFETY: `lparam` carries the &mut Vec<HMONITOR> set up in `enumerate`.
    let handles = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
    handles.push(hmon);
    BOOL(1)
}
