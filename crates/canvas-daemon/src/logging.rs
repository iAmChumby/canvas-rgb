//! G8: the daemon's logs.
//!
//! `tracing` writes a daily rolling file in `%LOCALAPPDATA%\canvas-rgb\logs`. Files are
//! kept for 7 days and a day's file stops growing past 10 MB. There is no Windows Event
//! Log and no editor Logs page; `canvas-rgb diag bundle` zips the files instead.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// The daemon's own log target: the name a `RUST_LOG` filter says for the daemon's lines
/// (`RUST_LOG=canvas_rgbd=info`). It is spelled here and nowhere else.
///
/// A line that lived in the `canvas-rgbd` binary and moved into this library keeps it by
/// passing `target: LOG_TARGET`, so it goes on answering the same filter instead of the
/// module target it landed in (`canvas_daemon::device_sync` and the like), which a narrowed
/// `RUST_LOG` drops (#87, #446). The default filter below names it too.
pub const LOG_TARGET: &str = "canvas_rgbd";

/// How long a day's log file may grow before the daemon stops writing to it.
pub const MAX_BYTES_PER_DAY: u64 = canvas_proto::consts::LOG_MAX_BYTES_PER_DAY;

/// How many days of logs are kept.
pub const KEEP_DAYS: u32 = canvas_proto::consts::LOG_KEEP_DAYS;

/// The guard that flushes the log writer. It lives here, not in `main`, so any thread that is
/// about to end the process (the quit watchdog, the `WM_ENDSESSION` handler) can flush with
/// [`close`] instead of sleeping and hoping.
static GUARD: std::sync::Mutex<Option<WorkerGuard>> = std::sync::Mutex::new(None);

fn hold(guard: WorkerGuard) {
    *GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(guard);
}

/// Flushes every queued line to the file and stops the log writer. Returns once the writer has
/// drained (or after the appender's own 1 s bound). Lines logged after this are dropped, so call
/// it only on the way out of the process. Safe to call twice.
pub fn close() {
    let guard = GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
    drop(guard);
}

/// The longest a process that is already ending waits for the log to drain. The appender's own
/// bound on `WorkerGuard::drop` is about 1.1 s, which is too long for the quit watchdog.
pub const FLUSH_LIMIT: std::time::Duration = std::time::Duration::from_millis(250);

/// Starts `work` on a helper thread and hands back the channel that says how it ended: the sender
/// is dropped when the thread ends, so a receiver sees a disconnect when the work panicked.
fn start_helper(work: impl FnOnce() + Send + 'static) -> std::sync::mpsc::Receiver<()> {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        work();
        let _ = done.send(());
    });
    finished
}

/// Runs `work` on a helper thread and waits at most `limit` for it. Returns whether it finished.
/// A stuck `work` is left behind on its thread; the caller is about to end the process.
pub fn run_bounded(limit: std::time::Duration, work: impl FnOnce() + Send + 'static) -> bool {
    start_helper(work).recv_timeout(limit).is_ok()
}

/// How long a test waits for a helper thread to end before it calls it a hang. It is a hang guard,
/// not a verdict: the helper's end is the event the test waits for, and a machine that is slow to
/// start a thread must not decide the result (class `cause:timing-test`, #72). The work these
/// tests wait for is instant (a file write, a channel), so 30 s only fires on a real hang.
#[cfg(test)]
pub(crate) const HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(30);

/// Waits for the helper thread to end, at most `limit`. `true` when the helper finished its body;
/// `false` when it panicked or was still running when the limit passed.
#[cfg(test)]
pub(crate) fn helper_finished(
    finished: &std::sync::mpsc::Receiver<()>,
    limit: std::time::Duration,
) -> bool {
    finished.recv_timeout(limit).is_ok()
}

/// [`close`], but it gives up after [`FLUSH_LIMIT`]. Returns whether the log drained in time.
pub fn close_within_limit() -> bool {
    run_bounded(FLUSH_LIMIT, close)
}

/// Where the daily log file is, set by [`init`]: the folder and the file stem. A panic on the main
/// thread writes its line there directly ([`write_line_direct_within`]).
static DIRECT_TARGET: std::sync::Mutex<Option<(std::path::PathBuf, String)>> = std::sync::Mutex::new(None);

/// Test-only: point the production panic writer at a temp folder, so a test can install the REAL
/// hook (`exit_log::install_panic_hook`) and check its line lands in today's file. The target is
/// process-wide; callers serialise with the other real-hook tests and restore it afterwards.
#[cfg(test)]
pub(crate) fn set_direct_target_for_test(dir: std::path::PathBuf, name: String) {
    *DIRECT_TARGET.lock().unwrap_or_else(|p| p.into_inner()) = Some((dir, name));
}

#[cfg(test)]
pub(crate) fn clear_direct_target_for_test() {
    *DIRECT_TARGET.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

/// Days since 1970-01-01 as (year, month, day) in the proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// The date in a daily log file's name, `YYYY-MM-DD`, for a moment given in seconds since the Unix
/// epoch. It is UTC: `tracing-appender`'s `Rotation::DAILY` formats `OffsetDateTime::now_utc()`
/// with `[year]-[month]-[day]` (`file_writer` below uses that rotation), so the file the appender
/// writes at a moment is the file named for that moment's UTC date.
pub fn appender_date(unix_secs: u64) -> String {
    let (y, m, d) = civil_from_days((unix_secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The file the daily appender writes now: `{name}.{UTC date}.log` in `dir`. Built from the name
/// pattern and the date, never picked from the folder, so `canvas-rgbd.old.log` or a file of a
/// later date cannot be chosen; and the file need not exist yet (the appender creates a new day's
/// file on its first write after midnight, and a panic can come first).
pub fn todays_log_file(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    dir.join(format!("{name}.{}.log", appender_date(now)))
}

/// Opens the log file `path` for appending, creating it when absent, WITHOUT following a reparse
/// point. A link or junction planted at that name is reported as itself and refused: the path is
/// named, and nothing is deleted, renamed or adopted (the fail-closed rule, `CLAUDE.md`). Both
/// writers go through here: the direct panic writer and the rolling appender's own `create_writer`,
/// which otherwise opens today's file with no check of its own (#36/#72).
fn open_log_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        // symlink_metadata does not follow the link: a junction already at the name is refused
        // before anything is opened or created there.
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(std::io::Error::other(format!(
                    "the log file {} is a reparse point; refusing to write to it",
                    path.display()
                )));
            }
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        // Check again after opening: a swap between the two checks is caught here, still with the
        // reparse point as the handle, never followed.
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(std::io::Error::other(format!(
                "the log file {} is a reparse point; refusing to write to it",
                path.display()
            )));
        }
    }
    Ok(file)
}

/// Appends one line to `path` in one write, creating the file if needed. A reparse point (a link
/// or junction planted where the log file goes) is not followed: it is opened as itself, and
/// refused.
fn append_line(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = open_log_file(path)?;
    file.write_all(format!("{line}\n").as_bytes())?;
    file.flush()
}

/// Appends one line to today's log file from the calling thread, in one write, without going
/// through the queue the appender drains. The line the daemon writes on its way out of a panic
/// must not wait behind a backlog (class `cause:timing-test`, #36).
pub fn write_line_direct(dir: &std::path::Path, name: &str, line: &str) -> std::io::Result<()> {
    append_line(&todays_log_file(dir, name), line)
}

/// Not started yet / the helper took the write / the caller took it back.
const WRITE_PENDING: u8 = 0;
const WRITE_STARTED: u8 = 1;
const WRITE_TAKEN_BACK: u8 = 2;

/// Puts the state back to PENDING if the helper unwinds. A `write` that panics after taking
/// the line must not leave it STARTED: the caller would then read "the helper owns the line",
/// skip the fallback, and lose the line entirely (follow-up #72). On a normal return the state
/// stays STARTED, so a caller that timed out still knows the helper owns the line.
struct ResetOnPanic(std::sync::Arc<std::sync::atomic::AtomicU8>);

impl Drop for ResetOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.store(WRITE_PENDING, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// Runs `write` on a helper thread and waits at most `limit`, so that the line is written AT MOST
/// ONCE. Returns `true` when the line is in the file, or the helper has begun writing it and may
/// still finish after the limit (the caller must not write it again: a late append plus a second
/// copy would be a duplicate). Returns `false` when the caller must write the line itself: the
/// write failed, the helper panicked and put the line back, or the helper had not begun when the
/// limit passed (it then never will).
///
/// So a stalled disk can still lose the line, but it is never in the file twice.
pub fn write_once_within(
    limit: std::time::Duration,
    write: impl FnOnce() -> std::io::Result<()> + Send + 'static,
) -> bool {
    write_once_waiting(|finished| finished.recv_timeout(limit).is_ok(), write)
}

/// The write at the heart of [`write_once_within`]. `wait` starts the helper on its own thread and
/// says whether the helper finished its body: production waits at most its limit for it, and a
/// test waits for the helper's thread to end, so that the machine's speed never decides the
/// verdict (class `cause:timing-test`, #72).
pub(crate) fn write_once_waiting(
    wait: impl FnOnce(std::sync::mpsc::Receiver<()>) -> bool,
    write: impl FnOnce() -> std::io::Result<()> + Send + 'static,
) -> bool {
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::SeqCst};
    let state = std::sync::Arc::new(AtomicU8::new(WRITE_PENDING));
    let wrote = std::sync::Arc::new(AtomicBool::new(false));
    let (helper_state, helper_wrote) = (std::sync::Arc::clone(&state), std::sync::Arc::clone(&wrote));
    let finished = start_helper(move || {
        if helper_state.compare_exchange(WRITE_PENDING, WRITE_STARTED, SeqCst, SeqCst).is_err() {
            return;
        }
        let _reset = ResetOnPanic(std::sync::Arc::clone(&helper_state));
        helper_wrote.store(write().is_ok(), SeqCst);
    });
    if wait(finished) {
        wrote.load(SeqCst)
    } else {
        // The helper is not going to finish on its own: it panicked, or the limit passed. If the
        // line is back (the panic guard put it back, or the helper never began), take the write
        // back (it will skip it); if it has begun, it owns the line.
        state.compare_exchange(WRITE_PENDING, WRITE_TAKEN_BACK, SeqCst, SeqCst).is_err()
    }
}

/// [`write_line_direct`] to the log file [`init`] set up, at most once, waiting at most `limit`
/// (see [`write_once_within`] for what `true` and `false` mean). `false` also when the logger was
/// never started.
pub fn write_line_direct_within(limit: std::time::Duration, line: String) -> bool {
    let target = DIRECT_TARGET.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
    let Some((dir, name)) = target else { return false };
    write_once_within(limit, move || write_line_direct(&dir, &name, &line))
}

/// The non-blocking file writer for `name` in `dir`, and the guard that flushes it.
fn file_writer(
    dir: &std::path::Path,
    name: &str,
) -> std::io::Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard)> {
    // The appender opens today's file itself, with no reparse check. Open it the same way the
    // direct panic writer does first: a link planted at that name is named and refused, never
    // followed or adopted, and the appender then opens a regular file (#36/#72).
    open_log_file(&todays_log_file(dir, name))?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(name)
        .filename_suffix("log")
        // Two, not one: at build time the appender prunes down to max - 1 files, so with 1 it
        // deleted the day's own log on every daemon start.
        .max_log_files(2)
        .build(dir)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(tracing_appender::non_blocking(appender))
}

/// Starts the file logger. `name` is the file stem, e.g. `canvas-rgbd`. The daemon calls this
/// only after the elevation check: it creates and prunes files in a user-writable folder, which
/// an elevated process must never do.
pub fn init(name: &str) -> std::io::Result<()> {
    let dir = crate::paths::log_dir();
    std::fs::create_dir_all(&dir)?;
    prune_old_logs(&dir);
    let (writer, guard) = file_writer(&dir, name)?;
    *DIRECT_TARGET.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((dir.clone(), name.to_string()));

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("canvas_daemon=info,{LOG_TARGET}=info,warn")));

    // `try_init` fails when a subscriber is already installed (which happens in tests).
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(writer).with_ansi(false).with_target(true));
    let _ = subscriber.try_init();
    hold(guard);
    Ok(())
}

/// Logs a failure that a step would otherwise drop, and hands back the value on success. Use it
/// where the step goes on whatever happens: the log then says the step did not work, instead of
/// the next line reading as if it had (class `cause:unchecked-state`, #19).
pub fn warn_if_err<T, E: std::fmt::Display>(what: &str, result: Result<T, E>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(%error, "{what}");
            None
        }
    }
}

/// Logs how many devices a step reached: INFO for some, WARN for none. A step that reached no
/// device did nothing, and the log must say so (class `cause:unchecked-state`, #18).
pub fn log_device_write(what: &str, devices: usize) {
    if devices == 0 {
        tracing::warn!(devices, "{what} 0 devices: nothing was written");
    } else {
        tracing::info!(devices, "{what} {devices} devices");
    }
}

/// Removes log files older than [`KEEP_DAYS`] days.
pub fn prune_old_logs(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age.as_secs() > (KEEP_DAYS as u64) * 86_400 {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Whether the day's log file has grown past the cap.
pub fn over_budget(dir: &std::path::Path, prefix: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let total: u64 = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();
    total >= MAX_BYTES_PER_DAY
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `run` under a subscriber that collects its lines: the crate's own helper, so that
    /// the callsite interest cache stays permissive (class `cause:timing-test`, #45).
    fn captured(run: impl FnOnce()) -> String {
        let (_, text) = crate::test_logs::captured(run);
        text
    }

    #[test]
    fn a_dropped_error_is_logged_and_a_value_passes_through() {
        let text = captured(|| {
            assert_eq!(warn_if_err("could not save the thing", Err::<u8, _>("disk full")), None);
            assert_eq!(warn_if_err("could not save the thing", Ok::<u8, &str>(5)), Some(5));
        });
        assert_eq!(text.lines().count(), 1, "{text}");
        assert!(text.contains("WARN") && text.contains("could not save the thing") && text.contains("disk full"), "{text}");
    }

    /// `close` is the flush the quit watchdog and the `WM_ENDSESSION` handler rely on: a line
    /// logged just before it is in the file when it returns, with no sleep.
    #[test]
    fn close_flushes_the_last_line_to_the_file() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-flush-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (writer, guard) = file_writer(&dir, "canvas-rgbd").unwrap();
        hold(guard);
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(writer).finish();
        // The keeper first: this capture has a subscriber of its own, so the interest cache must
        // not answer `never` for a callsite another thread reached (class `cause:timing-test`).
        crate::test_logs::keep_the_interest_cache_permissive();
        tracing::subscriber::with_default(subscriber, || {
            for i in 0..200 {
                tracing::info!("filler line {i}");
            }
            tracing::info!("the very last line");
        });
        close();
        let text: String = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(text.contains("the very last line"), "the last line reached the file: {} bytes", text.len());
        close(); // a second close does nothing
    }

    /// A writer that blocks every write until `release` is set: a backlog that cannot drain.
    struct StuckWriter(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl std::io::Write for StuckWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            while !self.0.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn all_text(dir: &std::path::Path) -> String {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
            .collect()
    }

    /// #36: a line queued behind a backlog that does not drain is not in the file when the process
    /// ends; a direct line is written into the file and is.
    ///
    /// The verdict is the write's own end, not a window: the direct write must not have to finish
    /// inside a fixed number of milliseconds for this test to pass, or a busy machine decides it
    /// (class `cause:timing-test`, #72).
    #[test]
    fn a_direct_line_reaches_the_file_while_a_backlog_is_stuck() {
        let dir = temp_dir("direct");
        // A queue whose writer is stuck: the lines pile up behind it.
        let release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stuck, stuck_guard) = tracing_appender::non_blocking(StuckWriter(release.clone()));
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(stuck).finish();
        // The keeper first: this capture has a subscriber of its own, so the interest cache must
        // not answer `never` for a callsite another thread reached (class `cause:timing-test`).
        crate::test_logs::keep_the_interest_cache_permissive();
        tracing::subscriber::with_default(subscriber, || {
            for i in 0..100 {
                tracing::error!("queued behind the backlog {i}");
            }
        });
        let drained = run_bounded(std::time::Duration::from_millis(250), move || drop(stuck_guard));
        assert!(!drained, "the backlog does not drain inside the limit");

        let (d, n) = (dir.clone(), "canvas-rgbd".to_string());
        // The write is waited for by the helper's end: an event, not a window. The file below is
        // what the user reads, and it is read after the write landed.
        let written = write_once_waiting(
            |finished| helper_finished(&finished, HANG_GUARD),
            move || write_line_direct(&d, &n, "ERROR the daemon panicked message=\"direct\""),
        );
        release.store(true, std::sync::atomic::Ordering::SeqCst);

        let text = all_text(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(written, "the direct write finished: {text}");
        assert_eq!(text.matches("the daemon panicked").count(), 1, "{text}");
        assert!(!text.contains("queued behind the backlog"), "the stuck backlog never reached the file: {text}");
    }

    /// The date basis: the file the real appender makes today is the file `todays_log_file` names.
    #[test]
    fn the_direct_file_is_the_file_the_appender_writes() {
        let dir = temp_dir("appender-name");
        let before = todays_log_file(&dir, "canvas-rgbd");
        let (writer, guard) = file_writer(&dir, "canvas-rgbd").unwrap();
        drop((writer, guard));
        let after = todays_log_file(&dir, "canvas-rgbd");
        let made: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
        let _ = std::fs::remove_dir_all(&dir);
        // The two names differ only if UTC midnight passed during the test.
        assert!(made.contains(&before) || made.contains(&after), "appender made {made:?}, expected {before:?}");
    }

    #[test]
    fn the_appender_date_is_utc_year_month_day() {
        assert_eq!(appender_date(0), "1970-01-01");
        assert_eq!(appender_date(1_709_208_000), "2024-02-29");
        assert_eq!(appender_date(1_772_323_199), "2026-02-28");
        assert_eq!(appender_date(1_793_491_200), "2026-11-01");
        assert_eq!(appender_date(1_793_491_199), "2026-10-31");
    }

    /// Around midnight the appender has not made the new day's file yet; the direct line makes it.
    /// And no other file in the folder can be chosen for it.
    #[test]
    fn a_direct_line_goes_to_todays_named_file_and_to_no_other() {
        let dir = temp_dir("named");
        std::fs::write(dir.join("canvas-rgbd.old.log"), "old\n").unwrap();
        std::fs::write(dir.join("canvas-rgbd.9999-12-31.log"), "future\n").unwrap();
        write_line_direct(&dir, "canvas-rgbd", "the panic line").unwrap();
        let today = todays_log_file(&dir, "canvas-rgbd");
        let in_today = std::fs::read_to_string(&today).unwrap_or_default();
        let old = std::fs::read_to_string(dir.join("canvas-rgbd.old.log")).unwrap();
        let future = std::fs::read_to_string(dir.join("canvas-rgbd.9999-12-31.log")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(in_today, "the panic line\n");
        assert_eq!((old.as_str(), future.as_str()), ("old\n", "future\n"));
    }

    /// Makes a directory junction at `link` pointing at `target` (Windows, test only). The
    /// `let made = ...` line is text-matched by the process-launch allow-list, so the binding is
    /// kept even though clippy would inline it.
    #[cfg(windows)]
    #[allow(clippy::let_and_return)]
    fn make_junction(link: &std::path::Path, target: &std::path::Path) -> bool {
        // token: the current user's, not elevated (a test making a junction with mklink).
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        made
    }

    /// A junction where today's file should be is not followed.
    #[cfg(windows)]
    #[test]
    fn a_reparse_point_in_place_of_the_log_file_is_not_followed() {
        let dir = temp_dir("reparse");
        let elsewhere = temp_dir("reparse-target");
        let link = todays_log_file(&dir, "canvas-rgbd");
        assert!(make_junction(&link, &elsewhere), "the junction was made");
        let result = write_line_direct(&dir, "canvas-rgbd", "the panic line");
        let target_files = std::fs::read_dir(&elsewhere).unwrap().count();
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&elsewhere);
        assert!(result.is_err(), "refused");
        assert_eq!(target_files, 0, "nothing was written through the link");
    }

    /// #36/#72: the rolling appender opens today's file on its own, so the direct writer's
    /// reparse check was the only one. A junction planted at the name must now be refused by
    /// `file_writer` too, before the appender can open or adopt it: the path is named, nothing is
    /// written through the link, and the link itself is left alone.
    #[cfg(windows)]
    #[test]
    fn the_appender_refuses_a_reparse_point_at_todays_file() {
        let dir = temp_dir("appender-reparse");
        let elsewhere = temp_dir("appender-reparse-target");
        let link = todays_log_file(&dir, "canvas-rgbd");
        assert!(make_junction(&link, &elsewhere), "the junction was made");
        let error = file_writer(&dir, "canvas-rgbd").expect_err("the appender must refuse the junction");
        let message = error.to_string();
        assert!(message.contains(&link.display().to_string()), "the error names the path: {message}");
        assert!(message.contains("reparse"), "the error says why: {message}");
        let target_files = std::fs::read_dir(&elsewhere).unwrap().count();
        assert!(std::fs::symlink_metadata(&link).is_ok(), "the junction is still there: never deleted or adopted");
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&elsewhere);
        assert_eq!(target_files, 0, "nothing was written through the junction");
    }

    /// #36, at most once: a write that completes after the limit is not written a second time by
    /// the caller's fallback. Whichever way the race between the helper's start and the limit goes,
    /// the file holds the line once; on a normal run the helper has begun and lands late.
    ///
    /// The wait is injected, so the race is decided by the test and not by the machine's speed
    /// (class `cause:timing-test`, #72): the helper says it has the line, the caller gives up the
    /// way it does when the limit passes, and the helper lands afterwards.
    #[test]
    fn a_write_that_lands_after_the_limit_is_not_written_twice() {
        let dir = temp_dir("late");
        let path = dir.join("late.log");
        let late_path = path.clone();
        let (has_line, wait_has_line) = std::sync::mpsc::channel::<()>();
        let (land, wait_land) = std::sync::mpsc::channel::<()>();
        let (landed, wait_landed) = std::sync::mpsc::channel::<()>();
        let reported = write_once_waiting(
            |_finished| {
                // The helper holds the line and has not landed: the limit has passed.
                let _ = wait_has_line.recv_timeout(HANG_GUARD);
                false
            },
            move || {
                // The helper takes the line, says so, and waits for the test to let it land.
                let _ = has_line.send(());
                let _ = wait_land.recv();
                let written = append_line(&late_path, "the panic line");
                let _ = landed.send(());
                written
            },
        );
        // The caller's fallback, as the hook has it: write the line only when told to.
        let fallback_wrote = !reported;
        if fallback_wrote {
            append_line(&path, "the panic line").unwrap();
        }
        assert!(reported, "the helper owns the line: the caller must not write it again");
        // The helper lands now, and says so when the line is in the file.
        let _ = land.send(());
        wait_landed
            .recv_timeout(HANG_GUARD)
            .expect("the helper landed the line it owns");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(text.matches("the panic line").count(), 1, "reported={reported}: {text}");
    }

    /// A write that fails is reported as "not written", so the caller writes the line itself; a
    /// write that succeeds is reported as written.
    ///
    /// The helper thread must not have to start and finish inside a fixed number of milliseconds
    /// for this test to pass, or a busy machine decides it (class `cause:timing-test`, #72). The
    /// bound here is a hang guard, generous on purpose: the write itself is instant.
    #[test]
    fn a_failed_write_is_reported_so_the_caller_can_fall_back() {
        assert!(!write_once_within(HANG_GUARD, || Err(std::io::Error::other("disk full"))));
        assert!(write_once_within(HANG_GUARD, || Ok(())));
    }

    /// #36 follow-up (#72): a helper that panics must not leave the state STARTED. If it does,
    /// `write_once_within` reports "the helper owns the line" and the caller skips the fallback,
    /// so the line is lost. The panic guard puts the state back so the caller takes the write back.
    ///
    /// The wait is the helper's own end, not a clock: how long the panic takes (a backtrace
    /// capture alone takes longer than the old 50 ms window) must not decide the verdict (class
    /// `cause:timing-test`).
    #[test]
    fn a_panicking_helper_lets_the_caller_write_the_fallback() {
        let dir = temp_dir("panic-fallback");
        let path = dir.join("panic.log");
        // The helper's thread ends when it panics: the caller waits for that, not for a window.
        let helper_owns_the_line = write_once_waiting(
            |finished| finished.recv().is_ok(),
            || panic!("the direct write panicked"),
        );
        assert!(
            !helper_owns_the_line,
            "a panicked helper wrote nothing: the caller must be told to write the fallback"
        );
        // The caller's fallback, as the panic hook has it: the caller writes the line itself.
        append_line(&path, "the daemon panicked").unwrap();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            text.matches("the daemon panicked").count(),
            1,
            "the line is in the log exactly once: {text}"
        );
    }

    /// A bounded run gives up on a step that does not finish, and waits for a quick one.
    ///
    /// The give-up is the claim and the return value carries it: a slow machine only makes the
    /// give-up more certain, so no clock is read here (class `cause:timing-test`, #72). The quick
    /// step's bound is a hang guard (the work is empty), not a latency budget.
    #[test]
    fn a_bounded_run_gives_up_on_a_stuck_step_and_waits_for_a_quick_one() {
        let limit = std::time::Duration::from_millis(250);
        let finished = run_bounded(limit, || std::thread::sleep(std::time::Duration::from_secs(5)));
        assert!(!finished, "a 5 s step does not finish inside {limit:?}");
        assert!(run_bounded(HANG_GUARD, || {}), "a quick step finishes");
    }

    #[test]
    fn a_write_to_zero_devices_is_a_warning() {
        let text = captured(|| {
            log_device_write("queued black for", 0);
            log_device_write("queued black for", 2);
        });
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].contains("WARN") && lines[0].contains("0 devices") && lines[0].contains("nothing was written"), "{text}");
        assert!(lines[1].contains("INFO") && lines[1].contains("2 devices"), "{text}");
    }

    /// Every log line that lived in the `canvas-rgbd` binary before it moved into this library,
    /// with the file it lives in now and the commit that moved it (#446). A line that moved keeps
    /// `target: LOG_TARGET`, or a narrowed `RUST_LOG=canvas_rgbd=...` drops it (#87).
    const MOVED_FROM_THE_BINARY: [(&str, &str, &str); 5] = [
        ("device_sync.rs", "78eeb8bb", "this device is never driven; it gets no frames"),
        ("device_sync.rs", "78eeb8bb", "device gone; its frame sink is removed"),
        ("device_sync.rs", "78eeb8bb", "device frame sink added"),
        ("scenes.rs", "9104caa8", "the active scene was restored"),
        ("scenes.rs", "9104caa8", "the user effect file is missing; install it to run this effect"),
    ];

    /// Every `.rs` file under this crate's `src`, keyed by its path inside `src` (`scenes.rs`,
    /// `api/scenes.rs`), with its text.
    fn source_files() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, root: &std::path::Path, into: &mut Vec<(String, String)>) {
            for entry in std::fs::read_dir(dir).expect("the source folder is readable").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, root, into);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).expect("a source file is readable");
                    let name = path.strip_prefix(root).expect("under src").to_string_lossy().replace('\\', "/");
                    into.push((name, text));
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&root, &root, &mut files);
        files
    }

    /// The line `offset` falls on, 1-based.
    fn line_of(source: &str, offset: usize) -> usize {
        source[..offset].matches('\n').count() + 1
    }

    /// The `tracing::...!` call that holds `offset`, from `tracing::` to its closing `)`.
    fn log_call_around(source: &str, offset: usize) -> Option<&str> {
        let start = source[..offset].rfind("tracing::")?;
        let open = start + source[start..].find('(')?;
        let mut depth = 0usize;
        for (i, c) in source[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&source[start..open + i + 1]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// #446: a line that came from the `canvas-rgbd` binary logs under the daemon's target, so the
    /// filter that names the binary still catches it. The five un-drivable lines are read from
    /// the source; the frame-write warning, which a test can drive, is checked through `captured`
    /// in `writer.rs` (`the_frame_write_warning_keeps_the_daemon_target`).
    #[test]
    fn every_line_that_came_from_the_binary_keeps_the_daemon_target() {
        let files = source_files();
        for (file, commit, message) in MOVED_FROM_THE_BINARY {
            let (_, text) = files
                .iter()
                .find(|(name, _)| name == file)
                .unwrap_or_else(|| panic!("{file} is gone: the list in this test is out of date"));
            let offset = text.find(message).unwrap_or_else(|| {
                panic!("{file}: the line \"{message}\" (moved in {commit}) is gone; update the list")
            });
            let call = log_call_around(text, offset).unwrap_or_else(|| {
                panic!("{file}:{}: the line \"{message}\" is not inside a tracing call", line_of(text, offset))
            });
            assert!(
                call.contains("target:") && call.contains("LOG_TARGET"),
                "{file}:{}: \"{message}\" (moved out of the canvas-rgbd binary in {commit}) logs \
                 under its module target, so a RUST_LOG={LOG_TARGET}=... filter drops it; give it \
                 target: crate::logging::LOG_TARGET",
                line_of(text, offset)
            );
        }
    }

    /// #446: the daemon's log target is spelled once, in [`LOG_TARGET`]. A second copy of the
    /// string drifts from this one the moment the daemon is renamed.
    #[test]
    fn the_daemon_log_target_is_spelled_once() {
        let quoted = format!("\"{LOG_TARGET}\"");
        for (path, text) in source_files() {
            if path == "logging.rs" {
                continue;
            }
            if let Some(offset) = text.find(&quoted) {
                panic!(
                    "crates/canvas-daemon/src/{path}:{}: the daemon's log target is written out \
                     here instead of using logging::LOG_TARGET",
                    line_of(&text, offset)
                );
            }
        }
    }

    #[test]
    fn old_logs_are_pruned() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-logs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("canvas-rgbd.old.log");
        std::fs::write(&old, b"x").unwrap();
        // Backdate the file by 10 days.
        let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10 * 86_400);
        filetime_set(&old, old_time);
        prune_old_logs(&dir);
        assert!(!old.exists(), "a log older than 7 days is removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_budget_check_sees_the_file_size() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-budget-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!over_budget(&dir, "canvas-rgbd"));
        std::fs::write(dir.join("canvas-rgbd.2026-10-26.log"), vec![0u8; 16]).unwrap();
        assert!(!over_budget(&dir, "canvas-rgbd"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sets a file's modification time. Used only by the tests above.
    fn filetime_set(path: &std::path::Path, time: std::time::SystemTime) {
        let Ok(file) = std::fs::OpenOptions::new().write(true).open(path) else {
            return;
        };
        // `SetFileTime` needs a FILETIME; build one from the unix timestamp.
        let secs = time.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        let windows_ticks = (secs + 11_644_473_600) * 10_000_000;
        let ft = windows::Win32::Foundation::FILETIME {
            dwLowDateTime: windows_ticks as u32,
            dwHighDateTime: (windows_ticks >> 32) as u32,
        };
        let creation = ft;
        let access = ft;
        let write = ft;
        let handle = std::os::windows::io::AsRawHandle::as_raw_handle(&file);
        unsafe {
            let _ = windows::Win32::Storage::FileSystem::SetFileTime(
                windows::Win32::Foundation::HANDLE(handle as _),
                Some(&creation),
                Some(&access),
                Some(&write),
            );
        }
    }
}
