//! Best-effort tray log at `%LOCALAPPDATA%\canvas-rgb\logs\tray.log` (the tray has no console).

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use canvas_proto::consts;

pub fn append(line: &str) {
    let dir = consts::user_log_dir();
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
    let _ = writeln!(file, "{secs} {line}");
}
