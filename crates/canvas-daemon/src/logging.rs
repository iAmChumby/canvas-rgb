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

/// Runs `work` on a helper thread and waits at most `limit` for it. Returns whether it finished.
/// A stuck `work` is left behind on its thread; the caller is about to end the process.
pub fn run_bounded(limit: std::time::Duration, work: impl FnOnce() + Send + 'static) -> bool {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        work();
        let _ = done.send(());
    });
    finished.recv_timeout(limit).is_ok()
}

/// [`close`], but it gives up after [`FLUSH_LIMIT`]. Returns whether the log drained in time.
pub fn close_within_limit() -> bool {
    run_bounded(FLUSH_LIMIT, close)
}

/// Where the daily log file is, set by [`init`]: the folder and the file stem. A panic on the main
/// thread writes its line there directly ([`write_line_direct_within`]).
static DIRECT_TARGET: std::sync::Mutex<Option<(std::path::PathBuf, String)>> = std::sync::Mutex::new(None);

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

/// Appends one line to `path` in one write, creating the file if needed. A reparse point (a link
/// or junction planted where the log file goes) is not followed: it is opened as itself, and
/// refused.
fn append_line(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(std::io::Error::other("the log file is a reparse point; not writing to it"));
        }
    }
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

/// Runs `write` on a helper thread and waits at most `limit`, so that the line is written AT MOST
/// ONCE. Returns `true` when the line is in the file, or the helper has begun writing it and may
/// still finish after the limit (the caller must not write it again: a late append plus a second
/// copy would be a duplicate). Returns `false` when the caller must write the line itself: the
/// write failed, or the helper had not begun when the limit passed (it then never will).
///
/// So a stalled disk can still lose the line, but it is never in the file twice.
pub fn write_once_within(
    limit: std::time::Duration,
    write: impl FnOnce() -> std::io::Result<()> + Send + 'static,
) -> bool {
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::SeqCst};
    let state = std::sync::Arc::new(AtomicU8::new(WRITE_PENDING));
    let wrote = std::sync::Arc::new(AtomicBool::new(false));
    let (helper_state, helper_wrote) = (std::sync::Arc::clone(&state), std::sync::Arc::clone(&wrote));
    let finished = run_bounded(limit, move || {
        if helper_state.compare_exchange(WRITE_PENDING, WRITE_STARTED, SeqCst, SeqCst).is_err() {
            return;
        }
        helper_wrote.store(write().is_ok(), SeqCst);
    });
    if finished {
        wrote.load(SeqCst)
    } else {
        // Past the limit: if the helper has not begun, take the write back (it will skip it);
        // if it has begun, it owns the line.
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
        .unwrap_or_else(|_| EnvFilter::new("canvas_daemon=info,canvas_rgbd=info,warn"));

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

    /// Runs `run` under a subscriber that collects its lines.
    fn captured(run: impl FnOnce()) -> String {
        #[derive(Clone, Default)]
        struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buf = Buf::default();
        let sink = buf.clone();
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || sink.clone()).finish();
        tracing::subscriber::with_default(subscriber, run);
        let text = String::from_utf8_lossy(&buf.0.lock().unwrap()).into_owned();
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
    #[test]
    fn a_direct_line_reaches_the_file_while_a_backlog_is_stuck() {
        let dir = temp_dir("direct");
        // A queue whose writer is stuck: the lines pile up behind it.
        let release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stuck, stuck_guard) = tracing_appender::non_blocking(StuckWriter(release.clone()));
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(stuck).finish();
        tracing::subscriber::with_default(subscriber, || {
            for i in 0..100 {
                tracing::error!("queued behind the backlog {i}");
            }
        });
        let drained = run_bounded(std::time::Duration::from_millis(250), move || drop(stuck_guard));
        assert!(!drained, "the backlog does not drain inside the limit");

        let (d, n) = (dir.clone(), "canvas-rgbd".to_string());
        let written = write_once_within(std::time::Duration::from_millis(250), move || {
            write_line_direct(&d, &n, "ERROR the daemon panicked message=\"direct\"")
        });
        release.store(true, std::sync::atomic::Ordering::SeqCst);

        let text = all_text(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(written, "the direct write finished inside its limit");
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

    /// A junction where today's file should be is not followed.
    #[cfg(windows)]
    #[test]
    fn a_reparse_point_in_place_of_the_log_file_is_not_followed() {
        let dir = temp_dir("reparse");
        let elsewhere = temp_dir("reparse-target");
        let link = todays_log_file(&dir, "canvas-rgbd");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&elsewhere)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(made, "the junction was made");
        let result = write_line_direct(&dir, "canvas-rgbd", "the panic line");
        let target_files = std::fs::read_dir(&elsewhere).unwrap().count();
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&elsewhere);
        assert!(result.is_err(), "refused");
        assert_eq!(target_files, 0, "nothing was written through the link");
    }

    /// #36, at most once: a write that completes after the limit is not written a second time by
    /// the caller's fallback. Whichever way the race between the helper's start and the limit goes,
    /// the file holds the line once; on a normal run the helper has begun and lands late.
    #[test]
    fn a_write_that_lands_after_the_limit_is_not_written_twice() {
        let dir = temp_dir("late");
        let path = dir.join("late.log");
        let late_path = path.clone();
        let landed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = landed.clone();
        let begun = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let began = begun.clone();
        let reported = write_once_within(std::time::Duration::from_millis(300), move || {
            began.store(true, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(900));
            let r = append_line(&late_path, "the panic line");
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            r
        });
        // The caller's fallback, as the hook has it: write the line only when told to.
        let fallback_wrote = !reported;
        if fallback_wrote {
            append_line(&path, "the panic line").unwrap();
        }
        // The helper lands late when it owns the line; when the caller took the write back it never runs.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while (reported || begun.load(std::sync::atomic::Ordering::SeqCst)) && !landed.load(std::sync::atomic::Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(text.matches("the panic line").count(), 1, "reported={reported}: {text}");
    }

    #[test]
    fn a_failed_write_is_reported_so_the_caller_can_fall_back() {
        assert!(!write_once_within(std::time::Duration::from_millis(250), || Err(std::io::Error::other("disk full"))));
        assert!(write_once_within(std::time::Duration::from_millis(250), || Ok(())));
    }

    #[test]
    fn a_bounded_run_gives_up_on_a_stuck_step_and_waits_for_a_quick_one() {
        let limit = std::time::Duration::from_millis(250);
        let started = std::time::Instant::now();
        let finished = run_bounded(limit, || std::thread::sleep(std::time::Duration::from_secs(5)));
        let took = started.elapsed();
        assert!(!finished, "a 5 s step does not finish inside {limit:?}");
        assert!(took < std::time::Duration::from_millis(1000), "gave up after {took:?}, not 5 s");
        assert!(run_bounded(limit, || {}), "a quick step finishes");
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
