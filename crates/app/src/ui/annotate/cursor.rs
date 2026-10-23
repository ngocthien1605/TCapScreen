//! The red crosshair mouse cursor shown over the annotation canvas. It is generated in code (no
//! image asset): thin red arms with a small gap at the centre and a soft white halo so it stays
//! visible on both dark and light screenshots. [`crosshair_pixels`] is pure and unit-tested;
//! [`create`] turns it into a Win32 cursor.

use std::ffi::c_void;
use std::ptr::null_mut;

use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HCURSOR, ICONINFO};

/// Cursor bitmap edge length in pixels.
pub const SIZE: i32 = 32;
/// The hot spot (the exact click point) — the crosshair's centre.
const HOT: i32 = 16;
/// Half-width of the empty gap around the centre, so the target pixel stays visible.
const GAP: i32 = 3;

/// Opaque pure red, as premultiplied BGRA (`0xAARRGGBB` in a little-endian `u32`).
const RED: u32 = 0xFFFF_0000;
/// White at ~78 % alpha, premultiplied.
const HALO: u32 = 0xC8C8_C8C8;

/// The cursor's pixels, top-down, `SIZE × SIZE`, premultiplied BGRA. Transparent (0) elsewhere.
pub fn crosshair_pixels() -> Vec<u32> {
    let mut px = vec![0u32; (SIZE * SIZE) as usize];
    let at = |x: i32, y: i32| (y * SIZE + x) as usize;
    for i in 0..SIZE {
        if (i - HOT).abs() < GAP {
            continue;
        }
        // Halo on both sides of each arm first, then the red line on top.
        for d in [-1, 1] {
            px[at(i, HOT + d)] = HALO;
            px[at(HOT + d, i)] = HALO;
        }
    }
    for i in 0..SIZE {
        if (i - HOT).abs() < GAP {
            continue;
        }
        px[at(i, HOT)] = RED;
        px[at(HOT, i)] = RED;
    }
    px
}

/// Build the cursor, or `None` if any GDI call fails (the caller falls back to the system cross).
pub fn create() -> Option<HCURSOR> {
    let pixels = crosshair_pixels();
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = SIZE;
    bmi.bmiHeader.biHeight = -SIZE; // top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = 0; // BI_RGB

    // SAFETY: a 32-bit DIB section whose pixel buffer we fill, plus a 1-bit mask, handed to
    // CreateIconIndirect (which copies both); our bitmaps are deleted before returning.
    unsafe {
        let screen = GetDC(None);
        let mut bits: *mut c_void = null_mut();
        let color = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, screen);
        let color = color.ok()?;
        if bits.is_null() {
            let _ = DeleteObject(HGDIOBJ(color.0));
            return None;
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());
        let mask = CreateBitmap(SIZE, SIZE, 1, 1, None);

        let info = ICONINFO {
            fIcon: false.into(), // a cursor, not an icon: the hot spot below applies
            xHotspot: HOT as u32,
            yHotspot: HOT as u32,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon.map(|i| HCURSOR(i.0))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn at(px: &[u32], x: i32, y: i32) -> u32 {
        px[(y * SIZE + x) as usize]
    }

    #[test]
    fn arms_are_pure_opaque_red() {
        let px = crosshair_pixels();
        for (x, y) in [(HOT, 2), (HOT, SIZE - 3), (2, HOT), (SIZE - 3, HOT)] {
            assert_eq!(at(&px, x, y), RED, "arm pixel at ({x},{y})");
        }
        // Opaque red: alpha FF, red FF, green 00, blue 00.
        assert_eq!(RED >> 24, 0xFF);
        assert_eq!((RED >> 16) & 0xFF, 0xFF);
        assert_eq!(RED & 0xFFFF, 0);
    }

    #[test]
    fn centre_has_a_gap_and_corners_are_transparent() {
        let px = crosshair_pixels();
        assert_eq!(at(&px, HOT, HOT), 0, "target pixel stays visible");
        assert_eq!(at(&px, HOT + GAP - 1, HOT), 0, "inside the gap");
        assert_ne!(at(&px, HOT + GAP, HOT), 0, "arm starts right after the gap");
        for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
            assert_eq!(at(&px, x, y), 0, "corner ({x},{y})");
        }
    }

    #[test]
    fn arms_have_a_white_halo_either_side() {
        let px = crosshair_pixels();
        assert_eq!(at(&px, 5, HOT - 1), HALO);
        assert_eq!(at(&px, 5, HOT + 1), HALO);
        assert_eq!(at(&px, HOT - 1, 5), HALO);
        assert_eq!(at(&px, HOT + 1, 5), HALO);
    }

    #[test]
    fn every_non_transparent_pixel_is_red_or_halo() {
        for p in crosshair_pixels() {
            assert!(
                p == 0 || p == RED || p == HALO,
                "unexpected colour {p:#010x}"
            );
        }
    }

    #[test]
    fn a_cursor_can_be_created() {
        assert!(create().is_some(), "CreateIconIndirect failed");
    }
}
