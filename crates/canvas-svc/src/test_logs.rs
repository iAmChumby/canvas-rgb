//! The shared buffer and subscriber the service's tests use to read what the code logs.
//!
//! Several tests assert that a line was logged (every kill of the daemon names its reason, a
//! failed setup is a warning, a write to no device is a warning). Each of them used to carry its
//! own copy of a buffer and a subscriber; they now share [`LogBuf`] (class `cause:timing-test`,
//! #45 and #55).
//!
//! [`captured`] holds a lock while it runs: one capture at a time among the tests that go through
//! it. Every service test capture goes through [`captured`] (or [`captured_async`] for the
//! async ones), so every capture takes that one lock.

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
    let out = tracing::subscriber::with_default(logs.subscriber(), || {
        // Per-thread `with_default` plus the global callsite interest cache can drop a
        // line (#45 settler): a callsite already judged off by an earlier subscriber
        // stays off. Rebuild the cache while this subscriber is installed, so every
        // callsite is judged against this subscriber's levels.
        tracing::callsite::rebuild_interest_cache();
        run()
    });
    (out, logs.text())
}

/// The async twin of [`captured`]: runs the future under the same lock and subscriber, via a
/// `set_default` guard held across `.await` points, so the subscriber stays installed
/// wherever the task wakes. `MutexGuard` is `!Send`, so the future is `!Send`; the
/// `#[tokio::test]` runtime is current-thread and runs it on one thread.
#[allow(clippy::await_holding_lock)]
pub(crate) async fn captured_async<F, T>(run: F) -> (T, String)
where
    F: std::future::Future<Output = T>,
{
    // Held across `.await` on purpose: the lock must serialize every capture for its
    // whole duration, including when a task sleeps inside the run.
    let _turn = one_capture_at_a_time();
    let logs = LogBuf::default();
    let _default = tracing::subscriber::set_default(logs.subscriber());
    // Same rebuild as `captured` (#45): rebuild while this subscriber is installed.
    tracing::callsite::rebuild_interest_cache();
    let out = run.await;
    drop(_default);
    (out, logs.text())
}
