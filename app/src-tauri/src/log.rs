//! Best-effort tray log at `%LOCALAPPDATA%\canvas-rgb\logs\tray.log` (the tray has no console).

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn append(line: &str) {
    let dir = log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("tray.log"))
    else {
        return;
    };
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // One `write_all` for the whole line, so a line from another thread or process cannot land
    // in the middle of this one (#553).
    let _ = file.write_all(format!("{secs} {line}\n").as_bytes());
}

/// The folder the tray log goes in.
///
/// The shipped binary uses `%LOCALAPPDATA%\canvas-rgb\logs`. A test build uses a folder of its
/// own, because a test that reaches `log::append` must never write into the person's real log
/// (#553).
fn log_dir() -> PathBuf {
    #[cfg(not(test))]
    {
        canvas_proto::consts::user_log_dir()
    }
    #[cfg(test)]
    {
        // One folder per test process, emptied on first use so a killed run's lines cannot
        // stand in for this run's.
        static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        DIR.get_or_init(|| {
            let dir = std::env::temp_dir()
                .join(format!("canvas-rgb-tray-test-log-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            dir
        })
        .clone()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::SystemTime;

    /// The file a bug report asks for: the person's real tray log.
    fn real_log() -> std::path::PathBuf {
        canvas_proto::consts::user_log_dir().join("tray.log")
    }

    /// A file's length and modified time; `None` when it does not exist.
    fn state(path: &Path) -> Option<(u64, Option<SystemTime>)> {
        let meta = std::fs::metadata(path).ok()?;
        Some((meta.len(), meta.modified().ok()))
    }

    #[test]
    fn a_test_that_logs_never_writes_the_real_tray_log() {
        let real = real_log();
        let before = state(&real);

        // A real path that logs (#553): an unreadable prefs file, the same thing the prefs
        // test writes when it loads `{ not json`.
        let dir = std::env::temp_dir().join(format!("canvas-rgb-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("prefs.json");
        std::fs::write(&bad, "{ not json").unwrap();
        let _ = crate::prefs::load(&bad);

        // The line did reach a log: the test build's own, not the person's.
        let own = std::fs::read_to_string(super::log_dir().join("tray.log")).unwrap_or_default();
        assert!(own.contains("is unreadable"), "the test logged to its own file:\n{own}");

        let after = state(&real);
        assert_eq!(
            before,
            after,
            "a test wrote the person's real tray log at {}",
            real.display()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
