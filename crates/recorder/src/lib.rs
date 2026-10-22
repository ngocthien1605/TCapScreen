//! `recorder`: screen-recording pipeline.
//!
//! M3 implements MP4 (H.264) video-only recording: a continuous WGC capture stream
//! ([`capture_core::CaptureStream`]) feeds a Media Foundation `IMFSinkWriter` on a dedicated
//! encoder thread. The encoder re-emits the latest frame at a constant rate (CFR), so a static
//! screen still produces a smooth, seekable file; pause simply stops the clock. Audio (M4) and
//! GIF (M5) are not yet implemented and return [`RecorderError::NotImplemented`].

use std::path::PathBuf;

use capture_core::CaptureTarget;
use thiserror::Error;

#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod mf;
#[cfg(windows)]
mod pipeline;
#[cfg(windows)]
pub mod probe;

/// Audio capture configuration for a recording.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioCfg {
    pub microphone: bool,
    pub system_audio: bool,
    pub microphone_device: Option<String>,
}

/// What kind of output a recording produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordOutput {
    Mp4 {
        fps: u32,
        bitrate: u32,
        audio: AudioCfg,
        cursor: bool,
    },
    Gif {
        fps: u32,
        cursor: bool,
        max_width: u32,
        max_seconds: u32,
    },
}

/// A full recording request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordCfg {
    pub target: CaptureTarget,
    pub output: RecordOutput,
    pub out_path: PathBuf,
}

/// Errors from the recording pipeline.
#[derive(Debug, Error)]
pub enum RecorderError {
    #[error("this recording feature is not implemented yet")]
    NotImplemented,
    #[error("no video encoder (Media Foundation H.264) is available")]
    NoEncoder,
    #[error("disk full while writing the recording")]
    DiskFull,
    #[error("capture error: {0}")]
    Capture(String),
    #[error("platform error: {0}")]
    Platform(String),
}

/// Result alias for recorder operations.
pub type Result<T> = std::result::Result<T, RecorderError>;

/// Round dimensions down to even values (H.264 rejects odd width/height).
pub fn even_dims(width: u32, height: u32) -> (u32, u32) {
    (width & !1, height & !1)
}

/// Handle to an in-progress recording. Dropping it stops and finalizes a playable file.
#[derive(Debug)]
pub struct RecorderHandle {
    #[cfg(windows)]
    inner: Option<pipeline::Session>,
    out_path: PathBuf,
}

impl RecorderHandle {
    /// Pause capture (the output clock stops; resume continues from where it left off).
    pub fn pause(&self) -> Result<()> {
        #[cfg(windows)]
        if let Some(s) = &self.inner {
            s.set_paused(true);
            return Ok(());
        }
        Err(RecorderError::NotImplemented)
    }

    /// Resume after a pause.
    pub fn resume(&self) -> Result<()> {
        #[cfg(windows)]
        if let Some(s) = &self.inner {
            s.set_paused(false);
            return Ok(());
        }
        Err(RecorderError::NotImplemented)
    }

    /// True if currently paused.
    pub fn is_paused(&self) -> bool {
        #[cfg(windows)]
        if let Some(s) = &self.inner {
            return s.is_paused();
        }
        false
    }

    /// Stop and finalize; returns the written file's path.
    pub fn stop(mut self) -> Result<PathBuf> {
        #[cfg(windows)]
        if let Some(s) = self.inner.take() {
            return s.stop();
        }
        Ok(self.out_path.clone())
    }
}

impl Drop for RecorderHandle {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(s) = self.inner.take() {
            // Finalize so a dropped handle still leaves a playable file.
            let _ = s.stop();
        }
    }
}

/// The recorder entry point.
#[derive(Debug, Default)]
pub struct Recorder;

impl Recorder {
    pub fn new() -> Self {
        Self
    }

    /// Begin a recording. Returns a handle for pause/resume/stop.
    pub fn start(&self, cfg: RecordCfg) -> Result<RecorderHandle> {
        match cfg.output {
            RecordOutput::Gif { .. } => Err(RecorderError::NotImplemented), // M5
            RecordOutput::Mp4 { .. } => {
                #[cfg(windows)]
                {
                    let out_path = cfg.out_path.clone();
                    let inner = pipeline::Session::start(cfg)?;
                    Ok(RecorderHandle {
                        inner: Some(inner),
                        out_path,
                    })
                }
                #[cfg(not(windows))]
                {
                    let _ = cfg;
                    Err(RecorderError::NotImplemented)
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use capture_core::MonitorId;

    #[test]
    fn even_dims_rounds_down() {
        assert_eq!(even_dims(1920, 1080), (1920, 1080));
        assert_eq!(even_dims(1235, 629), (1234, 628));
        assert_eq!(even_dims(1, 1), (0, 0));
    }

    #[test]
    fn gif_output_is_not_yet_implemented() {
        let r = Recorder::new();
        let cfg = RecordCfg {
            target: CaptureTarget::Monitor(MonitorId(0)),
            output: RecordOutput::Gif {
                fps: 15,
                cursor: true,
                max_width: 800,
                max_seconds: 30,
            },
            out_path: PathBuf::from("out.gif"),
        };
        assert!(matches!(r.start(cfg), Err(RecorderError::NotImplemented)));
    }
}
