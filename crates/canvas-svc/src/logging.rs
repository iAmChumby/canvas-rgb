//! Service logging (G8): daily rolling files under `%ProgramData%\canvas-rgb\logs`, keeping
//! `LOG_KEEP_DAYS` files, and at most `LOG_MAX_BYTES_PER_DAY` per day. Past the cap the day's
//! output stops after one "log cap reached" line, and writing resumes the next day.

use canvas_proto::consts::{LOG_KEEP_DAYS, LOG_MAX_BYTES_PER_DAY};
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

/// The single line written when a day's cap is reached.
pub const CAP_NOTE: &str = "log cap reached for today; further lines are dropped until the next day\n";

/// The day number (UTC) used to decide when a new day's allowance starts.
pub fn utc_day() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

/// Wraps a writer with a per-day byte cap. A line that would take the day over the cap is
/// dropped, and the cap note is written once in its place. Output resumes when the day key changes.
pub struct CappedWriter<W: Write> {
    inner: W,
    cap: u64,
    clock: Box<dyn Fn() -> u64 + Send>,
    day: u64,
    written: u64,
    capped: bool,
}

impl<W: Write> CappedWriter<W> {
    pub fn new(inner: W, cap: u64, clock: Box<dyn Fn() -> u64 + Send>) -> Self {
        let day = clock();
        Self { inner, cap, clock, day, written: 0, capped: false }
    }

    /// Whether today's allowance is used up.
    pub fn is_capped(&self) -> bool {
        self.capped
    }
}

impl<W: Write> Write for CappedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let today = (self.clock)();
        if today != self.day {
            self.day = today;
            self.written = 0;
            self.capped = false;
        }
        if self.capped {
            return Ok(buf.len());
        }
        if self.written + buf.len() as u64 > self.cap {
            self.capped = true;
            self.inner.write_all(CAP_NOTE.as_bytes())?;
            return Ok(buf.len());
        }
        self.inner.write_all(buf)?;
        self.written += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Installs the global subscriber. Returns the guard that flushes the file writer on drop.
/// Console mode writes to stderr and creates no directory.
pub fn init(logs: &Path, console: bool) -> io::Result<Option<WorkerGuard>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if console {
        let _ = tracing_subscriber::fmt()
            .with_writer(io::stderr)
            .with_ansi(false)
            .with_env_filter(filter)
            .try_init();
        return Ok(None);
    }
    std::fs::create_dir_all(logs)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("canvas-rgb-svc")
        .filename_suffix("log")
        .max_log_files(LOG_KEEP_DAYS as usize)
        .build(logs)
        .map_err(io::Error::other)?;
    let capped = CappedWriter::new(appender, LOG_MAX_BYTES_PER_DAY, Box::new(utc_day));
    let (writer, guard) = tracing_appender::non_blocking(capped);
    let _ = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init();
    Ok(Some(guard))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    #[test]
    fn the_cap_stops_a_day_after_one_note_and_resumes_the_next_day() {
        let day = Arc::new(AtomicU64::new(1));
        let clock = Arc::clone(&day);
        let mut w = CappedWriter::new(Vec::<u8>::new(), 100, Box::new(move || clock.load(Ordering::SeqCst)));

        w.write_all(&[b'a'; 60]).unwrap();
        w.write_all(&[b'b'; 30]).unwrap();
        assert_eq!(w.inner.len(), 90, "within the cap, everything is written");
        assert!(!w.is_capped());

        // 20 more would take the day to 110 bytes: the line is dropped and the note written.
        w.write_all(&[b'c'; 20]).unwrap();
        assert!(w.is_capped());
        assert!(w.inner.ends_with(CAP_NOTE.as_bytes()));
        let after_note = w.inner.len();

        // Later lines the same day are dropped silently: no second note, no data.
        w.write_all(&[b'd'; 5]).unwrap();
        w.write_all(b"more\n").unwrap();
        assert_eq!(w.inner.len(), after_note);
        let notes = String::from_utf8_lossy(&w.inner).matches("log cap reached").count();
        assert_eq!(notes, 1, "exactly one cap line for the day");

        // Next day: the allowance is back.
        day.store(2, Ordering::SeqCst);
        w.write_all(b"new day\n").unwrap();
        assert!(!w.is_capped());
        assert!(w.inner.ends_with(b"new day\n"));
    }

    #[test]
    fn a_line_that_fits_is_not_dropped_at_exactly_the_cap() {
        let mut w = CappedWriter::new(Vec::<u8>::new(), 10, Box::new(|| 1));
        w.write_all(&[b'x'; 10]).unwrap();
        assert_eq!(w.inner.len(), 10);
        assert!(!w.is_capped());
    }
}
