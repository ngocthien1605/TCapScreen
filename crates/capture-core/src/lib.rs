//! `capture-core`: platform-agnostic capture types, geometry, and the capture backend trait.
//!
//! Only a Windows (Windows.Graphics.Capture) backend is implemented and shipped. All Windows
//! API calls live behind [`CaptureBackend`] so other platforms *could* be added later. The
//! Windows backend is introduced in milestone M1; this module defines the shared contract and
//! the pure geometry used by both capture and the region overlay.

pub mod geometry;

pub use geometry::{
    bounding_box, dpi_to_scale, logical_to_physical, physical_to_logical, Point, Rect,
};

use std::time::Duration;

use thiserror::Error;

/// Opaque identifier for a monitor (the OS `HMONITOR`, stored portably as `isize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MonitorId(pub isize);

/// Opaque identifier for a window (the OS `HWND`, stored portably as `isize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub isize);

/// Pixel format of a captured [`Frame`]. WGC delivers BGRA8 premultiplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFmt {
    /// 8 bits per channel, byte order B, G, R, A.
    Bgra8,
}

/// Information about a connected display.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorInfo {
    pub id: MonitorId,
    /// Human-readable device name (e.g. `\\.\DISPLAY1`).
    pub name: String,
    /// Bounds in physical pixels, in virtual-desktop coordinates.
    pub bounds: Rect,
    /// DPI scale factor (1.0 = 96 DPI = 100%).
    pub scale_factor: f64,
    pub is_primary: bool,
}

/// Information about a top-level window.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    pub id: WindowId,
    pub title: String,
    /// Window bounds in physical pixels, in virtual-desktop coordinates.
    pub bounds: Rect,
}

/// What to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Monitor(MonitorId),
    Window(WindowId),
    /// A sub-rectangle (physical px, virtual-desktop coords) captured from the monitor it
    /// primarily sits on.
    Region {
        rect: Rect,
        monitor: MonitorId,
    },
    /// The whole virtual desktop (all monitors).
    VirtualDesktop,
}

/// Per-grab options for still captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GrabOpts {
    pub include_cursor: bool,
}

/// Configuration for a continuous capture stream (recording).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamCfg {
    pub fps: u32,
    pub include_cursor: bool,
}

impl Default for StreamCfg {
    fn default() -> Self {
        Self {
            fps: 30,
            include_cursor: true,
        }
    }
}

/// A single captured frame in CPU memory.
///
/// `stride` is the number of bytes per row and may be **greater** than `width * 4` because GPU
/// textures are row-aligned. Consumers must honour `stride` and never assume tight packing.
#[derive(Debug, Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFmt,
    /// Capture timestamp relative to stream start (zero for one-shot grabs).
    pub timestamp: Duration,
}

impl Frame {
    /// Borrow row `y` as a byte slice of exactly `width * 4` bytes (ignoring stride padding).
    /// Returns `None` if `y` is out of range or the buffer is too small.
    pub fn row(&self, y: u32) -> Option<&[u8]> {
        if y >= self.height {
            return None;
        }
        let start = y as usize * self.stride;
        let end = start + self.width as usize * 4;
        self.data.get(start..end)
    }

    /// Expected tightly-packed size in bytes (`width * height * 4`).
    pub fn tight_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// Errors from capture operations.
#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("no monitor matched the requested target")]
    MonitorNotFound,
    #[error("no window matched the requested target")]
    WindowNotFound,
    #[error("the capture region is empty or lies entirely off-screen")]
    EmptyRegion,
    #[error("Windows.Graphics.Capture is not available on this system")]
    CaptureUnavailable,
    #[error("the captured surface was protected/DRM-black")]
    ProtectedContent,
    #[error("platform error: {0}")]
    Platform(String),
}

/// Result alias for capture operations.
pub type Result<T> = std::result::Result<T, CaptureError>;

/// A capture backend. The Windows implementation is added in M1; this trait is the only
/// surface the rest of the app depends on.
pub trait CaptureBackend: Send {
    /// Enumerate connected monitors.
    fn monitors(&self) -> Result<Vec<MonitorInfo>>;

    /// Enumerate capturable top-level windows.
    fn windows(&self) -> Result<Vec<WindowInfo>>;

    /// Capture a single frame for `target`.
    fn grab(&self, target: &CaptureTarget, opts: GrabOpts) -> Result<Frame>;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn frame_row_respects_stride_padding() {
        // 2x2 image, stride 12 (width*4 = 8, so 4 bytes of row padding).
        let width = 2u32;
        let height = 2u32;
        let stride = 12usize;
        let mut data = vec![0u8; stride * height as usize];
        // Row 0 payload = 1..=8, row 1 payload = 9..=16; padding left as 0.
        for i in 0..8 {
            data[i] = (i + 1) as u8;
            data[stride + i] = (i + 9) as u8;
        }
        let f = Frame {
            data,
            width,
            height,
            stride,
            format: PixelFmt::Bgra8,
            timestamp: Duration::ZERO,
        };
        assert_eq!(f.row(0).unwrap(), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(f.row(1).unwrap(), &[9, 10, 11, 12, 13, 14, 15, 16]);
        assert!(f.row(2).is_none());
        assert_eq!(f.tight_len(), 16);
    }
}
