//! The one way the service's tests read what the code logs.
//!
//! Several tests assert that a line was logged (every kill of the daemon names its reason, a
//! failed setup is a warning, a write to no device is a warning). Each of them used to carry its
//! own copy of a buffer and a subscriber; they now share this one, so a fix to how logs are
//! captured is made once (class `cause:timing-test`, #45 and #55).
//!
//! [`captured`] holds a lock while it runs: one capture at a time, so two tests that install a
//! scoped subscriber never build their dispatchers at the same moment.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

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

    /// A subscriber that writes every level, without colour codes, into this sink.
    pub(crate) fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync {
        let buf = self.clone();
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || buf.clone())
            .finish()
    }
}

static CAPTURE: Mutex<()> = Mutex::new(());

fn one_capture_at_a_time() -> MutexGuard<'static, ()> {
    CAPTURE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Runs `run` on this thread under a subscriber that collects its lines; returns what `run`
/// returned and the text it logged.
pub(crate) fn captured<T>(run: impl FnOnce() -> T) -> (T, String) {
    let _turn = one_capture_at_a_time();
    let logs = LogBuf::default();
    let out = tracing::subscriber::with_default(logs.subscriber(), run);
    (out, logs.text())
}
