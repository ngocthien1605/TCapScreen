//! `recorder`: screen-recording pipeline.
//!
//! M3 implements MP4 (H.264) video-only recording: a continuous WGC capture stream
//! ([`capture_core::CaptureStream`]) feeds a Media Foundation `IMFSinkWriter` on a dedicated
//! encoder thread. The encoder re-emits the latest frame at a constant rate (CFR), so a static
//! screen still produces a smooth, seekable file; pause simply stops the clock. Optional system
//! and microphone audio (WASAPI) is muxed as AAC; GIF output has its own session.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use capture_core::CaptureTarget;
use thiserror::Error;

#[cfg(windows)]
mod audio;
#[cfg_attr(not(windows), allow(dead_code))]
mod clock;
#[cfg(windows)]
mod gif;
#[cfg(windows)]
mod mf;
#[cfg(windows)]
mod pipeline;
#[cfg(windows)]
pub mod probe;

#[cfg(windows)]
pub use audio::{list_inputs, probe_capture, AudioInput, AudioProbe, AudioRequest};

/// Things the user should know about a recording that still produced a file — chiefly audio
/// that could not be recorded (a missing microphone, a device that stopped). Shared between the
/// recording threads and the handle; each distinct note is kept once, in order.
#[derive(Debug, Clone, Default)]
pub struct Notes(Arc<Mutex<Vec<String>>>);

impl Notes {
    /// Record a note (and log it). Duplicates are ignored.
    pub fn push(&self, note: impl Into<String>) {
        let note = note.into();
        let mut v = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !v.contains(&note) {
            tracing::warn!("recording note: {note}");
            v.push(note);
        }
    }

    /// Take the notes gathered so far.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

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
    /// The recording stopped before the user asked it to — the source window was closed, the
    /// graphics device was lost, or a write failed — but everything captured up to that point
    /// was finalized into a playable file at `path`.
    #[error("the recording ended early ({reason}); what was captured is saved at {}", path.display())]
    EndedEarly { path: PathBuf, reason: String },
}

/// Result alias for recorder operations.
pub type Result<T> = std::result::Result<T, RecorderError>;

/// Round dimensions down to even values (H.264 rejects odd width/height).
pub fn even_dims(width: u32, height: u32) -> (u32, u32) {
    (width & !1, height & !1)
}

/// The active recording backend (MP4 video or animated GIF).
#[cfg(windows)]
#[derive(Debug)]
enum Backend {
    Video(pipeline::Session),
    Gif(gif::GifSession),
}

#[cfg(windows)]
impl Backend {
    fn set_paused(&self, v: bool) {
        match self {
            Backend::Video(s) => s.set_paused(v),
            Backend::Gif(s) => s.set_paused(v),
        }
    }
    fn is_paused(&self) -> bool {
        match self {
            Backend::Video(s) => s.is_paused(),
            Backend::Gif(s) => s.is_paused(),
        }
    }
    fn is_finished(&self) -> bool {
        match self {
            Backend::Video(s) => s.is_finished(),
            Backend::Gif(s) => s.is_finished(),
        }
    }
    fn stop(self) -> Result<PathBuf> {
        match self {
            Backend::Video(s) => s.stop(),
            Backend::Gif(s) => s.stop(),
        }
    }
}

/// Handle to an in-progress recording. Dropping it stops and finalizes a playable file.
#[derive(Debug)]
pub struct RecorderHandle {
    #[cfg(windows)]
    inner: Option<Backend>,
    out_path: PathBuf,
    notes: Notes,
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

    /// True once the recording has ended on its own (source closed, device lost, write
    /// failure, or the GIF length cap). The owner should then call [`stop`](Self::stop) to
    /// collect the result — typically [`RecorderError::EndedEarly`] — and tear down its UI.
    pub fn is_finished(&self) -> bool {
        #[cfg(windows)]
        if let Some(s) = &self.inner {
            return s.is_finished();
        }
        true
    }

    /// Stop and finalize; returns the written file's path.
    pub fn stop(self) -> Result<PathBuf> {
        self.finish().0
    }

    /// Stop and finalize, also returning the recording's [`Notes`] (for example "the
    /// microphone could not be recorded"), so the caller can tell the user what the file lacks.
    pub fn finish(mut self) -> (Result<PathBuf>, Vec<String>) {
        #[cfg(windows)]
        if let Some(s) = self.inner.take() {
            let result = s.stop();
            return (result, self.notes.take());
        }
        (Ok(self.out_path.clone()), self.notes.take())
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
        #[cfg(windows)]
        {
            let out_path = cfg.out_path.clone();
            let notes = Notes::default();
            let backend = match cfg.output {
                RecordOutput::Mp4 { .. } => {
                    Backend::Video(pipeline::Session::start(cfg, notes.clone())?)
                }
                RecordOutput::Gif { .. } => Backend::Gif(gif::GifSession::start(cfg)?),
            };
            Ok(RecorderHandle {
                inner: Some(backend),
                out_path,
                notes,
            })
        }
        #[cfg(not(windows))]
        {
            let _ = cfg;
            Err(RecorderError::NotImplemented)
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
    fn ended_early_names_the_file_and_reason() {
        let e = RecorderError::EndedEarly {
            path: PathBuf::from(r"C:\x\clip.mp4"),
            reason: "the captured window was closed".into(),
        };
        let s = e.to_string();
        assert!(s.contains(r"C:\x\clip.mp4"), "{s}");
        assert!(s.contains("window was closed"), "{s}");
    }

    #[test]
    fn record_output_variants_construct() {
        // Both outputs are representable; starting them needs a display (covered by dev tests).
        let gif = RecordCfg {
            target: CaptureTarget::Monitor(MonitorId(0)),
            output: RecordOutput::Gif {
                fps: 15,
                cursor: true,
                max_width: 800,
                max_seconds: 30,
            },
            out_path: PathBuf::from("out.gif"),
        };
        assert!(matches!(gif.output, RecordOutput::Gif { fps: 15, .. }));
    }
}
