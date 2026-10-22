//! `recorder`: screen-recording pipeline.
//!
//! Milestone status: the public shape (config + handle) is defined here; the Media Foundation
//! H.264 encoder, WASAPI/mic audio, muxing, and GIF export land in M3–M5. Until then
//! [`Recorder::start`] returns [`RecorderError::NotImplemented`] rather than panicking, so the
//! workspace stays green and callers get a clean error.

use std::path::PathBuf;

use capture_core::CaptureTarget;
use thiserror::Error;

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
    #[error("recording is not implemented until milestone M3")]
    NotImplemented,
    #[error("no video encoder (Media Foundation H.264) is available")]
    NoEncoder,
    #[error("disk full while writing the recording")]
    DiskFull,
    #[error("platform error: {0}")]
    Platform(String),
}

/// Result alias for recorder operations.
pub type Result<T> = std::result::Result<T, RecorderError>;

/// Handle to an in-progress recording. Dropping it without `stop()` aborts and finalizes.
#[derive(Debug)]
pub struct RecorderHandle {
    _private: (),
}

impl RecorderHandle {
    /// Pause capture (timing is preserved on resume).
    pub fn pause(&self) -> Result<()> {
        Err(RecorderError::NotImplemented)
    }
    /// Resume after a pause.
    pub fn resume(&self) -> Result<()> {
        Err(RecorderError::NotImplemented)
    }
    /// Stop and finalize a playable file; returns its path.
    pub fn stop(self) -> Result<PathBuf> {
        Err(RecorderError::NotImplemented)
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
    pub fn start(&self, _cfg: RecordCfg) -> Result<RecorderHandle> {
        Err(RecorderError::NotImplemented)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use capture_core::MonitorId;

    #[test]
    fn start_is_not_yet_implemented_but_does_not_panic() {
        let r = Recorder::new();
        let cfg = RecordCfg {
            target: CaptureTarget::Monitor(MonitorId(0)),
            output: RecordOutput::Mp4 {
                fps: 30,
                bitrate: 8_000_000,
                audio: AudioCfg::default(),
                cursor: true,
            },
            out_path: PathBuf::from("out.mp4"),
        };
        assert!(matches!(r.start(cfg), Err(RecorderError::NotImplemented)));
    }
}
