//! Continuous Windows.Graphics.Capture session used by the recorder. Unlike the one-shot
//! [`super::wgc::grab_item`], this keeps a session alive and delivers every frame to a channel
//! until stopped (or until the captured item is closed). Frame copies happen on the capture
//! thread-pool threads, serialized through a mutex because the D3D immediate context is not
//! thread-safe.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::IInspectable;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::win::d3d::{map_com, D3dDevice};
use crate::win::{monitors, windows_list};
use crate::{CaptureError, CaptureTarget, Frame, Point, Rect, Result};

/// An event delivered by a running [`CaptureStream`].
#[derive(Debug)]
pub enum StreamEvent {
    /// A newly captured frame (already cropped to the target region, if any).
    Frame(Frame),
    /// The captured item was closed (monitor disconnected / window destroyed).
    Closed,
}

/// A live capture session feeding frames to a channel.
#[allow(missing_debug_implementations)]
pub struct CaptureStream {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    item: GraphicsCaptureItem,
    frame_token: i64,
    closed_token: i64,
    _d3d: Arc<Mutex<D3dDevice>>,
    start: Instant,
}

fn interop() -> Result<IGraphicsCaptureItemInterop> {
    windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(map_com)
}

fn item_for_monitor(hmon: HMONITOR) -> Result<GraphicsCaptureItem> {
    // SAFETY: valid monitor handle from enumeration.
    unsafe { interop()?.CreateForMonitor(hmon) }.map_err(map_com)
}

fn item_for_window(hwnd: HWND) -> Result<GraphicsCaptureItem> {
    // SAFETY: valid window handle from enumeration.
    unsafe { interop()?.CreateForWindow(hwnd) }.map_err(map_com)
}

/// Resolve a capture target into a WGC item plus an optional monitor-local crop rect.
fn resolve(target: &CaptureTarget) -> Result<(GraphicsCaptureItem, Option<Rect>)> {
    match *target {
        CaptureTarget::Monitor(id) => Ok((item_for_monitor(monitors::hmonitor_of(id))?, None)),
        CaptureTarget::Window(id) => Ok((item_for_window(windows_list::hwnd_of(id))?, None)),
        CaptureTarget::Region { rect, monitor } => {
            let mons = monitors::enumerate()?;
            let mon = mons
                .iter()
                .find(|m| m.id == monitor)
                .ok_or(CaptureError::MonitorNotFound)?;
            let local = rect.to_local(Point::new(mon.bounds.x, mon.bounds.y));
            Ok((
                item_for_monitor(monitors::hmonitor_of(monitor))?,
                Some(local),
            ))
        }
        CaptureTarget::VirtualDesktop => Err(CaptureError::Platform(
            "recording the whole virtual desktop is not supported; pick a monitor".into(),
        )),
    }
}

impl CaptureStream {
    /// Start capturing `target`, sending each frame to `tx`. The session runs until [`stop`].
    pub fn start(
        target: &CaptureTarget,
        include_cursor: bool,
        tx: Sender<StreamEvent>,
    ) -> Result<Self> {
        let (item, crop) = resolve(target)?;
        let d3d = Arc::new(Mutex::new(D3dDevice::create()?));
        let wdev = {
            let guard = d3d.lock().map_err(|_| poisoned())?;
            guard.winrt_device()?
        };
        let size = item.Size().map_err(map_com)?;
        if size.Width <= 0 || size.Height <= 0 {
            return Err(CaptureError::EmptyRegion);
        }

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &wdev,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .map_err(map_com)?;
        let session = pool.CreateCaptureSession(&item).map_err(map_com)?;
        let _ = session.SetIsCursorCaptureEnabled(include_cursor);
        let _ = session.SetIsBorderRequired(false);

        let start = Instant::now();

        // FrameArrived: copy the surface to CPU, crop, and send. Runs on pool threads.
        let d3d_cb = d3d.clone();
        let tx_cb = tx.clone();
        let pool_cb = pool.clone();
        let handler =
            TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                if let Ok(frame) = pool_cb.TryGetNextFrame() {
                    if let Ok(guard) = d3d_cb.lock() {
                        let ts = start.elapsed();
                        if let Ok(cpu) = crate::win::wgc::frame_to_cpu(&guard, &frame, ts) {
                            let out = match crop {
                                Some(r) => cpu.crop(r),
                                None => cpu,
                            };
                            let _ = tx_cb.send(StreamEvent::Frame(out));
                        }
                    }
                    let _ = frame.Close();
                }
                Ok(())
            });
        let frame_token = pool.FrameArrived(&handler).map_err(map_com)?;

        // Closed: the monitor/window went away.
        let tx_closed = tx.clone();
        let closed_handler =
            TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                let _ = tx_closed.send(StreamEvent::Closed);
                Ok(())
            });
        let closed_token = item.Closed(&closed_handler).map_err(map_com)?;

        session.StartCapture().map_err(map_com)?;

        Ok(Self {
            session,
            pool,
            item,
            frame_token,
            closed_token,
            _d3d: d3d,
            start,
        })
    }

    /// How long the stream has been running.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Stop the session and release all callbacks and COM resources.
    pub fn stop(self) {
        let _ = self.pool.RemoveFrameArrived(self.frame_token);
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

fn poisoned() -> CaptureError {
    CaptureError::Platform("capture device mutex poisoned".into())
}
