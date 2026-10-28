//! On-disk state: `config.toml`, `layouts.json`, `zones.json`, `effects.json`, plus
//! snapshots (G10) and the export/import bundle.
//!
//! Every write goes to a temp file in the same directory and is renamed over the target,
//! so a crash mid-write never leaves a half-written file. A file that will not parse is
//! snapshotted aside, logged at ERROR, and replaced by the default.

use canvas_proto::config::{
    ConfigFile, EffectsFile, ExportBundle, FansFile, LayoutsFile, Settings, ZonesFile, CONFIG_FILE,
    EFFECTS_FILE, EFFECTS_SCHEMA, EXPORT_SCHEMA, LAYOUTS_FILE, LAYOUTS_SCHEMA, ZONES_FILE,
    ZONES_SCHEMA, CONFIG_SCHEMA,
};
use canvas_proto::fans::{validate_profile, FanValidationError};
use canvas_proto::layout::Layout;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::displays::{DisplaysFile, DISPLAYS_FILE, DISPLAYS_SCHEMA};
use crate::paths;
use crate::peripherals::{PeripheralsFile, PERIPHERALS_FILE, PERIPHERALS_SCHEMA};

/// Writes `bytes` to `path` atomically (temp file in the same dir, then rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{} has no parent", path.display()))
    })?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("canvas-rgb")
    ));
    // Write then flush: rename must not expose a partially written file.
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        // flush is not fsync: without sync_all a power loss after the rename can leave the
        // target empty. The service's store.rs does the same.
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Writes a JSON document atomically, pretty-printed so the editor can diff it.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(path, &bytes)
}

/// Writes a TOML document atomically.
pub fn write_toml<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let text = toml::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(path, text.as_bytes())
}

/// Reads and parses a JSON or TOML file.
///
/// A missing file yields `Ok(None)`. A file that parses but has the wrong schema version
/// yields `Ok(None)` too: the caller starts from defaults and the file is snapshotted
/// first. A file that does not parse is moved aside as `*.corrupt` and logged at ERROR.
pub fn read_parsed<T: DeserializeOwned + SchemaVersioned>(
    path: &Path,
    schema_version: u32,
    expected: &str,
) -> Result<Option<T>, StorageError> {
    let Some(bytes) = paths::read_optional(path) else {
        return Ok(None);
    };
    let parsed: Result<T, _> = if path.extension().is_some_and(|e| e == "toml") {
        match std::str::from_utf8(&bytes) {
            Ok(text) => toml::from_str(text).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        }
    } else {
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    };
    match parsed {
        Ok(value) => {
            if version_of(&value) != schema_version {
                tracing::warn!(
                    path = %path.display(),
                    found = version_of(&value),
                    expected = schema_version,
                    "schema version mismatch; starting from defaults"
                );
                quarantine(path)?;
                Ok(None)
            } else {
                Ok(Some(value))
            }
        }
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "{expected} did not parse; starting from defaults");
            quarantine(path)?;
            Ok(None)
        }
    }
}

/// A document that carries a schema version.
pub trait SchemaVersioned {
    fn schema_version(&self) -> u32;
}

macro_rules! impl_schema_version {
    ($($t:ty),* $(,)?) => {
        $(impl SchemaVersioned for $t {
            fn schema_version(&self) -> u32 {
                self.schema_version
            }
        })*
    };
}

impl_schema_version!(ConfigFile, LayoutsFile, ZonesFile, EffectsFile, FansFile);

impl SchemaVersioned for DisplaysFile {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

impl SchemaVersioned for PeripheralsFile {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

/// Reads the `schema_version` field of a parsed document, if it has one.
fn version_of<T: SchemaVersioned>(value: &T) -> u32 {
    value.schema_version()
}

/// Moves a file that would not parse aside, so it is not lost but no longer read.
pub fn quarantine(path: &Path) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let stamp = timestamp();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let target = path.with_file_name(format!("{name}.{stamp}.corrupt"));
    std::fs::rename(path, target)
}

/// A sortable timestamp for snapshot folder names: `20261026-141530`.
pub fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since epoch -> civil date (Howard Hinnant's algorithm).
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

/// The daemon's whole user directory, as a snapshot folder under `backups\`.
pub fn snapshot(home: &Path, label: &str) -> std::io::Result<PathBuf> {
    let dir = paths::backups_dir_in(home).join(format!("{}-{}", timestamp(), label));
    std::fs::create_dir_all(&dir)?;
    let keep: &[&str] = &[
        "config.toml",
        "layouts.json",
        "zones.json",
        "effects.json",
        DISPLAYS_FILE,
        PERIPHERALS_FILE,
        "fans.last.toml",
    ];
    for name in keep {
        let src = home.join(name);
        if src.exists() {
            std::fs::copy(&src, dir.join(name))?;
        }
    }
    prune_snapshots_in(&paths::backups_dir_in(home));
    Ok(dir)
}

/// Keeps the newest [`paths::SNAPSHOTS_KEPT`] snapshot folders.
pub fn prune_snapshots_in(backups: &Path) {
    let Ok(entries) = std::fs::read_dir(backups) else {
        return;
    };
    let mut dirs: Vec<(PathBuf, std::time::SystemTime)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), modified))
        })
        .collect();
    dirs.sort_by_key(|(_, modified)| std::cmp::Reverse(*modified));
    for (path, _) in dirs.into_iter().skip(paths::SNAPSHOTS_KEPT) {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// Keeps the newest [`paths::SNAPSHOTS_KEPT`] snapshot folders.
pub fn prune_snapshots() {
    prune_snapshots_in(&paths::backups_dir());
}

/// Everything the daemon keeps, loaded once and written back individually.
#[derive(Debug, Clone)]
pub struct Store {
    pub config: ConfigFile,
    pub layouts: LayoutsFile,
    pub zones: ZonesFile,
    pub effects: EffectsFile,
    /// The numeric display configs (`displays.json`). A display is driven only while it has one.
    pub displays: DisplaysFile,
    /// What the daemon may do with each peripheral (`peripherals.json`). A peripheral is
    /// touched only while it has an entry here.
    pub peripherals: PeripheralsFile,
    /// Where the files live. The daemon uses the user's home; the tests use a temp
    /// directory so they never touch the user's real config.
    pub home: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            config: ConfigFile::default(),
            layouts: default_layouts(),
            zones: ZonesFile { schema_version: ZONES_SCHEMA, sizes: BTreeMap::new() },
            effects: EffectsFile { schema_version: EFFECTS_SCHEMA, ..Default::default() },
            displays: DisplaysFile::default(),
            peripherals: PeripheralsFile::default(),
            home: paths::home(),
        }
    }
}

/// A store that lives in a directory of its own, for the tests and for dev runs.
impl Store {
    /// A store rooted at `home`, with the defaults.
    pub fn at(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into(), ..Store::default() }
    }
}

/// The layouts a fresh install starts from: one empty layout called "Default".
pub fn default_layouts() -> LayoutsFile {
    LayoutsFile {
        schema_version: LAYOUTS_SCHEMA,
        active: "default".into(),
        layouts: vec![Layout {
            id: "default".into(),
            name: "Default".into(),
            render_scale: Default::default(),
            sampling: Default::default(),
            zones: Vec::new(),
            effect: None,
        }],
    }
}

impl Store {
    /// Loads every file from the user's home.
    pub fn load() -> Self {
        Self::load_from(paths::home())
    }

    /// Loads every file from `home`. Missing or broken files fall back to the defaults.
    pub fn load_from(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        let config = home.join(CONFIG_FILE);
        let layouts = home.join(LAYOUTS_FILE);
        let zones = home.join(ZONES_FILE);
        let effects = home.join(EFFECTS_FILE);
        let displays = home.join(DISPLAYS_FILE);
        let peripherals = home.join(PERIPHERALS_FILE);
        let _ = std::fs::create_dir_all(&home);

        let config_value: Option<ConfigFile> =
            read_parsed(&config, CONFIG_SCHEMA, "config.toml").unwrap_or_else(|e| {
                tracing::error!(error = %e, "config.toml could not be read");
                None
            });
        let layouts_value: Option<LayoutsFile> = read_parsed(&layouts, LAYOUTS_SCHEMA, "layouts.json")
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "layouts.json could not be read");
                None
            });
        let zones_value: Option<ZonesFile> = read_parsed(&zones, ZONES_SCHEMA, "zones.json")
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "zones.json could not be read");
                None
            });
        let effects_value: Option<EffectsFile> =
            read_parsed(&effects, EFFECTS_SCHEMA, "effects.json").unwrap_or_else(|e| {
                tracing::error!(error = %e, "effects.json could not be read");
                None
            });
        let displays_value: Option<DisplaysFile> =
            read_parsed(&displays, DISPLAYS_SCHEMA, DISPLAYS_FILE).unwrap_or_else(|e| {
                tracing::error!(error = %e, "displays.json could not be read");
                None
            });
        let peripherals_value: Option<PeripheralsFile> =
            read_parsed(&peripherals, PERIPHERALS_SCHEMA, PERIPHERALS_FILE).unwrap_or_else(|e| {
                tracing::error!(error = %e, "peripherals.json could not be read");
                None
            });

        let mut store = Store {
            config: config_value.unwrap_or_default(),
            layouts: layouts_value.unwrap_or_else(default_layouts),
            zones: zones_value.unwrap_or_else(|| Store::default().zones),
            effects: effects_value.unwrap_or_else(|| Store::default().effects),
            displays: displays_value.unwrap_or_default(),
            peripherals: peripherals_value.unwrap_or_default(),
            home,
        };
        // A config from a newer schema is not silently downgraded.
        if store.config.schema_version != CONFIG_SCHEMA {
            store.config.schema_version = CONFIG_SCHEMA;
        }
        store.ensure_layouts();
        store
    }

    /// Makes sure there is always an active layout to write scale and sampling into.
    pub fn ensure_layouts(&mut self) {
        if self.layouts.layouts.is_empty() {
            self.layouts.layouts = Store::default().layouts.layouts;
        }
        if !self.layouts.layouts.iter().any(|l| l.id == self.layouts.active) {
            self.layouts.active = self.layouts.layouts[0].id.clone();
        }
    }

    /// The active layout, if it exists.
    pub fn active_layout(&self) -> Option<&Layout> {
        self.layouts.layouts.iter().find(|l| l.id == self.layouts.active)
    }

    /// The active layout, mutable.
    pub fn active_layout_mut(&mut self) -> Option<&mut Layout> {
        let id = self.layouts.active.clone();
        self.layouts.layouts.iter_mut().find(|l| l.id == id)
    }

    pub fn save_config(&self) -> std::io::Result<()> {
        write_toml(&self.home.join(CONFIG_FILE), &self.config)
    }

    pub fn save_layouts(&self) -> std::io::Result<()> {
        write_json(&self.home.join(LAYOUTS_FILE), &self.layouts)
    }

    pub fn save_zones(&self) -> std::io::Result<()> {
        write_json(&self.home.join(ZONES_FILE), &self.zones)
    }

    pub fn save_displays(&self) -> std::io::Result<()> {
        crate::displays::save(&self.home, &self.displays)
    }

    pub fn save_peripherals(&self) -> std::io::Result<()> {
        crate::peripherals::save(&self.home, &self.peripherals)
    }

    pub fn save_effects(&self) -> std::io::Result<()> {
        write_json(&self.home.join(EFFECTS_FILE), &self.effects)
    }

    pub fn save_all(&self) -> std::io::Result<()> {
        self.save_config()?;
        self.save_layouts()?;
        self.save_zones()?;
        self.save_effects()?;
        self.save_displays()?;
        self.save_peripherals()
    }
}

/// The last fan config the daemon pushed to the service, for export while it is down.
pub fn save_fans_last(home: &Path, fans: &FansFile) -> std::io::Result<()> {
    write_toml(&home.join("fans.last.toml"), fans)
}

pub fn load_fans_last(home: &Path) -> Option<FansFile> {
    read_parsed(&home.join("fans.last.toml"), canvas_proto::config::FANS_SCHEMA, "fans.last.toml")
        .ok()
        .flatten()
}

/// Builds the export bundle (G10). `fans` comes from the service when it is up, or from
/// the last config the daemon pushed.
pub fn export_bundle(home: &Path, fans: FansFile) -> ExportBundle {
    let store = Store::load_from(home);
    ExportBundle {
        schema_version: EXPORT_SCHEMA,
        exported_at: chrono_like_timestamp(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        settings: store.config.settings.clone(),
        layouts: store.layouts.clone(),
        zones: store.zones.clone(),
        effects: store.effects.clone(),
        fans,
        displays: store.displays.displays.clone(),
    }
}

/// Why an import was refused.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("not a canvas-rgb export: {0}")]
    NotAnExport(String),
    #[error("unsupported export schema {found}, expected {expected}")]
    Schema { found: u32, expected: u32 },
    #[error("fan values are not allowed: {0}")]
    Fans(#[from] FanValidationError),
    #[error("display config refused: {0}")]
    Displays(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Validates an export bundle. Fan values below the floors are refused.
pub fn validate_bundle(bundle: &ExportBundle) -> Result<(), ImportError> {
    if bundle.schema_version != EXPORT_SCHEMA {
        return Err(ImportError::Schema { found: bundle.schema_version, expected: EXPORT_SCHEMA });
    }
    for profile in &bundle.fans.profiles {
        validate_profile(profile, bundle.fans.floor_percent).map_err(ImportError::Fans)?;
    }
    // The same checks as a PUT to the display route, and one config per device.
    for (i, display) in bundle.displays.iter().enumerate() {
        crate::displays::validate(&display.device.0, display).map_err(ImportError::Displays)?;
        if bundle.displays[..i].iter().any(|d| d.device == display.device) {
            return Err(ImportError::Displays(format!("two configs for {}", display.device.0)));
        }
    }
    Ok(())
}

/// Writes a validated bundle over the daemon's own files.
pub fn import_bundle(home: &Path, bundle: &ExportBundle) -> Result<(), ImportError> {
    validate_bundle(bundle)?;
    // A snapshot before the import so a bad import is recoverable (G10).
    let _ = snapshot(home, "pre-import");

    let config = ConfigFile {
        schema_version: CONFIG_SCHEMA,
        settings: bundle.settings.clone(),
        ..ConfigFile::default()
    };

    let mut layouts = bundle.layouts.clone();
    layouts.schema_version = LAYOUTS_SCHEMA;

    let mut zones = bundle.zones.clone();
    zones.schema_version = ZONES_SCHEMA;

    let mut effects = bundle.effects.clone();
    effects.schema_version = EFFECTS_SCHEMA;

    // A bundle from before Poison became a user effect names it `poison`; the ids move
    // to the user effect, as they do on a normal start.
    crate::migration::migrate_poison(&mut layouts, &mut effects);

    let mut fans = bundle.fans.clone();
    fans.schema_version = canvas_proto::config::FANS_SCHEMA;

    // Replaces displays.json like the other files. A bundle from before displays has no list and
    // reads as an empty one, so it clears the saved display configs, as it clears the rest.
    let displays = DisplaysFile { schema_version: DISPLAYS_SCHEMA, displays: bundle.displays.clone() };

    write_toml(&home.join(CONFIG_FILE), &config)?;
    write_json(&home.join(LAYOUTS_FILE), &layouts)?;
    write_json(&home.join(ZONES_FILE), &zones)?;
    write_json(&home.join(EFFECTS_FILE), &effects)?;
    write_json(&home.join(DISPLAYS_FILE), &displays)?;
    // The daemon keeps its own copy so an export while the service is down still works.
    write_toml(&home.join("fans.last.toml"), &fans)?;
    Ok(())
}

/// An ISO-8601-ish timestamp for the export bundle.
pub fn export_timestamp() -> String {
    chrono_like_timestamp()
}

/// An ISO-8601-ish timestamp for the export bundle.
fn chrono_like_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
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
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Reads a bundle from a JSON file.
pub fn read_bundle(path: &Path) -> Result<ExportBundle, ImportError> {
    let bytes = std::fs::read(path)?;
    let bundle: ExportBundle = serde_json::from_slice(&bytes).map_err(|e| ImportError::NotAnExport(e.to_string()))?;
    validate_bundle(&bundle)?;
    Ok(bundle)
}

/// A storage failure.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
}

impl From<StorageError> for std::io::Error {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::Io(e) => e,
            StorageError::Parse(m) => std::io::Error::new(std::io::ErrorKind::InvalidData, m),
        }
    }
}

/// The default settings a fresh install starts from.
pub fn default_settings() -> Settings {
    Settings::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_content() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-storage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        // No temp file is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_quarantined_and_defaults_are_used() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, b"this is not toml {{{").unwrap();
        let value: Option<ConfigFile> = read_parsed(&path, CONFIG_SCHEMA, "config.toml").unwrap();
        assert!(value.is_none());
        assert!(!path.exists(), "the corrupt file is moved aside");
        let quarantined: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".corrupt"))
            .collect();
        assert_eq!(quarantined.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timestamp_is_sortable_and_well_formed() {
        let ts = timestamp();
        assert_eq!(ts.len(), 15, "{ts}");
        assert!(ts.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
