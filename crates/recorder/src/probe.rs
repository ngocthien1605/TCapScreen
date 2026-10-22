//! Read an MP4 back with a Media Foundation `IMFSourceReader` — the same decode stack Windows
//! Media Player uses — to prove it is a valid, decodable file. Reports frame size, sample count,
//! and the last timestamp, and can decode frame 0 to a PNG for an orientation/content check.

use std::path::Path;

use capture_core::{Frame, PixelFmt};
use windows::core::PCWSTR;
use windows::Win32::Media::MediaFoundation::{
    IMFSample, IMFSourceReader, MFAudioFormat_PCM, MFCreateMediaType, MFCreateSourceReaderFromURL,
    MFMediaType_Audio, MFMediaType_Video, MFStartup, MFVideoFormat_RGB32, MFSTARTUP_FULL,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM,
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

/// What a probe learned about a recorded file's audio stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioInfo {
    pub sample_rate: u32,
    pub channels: u32,
    pub frames: u64,
    /// RMS amplitude in [0, 1]; > ~0.1 means real audio (not silence).
    pub rms: f32,
    /// Dominant frequency estimate (Hz) via zero-crossing rate.
    pub dominant_hz: f32,
}

/// Decode the first audio stream (if any) and report RMS + dominant frequency.
pub fn probe_audio(path: &str) -> Option<AudioInfo> {
    // SAFETY: standard SourceReader audio decode loop.
    unsafe {
        let _ = MFStartup(0x0002_0070, MFSTARTUP_FULL);
        let url: Vec<u16> = path.encode_utf16().chain([0]).collect();
        let reader: IMFSourceReader =
            MFCreateSourceReaderFromURL(PCWSTR(url.as_ptr()), None).ok()?;
        let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

        let out = MFCreateMediaType().ok()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).ok()?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).ok()?;
        reader.SetCurrentMediaType(stream, None, &out).ok()?;

        let cur = reader.GetCurrentMediaType(stream).ok()?;
        let rate = cur
            .GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
            .unwrap_or(48_000);
        let channels = cur.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(2).max(1);

        let mut mono: Vec<f32> = Vec::new();
        let cap = (rate as usize) * 30; // probe at most ~30 s
        loop {
            let mut flags: u32 = 0;
            let mut sample: Option<IMFSample> = None;
            if reader
                .ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample))
                .is_err()
            {
                break;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            let Some(sample) = sample else { continue };
            let Ok(buffer) = sample.ConvertToContiguousBuffer() else {
                continue;
            };
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut len: u32 = 0;
            if buffer.Lock(&mut ptr, None, Some(&mut len)).is_err() || ptr.is_null() {
                let _ = buffer.Unlock();
                continue;
            }
            let bytes = std::slice::from_raw_parts(ptr, len as usize);
            for frame in bytes.chunks_exact(2 * channels as usize) {
                let mut acc = 0f32;
                for c in 0..channels as usize {
                    let s = i16::from_le_bytes([frame[c * 2], frame[c * 2 + 1]]);
                    acc += s as f32 / 32768.0;
                }
                mono.push(acc / channels as f32);
            }
            let _ = buffer.Unlock();
            if mono.len() >= cap {
                break;
            }
        }

        if mono.is_empty() {
            return Some(AudioInfo {
                sample_rate: rate,
                channels,
                frames: 0,
                rms: 0.0,
                dominant_hz: 0.0,
            });
        }
        let sum_sq: f64 = mono.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let rms = (sum_sq / mono.len() as f64).sqrt() as f32;
        // Zero-crossing frequency estimate.
        let mut crossings = 0u64;
        for w in mono.windows(2) {
            if (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0) {
                crossings += 1;
            }
        }
        let secs = mono.len() as f32 / rate as f32;
        let dominant_hz = if secs > 0.0 {
            (crossings as f32 / 2.0) / secs
        } else {
            0.0
        };

        Some(AudioInfo {
            sample_rate: rate,
            channels,
            frames: mono.len() as u64,
            rms,
            dominant_hz,
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
