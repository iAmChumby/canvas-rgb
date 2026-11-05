//! The buffer and subscriber this crate's tests use to read what the adapters log (#81). It is the
//! same shape as the service's `test_logs` and the daemon's: a sink, and a subscriber that writes
//! every level into it without colour codes.
//!
//! Tracing decides whether a call site is worth recording once per process. While exactly one
//! scoped dispatcher is live (a capture), it asks that from the thread that reached the call site
//! first, and with none live it answers "never"; a thread with no subscriber in sight makes it
//! "never", and the line is then dropped for every thread. [`InterestKeeper`] is installed once, as
//! the global default, so that cannot happen (class `cause:timing-test`, #45).

use std::io;
use std::sync::{Arc, Mutex, Once};

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

/// A subscriber that keeps tracing's callsite interest cache permissive, and records nothing.
///
/// tracing-core keeps one `Interest` per callsite for the whole process (`callsite::register` ->
/// `rebuild_callsite_interest`). It ANDs the answer of every live dispatcher; with exactly one
/// scoped dispatcher live it asks the registering thread's default, and with none it is `never`. A
/// thread with no subscriber in sight caches `Interest::never()` for that callsite, and `tracing`
/// then drops every later event on it -- including the one a capture on another thread is waiting
/// for (class `cause:timing-test`, #45). The cache is only recomputed when a dispatcher is
/// created, so a capture that starts after the bad registration gets the line back, and one that
/// runs across it does not.
///
/// This subscriber is installed once, as the global default, so the cache can never hold `never`:
/// it answers `Interest::sometimes()` for every callsite and `enabled()` is false, so it records
/// nothing and builds no event. A capture's own thread-local subscriber still decides what a
/// capture collects, and lines from other threads still go nowhere.
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
        tracing::span::Id::from_u64(1)
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
/// A global subscriber can be set once, and nothing else in this test binary sets one. If some
/// other subscriber got there first, the keeper is skipped: that subscriber's own interest decides
/// the cache from then on.
fn keep_the_interest_cache_permissive() {
    KEEPER.call_once(|| {
        let _ = tracing::subscriber::set_global_default(InterestKeeper);
    });
}

impl LogBuf {
    /// Everything logged into this sink so far.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    }

    /// Installs a subscriber for this thread that writes every level into this sink. The events
    /// of a `#[tokio::test]` (one thread) land here until the guard drops.
    pub(crate) fn install(&self) -> tracing::subscriber::DefaultGuard {
        // The keeper first: the capture's own subscriber decides what the capture collects, and
        // the keeper keeps the interest cache from answering `never` for a callsite another thread
        // reached (class `cause:timing-test`, #45).
        keep_the_interest_cache_permissive();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A line only this test logs, so that its callsite is reached for the first time inside it.
    fn log_a_line_only_this_test_logs() {
        tracing::info!("a line only this test logs");
    }

    /// A capture must see a line whose callsite was first reached on another thread.
    ///
    /// tracing-core keeps one `Interest` per callsite for the whole process; with exactly one
    /// scoped dispatcher live it computes it from the thread that reached the callsite first. A
    /// thread with no subscriber in sight caches `Interest::never()` for that callsite, and
    /// tracing then drops every later event on it -- including the one a capture on another thread
    /// is waiting for (class `cause:timing-test`, #45).
    #[test]
    fn a_capture_sees_a_line_whose_callsite_was_first_reached_on_another_thread() {
        let logs = LogBuf::default();
        let _guard = logs.install();
        // Another thread reaches the callsite first, with no subscriber in sight.
        std::thread::spawn(log_a_line_only_this_test_logs)
            .join()
            .unwrap();
        // The capture's own thread reaches it afterwards.
        log_a_line_only_this_test_logs();
        let text = logs.text();
        assert!(
            text.contains("a line only this test logs"),
            "the capture lost the line: {text}"
        );
    }
}
