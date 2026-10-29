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

/// The guard that flushes the log writer. Keep it alive for the process's life.
pub struct LogGuard {
    _writer: WorkerGuard,
}

impl LogGuard {
    pub fn writer(&self) -> &WorkerGuard {
        &self._writer
    }
}

/// Starts the file logger. `name` is the file stem, e.g. `canvas-rgbd`.
pub fn init(name: &str) -> std::io::Result<LogGuard> {
    let dir = crate::paths::log_dir();
    std::fs::create_dir_all(&dir)?;
    prune_old_logs(&dir);

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(name)
        .filename_suffix("log")
        // Two, not one: at build time the appender prunes down to max - 1 files, so with 1 it
        // deleted the day's own log on every daemon start.
        .max_log_files(2)
        .build(&dir)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("canvas_daemon=info,canvas_rgbd=info,warn"));

    // `try_init` fails when a subscriber is already installed (which happens in tests).
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(writer).with_ansi(false).with_target(true));
    let _ = subscriber.try_init();
    Ok(LogGuard { _writer: guard })
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
