//! The saved patterns (`patterns.json`): a colour per LED, built on one kind of device.
//!
//! Mirrors [`crate::stacks::Stacks`]: the file is read once at start-up, written whole on every
//! save (atomically, through [`crate::storage::write_json`]), and a file that cannot be read is
//! kept aside rather than lost. The rules a pattern keeps are the spine's (`validate_pattern`);
//! this module adds the name conflict and the load rules.
//!
//! The load rules are the damaged-file rules of the design (Review Focus 2):
//!
//! - A missing file is no patterns. A file that is not JSON, or is from another schema, is moved
//!   aside as `patterns.json.<stamp>.corrupt` and no patterns load.
//! - A file of this schema loads pattern by pattern: one that does not parse (a `motion` with no
//!   `type`, an arrangement this build does not know, a colour that is not `#rrggbb`) or breaks a
//!   rule is left out and logged with its id, the rest load, and the first write after such a
//!   load keeps the file aside first, so what this build does not run is never erased silently.
//! - More than one frame, a `motion` other than none and fields this build does not know all
//!   load and are kept whole. Only frame 0 plays (the engine's job, not this store's).

use canvas_proto::pattern::{
    validate_pattern, Pattern, PatternsFile, PATTERNS_FILE, PATTERNS_SCHEMA,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock;

use crate::stack::{read_recovering, write_recovering};

/// Why a pattern could not be saved.
#[derive(Debug, thiserror::Error)]
pub enum PatternsError {
    /// A save that breaks a rule (422).
    #[error("{0}")]
    Invalid(String),
    /// A name that is already another pattern's, ignoring case (409).
    #[error("{0}")]
    Conflict(String),
    /// The file could not be written (500).
    #[error("{0}")]
    Io(String),
}

/// The saved patterns, as one file.
pub struct Patterns {
    home: PathBuf,
    file: RwLock<PatternsFile>,
    generation: AtomicU64,
    /// True once a load left a pattern out: the file still holds it, so the first write keeps the
    /// file aside before the write erases it.
    dropped_on_load: AtomicBool,
}

impl Patterns {
    /// An empty store rooted at `home`. Nothing is read until [`Patterns::reload`].
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            file: RwLock::new(PatternsFile::default()),
            generation: AtomicU64::new(0),
            dropped_on_load: AtomicBool::new(false),
        }
    }

    /// The store's folder.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where `patterns.json` lives.
    pub fn path(&self) -> PathBuf {
        self.home.join(PATTERNS_FILE)
    }

    /// Reads `patterns.json`. A missing file is no patterns; a file that does not parse is
    /// quarantined and the store starts empty; a pattern that breaks a rule is dropped with the
    /// reason logged, and the rest load.
    ///
    /// Each pattern is read on its own (see [`Patterns::read_file`]), so one damaged entry costs
    /// its own pattern and not the file.
    pub fn reload(&self) {
        let path = self.path();
        let parsed: Option<PatternsFile> = self.read_file(&path);
        let Some(file) = parsed else {
            *write_recovering(&self.file) = PatternsFile::default();
            self.generation.fetch_add(1, Ordering::SeqCst);
            return;
        };
        let mut accepted: Vec<Pattern> = Vec::new();
        let mut dropped = false;
        for pattern in file.patterns {
            if let Err(reason) = validate_pattern(&pattern) {
                tracing::warn!(pattern = %pattern.id, %reason, "a pattern was dropped as it was loaded");
                dropped = true;
                continue;
            }
            accepted.push(pattern);
        }
        if dropped {
            // The pattern that was dropped is still in the file; the first write must not erase
            // it without a copy (see `keep_aside_after_a_dropped_load`).
            self.dropped_on_load.store(true, Ordering::SeqCst);
        }
        *write_recovering(&self.file) = PatternsFile {
            schema_version: PATTERNS_SCHEMA,
            patterns: accepted,
            // A field this build does not know is kept: a later version wrote it.
            extra: file.extra,
        };
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// `patterns.json` read pattern by pattern: `None` when there is no file, when it is not
    /// JSON, or when it is from another schema (those keep the damaged-file rule: the file is
    /// moved aside as `*.corrupt` and no pattern loads).
    ///
    /// A file of this schema is read twice: whole first, and then, when one entry stops the whole
    /// parse (a `motion` with no `type`, an arrangement this build does not know, a colour that is
    /// not `#rrggbb`), entry by entry, so the good patterns load and only the damaged one is left
    /// out. Its id is logged with the reason where the entry still gives one, and the first write
    /// after such a load keeps the file aside, so nothing is erased silently.
    fn read_file(&self, path: &Path) -> Option<PatternsFile> {
        let bytes = crate::paths::read_optional(path)?;
        let shape: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(error = %error, "patterns.json did not parse; starting with no patterns");
                self.quarantine(path);
                return None;
            }
        };
        match shape.get("schema_version").and_then(serde_json::Value::as_u64) {
            Some(found) if found == u64::from(PATTERNS_SCHEMA) => {}
            found => {
                tracing::warn!(
                    found = found.unwrap_or_default(),
                    expected = PATTERNS_SCHEMA,
                    "patterns.json is from another schema; starting with no patterns"
                );
                self.quarantine(path);
                return None;
            }
        }
        // A file this build wrote parses in one go, with every field it does not know kept.
        if let Ok(file) = serde_json::from_value::<PatternsFile>(shape.clone()) {
            return Some(file);
        }
        let entries = match shape.get("patterns") {
            None => Vec::new(),
            Some(serde_json::Value::Array(entries)) => entries.clone(),
            // A `patterns` that is not a list is not a file this build can read at all.
            Some(_) => {
                tracing::error!("patterns.json's patterns is not a list; starting with no patterns");
                self.quarantine(path);
                return None;
            }
        };
        let mut patterns = Vec::with_capacity(entries.len());
        let mut dropped = false;
        for entry in entries {
            match serde_json::from_value::<Pattern>(entry.clone()) {
                Ok(pattern) => patterns.push(pattern),
                Err(error) => {
                    let id = entry
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("<a pattern with no id>");
                    tracing::warn!(pattern = %id, error = %error, "a pattern was dropped as it was loaded");
                    dropped = true;
                }
            }
        }
        if dropped {
            self.dropped_on_load.store(true, Ordering::SeqCst);
        }
        Some(PatternsFile {
            schema_version: PATTERNS_SCHEMA,
            patterns,
            extra: other_fields(&shape),
        })
    }

    /// Moves a file that could not be read at all aside, as [`crate::storage::quarantine`] does for
    /// every other file. A copy that cannot be made is logged and the store still starts empty.
    fn quarantine(&self, path: &Path) {
        if let Err(error) = crate::storage::quarantine(path) {
            tracing::error!(error = %error, "patterns.json could not be moved aside; it is left as it is");
        }
    }

    /// Every saved pattern, in the order they are stored.
    pub fn all(&self) -> Vec<Pattern> {
        read_recovering(&self.file).patterns.clone()
    }

    /// The pattern with this id (`pattern:<name>`) or this name, ignoring ASCII case.
    pub fn get(&self, id: &str) -> Option<Pattern> {
        let wanted = id.trim();
        read_recovering(&self.file)
            .patterns
            .iter()
            .find(|pattern| {
                pattern.id.eq_ignore_ascii_case(wanted)
                    || pattern.name().eq_ignore_ascii_case(wanted)
            })
            .cloned()
    }

    /// The generation: it moves on every read of the file and every write.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Saves `pattern`, replacing a pattern of the same id (its case as stored) and refusing a
    /// name that is another pattern's, ignoring case.
    ///
    /// The file is written first: a save that cannot be written leaves the store as it was,
    /// rather than holding a pattern the next start will not see.
    pub fn put(&self, pattern: Pattern) -> Result<Pattern, PatternsError> {
        let name = pattern.name().to_string();
        let mut guard = write_recovering(&self.file);
        let others: Vec<Pattern> = guard
            .patterns
            .iter()
            .filter(|existing| existing.id != pattern.id)
            .cloned()
            .collect();
        // A name another pattern has, ignoring case, is a conflict before it is a validation
        // problem: the editor offers to replace it.
        if others
            .iter()
            .any(|other| other.name().eq_ignore_ascii_case(&name))
        {
            return Err(PatternsError::Conflict(format!(
                "{name:?} is already another pattern's name, ignoring case"
            )));
        }
        validate_pattern(&pattern).map_err(|error| PatternsError::Invalid(error.to_string()))?;
        let mut file = guard.clone();
        match file.patterns.iter_mut().find(|existing| existing.id == pattern.id) {
            Some(existing) => *existing = pattern.clone(),
            None => file.patterns.push(pattern.clone()),
        }
        self.keep_aside_after_a_dropped_load()?;
        crate::storage::write_json(&self.path(), &file)
            .map_err(|e| PatternsError::Io(e.to_string()))?;
        *guard = file;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(pattern)
    }

    /// Removes a pattern. `Ok(None)` when there was none.
    ///
    /// The file is written first, so a delete that cannot be written is an error to the caller
    /// and the pattern is still there.
    pub fn delete(&self, id: &str) -> Result<Option<Pattern>, PatternsError> {
        let mut guard = write_recovering(&self.file);
        let Some(index) = guard.patterns.iter().position(|pattern| {
            pattern.id.eq_ignore_ascii_case(id) || pattern.name().eq_ignore_ascii_case(id)
        }) else {
            return Ok(None);
        };
        let mut file = guard.clone();
        let removed = file.patterns.remove(index);
        self.keep_aside_after_a_dropped_load()?;
        crate::storage::write_json(&self.path(), &file)
            .map_err(|e| PatternsError::Io(e.to_string()))?;
        *guard = file;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(Some(removed))
    }

    /// Keeps the file as it was loaded aside before the first write after a load left a pattern
    /// out: what is not in memory is still in the file, and the write would erase it.
    ///
    /// The copy is kept as `read_parsed` keeps an unparseable file: renamed to
    /// `patterns.json.<stamp>.corrupt` beside it. A copy that cannot be made fails the write, so
    /// the file is never overwritten without it.
    fn keep_aside_after_a_dropped_load(&self) -> Result<(), PatternsError> {
        if !self.dropped_on_load.load(Ordering::SeqCst) {
            return Ok(());
        }
        let path = self.path();
        match crate::storage::quarantine(&path) {
            Ok(()) => {
                self.dropped_on_load.store(false, Ordering::SeqCst);
                tracing::warn!(
                    path = %path.display(),
                    "a pattern this build does not run was left out as patterns.json loaded; the file is kept aside before the first save"
                );
                Ok(())
            }
            Err(error) => {
                tracing::error!(
                    path = %path.display(),
                    %error,
                    "patterns.json could not be kept aside; nothing is written over it"
                );
                Err(PatternsError::Io(format!(
                    "patterns.json could not be kept aside: {error}"
                )))
            }
        }
    }
}

/// The file-level fields a `patterns.json` carries besides its schema and its patterns: a later
/// version's, kept as they are when the file is read entry by entry.
fn other_fields(file: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let serde_json::Value::Object(fields) = file else {
        return serde_json::Map::new();
    };
    fields
        .iter()
        .filter(|(key, _)| key.as_str() != "schema_version" && key.as_str() != "patterns")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Writes a patterns file into `home`, replacing the one there: what a config import does with
/// the bundle's patterns.
pub fn write_file(home: &Path, patterns: &[Pattern]) -> std::io::Result<()> {
    let file = PatternsFile {
        schema_version: PATTERNS_SCHEMA,
        patterns: patterns.to_vec(),
        extra: serde_json::Map::new(),
    };
    crate::storage::write_json(&home.join(PATTERNS_FILE), &file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::color::Rgb8;
    use canvas_proto::pattern::{BuiltOn, Frame, Motion};
    use canvas_proto::scene::Arrangement;

    fn dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-patterns-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A pattern built on a ring of 12 with one LED lit.
    fn pattern(name: &str) -> Pattern {
        let mut colors = vec![Rgb8::BLACK; 12];
        colors[0] = Rgb8::new(255, 0, 0);
        Pattern {
            id: format!("pattern:{name}"),
            built_on: BuiltOn { led_count: 12, arrangement: Arrangement::Ring, grid: None },
            frames: vec![Frame { colors, hold: 0, extra: serde_json::Map::new() }],
            motion: Motion::none(),
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn put_then_get_by_id_and_by_name_ignoring_case() {
        let dir = dir("round-trip");
        let store = Patterns::new(&dir);
        let saved = store.put(pattern("Intake")).expect("the save is written");
        assert_eq!(saved.name(), "Intake");

        assert_eq!(store.get("pattern:Intake"), Some(saved.clone()));
        assert_eq!(store.get("Intake"), Some(saved.clone()));
        assert_eq!(store.get("intake"), Some(saved), "the name is matched ignoring case");
        assert_eq!(store.get("pattern:INTAKE").map(|p| p.name().to_string()), Some("Intake".into()));
        assert_eq!(store.get("nothing"), None);

        // The file is there, and a fresh store reads the same pattern back.
        assert!(store.path().exists());
        let again = Patterns::new(&dir);
        again.reload();
        assert_eq!(again.all(), vec![pattern("Intake")]);
        assert_eq!(again.generation(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_taken_by_another_pattern_is_a_conflict() {
        let dir = dir("conflict");
        let store = Patterns::new(&dir);
        store.put(pattern("Intake")).expect("the first save is written");
        let taken = store.put(pattern("intake"));
        assert!(matches!(taken, Err(PatternsError::Conflict(_))), "{taken:?}");
        assert_eq!(store.all().len(), 1, "the refused save changed nothing");
        // The same pattern saved again replaces itself, whatever its case.
        store.put(pattern("Intake")).expect("a save of the same id replaces it");
        assert_eq!(store.all().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn put_writes_the_file_first_and_a_failed_write_changes_nothing() {
        // A folder where `patterns.json` is a directory, so no write can land on it.
        let folder = dir("unwritable");
        std::fs::create_dir_all(folder.join(PATTERNS_FILE)).unwrap();
        let store = Patterns::new(&folder);
        let outcome = store.put(pattern("Fail"));
        assert!(matches!(outcome, Err(PatternsError::Io(_))), "{outcome:?}");
        assert!(store.all().is_empty(), "a failed save holds no pattern");
        let _ = std::fs::remove_dir_all(&folder);

        // A delete that cannot be written keeps the pattern.
        let folder = dir("unwritable-delete");
        let store = Patterns::new(&folder);
        let saved = store.put(pattern("Kept")).expect("the first save is written");
        std::fs::remove_file(folder.join(PATTERNS_FILE)).unwrap();
        std::fs::create_dir(folder.join(PATTERNS_FILE)).unwrap();
        let outcome = store.delete("pattern:Kept");
        assert!(matches!(outcome, Err(PatternsError::Io(_))), "{outcome:?}");
        assert_eq!(store.all(), vec![saved]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_file_with_two_frames_and_a_motion_loads_plays_the_first_and_keeps_its_fields_on_save() {
        let dir = dir("future");
        std::fs::write(
            dir.join(PATTERNS_FILE),
            r##"{
                "schema_version": 1,
                "future": {"anything": [1, 2]},
                "patterns": [{
                    "id": "pattern:Spin",
                    "built_on": {"led_count": 2, "arrangement": "ring"},
                    "frames": [
                        {"colors": ["#ff0000", "#00ff00"], "hold": 250, "label": "first"},
                        {"colors": ["#0000ff", "#ffffff"], "hold": 250}
                    ],
                    "motion": {"type": "rotate", "speed": 2},
                    "tags": ["warm"]
                }]
            }"##,
        )
        .unwrap();
        let store = Patterns::new(&dir);
        store.reload();
        let loaded = store.all();
        assert_eq!(loaded.len(), 1, "two frames and a motion load");
        assert_eq!(loaded[0].frames.len(), 2);
        assert_eq!(loaded[0].first_frame().colors[0], Rgb8::new(255, 0, 0), "frame 0 is the one it reads");
        assert!(!loaded[0].motion.is_none(), "the motion is kept as written");

        // An unrelated save writes the file again: the future fields are still there.
        store.put(pattern("Other")).expect("the save is written");

        let text = std::fs::read_to_string(dir.join(PATTERNS_FILE)).unwrap();
        let file: serde_json::Value = serde_json::from_str(&text).unwrap();
        let spin = file["patterns"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == serde_json::json!("pattern:Spin"))
            .expect("the future pattern is still saved");
        for kept in ["label", "rotate", "speed", "tags"] {
            assert!(spin.to_string().contains(kept), "{kept} was dropped: {spin}");
        }
        assert_eq!(spin["frames"].as_array().map(Vec::len), Some(2), "both frames are kept");
        assert_eq!(spin["frames"][0]["hold"], serde_json::json!(250));
        assert_eq!(file["future"]["anything"], serde_json::json!([1, 2]), "the file's own field is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_patterns_file_keeps_the_good_patterns() {
        let dir = dir("damaged");
        // ONE pattern is fine; the other's frame is one colour short of its LED count.
        std::fs::write(
            dir.join(PATTERNS_FILE),
            r##"{"schema_version": 1, "patterns": [
                {"id": "pattern:Good", "built_on": {"led_count": 2, "arrangement": "ring"},
                 "frames": [{"colors": ["#ff0000", "#00ff00"]}]},
                {"id": "pattern:Short", "built_on": {"led_count": 2, "arrangement": "ring"},
                 "frames": [{"colors": ["#ff0000"]}]}
            ]}"##,
        )
        .unwrap();
        let store = Patterns::new(&dir);
        store.reload();
        assert_eq!(
            store.all().iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["pattern:Good"],
            "the good pattern loads and the damaged one is left out"
        );

        store.put(pattern("New")).expect("the save is written");

        // The file as it loaded is kept beside the new one, so the damaged pattern is not lost.
        let kept = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .find(|name| name.starts_with(PATTERNS_FILE) && name.ends_with(".corrupt"))
            .expect("the file as it loaded is kept aside");
        let text = std::fs::read_to_string(dir.join(&kept)).unwrap();
        assert!(text.contains("pattern:Short"), "the kept file still holds the damaged pattern");

        let after = std::fs::read_to_string(dir.join(PATTERNS_FILE)).unwrap();
        assert!(after.contains("pattern:Good"), "{after}");
        assert!(after.contains("pattern:New"), "{after}");
        assert!(!after.contains("pattern:Short"), "{after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pattern_with_a_bad_colour_is_left_out_and_the_good_ones_load() {
        let dir = dir("bad-colour");
        std::fs::write(
            dir.join(PATTERNS_FILE),
            r##"{"schema_version": 1, "future": {"anything": [1, 2]}, "patterns": [
                {"id": "pattern:Good", "built_on": {"led_count": 2, "arrangement": "ring"},
                 "frames": [{"colors": ["#ff0000", "#00ff00"]}]},
                {"id": "pattern:BadColour", "built_on": {"led_count": 2, "arrangement": "ring"},
                 "frames": [{"colors": ["#ff0000", "#ff00zz"]}]},
                {"id": "pattern:AlsoGood", "built_on": {"led_count": 1, "arrangement": "single"},
                 "frames": [{"colors": ["#0000ff"]}]}
            ]}"##,
        )
        .unwrap();
        let store = Patterns::new(&dir);
        store.reload();
        assert_eq!(
            store.all().iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["pattern:Good", "pattern:AlsoGood"],
            "a bad colour costs its own pattern only"
        );

        // The file as it loaded is kept beside the new one: the dropped pattern is not lost.
        store.put(pattern("New")).expect("the save is written");
        let kept = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .find(|name| name.starts_with(PATTERNS_FILE) && name.ends_with(".corrupt"))
            .expect("the file as it loaded is kept aside");
        let text = std::fs::read_to_string(dir.join(&kept)).unwrap();
        assert!(text.contains("pattern:BadColour"), "the kept file still holds it: {text}");

        let after = std::fs::read_to_string(dir.join(PATTERNS_FILE)).unwrap();
        assert!(after.contains("pattern:Good"), "{after}");
        assert!(!after.contains("pattern:BadColour"), "{after}");
        let file: serde_json::Value = serde_json::from_str(&after).unwrap();
        assert_eq!(file["future"]["anything"], serde_json::json!([1, 2]), "the file's own field is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pattern_with_a_motion_that_has_no_type_is_left_out_and_the_good_ones_load() {
        let dir = dir("bad-motion");
        std::fs::write(
            dir.join(PATTERNS_FILE),
            r##"{"schema_version": 1, "patterns": [
                {"id": "pattern:Good", "built_on": {"led_count": 2, "arrangement": "ring"},
                 "frames": [{"colors": ["#ff0000", "#00ff00"]}]},
                {"id": "pattern:BadMotion", "built_on": {"led_count": 2, "arrangement": "ring"},
                 "frames": [{"colors": ["#ff0000", "#00ff00"]}], "motion": {"speed": 2}},
                {"id": "pattern:AlsoGood", "built_on": {"led_count": 1, "arrangement": "single"},
                 "frames": [{"colors": ["#0000ff"]}], "motion": {"type": "none"}}
            ]}"##,
        )
        .unwrap();
        let store = Patterns::new(&dir);
        store.reload();
        assert_eq!(
            store.all().iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["pattern:Good", "pattern:AlsoGood"],
            "a motion without a type costs its own pattern only"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unparseable_file_is_moved_aside_and_the_store_is_empty() {
        let dir = dir("corrupt");
        std::fs::write(dir.join(PATTERNS_FILE), "{ this is not json").unwrap();
        let store = Patterns::new(&dir);
        store.reload();
        assert!(store.all().is_empty());
        assert!(!dir.join(PATTERNS_FILE).exists(), "the file was moved aside");
        assert!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .any(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    name.starts_with(PATTERNS_FILE) && name.ends_with(".corrupt")
                }),
            "a corrupt file is kept beside the store"
        );
        // A save after that writes a fresh file.
        store.put(pattern("Fresh")).expect("the save is written");
        assert_eq!(store.all().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_missing_file_is_no_patterns() {
        let dir = dir("missing");
        let store = Patterns::new(&dir);
        store.reload();
        assert!(store.all().is_empty());
        assert!(!store.path().exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
