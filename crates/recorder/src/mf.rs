//! A minimal Media Foundation `IMFSinkWriter` wrapper that encodes BGRA frames to H.264 in an
//! MP4 container. Input media type is RGB32 (== BGRA byte order); the SinkWriter inserts the
//! color converter to NV12 for the H.264 encoder. Callers drive it at a constant frame rate.

use std::ptr::null_mut;
use std::sync::Once;

use capture_core::Frame;
use windows::core::PCWSTR;
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaType, IMFSinkWriter, MFCreateAttributes, MFCreateMediaType,
    MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL, MFMediaType_Video, MFStartup,
    MFVideoFormat_H264, MFVideoFormat_RGB32, MFVideoInterlace_Progressive, MFSTARTUP_FULL,
    MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_DISABLE_THROTTLING,
};

use crate::RecorderError;

/// 100-ns ticks per second (Media Foundation time base).
const HNS_PER_SEC: i64 = 10_000_000;
/// `MF_VERSION` = (MF_SDK_VERSION << 16) | MF_API_VERSION.
const MF_VERSION: u32 = 0x0002_0070;

static MF_STARTUP: Once = Once::new();

fn mf(e: windows::core::Error) -> RecorderError {
    RecorderError::Platform(format!("{e}"))
}

fn set_u64_pair(
    ty: &IMFMediaType,
    key: &windows::core::GUID,
    hi: u32,
    lo: u32,
) -> Result<(), RecorderError> {
    let packed = ((hi as u64) << 32) | (lo as u64);
    // SAFETY: valid media type + key; stores a packed UINT64 attribute.
    unsafe { ty.SetUINT64(key, packed) }.map_err(mf)
}

/// An H.264/MP4 sink that accepts BGRA frames.
pub struct VideoSink {
    writer: IMFSinkWriter,
    stream_index: u32,
    width: u32,
    height: u32,
    frame_dur: i64,
}

impl VideoSink {
    /// Create a sink writing `path` at `width`×`height` (must be even), `fps`, and `bitrate`.
    pub fn new(
        path: &str,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<VideoSink, RecorderError> {
        MF_STARTUP.call_once(|| {
            // SAFETY: process-wide MF init; matched by no explicit shutdown (fine at exit).
            unsafe {
                let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
            }
        });

        let fps = fps.max(1);
        let frame_dur = HNS_PER_SEC / fps as i64;

        // SAFETY: standard SinkWriter setup; all COM calls are checked.
        unsafe {
            let mut attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attrs, 2).map_err(mf)?;
            let attrs =
                attrs.ok_or_else(|| RecorderError::Platform("MFCreateAttributes".into()))?;
            attrs
                .SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
                .map_err(mf)?;
            attrs
                .SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)
                .map_err(mf)?;

            let url: Vec<u16> = path.encode_utf16().chain([0]).collect();
            let writer =
                MFCreateSinkWriterFromURL(PCWSTR(url.as_ptr()), None, &attrs).map_err(mf)?;

            // Output type: H.264.
            let out_type = MFCreateMediaType().map_err(mf)?;
            out_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(mf)?;
            out_type
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
                .map_err(mf)?;
            out_type
                .SetUINT32(&MF_MT_AVG_BITRATE, bitrate)
                .map_err(mf)?;
            out_type
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(mf)?;
            set_u64_pair(&out_type, &MF_MT_FRAME_SIZE, width, height)?;
            set_u64_pair(&out_type, &MF_MT_FRAME_RATE, fps, 1)?;
            set_u64_pair(&out_type, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
            let stream_index = writer.AddStream(&out_type).map_err(mf)?;

            // Input type: RGB32 (BGRA), top-down (positive stride).
            let in_type = MFCreateMediaType().map_err(mf)?;
            in_type
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(mf)?;
            in_type
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                .map_err(mf)?;
            in_type
                .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
                .map_err(mf)?;
            in_type
                .SetUINT32(&MF_MT_DEFAULT_STRIDE, width * 4)
                .map_err(mf)?;
            set_u64_pair(&in_type, &MF_MT_FRAME_SIZE, width, height)?;
            set_u64_pair(&in_type, &MF_MT_FRAME_RATE, fps, 1)?;
            set_u64_pair(&in_type, &MF_MT_PIXEL_ASPECT_RATIO, 1, 1)?;
            writer
                .SetInputMediaType(stream_index, &in_type, None)
                .map_err(mf)?;

            writer.BeginWriting().map_err(mf)?;

            Ok(VideoSink {
                writer,
                stream_index,
                width,
                height,
                frame_dur,
            })
        }
    }

    /// The per-frame duration in 100-ns units.
    pub fn frame_duration(&self) -> i64 {
        self.frame_dur
    }

    /// Write one frame at `time_100ns`. The frame is copied (BGRA, tightly packed) into an MF
    /// buffer, cropping/zero-filling to the sink's dimensions.
    pub fn write_frame(&self, frame: &Frame, time_100ns: i64) -> Result<(), RecorderError> {
        let w = self.width as usize;
        let h = self.height as usize;
        let size = (w * h * 4) as u32;

        // SAFETY: fill a fresh MF buffer, attach to a sample, and write it.
        unsafe {
            let buffer = MFCreateMemoryBuffer(size).map_err(mf)?;
            let mut ptr: *mut u8 = null_mut();
            let mut max_len: u32 = 0;
            buffer
                .Lock(&mut ptr, Some(&mut max_len), None)
                .map_err(mf)?;
            if !ptr.is_null() {
                let copy_w = (frame.width as usize).min(w) * 4;
                let rows = (frame.height as usize).min(h);
                for y in 0..rows {
                    let src_off = y * frame.stride;
                    if let Some(src) = frame.data.get(src_off..src_off + copy_w) {
                        std::ptr::copy_nonoverlapping(src.as_ptr(), ptr.add(y * w * 4), copy_w);
                    }
                }
            }
            buffer.Unlock().map_err(mf)?;
            buffer.SetCurrentLength(size).map_err(mf)?;

            let sample = MFCreateSample().map_err(mf)?;
            sample.AddBuffer(&buffer).map_err(mf)?;
            sample.SetSampleTime(time_100ns).map_err(mf)?;
            sample.SetSampleDuration(self.frame_dur).map_err(mf)?;
            self.writer
                .WriteSample(self.stream_index, &sample)
                .map_err(mf)?;
        }
        Ok(())
    }

    /// Finalize the file (writes the MP4 `moov` box). Must be called or the file is unplayable.
    pub fn finalize(self) -> Result<(), RecorderError> {
        // SAFETY: finalizes and flushes the sink.
        unsafe { self.writer.Finalize() }.map_err(mf)
    }
}
