//! The buffer and subscriber this crate's tests use to read what the adapters log (#81). It is the
//! same shape as the service's `test_logs` and the daemon's: a sink, and a subscriber that writes
//! every level into it without colour codes.

use std::io;
use std::sync::{Arc, Mutex};

/// A log sink the tests read back.
#[derive(Clone, Default)]
pub(crate) struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl LogBuf {
    /// Everything logged into this sink so far.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    }

    /// Installs a subscriber for this thread that writes every level into this sink. The events
    /// of a `#[tokio::test]` (one thread) land here until the guard drops.
    pub(crate) fn install(&self) -> tracing::subscriber::DefaultGuard {
        let sink = self.clone();
        tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .with_writer(move || sink.clone())
                .finish(),
        )
    }

    /// The lines that contain every one of `needles`.
    pub(crate) fn lines_with(&self, needles: &[&str]) -> Vec<String> {
        self.text()
            .lines()
            .filter(|line| needles.iter().all(|needle| line.contains(needle)))
            .map(str::to_owned)
            .collect()
    }
}
