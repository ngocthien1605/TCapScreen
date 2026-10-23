//! Imaging: convert captured BGRA frames to RGBA, crop (stride-aware), encode PNG/JPEG,
//! make thumbnails, and generate deterministic output filenames.
//!
//! Everything here is pure (no OS calls except filesystem writes in [`save_image`] and
//! [`unique_path`]) so it is fully unit-testable on a headless runner.

use std::path::{Path, PathBuf};

use capture_core::{Frame, PixelFmt, Rect};
use image::{codecs::jpeg::JpegEncoder, ImageEncoder, RgbImage, RgbaImage};
use thiserror::Error;

/// Re-export so downstream crates can name the RGBA image type without depending on `image`.
pub use image::RgbaImage as Rgba;

/// Load an image file (PNG/JPEG) from disk as RGBA.
pub fn load_rgba(path: &Path) -> Result<RgbaImage, ImagingError> {
    Ok(image::open(path)?.to_rgba8())
}

/// Encode an RGBA image as a Windows `.ico` (single image; ≤256 px per side).
pub fn save_ico(img: &RgbaImage, path: &Path) -> Result<(), ImagingError> {
    use image::codecs::ico::IcoEncoder;
    use image::ExtendedColorType;

    let file = std::fs::File::create(path).map_err(|source| ImagingError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    IcoEncoder::new(std::io::BufWriter::new(file)).write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        ExtendedColorType::Rgba8,
    )?;
    Ok(())
}

/// What [`probe_gif`] found in a GIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GifInfo {
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    /// Total playback time (sum of the frame delays).
    pub duration: std::time::Duration,
}

/// Decode a GIF and report its size, frame count and playback time — used to verify output.
pub fn probe_gif(path: &Path) -> Result<GifInfo, ImagingError> {
    use image::codecs::gif::GifDecoder;
    use image::AnimationDecoder;

    let file = std::fs::File::open(path).map_err(|source| ImagingError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let decoder = GifDecoder::new(std::io::BufReader::new(file))?;
    let frames = decoder.into_frames().collect_frames()?;
    let (width, height) = match frames.first() {
        Some(f) => (f.buffer().width(), f.buffer().height()),
        None => (0, 0),
    };
    let duration = frames
        .iter()
        .map(|f| {
            let (num, den) = f.delay().numer_denom_ms();
            std::time::Duration::from_secs_f64(num as f64 / den.max(1) as f64 / 1000.0)
        })
        .sum();
    Ok(GifInfo {
        width,
        height,
        frames: frames.len(),
        duration,
    })
}

/// GIF frame delays in centiseconds (the format's unit) for frames shown for `durations`,
/// rounded on the running total so the rounding never accumulates into drift.
pub fn gif_delays_cs(durations: &[std::time::Duration]) -> Vec<u16> {
    let mut elapsed = std::time::Duration::ZERO;
    let mut prev_cs: u128 = 0;
    durations
        .iter()
        .map(|d| {
            elapsed += *d;
            let cs = (elapsed.as_micros() + 5_000) / 10_000;
            let delay = (cs - prev_cs).min(u16::MAX as u128) as u16;
            prev_cs = cs;
            delay
        })
        .collect()
}

/// Encode frames as an animated, looping GIF, each shown for its own duration (a frame that
/// stayed on screen across several capture ticks is stored once, with a longer delay). Written
/// to a temporary file and renamed, like [`save_image`].
pub fn save_gif(
    frames: &[(RgbaImage, std::time::Duration)],
    path: &Path,
) -> Result<(), ImagingError> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let encode = || -> Result<(), ImagingError> {
        let file = std::fs::File::create(&tmp).map_err(io_err(&tmp))?;
        let mut encoder = GifEncoder::new_with_speed(std::io::BufWriter::new(file), 15);
        encoder.set_repeat(Repeat::Infinite)?;
        let durations: Vec<_> = frames.iter().map(|(_, d)| *d).collect();
        for ((img, _), cs) in frames.iter().zip(gif_delays_cs(&durations)) {
            // A zero delay would be shown as 100 ms by browsers; keep at least one unit.
            let delay = Delay::from_numer_denom_ms(cs.max(1) as u32 * 10, 1);
            encoder.encode_frame(Frame::from_parts(img.clone(), 0, 0, delay))?;
        }
        Ok(())
    };
    let result = encode().and_then(|()| std::fs::rename(&tmp, path).map_err(io_err(path)));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Errors from imaging operations.
#[derive(Debug, Error)]
pub enum ImagingError {
    #[error("frame buffer is too small for its declared dimensions/stride")]
    FrameTooSmall,
    #[error("crop rectangle lies outside the image")]
    CropOutOfBounds,
    #[error("image encode/decode failed: {0}")]
    Codec(#[from] image::ImageError),
    #[error("could not write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Convert a captured BGRA8 [`Frame`] into an `RgbaImage`, honouring `stride` padding.
///
/// When `force_opaque` is true, the alpha channel is set to 255 (screenshots should not be
/// accidentally transparent just because the desktop surface carried alpha).
pub fn frame_to_rgba(frame: &Frame, force_opaque: bool) -> Result<RgbaImage, ImagingError> {
    debug_assert_eq!(frame.format, PixelFmt::Bgra8);
    let w = frame.width;
    let h = frame.height;
    let row_bytes = w as usize * 4;
    if frame.stride < row_bytes || frame.data.len() < frame.stride * h as usize {
        return Err(ImagingError::FrameTooSmall);
    }
    let mut out = vec![0u8; row_bytes * h as usize];
    for y in 0..h as usize {
        let src = &frame.data[y * frame.stride..y * frame.stride + row_bytes];
        let dst = &mut out[y * row_bytes..(y + 1) * row_bytes];
        let (src_px, _) = src.as_chunks::<4>();
        let (dst_px, _) = dst.as_chunks_mut::<4>();
        for (s, d) in src_px.iter().zip(dst_px.iter_mut()) {
            // BGRA -> RGBA
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = if force_opaque { 255 } else { s[3] };
        }
    }
    RgbaImage::from_raw(w, h, out).ok_or(ImagingError::FrameTooSmall)
}

/// Crop an image to `rect` (coordinates local to the image). Errors if the rect escapes the
/// image bounds. An empty rect is rejected by the caller (treated as "cancel").
pub fn crop(img: &RgbaImage, rect: Rect) -> Result<RgbaImage, ImagingError> {
    if rect.is_empty() {
        return Err(ImagingError::CropOutOfBounds);
    }
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    if rect.left() < 0 || rect.top() < 0 || rect.right() > iw || rect.bottom() > ih {
        return Err(ImagingError::CropOutOfBounds);
    }
    let sub = image::imageops::crop_imm(
        img,
        rect.x as u32,
        rect.y as u32,
        rect.width as u32,
        rect.height as u32,
    );
    Ok(sub.to_image())
}

/// Downscale (never upscale) to fit within `max_dim` on the longer edge, preserving aspect.
pub fn thumbnail(img: &RgbaImage, max_dim: u32) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    if w <= max_dim && h <= max_dim {
        return img.clone();
    }
    let scale = max_dim as f64 / w.max(h) as f64;
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    image::imageops::thumbnail(img, nw, nh)
}

/// Encode an RGBA image as PNG bytes.
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, ImagingError> {
    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buf);
    encoder.write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(buf)
}

/// Encode an RGBA image as JPEG bytes at `quality` (1..=100). Alpha is dropped (JPEG has none).
pub fn encode_jpeg(img: &RgbaImage, quality: u8) -> Result<Vec<u8>, ImagingError> {
    let rgb = rgba_to_rgb(img);
    let mut buf = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100));
    encoder.encode(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf)
}

fn rgba_to_rgb(img: &RgbaImage) -> RgbImage {
    let mut out = RgbImage::new(img.width(), img.height());
    for (dst, src) in out.pixels_mut().zip(img.pixels()) {
        dst.0 = [src.0[0], src.0[1], src.0[2]];
    }
    out
}

/// Encode for `path`'s extension (png/jpg/jpeg; anything else is PNG).
fn encode_for(img: &RgbaImage, path: &Path, jpeg_quality: u8) -> Result<Vec<u8>, ImagingError> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => encode_jpeg(img, jpeg_quality),
        _ => encode_png(img),
    }
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> ImagingError + '_ {
    move |source| ImagingError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Write `bytes` to `path` via a temporary sibling and a rename, so a crash or full disk
/// mid-write never leaves a truncated file under the final name. Replaces an existing file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ImagingError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(io_err(&tmp))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(path)(e)
    })
}

/// Save an image to `path`, choosing the encoder from the file extension (png/jpg/jpeg). The
/// write is atomic (temporary file + rename); an existing file at `path` is replaced.
pub fn save_image(img: &RgbaImage, path: &Path, jpeg_quality: u8) -> Result<(), ImagingError> {
    let bytes = encode_for(img, path, jpeg_quality)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_err(parent))?;
    }
    write_atomic(path, &bytes)
}

/// Save an image in `dir` under `file_name`, or `stem_2.ext`, `stem_3.ext`, … if taken, and
/// return the path used. The name is claimed with an exclusive create before anything is
/// written, so two saves in the same second can never pick (and overwrite) the same file, and
/// the content is then written atomically over that placeholder.
pub fn save_image_unique(
    img: &RgbaImage,
    dir: &Path,
    file_name: &str,
    jpeg_quality: u8,
) -> Result<PathBuf, ImagingError> {
    let bytes = encode_for(img, Path::new(file_name), jpeg_quality)?;
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    let mut last_err = None;
    for name in candidate_names(file_name) {
        let path = dir.join(&name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(placeholder) => {
                drop(placeholder);
                return match write_atomic(&path, &bytes) {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        let _ = std::fs::remove_file(&path);
                        Err(e)
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                last_err = Some(io_err(&path)(e));
                break;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        io_err(dir)(std::io::Error::other(format!(
            "no free file name for {file_name}"
        )))
    }))
}

/// `file_name`, then `stem_2.ext` … `stem_9999.ext`, then a last name made unique by the
/// process id and the current time.
fn candidate_names(file_name: &str) -> impl Iterator<Item = String> + '_ {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let ext = path.extension().and_then(|s| s.to_str());
    let with = move |suffix: String| match ext {
        Some(e) => format!("{stem}_{suffix}.{e}"),
        None => format!("{stem}_{suffix}"),
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::iter::once(file_name.to_string())
        .chain((2..10_000).map(move |n| with(n.to_string())))
        .chain(std::iter::once(with(format!(
            "{}-{nanos}",
            std::process::id()
        ))))
}

/// Heuristic: is the image almost entirely black? Protected/DRM windows captured via WGC come
/// back all-black, so the app can warn rather than silently save a black screenshot. Samples
/// every `sample_stride`-th pixel; returns true if ≥ 99% of samples have max(R,G,B) ≤ `threshold`.
pub fn is_near_black(img: &RgbaImage, threshold: u8, sample_stride: u32) -> bool {
    let stride = sample_stride.max(1);
    let (mut total, mut dark) = (0u64, 0u64);
    for (x, y, px) in img.enumerate_pixels() {
        if x % stride != 0 || y % stride != 0 {
            continue;
        }
        total += 1;
        let [r, g, b, _] = px.0;
        if r.max(g).max(b) <= threshold {
            dark += 1;
        }
    }
    if total == 0 {
        return false;
    }
    dark * 100 >= total * 99
}

/// A capture timestamp broken into fields (so naming is deterministic and testable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Timestamp {
    /// The current local time (falling back to UTC if the local offset can't be determined).
    pub fn now_local() -> Self {
        let now =
            time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
        Self {
            year: now.year(),
            month: u8::from(now.month()),
            day: now.day(),
            hour: now.hour(),
            minute: now.minute(),
            second: now.second(),
        }
    }
}

/// Build a base filename `RustShot_YYYY-MM-DD_HH-mm-ss.<ext>`.
pub fn auto_name(ts: Timestamp, ext: &str) -> String {
    format!(
        "RustShot_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.{}",
        ts.year, ts.month, ts.day, ts.hour, ts.minute, ts.second, ext
    )
}

/// Resolve a path in `dir` for `file_name` that does not exist yet: `file_name`, else
/// `stem_2.ext`, `stem_3.ext`, … Nothing is created, so prefer [`save_image_unique`] when this
/// process writes the file itself; this is for writers that create the file on their own (the
/// video encoder). Never returns an existing file.
pub fn unique_path(dir: &Path, file_name: &str) -> PathBuf {
    candidate_names(file_name)
        .map(|name| dir.join(name))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(file_name))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use capture_core::Frame;
    use std::time::Duration;

    fn solid_frame(w: u32, h: u32, bgra: [u8; 4], pad: usize) -> Frame {
        let stride = w as usize * 4 + pad;
        let mut data = vec![0u8; stride * h as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let o = y * stride + x * 4;
                data[o..o + 4].copy_from_slice(&bgra);
            }
        }
        Frame {
            data,
            width: w,
            height: h,
            stride,
            format: PixelFmt::Bgra8,
            timestamp: Duration::ZERO,
        }
    }

    #[test]
    fn bgra_to_rgba_channel_swap_and_opaque() {
        // BGRA (10, 20, 30, 40) -> RGBA (30, 20, 10, 255) when forced opaque.
        let f = solid_frame(3, 2, [10, 20, 30, 40], 0);
        let img = frame_to_rgba(&f, true).unwrap();
        assert_eq!(img.dimensions(), (3, 2));
        for px in img.pixels() {
            assert_eq!(px.0, [30, 20, 10, 255]);
        }
    }

    #[test]
    fn bgra_to_rgba_preserves_alpha_when_requested() {
        let f = solid_frame(1, 1, [10, 20, 30, 40], 0);
        let img = frame_to_rgba(&f, false).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [30, 20, 10, 40]);
    }

    #[test]
    fn stride_padding_does_not_shear_image() {
        // 4 bytes of padding per row must be skipped, not treated as pixels.
        let f = solid_frame(2, 2, [1, 2, 3, 255], 4);
        let img = frame_to_rgba(&f, true).unwrap();
        assert_eq!(img.dimensions(), (2, 2));
        for px in img.pixels() {
            assert_eq!(px.0, [3, 2, 1, 255]);
        }
    }

    #[test]
    fn frame_too_small_is_rejected() {
        let mut f = solid_frame(2, 2, [0, 0, 0, 0], 0);
        f.width = 100; // declare more than the buffer holds
        assert!(matches!(
            frame_to_rgba(&f, true),
            Err(ImagingError::FrameTooSmall)
        ));
    }

    #[test]
    fn crop_extracts_correct_region() {
        let mut img = RgbaImage::new(4, 4);
        // Mark pixel (1,2) distinctly.
        img.put_pixel(1, 2, image::Rgba([9, 8, 7, 255]));
        let sub = crop(&img, Rect::new(1, 2, 2, 1)).unwrap();
        assert_eq!(sub.dimensions(), (2, 1));
        assert_eq!(sub.get_pixel(0, 0).0, [9, 8, 7, 255]);
    }

    #[test]
    fn crop_out_of_bounds_errors() {
        let img = RgbaImage::new(4, 4);
        assert!(matches!(
            crop(&img, Rect::new(3, 3, 5, 5)),
            Err(ImagingError::CropOutOfBounds)
        ));
        assert!(matches!(
            crop(&img, Rect::new(0, 0, 0, 0)),
            Err(ImagingError::CropOutOfBounds)
        ));
    }

    #[test]
    fn thumbnail_preserves_aspect_and_never_upscales() {
        let img = RgbaImage::new(800, 400);
        let t = thumbnail(&img, 100);
        assert_eq!(t.dimensions(), (100, 50));
        let small = RgbaImage::new(50, 50);
        assert_eq!(thumbnail(&small, 100).dimensions(), (50, 50));
    }

    #[test]
    fn png_round_trips() {
        let mut img = RgbaImage::new(3, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let bytes = encode_png(&img).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (3, 2));
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn jpeg_encodes_decodably() {
        let img = RgbaImage::from_pixel(16, 16, image::Rgba([120, 60, 30, 255]));
        let bytes = encode_jpeg(&img, 90).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (16, 16));
        // JPEG is lossy; just check we're in the right neighbourhood.
        let p = decoded.get_pixel(8, 8).0;
        assert!((p[0] as i32 - 120).abs() < 20);
    }

    #[test]
    fn near_black_detection() {
        let black = RgbaImage::from_pixel(32, 32, image::Rgba([2, 1, 0, 255]));
        assert!(is_near_black(&black, 10, 1));
        let bright = RgbaImage::from_pixel(32, 32, image::Rgba([200, 200, 200, 255]));
        assert!(!is_near_black(&bright, 10, 1));
        // Mostly black with a small bright patch is still "near black".
        let mut mostly = RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 255]));
        for y in 0..5 {
            for x in 0..5 {
                mostly.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        assert!(is_near_black(&mostly, 10, 1));
    }

    #[test]
    fn auto_name_format() {
        let ts = Timestamp {
            year: 2026,
            month: 10,
            day: 22,
            hour: 9,
            minute: 7,
            second: 3,
        };
        assert_eq!(auto_name(ts, "png"), "RustShot_2026-10-22_09-07-03.png");
        assert_eq!(auto_name(ts, "jpg"), "RustShot_2026-10-22_09-07-03.jpg");
    }

    #[test]
    fn save_image_unique_claims_distinct_names_and_leaves_no_temp_files() {
        let dir = std::env::temp_dir().join(format!("rustshot_saveuniq_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let img = RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        let name = "RustShot_2026-10-23_10-00-00.png";
        let a = save_image_unique(&img, &dir, name, 90).unwrap();
        let b = save_image_unique(&img, &dir, name, 90).unwrap();
        assert_eq!(a, dir.join(name));
        assert_eq!(b, dir.join("RustShot_2026-10-23_10-00-00_2.png"));
        assert_eq!(
            image::open(&b).unwrap().width(),
            3,
            "content written, not the placeholder"
        );
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "no temp files left behind: {names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_image_replaces_atomically() {
        let dir = std::env::temp_dir().join(format!("rustshot_atomic_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("out.png");
        let img = RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]));
        save_image(&img, &path, 90).unwrap();
        save_image(
            &RgbaImage::from_pixel(5, 1, image::Rgba([0, 0, 0, 255])),
            &path,
            90,
        )
        .unwrap();
        assert_eq!(image::open(&path).unwrap().width(), 5);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn candidate_names_end_in_a_name_that_cannot_collide() {
        let names: Vec<String> = candidate_names("a.png").collect();
        assert_eq!(names[0], "a.png");
        assert_eq!(names[1], "a_2.png");
        assert_eq!(names.len(), 1 + 9_998 + 1);
        let last = names.last().unwrap();
        assert!(last.starts_with(&format!("a_{}-", std::process::id())) && last.ends_with(".png"));
    }

    #[test]
    fn gif_delays_round_on_the_running_total() {
        use std::time::Duration;
        // 15 fps: 66.67 ms per frame → 7, 6, 7, … centiseconds, summing to exactly 1 s / 15 f.
        let d = vec![Duration::from_nanos(1_000_000_000 / 15); 15];
        let cs = gif_delays_cs(&d);
        assert_eq!(cs.iter().map(|&c| c as u32).sum::<u32>(), 100);
        assert!(cs.iter().all(|&c| c == 6 || c == 7), "{cs:?}");
        // A frame held for three slots gets one long delay.
        assert_eq!(gif_delays_cs(&[Duration::from_millis(200)]), vec![20]);
    }

    #[test]
    fn save_gif_round_trips_frame_timing() {
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("rustshot_gif_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.gif");
        let red = RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]));
        let blue = RgbaImage::from_pixel(4, 3, image::Rgba([0, 0, 255, 255]));
        save_gif(
            &[
                (red, Duration::from_millis(500)),
                (blue, Duration::from_millis(250)),
            ],
            &path,
        )
        .unwrap();
        let info = probe_gif(&path).unwrap();
        assert_eq!((info.width, info.height, info.frames), (4, 3, 2));
        assert_eq!(info.duration, Duration::from_millis(750));
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "no temp file left"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_path_increments_on_collision() {
        let dir = std::env::temp_dir().join(format!("rustshot_uniq_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = "RustShot_2026-10-22_09-07-03.png";
        let first = unique_path(&dir, name);
        assert_eq!(first, dir.join(name));
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir, name);
        assert_eq!(second, dir.join("RustShot_2026-10-22_09-07-03_2.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_image_picks_encoder_from_extension() {
        let dir = std::env::temp_dir().join(format!("rustshot_save_{}", std::process::id()));
        let img = RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
        let png = dir.join("a.png");
        let jpg = dir.join("b.jpg");
        save_image(&img, &png, 90).unwrap();
        save_image(&img, &jpg, 90).unwrap();
        assert!(image::load_from_memory(&std::fs::read(&png).unwrap())
            .unwrap()
            .color()
            .has_alpha());
        // JPEG decodes as RGB (no alpha).
        assert!(!image::load_from_memory(&std::fs::read(&jpg).unwrap())
            .unwrap()
            .color()
            .has_alpha());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
