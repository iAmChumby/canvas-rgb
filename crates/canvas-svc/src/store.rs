//! Atomic file writes, `fans.toml` (the only config this service writes) and the
//! crash-restore record `bios-state.json`.

use canvas_proto::config::{FansFile, FANS_SCHEMA};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io on {path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
    #[error("cannot parse {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{path} has schema_version {found}, expected {expected}")]
    Schema { path: PathBuf, found: u32, expected: u32 },
    #[error("cannot encode {0}: {1}")]
    Encode(&'static str, String),
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::Io { path: path.to_path_buf(), source }
}

/// Writes `bytes` to a temp file in the same directory, flushes it, then renames it over
/// `path`. A reader sees the old file or the new one, never a partial write.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(io_err(dir))?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{file_name}.tmp"));
    {
        let mut file = fs::File::create(&tmp).map_err(io_err(&tmp))?;
        file.write_all(bytes).map_err(io_err(&tmp))?;
        file.sync_all().map_err(io_err(&tmp))?;
    }
    fs::rename(&tmp, path).map_err(io_err(path))
}

/// Reads `fans.toml`. `Ok(None)` when the file does not exist yet.
pub fn load_fans(path: &Path) -> Result<Option<FansFile>, StoreError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(StoreError::Io { path: path.to_path_buf(), source }),
    };
    let file: FansFile = toml::from_str(&text)
        .map_err(|e| StoreError::Parse { path: path.to_path_buf(), message: e.to_string() })?;
    if file.schema_version != FANS_SCHEMA {
        return Err(StoreError::Schema {
            path: path.to_path_buf(),
            found: file.schema_version,
            expected: FANS_SCHEMA,
        });
    }
    Ok(Some(file))
}

pub fn save_fans(path: &Path, file: &FansFile) -> Result<(), StoreError> {
    let text = toml::to_string_pretty(file).map_err(|e| StoreError::Encode("fans.toml", e.to_string()))?;
    write_atomic(path, text.as_bytes())
}

pub const CHIP_SCHEMA: u32 = 1;

/// `chip.toml`: the channel map the operator confirmed against LibreHardwareMonitor on this
/// board (`canvas-rgb-svc.exe --confirm-channel-map <name>`). Until a confirmation names the
/// map that is detected, the service makes no register writes that drive fans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChipState {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_map: Option<String>,
}

impl ChipState {
    pub fn unconfirmed() -> Self {
        Self { schema_version: CHIP_SCHEMA, confirmed_map: None }
    }

    pub fn confirmed(map: &str) -> Self {
        Self { schema_version: CHIP_SCHEMA, confirmed_map: Some(map.to_string()) }
    }
}

/// Reads `chip.toml`. A missing file is unconfirmed.
pub fn load_chip_state(path: &Path) -> Result<ChipState, StoreError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ChipState::unconfirmed()),
        Err(source) => return Err(StoreError::Io { path: path.to_path_buf(), source }),
    };
    let state: ChipState = toml::from_str(&text)
        .map_err(|e| StoreError::Parse { path: path.to_path_buf(), message: e.to_string() })?;
    if state.schema_version != CHIP_SCHEMA {
        return Err(StoreError::Schema {
            path: path.to_path_buf(),
            found: state.schema_version,
            expected: CHIP_SCHEMA,
        });
    }
    Ok(state)
}

pub fn save_chip_state(path: &Path, state: &ChipState) -> Result<(), StoreError> {
    let text = toml::to_string_pretty(state).map_err(|e| StoreError::Encode("chip.toml", e.to_string()))?;
    write_atomic(path, text.as_bytes())
}

/// One header's state before this service first took it off BIOS control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedHeader {
    /// The chip's manual-mode bit before the first manual write.
    pub manual: bool,
    /// Duty the chip reported then, if it could be read.
    pub duty_percent: Option<u8>,
}

/// `bios-state.json`: written before any manual write, so a restarted service knows which
/// headers it had taken over. Removed once every header is back on BIOS.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiosState {
    pub headers: BTreeMap<String, SavedHeader>,
}

pub fn load_bios_state(path: &Path) -> Result<BiosState, StoreError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| StoreError::Parse { path: path.to_path_buf(), message: e.to_string() }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BiosState::default()),
        Err(source) => Err(StoreError::Io { path: path.to_path_buf(), source }),
    }
}

pub fn save_bios_state(path: &Path, state: &BiosState) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| StoreError::Encode("bios-state.json", e.to_string()))?;
    write_atomic(path, &bytes)
}

pub fn remove_bios_state(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Io { path: path.to_path_buf(), source }),
    }
}
