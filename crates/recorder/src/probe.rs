//! Read an MP4 back with a Media Foundation `IMFSourceReader` — the same decode stack Windows
//! Media Player uses — to prove it is a valid, decodable file. Reports frame size, sample count,
//! and the last timestamp, and can decode frame 0 to a PNG for an orientation/content check.

use std::path::Path;

use capture_core::{Frame, PixelFmt};
use windows::core::PCWSTR;
use windows::Win32::Media::MediaFoundation::{
    IMFSample, IMFSourceReader, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Video,
    MFStartup, MFVideoFormat_RGB32, MFSTARTUP_FULL, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
};

use crate::RecorderError;

/// What a probe learned about a recorded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeInfo {
    pub width: u32,
    pub height: u32,
    pub sample_count: u64,
    /// Timestamp of the last decoded sample, in 100-ns units (≈ duration).
    pub last_timestamp_100ns: i64,
}

fn mf(e: windows::core::Error) -> RecorderError {
    RecorderError::Platform(format!("{e}"))
}

/// Decode `path`, counting frames; if `first_frame_png` is set, write frame 0 there as PNG.
pub fn probe(path: &str, first_frame_png: Option<&Path>) -> Result<ProbeInfo, RecorderError> {
    // SAFETY: MF init is idempotent per process.
    unsafe {
        let _ = MFStartup(0x0002_0070, MFSTARTUP_FULL);
    }

    // SAFETY: standard SourceReader decode loop; all COM calls are checked.
    unsafe {
        let attrs = {
            let mut a = None;
            windows::Win32::Media::MediaFoundation::MFCreateAttributes(&mut a, 2).map_err(mf)?;
            a.ok_or_else(|| RecorderError::Platform("MFCreateAttributes".into()))?
        };
        attrs
            .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
            .map_err(mf)?;
        attrs
            .SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
            .map_err(mf)?;

        let url: Vec<u16> = path.encode_utf16().chain([0]).collect();
        let reader: IMFSourceReader =
            MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), &attrs).map_err(mf)?;

        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

        // Ask for RGB32 output so we can read pixels directly.
        let out = MFCreateMediaType().map_err(mf)?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(mf)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
            .map_err(mf)?;
        reader.SetCurrentMediaType(stream, None, &out).map_err(mf)?;

        // Frame size from the negotiated current type.
        let cur = reader.GetCurrentMediaType(stream).map_err(mf)?;
        let packed = cur.GetUINT64(&MF_MT_FRAME_SIZE).map_err(mf)?;
        let width = (packed >> 32) as u32;
        let height = (packed & 0xFFFF_FFFF) as u32;

        let mut sample_count: u64 = 0;
        let mut last_ts: i64 = 0;
        loop {
            let mut flags: u32 = 0;
            let mut ts: i64 = 0;
            let mut sample: Option<IMFSample> = None;
            reader
                .ReadSample(
                    stream,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut ts),
                    Some(&mut sample),
                )
                .map_err(mf)?;

            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(sample) = sample else {
                // No sample but not EOS (e.g. a format change): keep reading.
                continue;
            };
            sample_count += 1;
            last_ts = ts;

            if sample_count == 1 {
                if let Some(png) = first_frame_png {
                    write_first_frame(&sample, width, height, png)?;
                }
            }
        }

        Ok(ProbeInfo {
            width,
            height,
            sample_count,
            last_timestamp_100ns: last_ts,
        })
    }
}

/// Copy a decoded RGB32 sample into a [`Frame`] and save it as PNG (via `imaging`).
unsafe fn write_first_frame(
    sample: &IMFSample,
    width: u32,
    height: u32,
    png: &Path,
) -> Result<(), RecorderError> {
    // SAFETY: contiguous-buffer lock/unlock around a bounded copy.
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer().map_err(mf)?;
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut cur: u32 = 0;
        buffer.Lock(&mut ptr, None, Some(&mut cur)).map_err(mf)?;
        let len = cur as usize;
        let data = if ptr.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(ptr, len).to_vec()
        };
        let _ = buffer.Unlock();

        let stride = if height > 0 { len / height as usize } else { 0 };
        let frame = Frame {
            data,
            width,
            height,
            stride,
            format: PixelFmt::Bgra8,
            timestamp: std::time::Duration::ZERO,
        };
        let img = imaging::frame_to_rgba(&frame, true)
            .map_err(|e| RecorderError::Platform(format!("decode→rgba: {e}")))?;
        imaging::save_image(&img, png, 90)
            .map_err(|e| RecorderError::Platform(format!("save png: {e}")))?;
    }
    Ok(())
}
