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
