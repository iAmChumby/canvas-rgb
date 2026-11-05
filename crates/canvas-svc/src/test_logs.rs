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
//!
//! The lock is not what keeps a capture whole. Tracing decides whether a call site is worth
//! recording once per process, and it decides it from the thread that reached the call site
//! first; a thread with no subscriber in sight makes it "never", and the line is then dropped for
//! every thread. [`InterestKeeper`] is installed once, as the global default, so that cannot
//! happen (class `cause:timing-test`, #45 and #55).

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, Once};

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

/// A subscriber that keeps tracing's callsite interest cache permissive, and records nothing.
///
/// tracing-core keeps one `Interest` per callsite for the whole process, and computes it from the
/// dispatcher of the thread that reached the callsite first (`callsite::register` ->
/// `rebuild_callsite_interest`). A thread with no subscriber in sight caches
/// `Interest::never()` for that callsite, and `tracing` then drops every later event on it --
/// including the one a capture on another thread is waiting for (#45, #55). The cache is only
/// recomputed when a dispatcher is created, so a capture that starts after the bad registration
/// gets the line back, and one that runs across it does not.
///
/// This subscriber is installed once, as the global default, so the cache can never hold
/// `never`: it answers `Interest::sometimes()` for every callsite and `enabled()` is false, so it
/// records nothing and builds no event. A capture's own thread-local subscriber still decides what
/// a capture collects, and lines from other threads still go nowhere.
struct InterestKeeper;

impl tracing::Subscriber for InterestKeeper {
    fn register_callsite(
        &self,
        _metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        // `Subscriber`'s default would ask `enabled()`, which is false, and answer `never`; the
        // cache would then hold `never` whenever no capture is running. `sometimes` keeps it
        // permissive for good.
        tracing::subscriber::Interest::sometimes()
    }
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        false
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(0)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, _event: &tracing::Event<'_>) {}
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

static KEEPER: Once = Once::new();

/// Installs the keeper, once per process.
///
/// A global subscriber can be set once, and nothing else in this test binary sets one (the
/// service's own `logging::init` runs in the binary, not in the library's tests). If some other
/// subscriber got there first, the keeper is skipped: that subscriber's own interest decides the
/// cache from then on.
fn keep_the_interest_cache_permissive() {
    KEEPER.call_once(|| {
        let _ = tracing::subscriber::set_global_default(InterestKeeper);
    });
}

/// Runs `run` on this thread under a subscriber that collects its lines; returns what `run`
/// returned and the text it logged.
pub(crate) fn captured<T>(run: impl FnOnce() -> T) -> (T, String) {
    let _turn = one_capture_at_a_time();
    keep_the_interest_cache_permissive();
    let logs = LogBuf::default();
    let out = tracing::subscriber::with_default(logs.subscriber(), || {
        // No `rebuild_interest_cache()` here: tracing-core already rebuilds callsite
        // interest whenever a dispatcher is created (`Dispatch::new` ->
        // `callsite::register_dispatch`), and the keeper keeps that cache permissive for a
        // callsite that is reached for the first time while a capture runs.
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
    keep_the_interest_cache_permissive();
    let logs = LogBuf::default();
    let _default = tracing::subscriber::set_default(logs.subscriber());
    let out = run.await;
    drop(_default);
    (out, logs.text())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line only this test logs, so that its callsite is reached for the first time inside it.
    fn log_a_line_only_this_test_logs() {
        tracing::info!("a line only this test logs");
    }

    /// A capture must see a line whose callsite was first reached on another thread.
    ///
    /// tracing-core keeps one `Interest` per callsite for the whole process and computes it from
    /// the dispatcher of the thread that reached the callsite first. A thread with no subscriber
    /// in sight caches `Interest::never()` for that callsite, and tracing then drops every later
    /// event on it -- including the one a capture on another thread is waiting for (#45, #55).
    #[test]
    fn a_capture_sees_a_line_whose_callsite_was_first_reached_on_another_thread() {
        let (_, text) = captured(|| {
            // Another thread reaches the callsite first, with no subscriber in sight.
            std::thread::spawn(log_a_line_only_this_test_logs)
                .join()
                .unwrap();
            // The capture's own thread reaches it afterwards.
            log_a_line_only_this_test_logs();
        });
        assert!(
            text.contains("a line only this test logs"),
            "the capture lost the line: {text}"
        );
    }
}
