//! Windows.Graphics.Capture one-shot grab shared by monitor/window/region captures.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::core::{IInspectable, Interface};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;

use crate::win::d3d::{map_com, D3dDevice};
use crate::{CaptureError, Frame, Result};

const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

/// Capture a single frame from a WGC capture item.
pub fn grab_item(
    d3d: &D3dDevice,
    winrt_device: &IDirect3DDevice,
    item: &GraphicsCaptureItem,
    include_cursor: bool,
) -> Result<Frame> {
    let size = item.Size().map_err(map_com)?;
    if size.Width <= 0 || size.Height <= 0 {
        return Err(CaptureError::EmptyRegion);
    }

    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        winrt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        size,
    )
    .map_err(map_com)?;
    let session = pool.CreateCaptureSession(item).map_err(map_com)?;

    // Best-effort UX/behaviour toggles (older Windows may lack these properties).
    let _ = session.SetIsCursorCaptureEnabled(include_cursor);
    let _ = session.SetIsBorderRequired(false);

    let (tx, rx) = mpsc::channel::<()>();
    let handler =
        TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
            let _ = tx.send(());
            Ok(())
        });
    let token = pool.FrameArrived(&handler).map_err(map_com)?;
    session.StartCapture().map_err(map_com)?;

    let result = (|| {
        let frame = wait_for_frame(&pool, &rx)?;
        let out = frame_to_cpu(d3d, &frame, Duration::ZERO)?;
        let _ = frame.Close();
        Ok(out)
    })();

    // Teardown regardless of success.
    let _ = pool.RemoveFrameArrived(token);
    let _ = session.Close();
    let _ = pool.Close();

    result
}

/// Copy a WGC frame's GPU surface into a CPU [`Frame`]. Shared by the one-shot grab and the
/// continuous recording stream.
pub(crate) fn frame_to_cpu(
    d3d: &D3dDevice,
    frame: &Direct3D11CaptureFrame,
    timestamp: Duration,
) -> Result<Frame> {
    let content = frame.ContentSize().map_err(map_com)?;
    let surface = frame.Surface().map_err(map_com)?;
    let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(map_com)?;
    // SAFETY: the surface's DXGI interface is an ID3D11Texture2D under WGC.
    let texture: ID3D11Texture2D = unsafe { access.GetInterface() }.map_err(map_com)?;
    d3d.texture_to_frame(
        &texture,
        content.Width as u32,
        content.Height as u32,
        timestamp,
    )
}

fn wait_for_frame(
    pool: &Direct3D11CaptureFramePool,
    rx: &mpsc::Receiver<()>,
) -> Result<Direct3D11CaptureFrame> {
    let deadline = Instant::now() + FRAME_TIMEOUT;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(CaptureError::Platform(
                "timed out waiting for a capture frame".into(),
            ));
        }
        match rx.recv_timeout(deadline - now) {
            Ok(()) => {
                // A frame was signalled; try to take it. If not actually ready, wait again.
                if let Ok(frame) = pool.TryGetNextFrame() {
                    return Ok(frame);
                }
            }
            Err(_) => {
                return Err(CaptureError::Platform(
                    "timed out waiting for a capture frame".into(),
                ));
            }
        }
    }
}
