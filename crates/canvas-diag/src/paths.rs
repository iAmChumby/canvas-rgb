//! The few path and timestamp helpers the bundle needs (R1).
//!
//! They are moved, not copied: the daemon's own `paths` module and `storage::timestamp` call
//! through these thin wrappers, so one rule covers both the live app and the diagnostics zip.
//! `CANVAS_RGB_HOME` relocates the whole tree, including the machine folder, so tests never
//! touch a real `%ProgramData%`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub use canvas_proto::consts::HOME_ENV;

/// The user's config directory, `%APPDATA%\canvas-rgb` or `$CANVAS_RGB_HOME`.
pub fn home() -> PathBuf {
    match std::env::var_os(HOME_ENV) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => canvas_proto::consts::user_config_dir(),
    }
}

/// The user's log directory, under [`home`] in a test run.
pub fn log_dir() -> PathBuf {
    match std::env::var_os(HOME_ENV) {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("logs"),
        _ => canvas_proto::consts::user_log_dir(),
    }
}

/// The service's machine directory. Read-only here: the bundle never writes under it.
pub fn machine_dir() -> PathBuf {
    match std::env::var_os(HOME_ENV) {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("programdata"),
        _ => canvas_proto::consts::machine_data_dir(),
    }
}

/// The service's machine log directory.
pub fn machine_log_dir() -> PathBuf {
    machine_dir().join("logs")
}

/// The one folder diagnostics zips are written to: `%LOCALAPPDATA%\canvas-rgb\diagnostics`.
/// A caller never chooses the destination (P2).
pub fn diagnostics_dir() -> PathBuf {
    match std::env::var_os(HOME_ENV) {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("diagnostics"),
        _ => canvas_proto::consts::user_log_dir()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("diagnostics"),
    }
}

/// A file-name-safe UTC stamp, `YYYYMMDD-HHMMSS`. Moved from the daemon's `storage` module.
pub fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_has_the_shape_used_in_file_names() {
        let stamp = timestamp();
        assert_eq!(stamp.len(), 15, "{stamp}");
        assert_eq!(&stamp[8..9], "-", "{stamp}");
        assert!(stamp.chars().all(|c| c.is_ascii_digit() || c == '-'));
    }
}
