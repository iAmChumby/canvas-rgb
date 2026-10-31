//! Where the service keeps its files. `CANVAS_RGB_MACHINE_HOME` replaces the
//! `%ProgramData%\canvas-rgb` directory (tests and diagnostics only).

use canvas_proto::config::FANS_FILE;
use canvas_proto::consts;
use std::path::{Path, PathBuf};

pub const MACHINE_HOME_ENV: &str = "CANVAS_RGB_MACHINE_HOME";
pub const BIOS_STATE_FILE: &str = "bios-state.json";

/// The machine data directory: the env override, else `%ProgramData%\canvas-rgb`.
pub fn machine_home() -> PathBuf {
    std::env::var_os(MACHINE_HOME_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(consts::machine_data_dir)
}

pub fn fans_path(home: &Path) -> PathBuf {
    home.join(FANS_FILE)
}

pub fn bios_state_path(home: &Path) -> PathBuf {
    home.join(BIOS_STATE_FILE)
}

/// The service's own record of the confirmed channel map (`chip.toml`).
pub const CHIP_FILE: &str = "chip.toml";

pub fn chip_path(home: &Path) -> PathBuf {
    home.join(CHIP_FILE)
}

pub fn logs_dir(home: &Path) -> PathBuf {
    home.join("logs")
}

/// The `--config` directory the OpenRGB server runs with.
pub fn openrgb_config_dir(home: &Path) -> PathBuf {
    home.join("openrgb")
}

/// `<home>/updates`: where the service stages an installer before it checks and runs it (#43).
/// With the real home this is [`consts::update_staging_dir`].
pub fn updates_dir(home: &Path) -> PathBuf {
    home.join("updates")
}
