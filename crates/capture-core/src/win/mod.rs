//! The Windows (Windows.Graphics.Capture + Direct3D11) capture backend.
//!
//! For M1 a fresh D3D device + capture item + frame pool is created per `grab()` call, which
//! keeps [`WindowsBackend`] trivially `Send` (it holds no COM state) and costs only tens of
//! milliseconds — well inside the screenshot budget. Streaming in M3 will keep a device alive.

pub mod d3d;
pub mod monitors;
pub mod stream;
pub mod wgc;
pub mod windows_list;

use windows::Graphics::Capture::GraphicsCaptureItem;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::win::d3d::{map_com, D3dDevice};
use crate::{
    bounding_box, CaptureBackend, CaptureError, CaptureTarget, Frame, GrabOpts, MonitorId,
    MonitorInfo, PixelFmt, Point, Rect, Result, WindowId, WindowInfo,
};

/// The Windows capture backend.
#[derive(Debug)]
pub struct WindowsBackend;

impl WindowsBackend {
    /// Create the backend, initialising the WinRT multithreaded apartment for this thread.
    pub fn new() -> Result<Self> {
        // SAFETY: initialises the MTA for the calling thread. S_FALSE (already init) and
        // RPC_E_CHANGED_MODE (thread already STA) are both tolerated — WGC's free-threaded
        // frame pool does not require a specific apartment.
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        Ok(Self)
    }

    fn interop() -> Result<IGraphicsCaptureItemInterop> {
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(map_com)
    }

    fn item_for_monitor(hmon: HMONITOR) -> Result<GraphicsCaptureItem> {
        // SAFETY: `hmon` is a valid monitor handle from enumeration.
        unsafe { Self::interop()?.CreateForMonitor(hmon) }.map_err(map_com)
    }

    fn item_for_window(hwnd: HWND) -> Result<GraphicsCaptureItem> {
        // SAFETY: `hwnd` is a valid window handle from enumeration.
        unsafe { Self::interop()?.CreateForWindow(hwnd) }.map_err(map_com)
    }

    fn grab_monitor(id: MonitorId, opts: GrabOpts) -> Result<Frame> {
        let item = Self::item_for_monitor(monitors::hmonitor_of(id))?;
        let d3d = D3dDevice::create()?;
        let wdev = d3d.winrt_device()?;
        wgc::grab_item(&d3d, &wdev, &item, opts.include_cursor)
    }

    fn grab_window(id: WindowId, opts: GrabOpts) -> Result<Frame> {
        let item = Self::item_for_window(windows_list::hwnd_of(id))?;
        let d3d = D3dDevice::create()?;
        let wdev = d3d.winrt_device()?;
        wgc::grab_item(&d3d, &wdev, &item, opts.include_cursor)
    }

    fn grab_region(rect: Rect, monitor: MonitorId, opts: GrabOpts) -> Result<Frame> {
        if rect.is_empty() {
            return Err(CaptureError::EmptyRegion);
        }
        let mons = monitors::enumerate()?;
        let mon = mons
            .iter()
            .find(|m| m.id == monitor)
            .ok_or(CaptureError::MonitorNotFound)?;
        let frame = Self::grab_monitor(monitor, opts)?;
        // Convert the virtual-desktop rect into monitor-local pixels and clamp to the frame.
        let local = rect.to_local(Point::new(mon.bounds.x, mon.bounds.y));
        let clamped = local
            .clamp_to(&Rect::new(0, 0, frame.width as i32, frame.height as i32))
            .ok_or(CaptureError::EmptyRegion)?;
        Ok(frame.crop(clamped))
    }

    fn grab_virtual_desktop(opts: GrabOpts) -> Result<Frame> {
        let mons = monitors::enumerate()?;
        let bbox = bounding_box(&mons.iter().map(|m| m.bounds).collect::<Vec<_>>());
        if bbox.is_empty() {
            return Err(CaptureError::EmptyRegion);
        }
        let d3d = D3dDevice::create()?;
        let wdev = d3d.winrt_device()?;

        let dest_stride = bbox.width as usize * 4;
        let mut data = vec![0u8; dest_stride * bbox.height as usize];
        for m in &mons {
            let item = Self::item_for_monitor(monitors::hmonitor_of(m.id))?;
            let frame = wgc::grab_item(&d3d, &wdev, &item, opts.include_cursor)?;
            let at_x = m.bounds.x - bbox.x;
            let at_y = m.bounds.y - bbox.y;
            frame.blit_into(&mut data, bbox.width as u32, bbox.height as u32, at_x, at_y);
        }
        Ok(Frame {
            data,
            width: bbox.width as u32,
            height: bbox.height as u32,
            stride: dest_stride,
            format: PixelFmt::Bgra8,
            timestamp: std::time::Duration::ZERO,
        })
    }
}

impl CaptureBackend for WindowsBackend {
    fn monitors(&self) -> Result<Vec<MonitorInfo>> {
        monitors::enumerate()
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        windows_list::enumerate()
    }

    fn grab(&self, target: &CaptureTarget, opts: GrabOpts) -> Result<Frame> {
        match *target {
            CaptureTarget::Monitor(id) => Self::grab_monitor(id, opts),
            CaptureTarget::Window(id) => Self::grab_window(id, opts),
            CaptureTarget::Region { rect, monitor } => Self::grab_region(rect, monitor, opts),
            CaptureTarget::VirtualDesktop => Self::grab_virtual_desktop(opts),
        }
    }
}
