//! The fixed-step frame timer.
//!
//! The loop sleeps until the next tick instead of spinning. On Windows the system
//! timer default (15.6 ms) is coarser than the 16.7 ms of a 60 fps frame, so the
//! timer resolution is raised with `timeBeginPeriod(1)` while the loop runs and
//! released again when it parks or pauses.
//!
//! `timeBeginPeriod` is only meaningful on Windows; on every other platform this is
//! a plain sleep-based timer.

use std::time::{Duration, Instant};

/// Raised timer resolution, in milliseconds. 1 ms is the finest Windows offers.
const TIMER_RESOLUTION_MS: u16 = 1;

/// A fixed-step frame timer that sleeps between frames.
///
/// The tick times are absolute: the frame at index `n` is due at
/// `start + n * interval`. Drift therefore does not accumulate, and a frame that
/// overruns its budget does not push the whole schedule back.
pub struct FrameTimer {
    /// The start of the schedule.
    epoch: Instant,
    /// The tick index of the next frame.
    next_tick: u64,
    /// Whether the high-resolution timer is currently held.
    resolution_held: bool,
}

impl FrameTimer {
    /// A timer whose schedule starts now.
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
            // The first tick is due one interval after the epoch, so the caller
            // always waits before the first frame.
            next_tick: 1,
            resolution_held: false,
        }
    }

    /// Asks the OS for a finer timer resolution. Idempotent.
    ///
    /// Only called while the loop is actually running.
    pub fn acquire_resolution(&mut self) {
        if self.resolution_held {
            return;
        }
        self.resolution_held = true;
        platform::begin_period(TIMER_RESOLUTION_MS);
    }

    /// Gives the OS timer resolution back. Idempotent.
    ///
    /// Called when the loop parks or pauses, so an idle canvas-rgb does not keep the
    /// whole system's timer at 1 ms.
    pub fn release_resolution(&mut self) {
        if !self.resolution_held {
            return;
        }
        self.resolution_held = false;
        platform::end_period(TIMER_RESOLUTION_MS);
    }

    /// The interval between frames at `fps`.
    pub fn interval(fps: u32) -> Duration {
        let fps = fps.clamp(1, 1000);
        Duration::from_nanos(1_000_000_000 / u64::from(fps))
    }

    /// Sleeps until the next tick at `fps`, then advances the schedule.
    ///
    /// Returns the instant the tick was due, which is the frame's logical time.
    ///
    /// The first tick is due one interval after the epoch, so the caller always
    /// waits before the first frame.
    pub fn sleep_until_next(&mut self, fps: u32) -> Instant {
        let interval = Self::interval(fps);
        // `Duration` scales by a u32; a tick count that large is decades away, so
        // saturating is safe.
        let ticks = u32::try_from(self.next_tick).unwrap_or(u32::MAX);
        let due = self.epoch + interval.saturating_mul(ticks);
        self.next_tick = self.next_tick.saturating_add(1);

        let now = Instant::now();
        if due > now {
            std::thread::sleep(due - now);
        }
        due
    }

    /// Restarts the schedule at `now`, so the next frame is due one interval from now.
    ///
    /// Used after a resume from pause or park: the effect's `dt` should be the real
    /// interval, and the first frame after a resume should not have to catch up on all
    /// the ticks that were missed while parked.
    pub fn reset_schedule(&mut self, now: Instant) {
        self.epoch = now;
        self.next_tick = 1;
    }
}

impl Default for FrameTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FrameTimer {
    fn drop(&mut self) {
        self.release_resolution();
    }
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};

    /// Raises the system timer resolution to `ms`.
    pub fn begin_period(ms: u16) {
        // SAFETY: `timeBeginPeriod` is a plain Win32 call with no preconditions.
        unsafe {
            timeBeginPeriod(u32::from(ms));
        }
    }

    /// Lowers the system timer resolution back.
    pub fn end_period(ms: u16) {
        // SAFETY: `timeEndPeriod` is a plain Win32 call with no preconditions.
        unsafe {
            timeEndPeriod(u32::from(ms));
        }
    }
}

#[cfg(not(windows))]
mod platform {
    /// No-op on platforms without a system-wide timer resolution.
    pub fn begin_period(_ms: u16) {}

    /// No-op on platforms without a system-wide timer resolution.
    pub fn end_period(_ms: u16) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_matches_the_fps() {
        assert_eq!(FrameTimer::interval(60), Duration::from_nanos(16_666_666));
        assert_eq!(FrameTimer::interval(10), Duration::from_millis(100));
        assert_eq!(FrameTimer::interval(30), Duration::from_nanos(33_333_333));
    }

    #[test]
    fn a_zero_fps_does_not_divide_by_zero() {
        assert!(FrameTimer::interval(0) > Duration::ZERO);
    }

    #[test]
    fn sleeping_takes_at_least_the_interval() {
        let mut timer = FrameTimer::new();
        let start = Instant::now();
        timer.sleep_until_next(20);
        let elapsed = start.elapsed();
        // 50 ms interval; allow a little slack for the sleep to overshoot.
        assert!(elapsed >= Duration::from_millis(45), "elapsed {elapsed:?}");
    }

    #[test]
    fn the_schedule_does_not_drift() {
        let mut timer = FrameTimer::new();
        let start = Instant::now();
        // Three frames at 100 fps: 30 ms of scheduled time.
        for _ in 0..3 {
            timer.sleep_until_next(100);
        }
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(25), "elapsed {elapsed:?}");
        assert!(elapsed < Duration::from_millis(120), "elapsed {elapsed:?}");
    }

    #[test]
    fn reset_moves_the_next_tick_to_now() {
        let mut timer = FrameTimer::new();
        timer.sleep_until_next(10);
        timer.reset_schedule(Instant::now());
        let start = Instant::now();
        timer.sleep_until_next(60);
        // The next frame is due one 60 fps interval from the reset, not ten frames later.
        assert!(start.elapsed() < Duration::from_millis(60));
    }
}
