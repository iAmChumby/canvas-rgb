//! Every path the daemon reads or writes, in one place.
//!
//! `CANVAS_RGB_HOME` overrides the whole user tree (`%APPDATA%\canvas-rgb` by default)
//! so tests and dev runs never touch Luke's real config. The OpenRGB config dir and the
//! machine-wide `%ProgramData%\canvas-rgb` follow the same override so a test run stays
//! self-contained.

use canvas_proto::config::{BACKUPS_DIR, BACKUPS_KEPT, CONFIG_FILE, EFFECTS_FILE, FANS_FILE, LAYOUTS_FILE, ZONES_FILE};
use canvas_proto::consts;
use std::path::{Path, PathBuf};

/// The environment variable that relocates every user file.
pub const HOME_ENV: &str = "CANVAS_RGB_HOME";

/// The daemon's user directory: `%APPDATA%\canvas-rgb`, or `$CANVAS_RGB_HOME`.
pub fn home() -> PathBuf {
    match std::env::var_os(HOME_ENV) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => consts::user_config_dir(),
    }
}

/// `%LOCALAPPDATA%\canvas-rgb\logs`, or `$CANVAS_RGB_HOME/logs`.
pub fn log_dir() -> PathBuf {
    match std::env::var_os(HOME_ENV) {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("logs"),
        _ => consts::user_log_dir(),
    }
}

/// The OpenRGB `--config` directory the daemon passes to the server it starts.
pub fn openrgb_config_dir() -> PathBuf {
    match std::env::var_os(HOME_ENV) {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("openrgb"),
        _ => consts::openrgb_config_dir(),
    }
}

/// `%ProgramData%\canvas-rgb` — the service's tree. Overridden by the same env var so
/// tests never write to the real machine-wide directory.
pub fn machine_dir() -> PathBuf {
    match std::env::var_os(HOME_ENV) {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("programdata"),
        _ => consts::machine_data_dir(),
    }
}

pub fn config_file() -> PathBuf {
    home().join(CONFIG_FILE)
}

pub fn layouts_file() -> PathBuf {
    home().join(LAYOUTS_FILE)
}

pub fn zones_file() -> PathBuf {
    home().join(ZONES_FILE)
}

pub fn effects_file() -> PathBuf {
    home().join(EFFECTS_FILE)
}

/// The user HTML effects folder: `effects\` under [`home`], so `CANVAS_RGB_HOME` moves it too.
pub fn effects_dir() -> PathBuf {
    home().join("effects")
}

/// The daemon's copy of the last fan config it pushed to the service (G10: exported
/// when the service is down).
pub fn fans_last_file() -> PathBuf {
    home().join("fans.last.toml")
}

/// Where the service keeps `%ProgramData%\canvas-rgb\fans.toml`.
pub fn service_fans_file() -> PathBuf {
    machine_dir().join(FANS_FILE)
}

pub fn token_file() -> PathBuf {
    match std::env::var_os(HOME_ENV) {
        Some(v) if !v.is_empty() => PathBuf::from(v).join(consts::TOKEN_FILE),
        _ => consts::token_path(),
    }
}

pub fn backups_dir() -> PathBuf {
    home().join(BACKUPS_DIR)
}

/// The backups folder inside a given home directory.
pub fn backups_dir_in(home: &Path) -> PathBuf {
    home.join(BACKUPS_DIR)
}

/// Creates a directory and every missing parent. Returns the path for chaining.
pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Reads a file, returning `None` when it does not exist.
pub fn read_optional(path: &Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => None,
    }
}

/// Where snapshots live and how many are kept (G10).
pub const SNAPSHOTS_KEPT: usize = BACKUPS_KEPT;
