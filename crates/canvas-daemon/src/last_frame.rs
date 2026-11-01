//! When the last frame reached a device (#81).
//!
//! A RAM stick that goes dark after an update raises one question the logs could not answer: were
//! frames still streaming to it when the service ended OpenRGB? The service logs the moment it
//! ends the daemon and OpenRGB; the daemon logs, with this, the moment its own last frame went out.
//! The two logs then line up. Nothing is logged per frame: the writer tasks only store a time here,
//! and the quit path reads it once.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The wall-clock time of the last frame a writer task handed to the adapter without an error.
#[derive(Default)]
pub struct LastFrame {
    /// Milliseconds since the Unix epoch; 0 until the first frame.
    unix_ms: AtomicU64,
    /// Frames sent since the daemon started.
    frames: AtomicU64,
}

fn now_unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl LastFrame {
    /// A frame went out just now.
    pub fn record(&self) {
        self.unix_ms.store(now_unix_ms(), Ordering::Relaxed);
        self.frames.fetch_add(1, Ordering::Relaxed);
    }

    /// Logs the time of the last frame, with `when` saying what asked (the daemon was told to
    /// stop, or it finished quitting). Written as UTC like the log's own timestamps, and as the
    /// gap to this line, so the service log and this one can be read side by side.
    pub fn log(&self, when: &str) {
        self.log_at(when, now_unix_ms());
    }

    fn log_at(&self, when: &str, now_ms: u64) {
        let at_ms = self.unix_ms.load(Ordering::Relaxed);
        let frames = self.frames.load(Ordering::Relaxed);
        if at_ms == 0 {
            tracing::info!(frames, "no frame was sent to any device since the daemon started ({when})");
            return;
        }
        let ago_ms = now_ms.saturating_sub(at_ms);
        let at = rfc3339_utc(at_ms);
        tracing::info!(
            last_frame_at = %at, ago_ms, frames,
            "last frame sent to a device at {at}, {ago_ms} ms before this line ({when}); {frames} frames since the daemon started"
        );
    }
}

/// `2023-11-14T22:13:20.000Z` for a Unix time in milliseconds.
fn rfc3339_utc(unix_ms: u64) -> String {
    let secs = unix_ms / 1000;
    let ms = unix_ms % 1000;
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unix_time_is_written_as_utc_to_the_millisecond() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_utc(1_700_000_000_000), "2023-11-14T22:13:20.000Z");
        // The incident's stop, 2026-11-01 15:44:42.710 UTC.
        assert_eq!(rfc3339_utc(1_793_547_882_710), "2026-11-01T15:44:42.710Z");
        // A leap day.
        assert_eq!(rfc3339_utc(1_709_210_096_789), "2024-02-29T12:34:56.789Z");
    }

    #[test]
    fn the_last_frame_time_is_logged_once_with_its_gap_and_nothing_per_frame() {
        let last = LastFrame::default();
        let (_, before) = crate::test_logs::captured(|| {
            for _ in 0..50 {
                last.record();
            }
        });
        assert!(before.is_empty(), "frames log nothing: {before}");

        let at_ms = last.unix_ms.load(Ordering::Relaxed);
        let (_, text) = crate::test_logs::captured(|| last.log_at("the daemon was told to quit", at_ms + 250));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1, "{text}");
        assert!(lines[0].contains("INFO"), "{text}");
        assert!(lines[0].contains(&format!("last frame sent to a device at {}", rfc3339_utc(at_ms))), "{text}");
        assert!(lines[0].contains("250 ms before this line"), "{text}");
        assert!(lines[0].contains("the daemon was told to quit") && lines[0].contains("50 frames"), "{text}");
    }

    #[test]
    fn with_no_frame_yet_the_line_says_so_instead_of_inventing_a_time() {
        let last = LastFrame::default();
        let (_, text) = crate::test_logs::captured(|| last.log("the daemon finished quitting"));
        assert!(text.contains("no frame was sent to any device"), "{text}");
        assert!(!text.contains("1970"), "{text}");
    }
}
