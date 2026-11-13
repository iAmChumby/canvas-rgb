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
use canvas_proto::fans::{validate_hot_limits, validate_profile, FanValidationError, HotLimitError};
use canvas_proto::layout::Layout;
use canvas_proto::scene::{
    LightsFile, ScenesFile, LIGHTS_FILE, LIGHTS_SCHEMA, SCENES_FILE, SCENES_SCHEMA,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use canvas_proto::device::DeviceInfo;

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
    parse_bytes(path, &bytes, schema_version, expected)
}

/// The parse half of [`read_parsed`], for bytes already read: a file that parses and has the
/// right schema yields `Ok(Some)`; one that does not is moved aside and yields `Ok(None)`.
fn parse_bytes<T: DeserializeOwned + SchemaVersioned>(
    path: &Path,
    bytes: &[u8],
    schema_version: u32,
    expected: &str,
) -> Result<Option<T>, StorageError> {
    let parsed: Result<T, _> = if path.extension().is_some_and(|e| e == "toml") {
        match std::str::from_utf8(bytes) {
            Ok(text) => toml::from_str(text).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        }
    } else {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
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

/// What reading a file that must not be mistaken for an absent one found.
enum Loaded<T> {
    /// The file does not exist.
    Absent,
    /// The file exists but could not be read (or could not be moved aside): it stays where it is
    /// and nothing is written over it.
    Unreadable,
    /// The file did not parse or had the wrong schema, and was moved aside as `*.corrupt`.
    Quarantined,
    Parsed(T),
}

/// [`read_parsed`] for the scenes files: tells absent from unreadable, and logs the path of a
/// read error. [`paths::read_optional`], which maps every error to "absent", stays as it is for
/// the other files.
fn load_checked<T: DeserializeOwned + SchemaVersioned>(
    path: &Path,
    schema_version: u32,
    expected: &str,
) -> Loaded<T> {
    let bytes = match paths::read_checked(path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Loaded::Absent,
        Err(e) => {
            tracing::error!(
                path = %path.display(), error = %e,
                "{expected} exists but could not be read; it is left alone and nothing is written over it"
            );
            return Loaded::Unreadable;
        }
    };
    match parse_bytes(path, &bytes, schema_version, expected) {
        Ok(Some(value)) => Loaded::Parsed(value),
        Ok(None) => Loaded::Quarantined,
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "{expected} could not be moved aside; it is left alone");
            Loaded::Unreadable
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

impl_schema_version!(ConfigFile, LayoutsFile, ZonesFile, EffectsFile, FansFile, LightsFile, ScenesFile);

impl SchemaVersioned for canvas_proto::stack::StackFile {
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

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

/// The newest `<name>.<stamp>.corrupt` in `home`, for a log line; the name itself when none is found.
fn quarantined_name(home: &Path, name: &str) -> String {
    let prefix = format!("{name}.");
    let mut found: Vec<String> = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&prefix) && n.ends_with(".corrupt"))
        .collect();
    found.sort();
    found.pop().unwrap_or_else(|| name.to_string())
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
        // The scenes files (§ review 323 B1): an import writes both, so a bundle that lacks a
        // split light or carries damaged scenes must leave the old ones recoverable.
        LIGHTS_FILE,
        SCENES_FILE,
        canvas_proto::stack::STACKS_FILE,
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
    /// The hardware map (`lights.json`): one light per zone, or per run of one. Scenes (0.14.0).
    pub lights: LightsFile,
    /// The scenes (`scenes.json`): named sets of groups, one active. Migrated from `layouts`
    /// once, when this file is absent; the layouts stay as the old path reads them until Task 12.
    pub scenes: ScenesFile,
    /// Set while the scenes are held in memory only: `scenes.json` was absent (or quarantined),
    /// so the first device list migrates the layouts (or makes the first-run scene) with the real
    /// devices and writes both files ([`Store::finish_scenes`]). Nothing is written before that.
    pub scenes_pending: Option<PendingScenes>,
    /// `lights.json` exists but could not be read at load. It is never written over.
    pub lights_blocked: bool,
    /// `scenes.json` (or the `layouts.json` it would migrate from) exists but could not be read at
    /// load. It is never written over.
    pub scenes_blocked: bool,
    /// A failed save of `lights.json` after new lights were found is logged once (while the
    /// migration is pending every list would otherwise repeat it); a success clears it.
    pub lights_save_warned: bool,
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

/// How long a held migration waits, after the first non-empty device list, for every device the
/// layouts place to show up. A keyboard whose HID discover fails is left out of the first list
/// (`composite.rs`), and a zone of an absent device can only be guessed from the layout; the wait
/// gives it a chance to arrive first. After it, the migration runs with the devices present.
pub const SCENES_DEVICE_WAIT: Duration = Duration::from_secs(60);

/// A scenes migration waiting for the first device list.
#[derive(Debug, Clone)]
pub struct PendingScenes {
    /// The saved layouts to migrate (Poison already renamed), or `None` for a first run.
    pub layouts: Option<LayoutsFile>,
    /// The lights `lights.json` held at load (empty when it was absent): they keep their
    /// settings through the migration. The in-memory placeholder lights are not these.
    pub held_lights: LightsFile,
    /// The `before-scenes` snapshot is taken once, however many lists it takes to write.
    pub snapshotted: bool,
    /// When the first non-empty device list arrived: the wait for the placed devices runs from it.
    pub first_list_at: Option<Instant>,
    /// The wait for a placed device is logged once, not on every list.
    pub logged_waiting: bool,
    /// A timer to settle the wait has been started. The supervisor sends a list only when the
    /// devices change, so a device that stays absent sends none: without the timer the wait would
    /// never end.
    pub deadline_armed: bool,
}

impl PendingScenes {
    /// The ids of the devices the layouts place, in first-seen order.
    fn placed_devices(&self) -> Vec<&canvas_proto::DeviceId> {
        let mut ids: Vec<&canvas_proto::DeviceId> = Vec::new();
        for layout in self.layouts.iter().flat_map(|l| &l.layouts) {
            for placed in &layout.zones {
                if !ids.contains(&&placed.device_id) {
                    ids.push(&placed.device_id);
                }
            }
        }
        ids
    }
}

impl Default for Store {
    fn default() -> Self {
        Self {
            config: ConfigFile::default(),
            layouts: default_layouts(),
            lights: crate::scenes::empty_lights(),
            scenes: crate::scenes::default_scenes(&crate::scenes::empty_lights()),
            scenes_pending: None,
            lights_blocked: false,
            scenes_blocked: false,
            lights_save_warned: false,
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
            case_form: canvas_proto::layout::CaseForm::None,
            landmarks: Vec::new(),
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
            read_parsed(&config, CONFIG_SCHEMA, "config.toml")
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "config.toml could not be read");
                    None
                })
                .map(|mut c: ConfigFile| {
                    clamp_settings(&mut c.settings);
                    c
                });
        let layouts_existed = layouts.exists();
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

        let lights_value: Loaded<LightsFile> = load_checked(&home.join(LIGHTS_FILE), LIGHTS_SCHEMA, LIGHTS_FILE);
        let scenes_value: Loaded<ScenesFile> = load_checked(&home.join(SCENES_FILE), SCENES_SCHEMA, SCENES_FILE);
        // A layouts.json that is still there but gave nothing could not be read (a parse failure
        // moved it aside, so it is gone): the scenes are not migrated from a file we cannot see.
        let layouts_unreadable = layouts_value.is_none() && layouts.exists();
        // One that was there, gave nothing and is gone now was moved aside by this load.
        let layouts_quarantined = layouts_existed && layouts_value.is_none() && !layouts.exists();

        let mut store = Store {
            config: config_value.unwrap_or_default(),
            lights: crate::scenes::empty_lights(),
            scenes: Store::default().scenes,
            scenes_pending: None,
            lights_blocked: false,
            scenes_blocked: false,
            lights_save_warned: false,
            layouts: layouts_value.clone().unwrap_or_else(default_layouts),
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
        store.load_scenes(lights_value, scenes_value, layouts_value.as_ref(), layouts_unreadable, layouts_quarantined);
        store
    }

    /// Settles `lights` and `scenes` after the files are read. Writes at most a repair of a
    /// `scenes.json` that read fine; never `layouts.json`.
    ///
    /// - A file that exists but could not be read is not absent: it is left alone, the store runs
    ///   on an in-memory default, and nothing is written to it (see `lights_blocked`,
    ///   `scenes_blocked`).
    /// - `scenes.json` read fine: it is repaired (see [`crate::scenes::repair`]) and the repair is
    ///   saved.
    /// - `scenes.json` is absent or was quarantined: nothing is written here. The migration is held
    ///   in memory ([`PendingScenes`]: from `layouts.json` when it was read, else the first-run
    ///   scene) and runs on the first device list, so the real devices decide arrangements and
    ///   zero-LED zones ([`Store::finish_scenes`]).
    /// - `layouts.json` was quarantined by this load (it did not parse): the migration is the
    ///   first-run one, and the log names the moved-aside file and says the scenes start fresh.
    fn load_scenes(
        &mut self,
        lights: Loaded<LightsFile>,
        scenes: Loaded<ScenesFile>,
        layouts: Option<&LayoutsFile>,
        layouts_unreadable: bool,
        layouts_quarantined: bool,
    ) {
        match lights {
            Loaded::Parsed(lights) => self.lights = lights,
            Loaded::Unreadable => self.lights_blocked = true,
            Loaded::Absent | Loaded::Quarantined => {}
        }
        match scenes {
            Loaded::Parsed(mut scenes) => {
                let changes = crate::scenes::repair(&mut scenes, &self.lights);
                for change in &changes {
                    tracing::warn!("scenes.json was repaired: {change}");
                }
                self.scenes = scenes;
                if !changes.is_empty() {
                    crate::logging::warn_if_err("the repaired scenes.json could not be saved", self.save_scenes());
                }
            }
            Loaded::Unreadable => {
                self.scenes_blocked = true;
                self.scenes = crate::scenes::default_scenes(&self.lights);
            }
            Loaded::Absent | Loaded::Quarantined if layouts_unreadable || self.lights_blocked => {
                self.scenes_blocked = true;
                self.scenes = crate::scenes::default_scenes(&self.lights);
            }
            Loaded::Absent | Loaded::Quarantined => {
                if layouts_quarantined {
                    tracing::warn!(
                        file = %quarantined_name(&self.home, LAYOUTS_FILE),
                        "layouts.json was moved aside because it could not be used, so the scenes start fresh from the first-run default; the old file is kept"
                    );
                }
                // A layouts file from before Poison became a user effect names it `poison`; the
                // scenes are built from the ids the daemon uses today. Only the layouts half of
                // `migrate_poison` matters here: `main` runs `migrate_store` on the real store,
                // which moves the saved props, presets and favourites, so the effects half is
                // thrown away on purpose.
                let layouts = layouts.map(|l| {
                    let mut l = l.clone();
                    crate::migration::migrate_poison(&mut l, &mut EffectsFile::default());
                    l
                });
                let held_lights = self.lights.clone();
                let (lights, scenes) = Self::build_scenes(&held_lights, layouts.as_ref(), &[]);
                self.lights = lights;
                self.scenes = scenes;
                self.scenes_pending = Some(PendingScenes {
                    layouts,
                    held_lights,
                    snapshotted: false,
                    first_list_at: None,
                    logged_waiting: false,
                    deadline_armed: false,
                });
            }
        }
    }

    /// The lights and scenes a migration makes: the lights already held, plus the ones the
    /// layouts (or, on a first run, the devices) bring.
    ///
    /// A migrated light replaces a held one of the same id (in the held one's place). While the
    /// migration is pending a held light is a placeholder: `lights.json` as `sync_lights` left it,
    /// with a default arrangement and no label, so the layout's label, arrangement and `reversed`
    /// win. A light the OWNER changed cannot exist yet: there is no scene API until Task 6, so
    /// nothing edits a light before the files are written. (Task 6 must revisit this if a pending
    /// store can ever hold an edit.)
    ///
    /// An import with lights and no scenes also comes through here (bundle → migration): the
    /// bundle's lights are what it holds, so a split light's label, slice and `reversed` flag are
    /// not lost to a migration that can only re-derive whole-zone lights.
    pub(crate) fn build_scenes(
        held: &LightsFile,
        layouts: Option<&LayoutsFile>,
        devices: &[DeviceInfo],
    ) -> (LightsFile, ScenesFile) {
        let (migrated, scenes) = crate::scene_migration::migrate(layouts, devices);
        let mut lights = held.clone();
        for light in migrated.lights {
            match lights.lights.iter_mut().find(|l| l.id == light.id) {
                Some(held) => *held = light,
                None => lights.lights.push(light),
            }
        }
        let scenes = match layouts {
            Some(_) => scenes,
            // The first-run scene holds every light, the ones already held included.
            None => crate::scenes::default_scenes(&lights),
        };
        (lights, scenes)
    }

    /// Runs the held scenes migration with the first device list: migrates with the real devices,
    /// takes the `before-scenes` snapshot, writes `lights.json` and `scenes.json`, and clears
    /// `scenes_pending`. Returns true when the files were written. Never writes `layouts.json`.
    ///
    /// An empty list waits for the next one: with no devices there are no lights to group, and a
    /// first-run `Everything` made now would hold none. A failed write keeps the migration
    /// pending: the next list tries again, and so does the wait's timer (`device_sync`), every
    /// 30 s, when no new list comes.
    ///
    /// It also waits until every device the layouts place has appeared in a list, or until
    /// [`SCENES_DEVICE_WAIT`] has passed since the first non-empty list (the clock is the `now` of
    /// [`Store::finish_scenes_at`]). After the wait it migrates with the devices present, and a zone
    /// of an absent device becomes a light from the layout's own data: its label, its shape (as
    /// `arrangement_of`: a ring stays a ring, a grid shape is a grid, a point is a single, anything
    /// else a strip), `reversed` and slice. It has LEDs unless the layout gives it a slice of no
    /// LEDs: the layouts only place a zone that had LEDs when it was placed (`first_run.rs`
    /// skips `led_count == 0`), so a whole-zone placement is the evidence that it has some. The
    /// layout does not record a zone's kind, so an absent device's matrix or single zone placed
    /// `Auto` becomes a strip; the light keeps it until the owner changes it in Devices (a single
    /// LED reads the same point either way).
    ///
    /// Refuses while `lights.json` or `scenes.json` is blocked (could not be read at load).
    pub fn finish_scenes(&mut self, devices: &[DeviceInfo]) -> bool {
        self.finish_scenes_at(devices, Instant::now())
    }

    /// While the migration waits for a placed device, the time left on that wait, once: the caller
    /// starts one timer that runs [`Store::finish_scenes`] again when it ends. `None` when nothing
    /// waits (no migration held, no list yet) or the timer is already running.
    pub fn scenes_deadline_to_arm(&mut self, now: Instant) -> Option<Duration> {
        let pending = self.scenes_pending.as_mut()?;
        let first = pending.first_list_at?;
        if pending.deadline_armed {
            return None;
        }
        pending.deadline_armed = true;
        Some(SCENES_DEVICE_WAIT.saturating_sub(now.saturating_duration_since(first)))
    }

    /// [`Store::finish_scenes`] with the clock passed in, so a test drives the wait.
    pub fn finish_scenes_at(&mut self, devices: &[DeviceInfo], now: Instant) -> bool {
        if self.lights_blocked || self.scenes_blocked || devices.is_empty() {
            return false;
        }
        let Some(pending) = self.scenes_pending.as_mut() else {
            return false;
        };
        let first_list_at = *pending.first_list_at.get_or_insert(now);
        let absent: Vec<String> = pending
            .placed_devices()
            .into_iter()
            .filter(|id| !devices.iter().any(|d| &d.id == *id))
            .map(|id| id.0.clone())
            .collect();
        if !absent.is_empty() {
            let waited = now.saturating_duration_since(first_list_at);
            if waited < SCENES_DEVICE_WAIT {
                if !pending.logged_waiting {
                    pending.logged_waiting = true;
                    tracing::info!(
                        absent = ?absent,
                        wait_secs = SCENES_DEVICE_WAIT.as_secs(),
                        "the scenes wait for the devices the layouts place before they migrate"
                    );
                }
                return false;
            }
            tracing::warn!(
                absent = ?absent,
                "these devices never showed up; their zones are migrated from the layouts alone"
            );
        }
        if !pending.snapshotted {
            pending.snapshotted = true;
            crate::logging::warn_if_err(
                "the snapshot before the scenes migration could not be taken; it goes on without one",
                snapshot(&self.home, "before-scenes"),
            );
        }
        let layouts = pending.layouts.clone();
        let (lights, scenes) = Self::build_scenes(&pending.held_lights, layouts.as_ref(), devices);
        let wrote_lights = crate::logging::warn_if_err(
            "lights.json could not be written",
            write_json(&self.home.join(LIGHTS_FILE), &lights),
        );
        let wrote_scenes = crate::logging::warn_if_err(
            "scenes.json could not be written",
            write_json(&self.home.join(SCENES_FILE), &scenes),
        );
        tracing::info!(
            from_layouts = layouts.as_ref().map_or(0, |l| l.layouts.len()),
            lights = lights.lights.len(),
            scenes = scenes.scenes.len(),
            "the scenes were made from the first device list"
        );
        self.lights = lights;
        self.scenes = scenes;
        if wrote_lights.is_some() && wrote_scenes.is_some() {
            self.scenes_pending = None;
            true
        } else {
            false
        }
    }

    /// Makes sure there is always an active layout to write scale and sampling into.
    /// Adds a light for every zone with LEDs that has none, and saves when any was added. A save
    /// that fails (the migration is pending, or a file is blocked) is logged once, not on every
    /// device list.
    pub fn sync_lights_from(&mut self, devices: &[DeviceInfo]) -> usize {
        let new_lights = crate::scenes::sync_lights(&mut self.lights, devices);
        if !new_lights.is_empty() {
            tracing::info!(added = new_lights.len(), "new lights were added");
            match self.save_lights() {
                Ok(()) => self.lights_save_warned = false,
                Err(e) if !self.lights_save_warned => {
                    self.lights_save_warned = true;
                    tracing::warn!(error = %e, "could not save the lights (said once until a save works)");
                }
                Err(_) => {}
            }
        }
        new_lights.len()
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

    /// Refuses while the file could not be read at load, or while the scenes are only in memory
    /// ([`Store::finish_scenes`] writes them first).
    ///
    /// Also refuses while `scenes.json` is blocked: a session that cannot read the scenes holds
    /// whole-zone lights only, and writing them would drop split members on the next load.
    pub fn save_lights(&self) -> std::io::Result<()> {
        self.may_write_scenes_files(LIGHTS_FILE, self.lights_blocked || self.scenes_blocked)?;
        write_json(&self.home.join(LIGHTS_FILE), &self.lights)
    }

    /// Refuses while the file could not be read at load, or while the scenes are only in memory
    /// ([`Store::finish_scenes`] writes them first).
    pub fn save_scenes(&self) -> std::io::Result<()> {
        self.may_write_scenes_files(SCENES_FILE, self.scenes_blocked)?;
        write_json(&self.home.join(SCENES_FILE), &self.scenes)
    }

    fn may_write_scenes_files(&self, name: &str, blocked: bool) -> std::io::Result<()> {
        if blocked {
            return Err(std::io::Error::other(format!(
                "{name} (or the scenes it belongs with) could not be read at start, so it is not written"
            )));
        }
        if self.scenes_pending.is_some() {
            return Err(std::io::Error::other(format!(
                "{name} waits for the first device list before it is written"
            )));
        }
        Ok(())
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
    let (lights, scenes) = scenes_for_export(&store);
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
        stacks: crate::stacks::read_file(home).stacks,
        lights,
        scenes,
    }
}

/// The lights and scenes a bundle carries: only what the store really holds.
///
/// - A file that could not be read at load (`lights_blocked`, `scenes_blocked`) is left out. The
///   in-memory value beside it is a default (an empty lights file, a one-`Everything` scenes file),
///   and an import would write that default over the target's real data: a fabricated scene or
///   whole-zone-only lights are never exported. The other file still goes in where it is real.
/// - While the migration is pending, `scenes.json` does not exist, so no scenes are exported. The
///   lights do: `lights.json` as it was held at load, which the importing machine holds through its
///   own migration (a first run with no file yet exports neither).
/// - Otherwise both are the store's own.
pub fn scenes_for_export(store: &Store) -> (Option<LightsFile>, Option<ScenesFile>) {
    let lights = if store.lights_blocked {
        None
    } else if let Some(pending) = &store.scenes_pending {
        (!pending.held_lights.lights.is_empty()).then(|| pending.held_lights.clone())
    } else {
        Some(store.lights.clone())
    };
    let scenes = if store.scenes_blocked || store.scenes_pending.is_some() {
        None
    } else {
        Some(store.scenes.clone())
    };
    (lights, scenes)
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
    #[error("hot limits are not allowed: {0}")]
    HotLimits(#[from] HotLimitError),
    #[error("display config refused: {0}")]
    Displays(String),
    #[error("settings refused: {0}")]
    Settings(String),
    #[error("scenes refused: {0}")]
    Scenes(String),
    #[error("a file the import would replace could not be read: {0}")]
    Blocked(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Holds settings read from disk to the ranges [`validate_settings`] checks (#50): a
/// hand-edited `config.toml` with `warmth = 100` would otherwise report one thing and apply
/// another, and every whole-object settings PUT from the editor's snapshot would then be
/// refused. Each value out of range is logged and moved to the nearest end.
pub fn clamp_settings(settings: &mut canvas_proto::config::Settings) {
    use canvas_proto::consts::{FPS_MAX, FPS_MIN, WARMTH_MIN, WARMTH_OFF};
    let fps = settings.fps.clamp(FPS_MIN, FPS_MAX);
    let brightness = settings.brightness.min(100);
    let warmth = settings.warmth.clamp(WARMTH_MIN, WARMTH_OFF);
    if (fps, brightness, warmth) != (settings.fps, settings.brightness, settings.warmth) {
        tracing::warn!(
            fps = settings.fps, brightness = settings.brightness, warmth = settings.warmth,
            "config.toml held settings out of range; they were moved into range"
        );
    }
    settings.fps = fps;
    settings.brightness = brightness;
    settings.warmth = warmth;
}

/// The ranges `PUT /api/v1/settings` and a config import hold `Settings` to, so what is
/// stored is what the engine applies.
pub fn validate_settings(settings: &canvas_proto::config::Settings) -> Result<(), String> {
    use canvas_proto::consts::{FPS_MAX, FPS_MIN, WARMTH_MIN, WARMTH_OFF};
    if !(FPS_MIN..=FPS_MAX).contains(&settings.fps) {
        return Err(format!("fps must be {FPS_MIN}..={FPS_MAX}, got {}", settings.fps));
    }
    if settings.brightness > 100 {
        return Err(format!("brightness must be 0..100, got {}", settings.brightness));
    }
    if !(WARMTH_MIN..=WARMTH_OFF).contains(&settings.warmth) {
        return Err(format!("warmth must be {WARMTH_MIN}..={WARMTH_OFF} K, got {}", settings.warmth));
    }
    Ok(())
}

/// Validates an export bundle. Fan values below the floors are refused.
pub fn validate_bundle(bundle: &ExportBundle) -> Result<(), ImportError> {
    validate_bundle_rated(bundle, &[], None, None)
}

/// Validates an export bundle, with the rated maxima the service last reported
/// (issue #130: the hot limits are checked against the real parts' ceilings when
/// the daemon knows them, exactly as `put_fan_hot` does). With no status known,
/// the spine's fallback ceilings apply and the service validates again with the
/// real ones on receipt.
pub fn validate_bundle_rated(
    bundle: &ExportBundle,
    devices: &[DeviceInfo],
    rated_cpu_max_c: Option<f32>,
    rated_gpu_max_c: Option<f32>,
) -> Result<(), ImportError> {
    if bundle.schema_version != EXPORT_SCHEMA {
        return Err(ImportError::Schema { found: bundle.schema_version, expected: EXPORT_SCHEMA });
    }
    validate_settings(&bundle.settings).map_err(ImportError::Settings)?;
    for profile in &bundle.fans.profiles {
        validate_profile(profile, bundle.fans.floor_percent).map_err(ImportError::Fans)?;
    }
    validate_hot_limits(&bundle.fans.hot_limits, rated_cpu_max_c, rated_gpu_max_c).map_err(ImportError::HotLimits)?;
    // The same checks as a PUT to the display route, and one config per device.
    for (i, display) in bundle.displays.iter().enumerate() {
        crate::displays::validate(&display.device.0, display).map_err(ImportError::Displays)?;
        if bundle.displays[..i].iter().any(|d| d.device == display.device) {
            return Err(ImportError::Displays(format!("two configs for {}", display.device.0)));
        }
    }
    validate_bundle_scenes(bundle, devices)?;
    Ok(())
}

/// Refuses a bundle whose scenes cannot be made valid. The bundle's scenes are repaired on the
/// way in (a light in two groups, an empty group, an `active` that names nothing), and what the
/// repair cannot fix is a validation error here, before anything is written.
fn validate_bundle_scenes(bundle: &ExportBundle, devices: &[DeviceInfo]) -> Result<(), ImportError> {
    let Some(scenes) = &bundle.scenes else {
        return Ok(());
    };
    let (lights, scenes, _) = prepared_scenes(bundle.lights.as_ref(), scenes, devices);
    if !scenes.scenes.iter().any(|s| s.id == scenes.active) {
        return Err(ImportError::Scenes(format!("the active scene {} names no scene", scenes.active)));
    }
    for scene in &scenes.scenes {
        canvas_proto::scene::validate_scene(scene, &lights.lights)
            .map_err(|e| ImportError::Scenes(format!("scene {}: {e}", scene.id)))?;
    }
    Ok(())
}

/// The lights and scenes a bundle's `scenes` half is written as: the bundle's own, with the schema
/// versions this daemon writes, repaired (see [`crate::scenes::repair`]) so every scene passes
/// [`canvas_proto::scene::validate_scene`]. Returns what the repair changed, for the log.
///
/// A bundle with scenes and no (or an empty) lights file is one exported while its `lights.json`
/// was missing: the lights are rebuilt from `devices` with [`crate::scenes::sync_lights`] first, so
/// the scenes' whole-zone members are known lights again and the repair does not drop them. A
/// member the devices do not bring back is still dropped, and the scenes are refused only if that
/// leaves one invalid.
fn prepared_scenes(
    lights: Option<&LightsFile>,
    scenes: &ScenesFile,
    devices: &[DeviceInfo],
) -> (LightsFile, ScenesFile, Vec<String>) {
    let mut lights = lights.cloned().unwrap_or_else(crate::scenes::empty_lights);
    if lights.lights.is_empty() && !devices.is_empty() {
        crate::scenes::sync_lights(&mut lights, devices);
    }
    lights.schema_version = LIGHTS_SCHEMA;
    let mut scenes = scenes.clone();
    scenes.schema_version = SCENES_SCHEMA;
    let changes = crate::scenes::repair(&mut scenes, &lights);
    (lights, scenes, changes)
}

/// Refuses an import while `lights.json` or `scenes.json` exists but cannot be read.
///
/// That is exactly the state [`Store::load_from`] holds as blocked (`lights_blocked`,
/// `scenes_blocked`): the store promises never to write over such a file, and the import writes
/// both. The check reads nothing and moves nothing; it only asks whether the file can be read.
fn refuse_unreadable_scenes_files(home: &Path) -> Result<(), ImportError> {
    for name in [LIGHTS_FILE, SCENES_FILE] {
        if let Err(e) = paths::read_checked(&home.join(name)) {
            return Err(ImportError::Blocked(format!("{name} could not be read ({e})")));
        }
    }
    Ok(())
}

/// Writes a validated bundle over the daemon's own files. `devices` is the daemon's current
/// device list, which a 0.13 bundle's layouts are migrated with.
pub fn import_bundle(
    home: &Path,
    bundle: &ExportBundle,
    devices: &[DeviceInfo],
) -> Result<(), ImportError> {
    import_bundle_rated(home, bundle, devices, None, None)
}

/// Like [`import_bundle`], with the rated maxima the service last reported for the
/// hot-limit check. The import route passes them from the last fans status when known.
pub fn import_bundle_rated(
    home: &Path,
    bundle: &ExportBundle,
    devices: &[DeviceInfo],
    rated_cpu_max_c: Option<f32>,
    rated_gpu_max_c: Option<f32>,
) -> Result<(), ImportError> {
    // The store never writes over a scenes file it could not read at load (`lights_blocked`,
    // `scenes_blocked`); an import must not either. It refuses here, before the snapshot and
    // before the first write, so a machine with a locked or unreadable file is left as it was
    // instead of half imported (§ review 323 B1).
    refuse_unreadable_scenes_files(home)?;
    validate_bundle_rated(bundle, devices, rated_cpu_max_c, rated_gpu_max_c)?;
    // A snapshot before the import so a bad import is recoverable (G10).
    crate::logging::warn_if_err("the pre-import snapshot could not be taken; the import goes on without one", snapshot(home, "pre-import"));

    // Endpoint ids are machine-specific: an import keeps this machine's
    // current audio_source, whatever the bundle came from (#166).
    // The same goes for the white balance (#50): it is how this machine's LEDs mix, by device.
    let current = read_parsed::<ConfigFile>(&home.join(CONFIG_FILE), CONFIG_SCHEMA, "config.toml")
        .unwrap_or(None)
        .unwrap_or_default();

    let config = ConfigFile {
        schema_version: CONFIG_SCHEMA,
        settings: bundle.settings.clone(),
        audio_source: current.audio_source,
        white_balance: current.white_balance,
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

    // The scenes (0.14.0). A bundle that carries them replaces `lights.json` and `scenes.json`,
    // repaired so every scene passes `validate_scene`; a 0.13 bundle carries layouts only and is
    // migrated with THIS machine's current devices, so a matrix zone placed `Auto` becomes a Grid
    // here and a Strip on a machine that cannot see the keyboard. A bundle with lights and no
    // scenes still has its own hardware map: those lights are held through the migration, as the
    // pending migration holds `lights.json` (labels, slices and `reversed` survive). The scenes
    // code path below writes nothing to `layouts.json`: the old layout import still writes it from
    // the bundle until Task 12, exactly as it did before.
    let (lights, scenes, repairs) = match &bundle.scenes {
        Some(scenes) => prepared_scenes(bundle.lights.as_ref(), scenes, devices),
        None => {
            let held = bundle.lights.clone().unwrap_or_else(crate::scenes::empty_lights);
            let (lights, scenes) = Store::build_scenes(&held, Some(&layouts), devices);
            (lights, scenes, Vec::new())
        }
    };
    for change in &repairs {
        tracing::warn!("the imported scenes.json was repaired: {change}");
    }

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
    write_json(&home.join(LIGHTS_FILE), &lights)?;
    write_json(&home.join(SCENES_FILE), &scenes)?;
    // The stacks replace the saved ones like the other files; a bundle from before stacks has
    // none and leaves none behind. The route validates them before it gets here.
    crate::stacks::write_file(home, &bundle.stacks)?;
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

    /// #166: `ConfigFile.audio_source` round-trips through TOML, both ways.
    #[test]
    fn config_file_audio_source_round_trips_through_toml() {
        use canvas_proto::audio::{AudioFlow, AudioSource};
        // FollowDefault.
        let mut config = ConfigFile::default();
        let text = toml::to_string_pretty(&config).unwrap();
        let back: ConfigFile = toml::from_str(&text).unwrap();
        assert_eq!(back.audio_source, AudioSource::FollowDefault);
        assert_eq!(back, config);
        // A pinned endpoint.
        config.audio_source = AudioSource::Endpoint {
            id: "{0.0.0.00000000}.{abc}".into(),
            flow: AudioFlow::Capture,
            name_hint: "Microphone".into(),
        };
        let text = toml::to_string_pretty(&config).unwrap();
        let back: ConfigFile = toml::from_str(&text).unwrap();
        assert_eq!(back.audio_source, config.audio_source);
        assert_eq!(back, config);
    }

    /// #166: a config saved before `audio_source` existed loads as FollowDefault.
    #[test]
    fn an_old_config_without_audio_source_loads_as_follow_default() {
        use canvas_proto::audio::AudioSource;
        let text = r#"
schema_version = 1
api_port = 9430
openrgb_port = 6742
first_run_done = false
zone_panel_dismissed = false

[settings]
fps = 30
black_on_lock = false
on_quit = "black"
brightness = 100
power = "running"
"#;
        let config: ConfigFile = toml::from_str(text).unwrap();
        assert_eq!(config.audio_source, AudioSource::FollowDefault);
        // #50: nor the warmth or the white balance: off and empty.
        assert_eq!(config.settings.warmth, canvas_proto::consts::WARMTH_OFF);
        assert!(config.white_balance.is_empty());
    }

    /// #166: `PUT /settings` replaces only `Settings`, so it can never undo a
    /// pick: the store's audio_source survives a whole-object settings write.
    #[test]
    fn a_settings_write_leaves_the_audio_source_alone() {
        use canvas_proto::audio::{AudioFlow, AudioSource};
        let mut config = ConfigFile {
            audio_source: AudioSource::Endpoint {
                id: "pinned-id".into(),
                flow: AudioFlow::Render,
                name_hint: "Speakers".into(),
            },
            ..ConfigFile::default()
        };
        // This is what `PUT /settings` does: it replaces config.settings only.
        let mut next_settings = config.settings.clone();
        next_settings.brightness = 42;
        config.settings = next_settings;
        assert_eq!(
            config.audio_source,
            AudioSource::Endpoint { id: "pinned-id".into(), flow: AudioFlow::Render, name_hint: "Speakers".into() }
        );
        // And `Settings` itself carries no audio_source field to write.
        let settings_json = serde_json::to_string(&config.settings).unwrap();
        assert!(!settings_json.contains("audio_source"), "{settings_json}");
    }

    /// #50: settings read from a hand-edited config.toml are moved into range, and a restart
    /// keeps the warmth and the white balance.
    #[test]
    fn a_config_is_loaded_into_range_and_keeps_warmth_and_balance() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-load-range-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = ConfigFile::default();
        config.settings.warmth = 2700;
        config.white_balance.insert(
            canvas_proto::device::DeviceId("openrgb:kb".into()),
            canvas_proto::color::WhiteBalance { red: 0.8, green: 1.0, blue: 1.0 },
        );
        write_toml(&dir.join(CONFIG_FILE), &config).unwrap();
        let loaded = Store::load_from(&dir);
        assert_eq!(loaded.config.settings.warmth, 2700, "a restart keeps the warmth");
        assert_eq!(loaded.config.white_balance, config.white_balance, "a restart keeps the balance");

        config.settings.warmth = 100;
        config.settings.brightness = 250;
        config.settings.fps = 500;
        write_toml(&dir.join(CONFIG_FILE), &config).unwrap();
        let loaded = Store::load_from(&dir);
        assert_eq!(loaded.config.settings.warmth, canvas_proto::consts::WARMTH_MIN);
        assert_eq!(loaded.config.settings.brightness, 100);
        assert_eq!(loaded.config.settings.fps, canvas_proto::consts::FPS_MAX);
        assert!(validate_settings(&loaded.config.settings).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #50: an import holds the bundle's settings to the same ranges as `PUT /settings`.
    #[test]
    fn an_import_with_out_of_range_settings_is_refused() {
        let bundle = |settings: Settings| ExportBundle {
            schema_version: EXPORT_SCHEMA,
            exported_at: "2026-11-08T00:00:00Z".into(),
            app_version: "0.0.0".into(),
            settings,
            layouts: default_layouts(),
            zones: ZonesFile { schema_version: ZONES_SCHEMA, sizes: BTreeMap::new() },
            effects: EffectsFile { schema_version: EFFECTS_SCHEMA, ..Default::default() },
            fans: canvas_proto::config::FansFile::default(),
            displays: Vec::new(),
            stacks: Vec::new(),
            lights: None,
            scenes: None,
        };
        assert!(validate_bundle(&bundle(Settings::default())).is_ok());
        for bad in [
            Settings { warmth: 100, ..Settings::default() },
            Settings { brightness: 200, ..Settings::default() },
            Settings { fps: 500, ..Settings::default() },
        ] {
            assert!(matches!(validate_bundle(&bundle(bad.clone())), Err(ImportError::Settings(_))), "{bad:?}");
        }
    }

    /// #166: a config import keeps this machine's current audio_source.
    #[test]
    fn a_config_import_keeps_this_machines_audio_source() {
        use canvas_proto::audio::{AudioFlow, AudioSource};
        let dir = std::env::temp_dir().join(format!("canvas-rgb-import-audio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // This machine's current pick.
        let current = ConfigFile {
            audio_source: AudioSource::Endpoint {
                id: "this-machines-id".into(),
                flow: AudioFlow::Render,
                name_hint: "This machine".into(),
            },
            // #50: this machine's white balance, by its own devices.
            white_balance: [(
                canvas_proto::device::DeviceId("royuan:KB".into()),
                canvas_proto::color::WhiteBalance { red: 0.7, green: 1.0, blue: 1.0 },
            )]
            .into_iter()
            .collect(),
            ..ConfigFile::default()
        };
        write_toml(&dir.join(CONFIG_FILE), &current).unwrap();
        // A bundle from another machine (its settings, no audio_source at all).
        let bundle = ExportBundle {
            schema_version: EXPORT_SCHEMA,
            exported_at: "2026-11-05T00:00:00Z".into(),
            app_version: "0.0.0".into(),
            settings: Settings { brightness: 7, ..Settings::default() },
            layouts: default_layouts(),
            zones: ZonesFile { schema_version: ZONES_SCHEMA, sizes: BTreeMap::new() },
            effects: EffectsFile { schema_version: EFFECTS_SCHEMA, ..Default::default() },
            fans: canvas_proto::config::FansFile::default(),
            displays: Vec::new(),
            stacks: Vec::new(),
            lights: None,
            scenes: None,
        };
        import_bundle(&dir, &bundle, &[]).unwrap();
        let loaded = Store::load_from(&dir);
        assert_eq!(loaded.config.settings.brightness, 7, "the import's settings land");
        assert_eq!(
            loaded.config.audio_source,
            AudioSource::Endpoint { id: "this-machines-id".into(), flow: AudioFlow::Render, name_hint: "This machine".into() },
            "the import keeps this machine's pick"
        );
        assert_eq!(loaded.config.white_balance, current.white_balance, "the import keeps this machine's white balance");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_import_hot_limit_is_checked_against_the_known_rated_maximum() {
        use canvas_proto::config::EXPORT_SCHEMA;
        // A 9800X3D's 95 °C maximum allows a 85 °C CPU limit (max settable 88 °C);
        // the 89 °C fallback ceiling (max settable 82 °C) refuses it.
        let fans = canvas_proto::config::FansFile {
            hot_limits: canvas_proto::fans::HotLimits { cpu_c: Some(85.0), gpu_c: None, board_c: None },
            ..Default::default()
        };
        let export = export_bundle(
            &std::env::temp_dir().join(format!("canvas-rgb-import-{}", std::process::id())),
            fans,
        );
        let bundle = ExportBundle { schema_version: EXPORT_SCHEMA, fans: export.fans, ..export };
        assert!(
            validate_bundle_rated(&bundle, &[], Some(95.0), None).is_ok(),
            "85 °C is inside the 95 °C part's settable range"
        );
        assert!(
            validate_bundle_rated(&bundle, &[], None, None).is_err(),
            "85 °C is past the fallback 89 °C ceiling gap"
        );
        assert!(
            validate_bundle(&bundle).is_err(),
            "the fallback path refuses 85 °C"
        );
    }

    /// A temp home of its own for the bundle tests.
    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-bundle-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bundle_zone(index: u32, leds: u32) -> canvas_proto::device::ZoneInfo {
        canvas_proto::device::ZoneInfo {
            index,
            name: format!("Zone {index}"),
            kind: canvas_proto::device::ZoneKind::Linear,
            led_count: leds,
            min_leds: 0,
            max_leds: 64,
            resizable: false,
        }
    }

    fn bundle_device(id: &str, zones: Vec<canvas_proto::device::ZoneInfo>) -> DeviceInfo {
        DeviceInfo {
            id: canvas_proto::DeviceId(id.into()),
            name: id.into(),
            vendor: "v".into(),
            kind: canvas_proto::device::DeviceKind::Other,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones,
            connected: true,
        }
    }

    /// A board with a 30-LED strip and a 20-LED strip.
    fn bundle_board() -> DeviceInfo {
        bundle_device("openrgb:board", vec![bundle_zone(0, 30), bundle_zone(1, 20)])
    }

    /// A keyboard with one 2-LED matrix zone.
    fn bundle_kb() -> DeviceInfo {
        let mut kb = bundle_device(
            "openrgb:kb",
            vec![canvas_proto::device::ZoneInfo {
                kind: canvas_proto::device::ZoneKind::Matrix {
                    width: 2,
                    height: 1,
                    map: vec![Some(0), Some(1)],
                },
                ..bundle_zone(0, 2)
            }],
        );
        kb.adapter_index = 1;
        kb
    }

    fn bundle_placed(device: &str, zone: u32) -> canvas_proto::layout::PlacedZone {
        canvas_proto::layout::PlacedZone {
            device_id: canvas_proto::DeviceId(device.into()),
            zone_index: zone,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rotation_deg: 0.0,
            reversed: false,
            visible: true,
            shape: canvas_proto::layout::ZoneShape::Auto,
            label: None,
            leds: None,
            part: None,
            locked: false,
        }
    }

    fn bundle_layout(id: &str, name: &str, zones: Vec<canvas_proto::layout::PlacedZone>) -> canvas_proto::layout::Layout {
        canvas_proto::layout::Layout {
            id: id.into(),
            name: name.into(),
            render_scale: Default::default(),
            sampling: Default::default(),
            zones,
            effect: None,
            case_form: canvas_proto::layout::CaseForm::None,
            landmarks: Vec::new(),
        }
    }

    fn bundle_of(
        layouts: LayoutsFile,
        lights: Option<LightsFile>,
        scenes: Option<ScenesFile>,
    ) -> ExportBundle {
        ExportBundle {
            schema_version: EXPORT_SCHEMA,
            exported_at: "2026-11-12T00:00:00Z".into(),
            app_version: "0.13.0".into(),
            settings: Settings::default(),
            layouts,
            zones: ZonesFile { schema_version: ZONES_SCHEMA, sizes: BTreeMap::new() },
            effects: EffectsFile { schema_version: EFFECTS_SCHEMA, ..Default::default() },
            fans: canvas_proto::config::FansFile::default(),
            displays: Vec::new(),
            stacks: Vec::new(),
            lights,
            scenes,
        }
    }

    fn read_lights(home: &Path) -> LightsFile {
        serde_json::from_slice(&std::fs::read(home.join(LIGHTS_FILE)).expect("lights.json is written")).unwrap()
    }

    fn read_scenes(home: &Path) -> ScenesFile {
        serde_json::from_slice(&std::fs::read(home.join(SCENES_FILE)).expect("scenes.json is written")).unwrap()
    }

    fn bundle_group(id: &str, members: &[&str]) -> canvas_proto::scene::Group {
        canvas_proto::scene::Group {
            id: id.into(),
            name: id.into(),
            members: members.iter().map(|m| m.to_string()).collect(),
            plays: canvas_proto::scene::Plays::Off,
            mode: canvas_proto::scene::GroupMode::FlowAcross,
            brightness: 100,
        }
    }

    /// A 0.13 bundle carries only layouts. The import makes lights and scenes from them, and still
    /// writes `layouts.json` the way the old layout import does (Task 12 removes that).
    #[test]
    fn a_0_13_bundle_imports_as_scenes() {
        let dir = temp_home("import-0-13");
        let layouts = LayoutsFile {
            schema_version: LAYOUTS_SCHEMA,
            active: "a".into(),
            layouts: vec![
                bundle_layout("a", "Gaming", vec![bundle_placed("openrgb:board", 0), bundle_placed("openrgb:board", 1)]),
                bundle_layout("b", "Quiet", vec![bundle_placed("openrgb:board", 1)]),
            ],
        };
        let bundle = bundle_of(layouts, None, None);
        let json = serde_json::to_string(&bundle).unwrap();
        assert!(!json.contains("\"scenes\""), "a 0.13 bundle carries no scenes: {json}");

        import_bundle(&dir, &bundle, &[bundle_board()]).unwrap();

        let lights = read_lights(&dir);
        let scenes = read_scenes(&dir);
        assert_eq!(lights.lights.len(), 2);
        let names: Vec<&str> = scenes.scenes.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Gaming", "Quiet"]);
        assert_eq!(scenes.active, "a", "the active layout is the active scene");
        for scene in &scenes.scenes {
            canvas_proto::scene::validate_scene(scene, &lights.lights).expect("every imported scene is valid");
        }
        // The old layout import still writes layouts.json until Task 12; the scenes code path
        // neither writes nor renames it.
        assert!(dir.join(LAYOUTS_FILE).exists());
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with("layouts.json.")).count(),
            0,
            "layouts.json was not renamed"
        );
        // What the daemon loads next is exactly what the import wrote.
        let store = Store::load_from(&dir);
        assert_eq!(store.lights, lights);
        assert_eq!(store.scenes, scenes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A 0.14 bundle carries both files: export takes them from the store and import writes them
    /// back unchanged, groups, modes, brightnesses and revisions included.
    #[test]
    fn a_0_14_bundle_round_trips_lights_and_scenes() {
        use canvas_proto::scene::{Group, GroupMode, Plays, Scene, ScenesFile, SCENES_SCHEMA};

        let from = temp_home("round-trip-from");
        let mut lights = crate::scenes::empty_lights();
        crate::scenes::sync_lights(&mut lights, &[bundle_board()]);
        let scenes = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "evening".into(),
            scenes: vec![Scene {
                id: "evening".into(),
                name: "Evening".into(),
                rev: 4,
                groups: vec![
                    Group {
                        id: "intake".into(),
                        name: "Intake".into(),
                        members: vec!["openrgb:board#z0".into()],
                        plays: Plays::Off,
                        mode: GroupMode::EachItsOwn,
                        brightness: 60,
                    },
                    Group {
                        id: "exhaust".into(),
                        name: "Exhaust".into(),
                        members: vec!["openrgb:board#z1".into()],
                        plays: Plays::Effect { effect: "aurora".into(), props: Default::default() },
                        mode: GroupMode::FlowAcross,
                        brightness: 100,
                    },
                ],
            }],
        };
        write_json(&from.join(LIGHTS_FILE), &lights).unwrap();
        write_json(&from.join(SCENES_FILE), &scenes).unwrap();
        // No layouts.json: the store is settled and exports the scenes as they are.
        let bundle = export_bundle(&from, canvas_proto::config::FansFile::default());
        assert_eq!(bundle.lights.as_ref(), Some(&lights), "the export carries lights.json");
        assert_eq!(bundle.scenes.as_ref(), Some(&scenes), "the export carries scenes.json");

        let to = temp_home("round-trip-to");
        import_bundle(&to, &bundle, &[]).unwrap();
        let loaded = Store::load_from(&to);
        assert_eq!(loaded.lights, lights);
        assert_eq!(loaded.scenes, scenes);
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }

    /// A bundle whose scenes are damaged is repaired on the way in, and the repairs are logged.
    /// One that cannot be made valid is refused as a validation error, and nothing is written.
    #[test]
    fn an_import_with_a_broken_scene_is_repaired_or_refused() {
        use canvas_proto::scene::{Group, Plays, Scene, ScenesFile, SCENES_SCHEMA};

        let dir = temp_home("broken-scene");
        let mut lights = crate::scenes::empty_lights();
        crate::scenes::sync_lights(&mut lights, &[bundle_board()]);
        let damage = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "gone".into(),
            scenes: vec![Scene {
                id: "s".into(),
                name: "S".into(),
                rev: 2,
                groups: vec![
                    bundle_group("a", &["openrgb:board#z0"]),
                    // The same light again, and one that names no light.
                    bundle_group("b", &["openrgb:board#z0", "nowhere#z0"]),
                    // No members at all.
                    bundle_group("empty", &[]),
                ],
            }],
        };
        let bundle = bundle_of(default_layouts(), Some(lights.clone()), Some(damage));

        let (result, text) = crate::test_logs::captured(|| import_bundle(&dir, &bundle, &[]));
        result.expect("a repairable scene imports");
        let scenes = read_scenes(&dir);
        assert_eq!(scenes.active, "s", "the active scene that named nothing now names the scene");
        assert_eq!(scenes.scenes[0].groups.len(), 1, "{scenes:?}");
        assert_eq!(scenes.scenes[0].groups[0].id, "a", "the first group keeps the light");
        assert_eq!(scenes.scenes[0].groups[0].members, vec!["openrgb:board#z0".to_string()]);
        assert_eq!(scenes.scenes[0].rev, 2, "a repair keeps the revision");
        canvas_proto::scene::validate_scene(&scenes.scenes[0], &read_lights(&dir).lights).unwrap();
        assert!(text.contains("repaired"), "the import logs the repair: {text}");

        // A scene whose members name no light, in a bundle with no lights file, cannot be made
        // valid: it is a validation error, and nothing is written.
        let refused_home = temp_home("broken-scene-refused");
        let nameless = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![Scene {
                id: "s".into(),
                name: "S".into(),
                rev: 1,
                groups: vec![Group {
                    plays: Plays::Off,
                    ..bundle_group("a", &["nowhere#z0"])
                }],
            }],
        };
        let refused = bundle_of(default_layouts(), None, Some(nameless));
        assert!(
            matches!(validate_bundle(&refused), Err(ImportError::Scenes(_))),
            "a scene that names no light is refused before anything is written"
        );
        assert!(import_bundle(&refused_home, &refused, &[]).is_err());
        assert!(!refused_home.join(SCENES_FILE).exists(), "a refused import writes no scenes.json");
        assert!(!refused_home.join(LIGHTS_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&refused_home);
    }

    /// The migration of a 0.13 bundle runs with the daemon's CURRENT devices: a matrix zone placed
    /// `Auto` becomes a Grid light here and a Strip one on a machine without the keyboard.
    #[test]
    fn a_0_13_bundle_migrates_with_the_current_devices() {
        use canvas_proto::scene::Arrangement;

        let layouts = LayoutsFile {
            schema_version: LAYOUTS_SCHEMA,
            active: "a".into(),
            layouts: vec![bundle_layout("a", "A", vec![bundle_placed("openrgb:kb", 0)])],
        };
        let bundle = bundle_of(layouts, None, None);

        let with_keyboard = temp_home("migrate-with-kb");
        import_bundle(&with_keyboard, &bundle, &[bundle_board(), bundle_kb()]).unwrap();
        let lights = read_lights(&with_keyboard);
        let kb = lights.lights.iter().find(|l| l.id == "openrgb:kb#z0").expect("the keyboard's zone is a light");
        assert_eq!(kb.arrangement, Arrangement::Grid, "the current device list says it is a matrix");

        let without = temp_home("migrate-without-kb");
        import_bundle(&without, &bundle, &[bundle_board()]).unwrap();
        let lights = read_lights(&without);
        let kb = lights.lights.iter().find(|l| l.id == "openrgb:kb#z0").expect("the placed zone is still a light");
        assert_eq!(kb.arrangement, Arrangement::Strip, "with no device list there is nothing that says matrix");
        let _ = std::fs::remove_dir_all(&with_keyboard);
        let _ = std::fs::remove_dir_all(&without);
    }

    /// Blocker 1 (review 323): the snapshot taken before an import carries every file the import
    /// writes over, `lights.json` and `scenes.json` included. A bundle whose lights lack a split
    /// light loses it on the import; the snapshot is the only way back.
    #[test]
    fn the_pre_import_snapshot_keeps_the_lights_and_scenes() {
        let dir = temp_home("snapshot-scenes");
        let mut lights = crate::scenes::empty_lights();
        crate::scenes::sync_lights(&mut lights, &[bundle_board()]);
        write_json(&dir.join(LIGHTS_FILE), &lights).unwrap();
        write_json(&dir.join(SCENES_FILE), &crate::scenes::default_scenes(&lights)).unwrap();

        let snap = snapshot(&dir, "before-import").expect("the snapshot is taken");

        assert!(snap.join(LIGHTS_FILE).exists(), "the snapshot keeps lights.json");
        assert!(snap.join(SCENES_FILE).exists(), "the snapshot keeps scenes.json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Blocker 1 (review 323): an import never writes over a scenes file the store holds as
    /// blocked (it exists but cannot be read), and it refuses before writing anything at all.
    #[test]
    fn an_import_refuses_a_scenes_file_it_could_not_read() {
        for name in [SCENES_FILE, LIGHTS_FILE] {
            let dir = temp_home("blocked-import");
            // A directory where the file belongs: it exists, and no read returns a file.
            std::fs::create_dir_all(dir.join(name)).unwrap();
            let bundle = bundle_of(default_layouts(), None, None);

            let err = import_bundle(&dir, &bundle, &[bundle_board()]).expect_err("the import is refused");

            let text = err.to_string();
            assert!(
                text.contains(name) && text.contains("could not be read"),
                "the refusal names {name} and says it could not be read: {text}"
            );
            assert!(!dir.join(LAYOUTS_FILE).exists(), "nothing was written before the refusal");
            assert!(!dir.join(ZONES_FILE).exists(), "nothing was written before the refusal");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Should-fix 1 (review 323): a bundle with lights and no scenes migrates its layouts with the
    /// bundle's own lights held, as `Store::build_scenes` does. Re-deriving the lights from the
    /// layouts alone loses a split light's label, its slice and its `reversed` flag.
    #[test]
    fn a_bundle_with_lights_and_no_scenes_keeps_its_lights_through_the_migration() {
        use canvas_proto::layout::LedSlice;
        use canvas_proto::scene::{Arrangement, Light};

        let held = LightsFile {
            schema_version: LIGHTS_SCHEMA,
            lights: vec![
                Light {
                    id: "openrgb:board#z0p1".into(),
                    device_id: canvas_proto::DeviceId("openrgb:board".into()),
                    zone_index: 0,
                    leds: Some(LedSlice { start: 0, count: 10 }),
                    name: Some("Intake".into()),
                    arrangement: Arrangement::Ring,
                    reversed: true,
                },
                Light {
                    id: "openrgb:board#z0p2".into(),
                    device_id: canvas_proto::DeviceId("openrgb:board".into()),
                    zone_index: 0,
                    leds: Some(LedSlice { start: 10, count: 20 }),
                    name: Some("Exhaust".into()),
                    arrangement: Arrangement::Strip,
                    reversed: false,
                },
            ],
        };
        // The layouts place the whole zone, as a 0.13 bundle does: the migration can only make a
        // whole-zone light, so the two split lights are the bundle's own data.
        let layouts = LayoutsFile {
            schema_version: LAYOUTS_SCHEMA,
            active: "a".into(),
            layouts: vec![bundle_layout("a", "Gaming", vec![bundle_placed("openrgb:board", 0)])],
        };
        let dir = temp_home("held-lights");
        import_bundle(&dir, &bundle_of(layouts, Some(held), None), &[bundle_board()]).unwrap();

        let lights = read_lights(&dir);
        let intake = lights.lights.iter().find(|l| l.id == "openrgb:board#z0p1").expect(
            "the bundle's split light survives the migration",
        );
        assert_eq!(intake.name.as_deref(), Some("Intake"), "its label survives");
        assert_eq!(intake.leds, Some(LedSlice { start: 0, count: 10 }), "its slice survives");
        assert_eq!(intake.arrangement, Arrangement::Ring, "its arrangement survives");
        assert!(intake.reversed, "its reversed flag survives");
        assert!(lights.lights.iter().any(|l| l.id == "openrgb:board#z0p2"), "the second slice survives too");
        assert!(lights.lights.iter().any(|l| l.id == "openrgb:board#z0"), "the layout's own light is there");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Should-fix 2 (review 323): a bundle with scenes and no lights file (or an empty one) is
    /// importable. Its lights are rebuilt from the current devices before the repair, so the
    /// members name real lights; a bundle exported while `lights.json` was missing comes back.
    /// It is refused only when the devices do not cover the members either.
    #[test]
    fn a_bundle_with_scenes_and_no_lights_is_rebuilt_from_the_devices() {
        use canvas_proto::scene::Scene;

        let scenes = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![Scene {
                id: "s".into(),
                name: "S".into(),
                rev: 1,
                groups: vec![bundle_group("a", &["openrgb:board#z0"])],
            }],
        };
        let bundle = bundle_of(default_layouts(), None, Some(scenes.clone()));

        let dir = temp_home("scenes-no-lights");
        import_bundle(&dir, &bundle, &[bundle_board()]).expect("the devices bring the lights back");
        let lights = read_lights(&dir);
        assert!(
            lights.lights.iter().any(|l| l.id == "openrgb:board#z0"),
            "the member's light was rebuilt from the device"
        );
        assert_eq!(read_scenes(&dir), scenes, "the scene is imported as it was");
        let _ = std::fs::remove_dir_all(&dir);

        // The same bundle on a machine with no devices still names no light: refused, and
        // nothing is written.
        let refused = temp_home("scenes-no-lights-refused");
        assert!(import_bundle(&refused, &bundle, &[]).is_err());
        assert!(!refused.join(SCENES_FILE).exists());
        let _ = std::fs::remove_dir_all(&refused);
    }

    /// Should-fix 3 (review 323): an export never carries the in-memory default. `lights.json` or
    /// `scenes.json` that could not be read is left out of the bundle, and the scenes the store
    /// holds only in memory are not exported either: the file is on disk, the bundle names the
    /// real one or none.
    #[test]
    fn the_export_omits_a_file_the_store_could_not_read() {
        let mut lights = crate::scenes::empty_lights();
        crate::scenes::sync_lights(&mut lights, &[bundle_board()]);
        let scenes = crate::scenes::default_scenes(&lights);

        // scenes.json exists but cannot be read: the scenes stay out of the bundle, and the
        // real lights go in. A default scene would replace the target's scenes on import.
        let dir = temp_home("export-scenes-blocked");
        write_json(&dir.join(LIGHTS_FILE), &lights).unwrap();
        std::fs::create_dir_all(dir.join(SCENES_FILE)).unwrap();
        let store = Store::load_from(&dir);
        assert!(store.scenes_blocked, "the store holds the file as blocked");
        let bundle = export_bundle(&dir, canvas_proto::config::FansFile::default());
        assert_eq!(bundle.scenes, None, "a blocked scenes file is not exported as a made-up default");
        assert_eq!(bundle.lights, Some(lights.clone()), "the real lights are exported");
        let _ = std::fs::remove_dir_all(&dir);

        // lights.json exists but cannot be read: the lights stay out (the bundle's scenes are
        // rebuilt from the importing machine's devices), and the real scenes go in.
        let dir = temp_home("export-lights-blocked");
        write_json(&dir.join(SCENES_FILE), &scenes).unwrap();
        std::fs::create_dir_all(dir.join(LIGHTS_FILE)).unwrap();
        let store = Store::load_from(&dir);
        assert!(store.lights_blocked, "the store holds the file as blocked");
        let bundle = export_bundle(&dir, canvas_proto::config::FansFile::default());
        assert_eq!(bundle.lights, None, "a blocked lights file is not exported as an empty one");
        assert_eq!(bundle.scenes, Some(scenes.clone()), "the real scenes are exported");

        // While the migration is pending, the lights on disk are what the store holds, and no
        // scenes exist yet: the bundle carries the lights and no scenes, so the import migrates
        // its layouts with them held.
        let dir = temp_home("export-pending");
        write_json(&dir.join(LAYOUTS_FILE), &default_layouts()).unwrap();
        write_json(&dir.join(LIGHTS_FILE), &lights).unwrap();
        let store = Store::load_from(&dir);
        assert!(store.scenes_pending.is_some(), "the migration is held");
        let bundle = export_bundle(&dir, canvas_proto::config::FansFile::default());
        assert_eq!(bundle.lights, Some(lights), "the lights.json held on disk is exported");
        assert_eq!(bundle.scenes, None, "there is no scenes.json to export");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
