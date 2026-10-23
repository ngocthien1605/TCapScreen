//! Owned GDI handles. Each guard releases what it holds on drop, so a failure part-way through
//! building a set of GDI objects never leaks the ones already created.

use std::ffi::c_void;
use std::ptr::null_mut;

use anyhow::{Context, Result};
use capture_core::Frame;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HDC, HGDIOBJ,
};

/// The screen DC, released on drop.
pub(crate) struct ScreenDc(pub(crate) HDC);

impl ScreenDc {
    pub(crate) fn get() -> Result<ScreenDc> {
        // SAFETY: GetDC(NULL) returns the screen DC or null.
        let dc = unsafe { windows::Win32::Graphics::Gdi::GetDC(None) };
        anyhow::ensure!(!dc.is_invalid(), "GetDC failed");
        Ok(ScreenDc(dc))
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: obtained from GetDC(NULL) above.
        unsafe { ReleaseDC(None, self.0) };
    }
}

/// A GDI object (pen, brush, font, bitmap), deleted on drop.
pub(crate) struct GdiObj(pub(crate) HGDIOBJ);

impl GdiObj {
    pub(crate) fn new(raw: *mut c_void) -> Result<GdiObj> {
        anyhow::ensure!(!raw.is_null(), "GDI object creation failed");
        Ok(GdiObj(HGDIOBJ(raw)))
    }
}

impl Drop for GdiObj {
    fn drop(&mut self) {
        // SAFETY: we created this object and it is no longer selected into a DC (the owning
        // `Surface` restores its DC's original object before the bitmap is deleted).
        unsafe {
            let _ = DeleteObject(self.0);
        }
    }
}

/// A memory DC with a bitmap selected into it. Dropping restores the DC's original object, then
/// deletes the DC and the bitmap. A DIB-backed surface also exposes its pixels (top-down BGRA,
/// tightly packed).
pub(crate) struct Surface {
    pub(crate) dc: HDC,
    old: HGDIOBJ,
    _bitmap: GdiObj,
    /// The DIB's pixel memory (owned by the bitmap) and its length; null for a compatible bitmap.
    bits: *mut u8,
    len: usize,
}

impl Surface {
    /// Select `bitmap` into a fresh memory DC compatible with `ref_dc`.
    fn select(ref_dc: HDC, bitmap: GdiObj, bits: *mut u8, len: usize) -> Result<Surface> {
        // SAFETY: a memory DC compatible with the reference DC.
        let dc = unsafe { CreateCompatibleDC(Some(ref_dc)) };
        anyhow::ensure!(!dc.is_invalid(), "CreateCompatibleDC failed");
        // SAFETY: select the bitmap into its own DC.
        let old = unsafe { SelectObject(dc, bitmap.0) };
        Ok(Surface {
            dc,
            old,
            _bitmap: bitmap,
            bits,
            len,
        })
    }

    /// A device-compatible bitmap of `w`×`h` (e.g. a back buffer).
    pub(crate) fn compatible(ref_dc: HDC, w: i32, h: i32) -> Result<Surface> {
        // SAFETY: plain bitmap creation.
        let bmp = unsafe { CreateCompatibleBitmap(ref_dc, w, h) };
        Surface::select(ref_dc, GdiObj::new(bmp.0)?, null_mut(), 0)
    }

    /// A black, top-down 32-bpp DIB section of `w`×`h`; fill it through [`pixels_mut`].
    ///
    /// [`pixels_mut`]: Surface::pixels_mut
    pub(crate) fn dib(ref_dc: HDC, w: i32, h: i32) -> Result<Surface> {
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h; // negative = top-down, matching our frame layout
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = 0; // BI_RGB

        let mut bits: *mut c_void = null_mut();
        // SAFETY: valid BITMAPINFO; `bits` receives the pixel pointer.
        let bmp = unsafe {
            CreateDIBSection(Some(ref_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                .context("CreateDIBSection")?
        };
        let bitmap = GdiObj::new(bmp.0)?;
        anyhow::ensure!(!bits.is_null(), "CreateDIBSection returned no pixels");
        let len = w.max(0) as usize * 4 * h.max(0) as usize;
        let mut surface = Surface::select(ref_dc, bitmap, bits as *mut u8, len)?;
        if let Some(px) = surface.pixels_mut() {
            px.fill(0);
        }
        Ok(surface)
    }

    /// The DIB's pixels (top-down BGRA); `None` for a compatible bitmap. Call `GdiFlush` after
    /// drawing into the DC before reading them.
    pub(crate) fn pixels(&self) -> Option<&[u8]> {
        // SAFETY: a w×h 32-bpp top-down DIB section owns exactly `len` bytes at `bits`, alive
        // as long as the bitmap (owned by `self`).
        (!self.bits.is_null()).then(|| unsafe { std::slice::from_raw_parts(self.bits, self.len) })
    }

    /// Mutable access to the DIB's pixels; see [`pixels`](Surface::pixels).
    pub(crate) fn pixels_mut(&mut self) -> Option<&mut [u8]> {
        // SAFETY: as in `pixels`; `&mut self` makes the borrow exclusive.
        (!self.bits.is_null())
            .then(|| unsafe { std::slice::from_raw_parts_mut(self.bits, self.len) })
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: restore the original object, then delete the DC; the bitmap (no longer
        // selected) is deleted by its `GdiObj` right after.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Copy `f` into a tightly packed BGRA buffer `row` bytes wide, clipping to whichever is
/// smaller. Uncovered parts of `dst` are left as they are.
pub(crate) fn copy_frame(f: &Frame, dst: &mut [u8], row: usize) {
    if row == 0 {
        return;
    }
    for (y, out) in dst.chunks_exact_mut(row).enumerate() {
        let Some(src) = f.row(y as u32) else {
            break;
        };
        let n = src.len().min(out.len());
        out[..n].copy_from_slice(&src[..n]);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, stride: usize, fill: u8) -> Frame {
        Frame {
            data: vec![fill; stride * h as usize],
            width: w,
            height: h,
            stride,
            format: capture_core::PixelFmt::Bgra8,
            timestamp: std::time::Duration::ZERO,
        }
    }

    #[test]
    fn copy_frame_clips_a_frame_smaller_than_the_overlay() {
        // Overlay 4×3, frozen frame only 2×2 (the display shrank): no overrun, rest stays 0.
        let mut dst = vec![0u8; 4 * 4 * 3];
        copy_frame(&frame(2, 2, 2 * 4 + 8, 7), &mut dst, 4 * 4);
        for (y, row) in dst.as_chunks::<16>().0.iter().enumerate() {
            let expect_lit = if y < 2 { 8 } else { 0 };
            assert!(row[..expect_lit].iter().all(|&b| b == 7), "row {y}");
            assert!(row[expect_lit..].iter().all(|&b| b == 0), "row {y}");
        }
    }

    #[test]
    fn copy_frame_crops_a_frame_larger_than_the_overlay() {
        let mut dst = vec![0u8; 2 * 4 * 2];
        copy_frame(&frame(5, 5, 5 * 4, 9), &mut dst, 2 * 4);
        assert!(dst.iter().all(|&b| b == 9));
    }

    #[test]
    fn a_dib_surface_exposes_zeroed_pixels() {
        let screen = ScreenDc::get().unwrap();
        let mut s = Surface::dib(screen.0, 3, 2).unwrap();
        assert_eq!(s.pixels().unwrap().len(), 3 * 2 * 4);
        assert!(s.pixels().unwrap().iter().all(|&b| b == 0));
        s.pixels_mut().unwrap()[0] = 9;
        assert_eq!(s.pixels().unwrap()[0], 9);
        let c = Surface::compatible(screen.0, 3, 2).unwrap();
        assert!(c.pixels().is_none());
    }
}
