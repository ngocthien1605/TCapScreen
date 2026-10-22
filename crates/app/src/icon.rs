//! A small, original RustShot tray icon drawn programmatically (no third-party assets).
//! A teal rounded square with a white camera "lens" ring — a generic capture motif, not a
//! copy of any existing product's artwork. Real branding is finalised in M6.

/// Returns `(rgba, width, height)` for the tray/app icon.
pub fn app_icon_rgba() -> (Vec<u8>, u32, u32) {
    let size: i32 = 32;
    let mut px = vec![0u8; (size * size * 4) as usize];

    let teal = [0x1f, 0x9e, 0x9e, 0xffu8];
    let teal_dark = [0x15, 0x74, 0x74, 0xffu8];
    let white = [0xff, 0xff, 0xff, 0xffu8];

    let center = (size as f32 - 1.0) / 2.0;
    let corner_r = 7.0f32;
    let lens_outer = 10.0f32;
    let lens_inner = 6.0f32;

    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let (fx, fy) = (x as f32, y as f32);

            // Rounded-rect background mask.
            let inside = rounded_rect_contains(fx, fy, size as f32, corner_r);
            if !inside {
                continue; // leave fully transparent
            }

            let d = ((fx - center).powi(2) + (fy - center).powi(2)).sqrt();
            let color = if d <= lens_inner {
                teal_dark // lens pupil
            } else if d <= lens_outer {
                white // lens ring
            } else {
                teal // body
            };
            px[idx..idx + 4].copy_from_slice(&color);
        }
    }
    (px, size as u32, size as u32)
}

/// Whether `(x, y)` is inside a `size`×`size` rounded rectangle with corner radius `r`.
fn rounded_rect_contains(x: f32, y: f32, size: f32, r: f32) -> bool {
    let min = 0.5f32;
    let max = size - 0.5;
    if x < min || y < min || x > max || y > max {
        return false;
    }
    // Clamp to the inner rectangle; distance to that defines the corner rounding.
    let cx = x.clamp(min + r, max - r);
    let cy = y.clamp(min + r, max - r);
    let dx = x - cx;
    let dy = y - cy;
    (dx * dx + dy * dy) <= (r * r) + 0.5
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_expected_size_and_some_opaque_pixels() {
        let (px, w, h) = app_icon_rgba();
        assert_eq!((w, h), (32, 32));
        assert_eq!(px.len(), (w * h * 4) as usize);
        let opaque = px.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).count();
        // Most of the rounded square should be opaque, corners transparent.
        assert!(opaque > 600 && opaque < (w * h) as usize);
    }

    #[test]
    fn corners_are_transparent() {
        let (px, w, _h) = app_icon_rgba();
        let corner = px[0..4].to_vec(); // (0,0)
        assert_eq!(corner[3], 0);
        let _ = w;
    }
}
