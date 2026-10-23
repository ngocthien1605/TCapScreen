//! The constant-frame-rate clock shared by the MP4 and GIF recorders: how many frames should
//! exist after a given amount of *unpaused* wall-clock time, and the exact timestamp of each.
//!
//! Output length tracks real time: when capture or encoding falls behind, the recorders repeat
//! the latest frame to fill the frames that are due rather than playing back too fast.

use std::time::{Duration, Instant};

/// 100-ns units per second (Media Foundation's time base).
pub(crate) const HNS_PER_SEC: i64 = 10_000_000;

/// Unpaused-time clock for a recording at `fps`.
#[derive(Debug, Clone)]
pub(crate) struct FrameClock {
    fps: u32,
    start: Instant,
    /// Total length of finished pauses.
    paused_total: Duration,
    /// When the current pause began, if paused.
    paused_since: Option<Instant>,
}

impl FrameClock {
    pub(crate) fn new(fps: u32, start: Instant) -> Self {
        FrameClock {
            fps: fps.max(1),
            start,
            paused_total: Duration::ZERO,
            paused_since: None,
        }
    }

    /// Record the pause state at `now`; pauses are accumulated at their edges.
    pub(crate) fn set_paused(&mut self, now: Instant, paused: bool) {
        match (paused, self.paused_since) {
            (true, None) => self.paused_since = Some(now),
            (false, Some(since)) => {
                self.paused_total += now.saturating_duration_since(since);
                self.paused_since = None;
            }
            _ => {}
        }
    }

    /// Recording time at `now`, excluding every pause (including one in progress).
    pub(crate) fn active(&self, now: Instant) -> Duration {
        let current_pause = self
            .paused_since
            .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
        now.saturating_duration_since(self.start)
            .saturating_sub(self.paused_total)
            .saturating_sub(current_pause)
    }

    /// Frames that should exist by `now`: frame `n` covers `[n/fps, (n+1)/fps)`, so one frame is
    /// due at t = 0.
    pub(crate) fn frames_due(&self, now: Instant) -> u64 {
        (self.active(now).as_nanos() * self.fps as u128 / 1_000_000_000) as u64 + 1
    }
}

/// Start time of frame `index` at `fps`, in 100-ns units — computed from the index rather than
/// by adding a rounded per-frame duration, so timestamps never drift (at 30 fps a rounded
/// 333 333 would lose ~36 ms an hour against the audio track).
pub(crate) fn frame_time_100ns(index: i64, fps: u32) -> i64 {
    index * HNS_PER_SEC / fps.max(1) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_frame_is_due_at_the_start_and_fps_per_second_after() {
        let t0 = Instant::now();
        let c = FrameClock::new(30, t0);
        assert_eq!(c.frames_due(t0), 1);
        assert_eq!(c.frames_due(t0 + Duration::from_millis(999)), 30);
        assert_eq!(c.frames_due(t0 + Duration::from_secs(1)), 31);
        assert_eq!(c.frames_due(t0 + Duration::from_secs(60)), 1801);
    }

    #[test]
    fn pauses_are_excluded_including_one_in_progress() {
        let t0 = Instant::now();
        let mut c = FrameClock::new(10, t0);
        c.set_paused(t0 + Duration::from_secs(2), true);
        assert_eq!(
            c.active(t0 + Duration::from_secs(5)),
            Duration::from_secs(2)
        );
        c.set_paused(t0 + Duration::from_secs(5), false);
        assert_eq!(
            c.active(t0 + Duration::from_secs(6)),
            Duration::from_secs(3)
        );
        // Repeated edges are idempotent.
        c.set_paused(t0 + Duration::from_secs(6), false);
        assert_eq!(c.frames_due(t0 + Duration::from_secs(6)), 31);
    }

    #[test]
    fn frame_times_are_exact_over_long_recordings() {
        // One hour at 30 fps lands exactly on 3600 s; a rounded 333_333-unit step would not.
        assert_eq!(frame_time_100ns(30 * 3600, 30), 3600 * HNS_PER_SEC);
        assert_eq!(frame_time_100ns(1, 30), 333_333);
        assert_eq!(frame_time_100ns(2, 30), 666_666);
        assert_eq!(frame_time_100ns(3, 30), 1_000_000);
    }
}
