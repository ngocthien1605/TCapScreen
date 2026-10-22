//! Direct3D11 device creation and GPU-texture → CPU `Frame` copy.

use std::time::Duration;

use windows::core::Interface;
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;

use crate::{CaptureError, Frame, PixelFmt, Result};

/// Map a `windows::core::Error` into a `CaptureError::Platform`.
pub fn map_com(e: windows::core::Error) -> CaptureError {
    CaptureError::Platform(format!("{e}"))
}

/// A D3D11 device + immediate context used for capture.
pub struct D3dDevice {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
}

impl std::fmt::Debug for D3dDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3dDevice").finish_non_exhaustive()
    }
}

impl D3dDevice {
    /// Create a hardware device (BGRA support), falling back to the WARP software renderer.
    pub fn create() -> Result<Self> {
        let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
        let mut last: Option<windows::core::Error> = None;
        for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            let mut level = D3D_FEATURE_LEVEL::default();
            // SAFETY: all out-pointers are valid for the call; no borrowed inputs escape.
            let res = unsafe {
                D3D11CreateDevice(
                    None,
                    driver,
                    HMODULE::default(),
                    flags,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    Some(&mut level),
                    Some(&mut context),
                )
            };
            match res {
                Ok(()) => {
                    if let (Some(device), Some(context)) = (device, context) {
                        return Ok(Self { device, context });
                    }
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last
            .map(map_com)
            .unwrap_or(CaptureError::CaptureUnavailable))
    }

    /// Why the device was removed (driver crash/upgrade, TDR reset, adapter unplugged), or
    /// `None` while it is healthy. A removed device never recovers: every copy through it fails
    /// from then on, so a long-running capture must poll this and give up.
    pub fn removed_reason(&self) -> Option<String> {
        // SAFETY: a query on our own live device; returns S_OK while the device is usable.
        unsafe { self.device.GetDeviceRemovedReason() }
            .err()
            .map(|e| e.to_string())
    }

    /// Build the WinRT `IDirect3DDevice` projection needed by the WGC frame pool.
    pub fn winrt_device(&self) -> Result<IDirect3DDevice> {
        let dxgi: IDXGIDevice = self.device.cast().map_err(map_com)?;
        // SAFETY: `dxgi` is a valid IDXGIDevice obtained from our own device.
        let inspectable =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }.map_err(map_com)?;
        inspectable.cast::<IDirect3DDevice>().map_err(map_com)
    }

    /// Copy a GPU texture into a CPU-readable staging texture and read it into a [`Frame`].
    /// `content_w`/`content_h` come from the WGC frame's `ContentSize`; `stride` is taken from
    /// the staging texture's `RowPitch` (may exceed `width * 4`).
    pub fn texture_to_frame(
        &self,
        texture: &ID3D11Texture2D,
        content_w: u32,
        content_h: u32,
        timestamp: Duration,
    ) -> Result<Frame> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: `texture` is a valid ID3D11Texture2D; GetDesc only writes `desc`.
        unsafe { texture.GetDesc(&mut desc) };
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;

        let mut staging: Option<ID3D11Texture2D> = None;
        // SAFETY: valid desc; no initial data; writes the new texture into `staging`.
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut staging)) }.map_err(map_com)?;
        let staging = staging
            .ok_or_else(|| CaptureError::Platform("CreateTexture2D returned null".into()))?;

        // SAFETY: both resources are valid and compatible (same desc minus usage/cpu flags).
        unsafe { self.context.CopyResource(&staging, texture) };

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: staging is CPU-readable; subresource 0 exists; `mapped` receives the mapping.
        unsafe {
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .map_err(map_com)?;

        let stride = mapped.RowPitch as usize;
        let surface_h = desc.Height;
        let width = content_w.min(desc.Width);
        let height = content_h.min(surface_h);
        let byte_len = stride.saturating_mul(surface_h as usize);

        let mut data = vec![0u8; byte_len];
        if !mapped.pData.is_null() && byte_len > 0 {
            // SAFETY: the mapped region is at least `RowPitch * Height` bytes; we copy exactly
            // that many into an equally-sized Vec.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    mapped.pData as *const u8,
                    data.as_mut_ptr(),
                    byte_len,
                );
            }
        }
        // SAFETY: balances the Map above on the same resource/subresource.
        unsafe { self.context.Unmap(&staging, 0) };

        Ok(Frame {
            data,
            width,
            height,
            stride,
            format: PixelFmt::Bgra8,
            timestamp,
        })
    }
}
