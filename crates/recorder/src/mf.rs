//! A minimal Media Foundation `IMFSinkWriter` wrapper that encodes BGRA frames to H.264 in an
//! MP4 container. Input media type is RGB32 (== BGRA byte order); the SinkWriter inserts the
//! color converter to NV12 for the H.264 encoder. Callers drive it at a constant frame rate.

use std::ptr::null_mut;
use std::sync::Once;

use capture_core::Frame;
use windows::core::PCWSTR;
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaType, IMFSinkWriter, MFAudioFormat_AAC, MFAudioFormat_PCM,
    MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSinkWriterFromURL, MFMediaType_Audio, MFMediaType_Video, MFStartup, MFVideoFormat_H264,
    MFVideoFormat_RGB32, MFVideoInterlace_Progressive, MFSTARTUP_FULL,
    MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
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

/// Canonical audio format muxed alongside the video.
pub const AUDIO_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u32 = 2;

/// An H.264/MP4 sink that accepts BGRA frames (and optionally 48 kHz stereo PCM16 audio).
pub struct VideoSink {
    writer: IMFSinkWriter,
    stream_index: u32,
    audio_index: Option<u32>,
    width: u32,
    height: u32,
    frame_dur: i64,
}

impl VideoSink {
    /// Create a sink writing `path` at `width`×`height` (must be even), `fps`, and `bitrate`.
    /// When `audio` is true, an AAC stream (48 kHz stereo) is added for [`write_audio`].
    pub fn new(
        path: &str,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        audio: bool,
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

            // Optional AAC audio stream (48 kHz stereo 16-bit). Added before BeginWriting.
            let audio_index = if audio {
                let aac_out = MFCreateMediaType().map_err(mf)?;
                aac_out
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                    .map_err(mf)?;
                aac_out
                    .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)
                    .map_err(mf)?;
                aac_out
                    .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, AUDIO_RATE)
                    .map_err(mf)?;
                aac_out
                    .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)
                    .map_err(mf)?;
                aac_out
                    .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                    .map_err(mf)?;
                // 24000 B/s = 192 kbps (the AAC encoder only accepts 12k/16k/20k/24k).
                aac_out
                    .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 24_000)
                    .map_err(mf)?;
                let aidx = writer.AddStream(&aac_out).map_err(mf)?;

                let pcm_in = MFCreateMediaType().map_err(mf)?;
                pcm_in
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
                    .map_err(mf)?;
                pcm_in
                    .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)
                    .map_err(mf)?;
                pcm_in
                    .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, AUDIO_RATE)
                    .map_err(mf)?;
                pcm_in
                    .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, AUDIO_CHANNELS)
                    .map_err(mf)?;
                pcm_in
                    .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                    .map_err(mf)?;
                pcm_in
                    .SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)
                    .map_err(mf)?;
                pcm_in
                    .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, AUDIO_RATE * 4)
                    .map_err(mf)?;
                writer.SetInputMediaType(aidx, &pcm_in, None).map_err(mf)?;
                Some(aidx)
            } else {
                None
            };

            writer.BeginWriting().map_err(mf)?;

            Ok(VideoSink {
                writer,
                stream_index,
                audio_index,
                width,
                height,
                frame_dur,
            })
        }
    }

    /// Write a chunk of interleaved-stereo PCM16 audio at `time_100ns` (no-op if no audio stream).
    pub fn write_audio(&self, pcm: &[i16], time_100ns: i64) -> Result<(), RecorderError> {
        let Some(aidx) = self.audio_index else {
            return Ok(());
        };
        if pcm.is_empty() {
            return Ok(());
        }
        let frames = (pcm.len() / AUDIO_CHANNELS as usize) as i64;
        let dur = frames * 10_000_000 / AUDIO_RATE as i64;
        let bytes = pcm.len() * 2;
        // SAFETY: fill an MF buffer with the PCM bytes and write it to the audio stream.
        unsafe {
            let buffer = MFCreateMemoryBuffer(bytes as u32).map_err(mf)?;
            let mut ptr: *mut u8 = null_mut();
            buffer.Lock(&mut ptr, None, None).map_err(mf)?;
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(pcm.as_ptr() as *const u8, ptr, bytes);
            }
            buffer.Unlock().map_err(mf)?;
            buffer.SetCurrentLength(bytes as u32).map_err(mf)?;
            let sample = MFCreateSample().map_err(mf)?;
            sample.AddBuffer(&buffer).map_err(mf)?;
            sample.SetSampleTime(time_100ns).map_err(mf)?;
            sample.SetSampleDuration(dur).map_err(mf)?;
            self.writer.WriteSample(aidx, &sample).map_err(mf)?;
        }
        Ok(())
    }

    /// The per-frame duration in 100-ns units.
    pub fn frame_duration(&self) -> i64 {
        self.frame_dur
    }

    /// Write one frame at `time_100ns`. The frame is copied (BGRA, tightly packed) into an MF
    /// buffer, cropped to the sink's dimensions; a smaller frame (the source window shrank
    /// mid-recording) is padded with black. The sink's size is fixed at creation, so a source
    /// that grew is cropped to the original size rather than restarting the encoder.
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
            if ptr.is_null() || max_len < size {
                let _ = buffer.Unlock();
                return Err(RecorderError::Platform(format!(
                    "MF buffer too small: {max_len} < {size}"
                )));
            }
            // SAFETY: the buffer is locked and at least `size` bytes long.
            let dst = std::slice::from_raw_parts_mut(ptr, size as usize);
            pack_bgra(dst, self.width, self.height, frame);
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

/// Copy `frame` into `dst`, a tightly packed `dst_w`×`dst_h` BGRA buffer. Rows are copied up
/// to the smaller of the two widths; anything the frame does not cover (a smaller frame, a
/// short row in a truncated buffer) is zeroed. `MFCreateMemoryBuffer` memory is uninitialized,
/// so without this a shrunken source would leave garbage in the margins.
pub(crate) fn pack_bgra(dst: &mut [u8], dst_w: u32, dst_h: u32, frame: &Frame) {
    let (w, h) = (dst_w as usize, dst_h as usize);
    let dst_stride = w * 4;
    debug_assert!(dst.len() >= dst_stride * h);
    let copy_w = (frame.width as usize).min(w) * 4;
    let rows = (frame.height as usize).min(h);
    if frame.width as usize != w || frame.height as usize != h {
        dst.fill(0);
    }
    for y in 0..rows {
        let src_off = y * frame.stride;
        let dst_off = y * dst_stride;
        let Some(src) = frame.data.get(src_off..src_off + copy_w) else {
            // Short source buffer: blank this row rather than reading past the end.
            if let Some(d) = dst.get_mut(dst_off..dst_off + dst_stride) {
                d.fill(0);
            }
            continue;
        };
        if let Some(d) = dst.get_mut(dst_off..dst_off + copy_w) {
            d.copy_from_slice(src);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture_core::PixelFmt;
    use std::time::Duration;

    fn frame(w: u32, h: u32, stride: usize, fill: u8) -> Frame {
        let mut data = vec![0xEEu8; stride * h as usize]; // padding bytes are junk on purpose
        for y in 0..h as usize {
            for x in 0..w as usize * 4 {
                data[y * stride + x] = fill;
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
    fn smaller_frame_is_padded_with_black_not_garbage() {
        let mut dst = vec![0xAAu8; 4 * 4 * 4]; // 4x4 sink, pre-filled with "uninitialized" junk
        pack_bgra(&mut dst, 4, 4, &frame(2, 2, 2 * 4 + 8, 7));
        for y in 0..4 {
            for x in 0..4 {
                let px = &dst[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
                let want = if x < 2 && y < 2 { 7 } else { 0 };
                assert!(px.iter().all(|&b| b == want), "pixel ({x},{y}) = {px:?}");
            }
        }
    }

    #[test]
    fn larger_frame_is_cropped_and_stride_is_honoured() {
        let mut dst = vec![0u8; 2 * 2 * 4];
        let mut f = frame(4, 4, 4 * 4 + 12, 1);
        // Mark pixel (1,1) of the source distinctly; it must land at (1,1) of the 2x2 crop.
        let off = f.stride + 4;
        f.data[off..off + 4].copy_from_slice(&[9, 8, 7, 6]);
        pack_bgra(&mut dst, 2, 2, &f);
        assert_eq!(&dst[0..4], &[1, 1, 1, 1]);
        assert_eq!(&dst[12..16], &[9, 8, 7, 6]);
    }

    #[test]
    fn truncated_source_buffer_leaves_rows_black() {
        let mut dst = vec![0xAAu8; 2 * 2 * 4];
        let mut f = frame(2, 2, 8, 5);
        f.data.truncate(8); // only one row is actually present
        pack_bgra(&mut dst, 2, 2, &f);
        assert!(dst[..8].iter().all(|&b| b == 5));
        // Same-size frame but the missing row must still be zeroed, not left as junk.
        assert!(dst[8..].iter().all(|&b| b == 0), "{:?}", &dst[8..]);
    }
}
