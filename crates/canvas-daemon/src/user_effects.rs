//! User HTML effects: every `*.html` file in `%APPDATA%\canvas-rgb\effects\` is an effect
//! with the id `html:<file stem>`.
//!
//! Each file is parsed once to read its schema. The source text is kept, so activation
//! builds a fresh instance from it. The folder is rescanned on a fixed interval; only
//! files whose size or modified time changed are parsed again. A file that fails to
//! parse is logged with its error and left out of the list until it changes.
//!
//! This module also holds the rules an imported file name must follow
//! ([`check_file_name`]) and the atomic write that puts a file in the folder
//! ([`write_atomic`]).

use canvas_proto::props::{Props, PropertySchema};
use canvas_proto::traits::Effect;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, SystemTime};

/// The id prefix every user HTML effect carries.
pub const USER_EFFECT_PREFIX: &str = "html:";

/// How often the effects folder is rescanned.
pub const RESCAN_INTERVAL: Duration = Duration::from_secs(5);

/// Turns an effect id and its HTML source into a runnable effect.
///
/// The daemon uses [`html_builder`]; the tests inject fakes.
pub type Builder = Arc<dyn Fn(&str, &str) -> Result<Box<dyn Effect>, String> + Send + Sync>;

/// The builder that runs real SignalRGB-format HTML effects through `canvas-html`.
pub fn html_builder() -> Builder {
    Arc::new(|id: &str, source: &str| {
        canvas_html::HtmlEffect::from_source(id, source, &Props::new())
            .map(|effect| Box::new(effect) as Box<dyn Effect>)
            .map_err(|e| e.to_string())
    })
}

/// A file's identity for change detection: modified time and size.
type Stamp = (Option<SystemTime>, u64);

#[derive(Clone)]
struct LoadedEffect {
    stamp: Stamp,
    schema: PropertySchema,
    source: Arc<str>,
}

#[derive(Default)]
struct State {
    /// Files that parsed, by path.
    loaded: BTreeMap<PathBuf, LoadedEffect>,
    /// Files that failed to parse, by path, with the stamp they failed at and the error.
    failed: BTreeMap<PathBuf, (Stamp, String)>,
}

/// The user HTML effects found in the effects folder.
pub struct UserEffects {
    builder: Builder,
    state: RwLock<State>,
    /// Bumped whenever a rescan changes the set of user effects, so caches keyed on it
    /// (the thumbnails) know to drop their entries.
    generation: AtomicU64,
}

impl UserEffects {
    pub fn new(builder: Builder) -> Self {
        Self { builder, state: RwLock::new(State::default()), generation: AtomicU64::new(0) }
    }

    /// The current generation. It changes exactly when a rescan changes the set.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// The schemas of the loaded user effects, sorted by name (case-insensitive).
    pub fn schemas(&self) -> Vec<PropertySchema> {
        let state = read_lock(&self.state);
        let mut out: Vec<PropertySchema> =
            state.loaded.values().map(|e| e.schema.clone()).collect();
        out.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    /// Builds a fresh instance of a loaded user effect. `None` when `id` is not a loaded
    /// user effect; the caller then tries the built-ins.
    pub fn create(&self, id: &str) -> Option<Result<Box<dyn Effect>, String>> {
        let source = {
            let state = read_lock(&self.state);
            state
                .loaded
                .values()
                .find(|e| e.schema.id == id)
                .map(|e| e.source.clone())
        }?;
        Some((self.builder)(id, &source))
    }

    /// The files that failed to parse at their last scan, with the error.
    pub fn failures(&self) -> Vec<(PathBuf, String)> {
        let state = read_lock(&self.state);
        state
            .failed
            .iter()
            .map(|(path, (_, error))| (path.clone(), error.clone()))
            .collect()
    }

    /// Brings the loaded list in line with `dir`. Returns true when anything changed.
    ///
    /// A missing folder is an empty folder. An unreadable folder keeps the last known
    /// list. Parsing happens outside the lock, so the API keeps answering while a file
    /// is read.
    pub fn rescan(&self, dir: &Path) -> bool {
        let listing = match listing(dir) {
            Ok(listing) => listing,
            Err(error) => {
                tracing::warn!(dir = %dir.display(), %error, "could not read the user effects folder");
                return false;
            }
        };

        // The files whose stamp differs from what is already known.
        let todo: Vec<(PathBuf, Stamp)> = {
            let state = read_lock(&self.state);
            listing
                .iter()
                .filter(|(path, stamp)| {
                    state.loaded.get(*path).map(|e| e.stamp) != Some(**stamp)
                        && state.failed.get(*path).map(|f| f.0) != Some(**stamp)
                })
                .map(|(path, stamp)| (path.clone(), *stamp))
                .collect()
        };

        let parsed: Vec<(PathBuf, Stamp, Result<LoadedEffect, String>)> = todo
            .into_iter()
            .map(|(path, stamp)| {
                let result = self.parse(&path, stamp);
                (path, stamp, result)
            })
            .collect();

        let mut state = write_lock(&self.state);
        let mut changed = false;

        for (path, stamp, result) in parsed {
            changed = true;
            match result {
                Ok(effect) => {
                    let duplicate = state
                        .loaded
                        .iter()
                        .any(|(other, l)| other != &path && l.schema.id == effect.schema.id);
                    if duplicate {
                        let error = format!("duplicate effect id {}", effect.schema.id);
                        tracing::warn!(path = %path.display(), %error, "a user effect was skipped");
                        state.loaded.remove(&path);
                        state.failed.insert(path, (stamp, error));
                    } else {
                        tracing::info!(path = %path.display(), id = %effect.schema.id, "a user effect was loaded");
                        state.failed.remove(&path);
                        state.loaded.insert(path, effect);
                    }
                }
                Err(error) => {
                    tracing::warn!(path = %path.display(), %error, "a user effect failed to parse and was skipped");
                    state.loaded.remove(&path);
                    state.failed.insert(path, (stamp, error));
                }
            }
        }

        let before = state.loaded.len() + state.failed.len();
        state.loaded.retain(|path, _| listing.contains_key(path));
        state.failed.retain(|path, _| listing.contains_key(path));
        if state.loaded.len() + state.failed.len() != before {
            changed = true;
        }
        if changed {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        changed
    }

    /// Reads and parses one file. The id comes from the file stem.
    fn parse(&self, path: &Path, stamp: Stamp) -> Result<LoadedEffect, String> {
        let stem = path
            .file_stem()
            .ok_or_else(|| "the file has no name".to_string())?
            .to_string_lossy()
            .into_owned();
        let id = format!("{USER_EFFECT_PREFIX}{stem}");
        let source = std::fs::read_to_string(path)
            .map_err(|e| format!("could not read the file: {e}"))?;
        let effect = (self.builder)(&id, &source)?;
        let mut schema = effect.schema();
        // The id is the file's, whatever the effect reports.
        schema.id = id;
        Ok(LoadedEffect { stamp, schema, source: Arc::from(source) })
    }
}

/// The `.html` files in `dir`, with their stamps. A missing folder is empty.
fn listing(dir: &Path) -> Result<BTreeMap<PathBuf, Stamp>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut out = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_html = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("html"));
        if !is_html || !path.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        out.insert(path, (meta.modified().ok(), meta.len()));
    }
    Ok(out)
}

/// The file names Windows reserves. A file with one of these names cannot be created,
/// so the import refuses it instead of failing at the write.
///
/// Windows reserves the name with any extension, and the part before the first dot is
/// what counts: `con.html`, `con.x.html` and `con.x.y.html` are all the name `con`.
const RESERVED_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6",
    "com7", "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7",
    "lpt8", "lpt9",
];

/// Checks a file name for an effect and returns the id the file will carry.
///
/// The name may hold letters, digits, spaces, `-`, `_` and `.`, and it must end in
/// `.html`. A folder, a `..`, a reserved Windows name and a name that would carry a
/// built-in effect's id are all refused. The error message says why.
pub fn check_file_name(file_name: &str) -> Result<String, String> {
    let trimmed = file_name.trim();
    if trimmed.is_empty() {
        return Err("the file name is empty".to_string());
    }
    if trimmed != file_name {
        return Err(format!("{file_name:?} has a space at either end"));
    }
    for c in trimmed.chars() {
        if !(c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.') {
            return Err(format!(
                "{file_name:?} may only hold letters, digits, spaces, '-', '_' and '.'"
            ));
        }
    }
    if trimmed.contains("..") {
        return Err(format!("{file_name:?} must not hold '..'"));
    }
    if !trimmed.to_ascii_lowercase().ends_with(".html") {
        return Err(format!("{file_name:?} must end in .html"));
    }
    let stem = &trimmed[..trimmed.len() - ".html".len()];
    if stem.is_empty() {
        return Err(format!("{file_name:?} needs a name before .html"));
    }
    if stem.ends_with(' ') || stem.ends_with('.') {
        return Err(format!("{file_name:?} must not end in a space or a dot"));
    }
    // The part of the stem before the first dot is the name Windows reserves.
    let base = stem.split('.').next().unwrap_or(stem);
    if RESERVED_NAMES.contains(&base.to_ascii_lowercase().as_str()) {
        return Err(format!("{base:?} is a name Windows reserves"));
    }
    if canvas_proto::EFFECT_IDS.iter().any(|id| id.eq_ignore_ascii_case(stem)) {
        return Err(format!(
            "{stem:?} is a built-in effect; give the file another name"
        ));
    }
    Ok(format!("{USER_EFFECT_PREFIX}{stem}"))
}

/// The file in `dir` whose name is `file_name`, compared without regard to case.
///
/// Two names that differ only in case are the same file on NTFS, so the clash check
/// must not rely on that accident: the folder is listed and the names are compared
/// case-insensitively, the same way on every file system.
pub fn existing_file(dir: &Path, file_name: &str) -> Option<PathBuf> {
    let wanted = file_name.to_ascii_lowercase();
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.to_ascii_lowercase() == wanted {
            return Some(entry.path());
        }
    }
    None
}

/// Counts the temp files, so two writes in the same process never share a name.
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The temp file a write goes through: in the same folder as the target, so the
/// rename that follows is on the same volume.
fn temp_path(path: &Path) -> std::io::Result<PathBuf> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the path has no folder"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "effect.html".to_string());
    Ok(dir.join(format!(
        ".{name}.tmp{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::SeqCst)
    )))
}

/// Writes `bytes` to `path` atomically: a temp file in the same folder first, then a
/// rename over the target. A reader never sees a half-written file, and a failed write
/// leaves the old file alone.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the path has no folder"))?;
    std::fs::create_dir_all(dir)?;
    let temp = temp_path(path)?;
    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(bytes)?;
        // Data on disk before the rename commits, so a power loss cannot leave an empty target.
        file.sync_all()?;
    }
    match std::fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// Writes `bytes` to `path`, but only when nothing is there.
///
/// The file is written to a temp file and then hard-linked to its final name: a link
/// fails when the name is taken, so a file that appears between the check and the
/// write is never overwritten.
pub fn write_atomic_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the path has no folder"))?;
    std::fs::create_dir_all(dir)?;
    let temp = temp_path(path)?;
    std::fs::write(&temp, bytes)?;
    match std::fs::hard_link(&temp, path) {
        Ok(()) => {
            // The temp file is only a means to an atomic create; it goes now.
            let _ = std::fs::remove_file(&temp);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

fn read_lock(lock: &RwLock<State>) -> RwLockReadGuard<'_, State> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_lock(lock: &RwLock<State>) -> RwLockWriteGuard<'_, State> {
    lock.write().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::traits::{Canvas, Inputs};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A stand-in for an HTML effect: its name is the source's first line. A source that
    /// starts with `BROKEN` fails to parse.
    struct FakeHtml {
        schema: PropertySchema,
    }

    impl Effect for FakeHtml {
        fn schema(&self) -> PropertySchema {
            self.schema.clone()
        }

        fn init(&mut self, _props: &Props) {}

        fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {}
    }

    /// A builder that counts how many times it parses, so the tests can check that only
    /// changed files are parsed again.
    fn fake_builder(parses: Arc<AtomicUsize>) -> Builder {
        Arc::new(move |id: &str, source: &str| {
            parses.fetch_add(1, Ordering::SeqCst);
            if source.starts_with("BROKEN") {
                return Err("parse: no script".to_string());
            }
            let name = source.lines().next().unwrap_or("").trim().to_string();
            Ok(Box::new(FakeHtml {
                schema: PropertySchema {
                    id: id.to_string(),
                    name,
                    description: String::new(),
                    needs: Default::default(),
                    properties: Vec::new(),
                    presets: Vec::new(),
                },
            }) as Box<dyn Effect>)
        })
    }

    /// A fresh folder for one test, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("canvas-rgb-user-effects-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the temp folder is created");
            Self(path)
        }

        fn write(&self, file: &str, contents: &str) {
            std::fs::write(self.0.join(file), contents).expect("the file is written");
        }

        fn remove(&self, file: &str) {
            std::fs::remove_file(self.0.join(file)).expect("the file is removed");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(effects: &UserEffects) -> Vec<(String, String)> {
        effects.schemas().into_iter().map(|s| (s.id, s.name)).collect()
    }

    #[test]
    fn html_files_load_as_prefixed_ids_sorted_by_name() {
        let dir = TempDir::new("sorted");
        dir.write("zeta.html", "Zeta\n<canvas></canvas>");
        dir.write("alpha.html", "alpha lower\n<canvas></canvas>");
        dir.write("notes.txt", "Not an effect");
        let parses = Arc::new(AtomicUsize::new(0));
        let effects = UserEffects::new(fake_builder(parses.clone()));

        assert!(effects.rescan(&dir.0));

        assert_eq!(
            names(&effects),
            vec![
                ("html:alpha".to_string(), "alpha lower".to_string()),
                ("html:zeta".to_string(), "Zeta".to_string()),
            ]
        );
        assert_eq!(parses.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_broken_file_is_skipped_and_reported() {
        let dir = TempDir::new("broken");
        dir.write("good.html", "Good\n<canvas></canvas>");
        dir.write("bad.html", "BROKEN");
        let effects = UserEffects::new(fake_builder(Arc::new(AtomicUsize::new(0))));

        effects.rescan(&dir.0);

        assert_eq!(names(&effects), vec![("html:good".to_string(), "Good".to_string())]);
        let failures = effects.failures();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].0.ends_with("bad.html"));
        assert_eq!(failures[0].1, "parse: no script");
        assert!(effects.create("html:bad").is_none());
    }

    #[test]
    fn a_missing_folder_is_an_empty_folder() {
        let dir = TempDir::new("missing");
        let effects = UserEffects::new(fake_builder(Arc::new(AtomicUsize::new(0))));

        assert!(!effects.rescan(&dir.0.join("does-not-exist")));
        assert!(effects.schemas().is_empty());
    }

    #[test]
    fn only_changed_files_are_parsed_again() {
        let dir = TempDir::new("changes");
        dir.write("a.html", "A\n<canvas></canvas>");
        let parses = Arc::new(AtomicUsize::new(0));
        let effects = UserEffects::new(fake_builder(parses.clone()));
        effects.rescan(&dir.0);
        assert_eq!(parses.load(Ordering::SeqCst), 1);

        // Nothing changed: no parse, no change reported.
        assert!(!effects.rescan(&dir.0));
        assert_eq!(parses.load(Ordering::SeqCst), 1);

        // Edited (the size differs): parsed again, and the new name is listed.
        dir.write("a.html", "Renamed A\n<canvas></canvas>");
        assert!(effects.rescan(&dir.0));
        assert_eq!(parses.load(Ordering::SeqCst), 2);
        assert_eq!(names(&effects), vec![("html:a".to_string(), "Renamed A".to_string())]);
    }

    #[test]
    fn a_removed_file_drops_out_of_the_list() {
        let dir = TempDir::new("removed");
        dir.write("gone.html", "Gone\n<canvas></canvas>");
        let effects = UserEffects::new(fake_builder(Arc::new(AtomicUsize::new(0))));
        effects.rescan(&dir.0);
        assert_eq!(effects.schemas().len(), 1);

        dir.remove("gone.html");
        assert!(effects.rescan(&dir.0));
        assert!(effects.schemas().is_empty());
        assert!(effects.create("html:gone").is_none());
    }

    #[test]
    fn create_builds_from_the_loaded_source_and_ignores_built_ins() {
        let dir = TempDir::new("create");
        dir.write("live.html", "Live\n<canvas></canvas>");
        let parses = Arc::new(AtomicUsize::new(0));
        let effects = UserEffects::new(fake_builder(parses.clone()));
        effects.rescan(&dir.0);

        let made = effects.create("html:live").expect("a loaded effect is found");
        let effect = made.expect("the fake builds");
        assert_eq!(effect.schema().id, "html:live");
        // One parse for the scan, one for the build.
        assert_eq!(parses.load(Ordering::SeqCst), 2);

        assert!(effects.create("solid").is_none());
    }

    #[test]
    fn a_good_file_name_carries_the_html_prefix() {
        assert_eq!(check_file_name("my-effect.html").unwrap(), "html:my-effect");
        assert_eq!(check_file_name("My Effect 2.html").unwrap(), "html:My Effect 2");
        assert_eq!(check_file_name("a.b.html").unwrap(), "html:a.b");
    }

    #[test]
    fn the_bad_file_names_are_refused() {
        for name in [
            "",
            "   ",
            "my effect.html ",
            "sub/my-effect.html",
            "sub\\my-effect.html",
            "my-effect.txt",
            "my-effect",
            ".html",
            "..html",
            "..my-effect.html",
            "my..effect.html",
            "con.html",
            "NUL.html",
            "com1.html",
            "solid.html",
            "aurora.html",
            "my-effect.html.txt",
            "my:effect.html",
        ] {
            assert!(
                check_file_name(name).is_err(),
                "{name:?} must be refused"
            );
        }
    }

    #[test]
    fn a_reserved_name_is_refused_whatever_comes_after_the_first_dot() {
        // Windows reserves the name with any extension: the part before the first dot
        // is the name.
        for name in [
            "con.html",
            "con.x.html",
            "nul.a.html",
            "COM1.foo.html",
            "com0.html",
            "LPT9.html",
            "prn.anything.html",
            "aux.x.y.html",
        ] {
            assert!(check_file_name(name).is_err(), "{name:?} must be refused");
        }
        // A name that only holds a reserved word further along is fine.
        assert!(check_file_name("console.html").is_ok());
        assert!(check_file_name("com10.html").is_ok());
        assert!(check_file_name("nul.a.html.txt").is_err(), "it does not end in .html");
    }

    #[test]
    fn a_new_file_is_never_written_over_an_existing_one() {
        let dir = TempDir::new("create-new");
        let path = dir.0.join("effect.html");
        write_atomic(&path, b"first").expect("the file is written");

        // The second write must fail: the name is taken.
        let error = write_atomic_new(&path, b"second").expect_err("the write must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        // The file that was there is untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");

        // A name that is free is written.
        let fresh = dir.0.join("other.html");
        write_atomic_new(&fresh, b"new").expect("the file is written");
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "new");

        // No temp file is left behind.
        let leftovers: Vec<String> = std::fs::read_dir(&dir.0)
            .expect("the folder")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.ends_with(".html"))
            .collect();
        assert!(leftovers.is_empty(), "the temp files are gone: {leftovers:?}");
    }

    #[test]
    fn a_file_is_found_without_regard_to_case() {
        let dir = TempDir::new("case");
        dir.write("Foo.html", "F");
        assert!(existing_file(&dir.0, "foo.html").is_some());
        assert!(existing_file(&dir.0, "FOO.HTML").is_some());
        assert!(existing_file(&dir.0, "Foo.html").is_some());
        assert!(existing_file(&dir.0, "bar.html").is_none());
    }

    #[test]
    fn the_file_is_written_atomically_and_the_old_one_survives_a_failure() {
        let dir = TempDir::new("atomic");
        let path = dir.0.join("effect.html");
        write_atomic(&path, b"first").expect("the file is written");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");

        // A second write replaces it.
        write_atomic(&path, b"second").expect("the file is written");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");

        // No temp file is left behind.
        let leftovers: Vec<String> = std::fs::read_dir(&dir.0)
            .expect("the folder")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "effect.html")
            .collect();
        assert!(leftovers.is_empty(), "the temp files are gone: {leftovers:?}");
    }
}
