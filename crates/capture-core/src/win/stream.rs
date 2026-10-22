//! Continuous Windows.Graphics.Capture session used by the recorder. Unlike the one-shot
//! [`super::wgc::grab_item`], this keeps a session alive and delivers every frame to a channel
//! until stopped (or until the captured item is closed). Frame copies happen on the capture
//! thread-pool threads, serialized through a mutex because the D3D immediate context is not
//! thread-safe.
//!
//! Failure model. WGC delivers a frame only when the captured content *changes*, so "no frame
//! for N seconds" is normal on an idle desktop and is **not** a health signal. What does mean
//! trouble: the GPU→CPU copy failing repeatedly (a removed/reset D3D device never recovers), or
//! the item being closed. Both are reported as events so the consumer can stop and finalize
//! instead of re-emitting a stale frame forever; the consumer should also poll
//! [`CaptureStream::check_health`] periodically, which catches a dead device (or a destroyed
//! window, for which WGC was observed not to raise `Closed`) even when no callback fires.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::IInspectable;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::IsWindow;

use crate::win::d3d::{map_com, D3dDevice};
use crate::win::{monitors, windows_list};
use crate::{CaptureError, CaptureTarget, Frame, Point, Rect, Result};

/// Consecutive GPU→CPU copy failures before the stream gives up even though the device still
/// claims to be healthy (roughly a second at 60 Hz).
const MAX_CONSECUTIVE_FAILURES: u32 = 60;
/// Log every Nth failure after the first, so a failing callback at refresh rate does not flood.
const FAILURE_LOG_EVERY: u32 = 60;

/// An event delivered by a running [`CaptureStream`].
#[derive(Debug)]
pub enum StreamEvent {
    /// A newly captured frame (already cropped to the target region, if any).
    Frame(Frame),
    /// The captured item was closed (monitor disconnected / window destroyed).
    Closed,
    /// The stream cannot continue (the D3D device was removed/reset, or frame copies keep
    /// failing). Sent at most once; no more frames will follow.
    Error(CaptureError),
}

/// A live capture session feeding frames to a channel.
#[allow(missing_debug_implementations)]
pub struct CaptureStream {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
    item: GraphicsCaptureItem,
    frame_token: i64,
    closed_token: i64,
    d3d: Arc<Mutex<D3dDevice>>,
    start: Instant,
    /// Current frame-pool surface size as `(w << 32) | h`; updated after a `Recreate`.
    pool_size: Arc<AtomicU64>,
    /// The captured window (as an integer handle) when the target is a window, for the health
    /// poll: WGC does not reliably raise `Closed` for a destroyed window.
    window: Option<isize>,
}

/// Tracks consecutive copy failures so the stream can log sparsely and give up once.
/// Pure (no COM) so the policy is unit-testable.
#[derive(Debug, Default)]
struct FailureTracker {
    consecutive: AtomicU32,
    errored: AtomicBool,
}

/// What [`FailureTracker::record_failure`] asks the caller to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureAction {
    /// Log this failure (first of a run, or every Nth).
    Log,
    /// Stay quiet.
    Quiet,
    /// Too many in a row: send the terminal error (returned once).
    GiveUp,
}

impl FailureTracker {
    fn record_success(&self) {
        self.consecutive.store(0, Ordering::Relaxed);
    }

    /// Returns what to do for this failure; `device_dead` short-circuits to `GiveUp`.
    fn record_failure(&self, device_dead: bool) -> FailureAction {
        let n = self.consecutive.fetch_add(1, Ordering::Relaxed) + 1;
        if device_dead || n >= MAX_CONSECUTIVE_FAILURES {
            return if self.errored.swap(true, Ordering::SeqCst) {
                FailureAction::Quiet
            } else {
                FailureAction::GiveUp
            };
        }
        if n == 1 || n.is_multiple_of(FAILURE_LOG_EVERY) {
            FailureAction::Log
        } else {
            FailureAction::Quiet
        }
    }
}

fn pack_size(s: SizeInt32) -> u64 {
    ((s.Width as u32 as u64) << 32) | (s.Height as u32 as u64)
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
fn resolve(target: &CaptureTarget) -> Result<(GraphicsCaptureItem, Option<Rect>, Option<HWND>)> {
    match *target {
        CaptureTarget::Monitor(id) => {
            Ok((item_for_monitor(monitors::hmonitor_of(id))?, None, None))
        }
        CaptureTarget::Window(id) => {
            let hwnd = windows_list::hwnd_of(id);
            Ok((item_for_window(hwnd)?, None, Some(hwnd)))
        }
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
                None,
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
        let (item, crop, window) = resolve(target)?;
        let window = window.map(|h| h.0 as isize);
        let d3d = Arc::new(Mutex::new(D3dDevice::create()?));
        let wdev = {
            let guard = d3d.lock().map_err(|_| poisoned())?;
            guard.winrt_device()?
        };
        let size = item.Size().map_err(map_com)?;
        if size.Width <= 0 || size.Height <= 0 {
            return Err(CaptureError::EmptyRegion);
        }

        const FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;
        const BUFFERS: i32 = 2;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&wdev, FORMAT, BUFFERS, size)
            .map_err(map_com)?;
        let session = pool.CreateCaptureSession(&item).map_err(map_com)?;
        let _ = session.SetIsCursorCaptureEnabled(include_cursor);
        let _ = session.SetIsBorderRequired(false);

        let start = Instant::now();
        let pool_size = Arc::new(AtomicU64::new(pack_size(size)));
        let failures = Arc::new(FailureTracker::default());

        // FrameArrived: copy the surface to CPU, crop, and send. Runs on pool threads.
        let d3d_cb = d3d.clone();
        let tx_cb = tx.clone();
        let pool_cb = pool.clone();
        let size_cb = pool_size.clone();
        let failures_cb = failures.clone();
        let handler =
            TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                // This runs on a WGC pool thread inside a COM callback: a panic here would
                // unwind across the FFI boundary and abort the whole process, so contain it.
                let body = || {
                    // No frame ready is not a failure (the pool signals ahead of readiness).
                    let Ok(frame) = pool_cb.TryGetNextFrame() else {
                        return;
                    };
                    let Ok(guard) = d3d_cb.lock() else {
                        let _ = frame.Close();
                        return;
                    };
                    let ts = start.elapsed();
                    match crate::win::wgc::frame_to_cpu(&guard, &frame, ts) {
                        Ok(cpu) => {
                            failures_cb.record_success();
                            let out = match crop {
                                Some(r) => cpu.crop(r),
                                None => cpu,
                            };
                            let _ = tx_cb.send(StreamEvent::Frame(out));
                        }
                        Err(e) => {
                            let dead = guard.removed_reason();
                            match failures_cb.record_failure(dead.is_some()) {
                                FailureAction::Log => {
                                    tracing::warn!("capture frame copy failed: {e}")
                                }
                                FailureAction::Quiet => {}
                                FailureAction::GiveUp => {
                                    let err = match dead {
                                        Some(r) => CaptureError::DeviceLost(r),
                                        None => CaptureError::Platform(format!(
                                            "capture frame copy keeps failing \
                                             ({MAX_CONSECUTIVE_FAILURES}x): {e}"
                                        )),
                                    };
                                    tracing::error!("{err}; stopping the stream");
                                    let _ = tx_cb.send(StreamEvent::Error(err));
                                }
                            }
                        }
                    }
                    // Resize: the pool's surfaces keep their original size, so a window that
                    // grew would be clipped and one that shrank would carry stale margins. The
                    // documented pattern is to recreate the pool after handling the frame.
                    let content = frame.ContentSize().unwrap_or(SizeInt32::default());
                    let _ = frame.Close();
                    if content.Width > 0
                        && content.Height > 0
                        && pack_size(content) != size_cb.load(Ordering::Relaxed)
                    {
                        // The WinRT device projection is not `Send`, so it is re-derived here
                        // from the (shared) D3D device rather than captured by the closure.
                        match guard.winrt_device().and_then(|dev| {
                            pool_cb
                                .Recreate(&dev, FORMAT, BUFFERS, content)
                                .map_err(map_com)
                        }) {
                            Ok(()) => {
                                size_cb.store(pack_size(content), Ordering::Relaxed);
                                tracing::info!(
                                    "capture source resized; frame pool recreated at {}x{}",
                                    content.Width,
                                    content.Height
                                );
                            }
                            Err(e) => tracing::warn!("frame pool recreate failed: {e}"),
                        }
                    }
                };
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)).is_err() {
                    tracing::error!("panic in the capture frame callback; frame dropped");
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
            d3d,
            start,
            pool_size,
            window,
        })
    }

    /// How long the stream has been running.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// The current frame-pool surface size (follows the source after a resize).
    pub fn surface_size(&self) -> (u32, u32) {
        let packed = self.pool_size.load(Ordering::Relaxed);
        ((packed >> 32) as u32, packed as u32)
    }

    /// Health check for the consumer to poll (about once a second): `Err` once the stream can
    /// no longer deliver anything useful — the graphics device behind it was removed or reset,
    /// or the captured window was destroyed. Needed because a dead device may simply stop
    /// delivering callbacks, a quiet stream is otherwise indistinguishable from an idle desktop,
    /// and WGC was observed *not* to raise `Closed` for a window killed mid-capture.
    pub fn check_health(&self) -> Result<()> {
        if let Some(h) = self.window {
            // SAFETY: IsWindow accepts any handle value and only reports whether it is live.
            let alive = unsafe { IsWindow(Some(HWND(h as *mut std::ffi::c_void))) }.as_bool();
            if !alive {
                return Err(CaptureError::SourceClosed);
            }
        }
        let guard = self.d3d.lock().map_err(|_| poisoned())?;
        match guard.removed_reason() {
            None => Ok(()),
            Some(r) => Err(CaptureError::DeviceLost(r)),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_tracker_logs_first_then_sparsely() {
        let t = FailureTracker::default();
        assert_eq!(t.record_failure(false), FailureAction::Log);
        for _ in 1..FAILURE_LOG_EVERY - 1 {
            assert_eq!(t.record_failure(false), FailureAction::Quiet);
        }
        // Reaching the threshold gives up exactly once, then stays quiet.
        assert_eq!(t.record_failure(false), FailureAction::GiveUp);
        assert_eq!(t.record_failure(false), FailureAction::Quiet);
    }

    #[test]
    fn failure_tracker_resets_on_success() {
        let t = FailureTracker::default();
        for _ in 0..10 {
            t.record_failure(false);
        }
        t.record_success();
        assert_eq!(
            t.record_failure(false),
            FailureAction::Log,
            "a new run logs again"
        );
    }

    #[test]
    fn dead_device_gives_up_immediately_and_once() {
        let t = FailureTracker::default();
        assert_eq!(t.record_failure(true), FailureAction::GiveUp);
        assert_eq!(t.record_failure(true), FailureAction::Quiet);
    }

    #[test]
    fn size_packs_both_dimensions() {
        let s = SizeInt32 {
            Width: 1920,
            Height: 1080,
        };
        let packed = pack_size(s);
        assert_eq!(((packed >> 32) as u32, packed as u32), (1920, 1080));
        assert_ne!(
            packed,
            pack_size(SizeInt32 {
                Width: 1080,
                Height: 1920
            })
        );
    }
}
