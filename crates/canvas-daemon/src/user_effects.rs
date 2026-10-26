//! User HTML effects: every `*.html` file in `%APPDATA%\canvas-rgb\effects\` is an effect
//! with the id `html:<file stem>`.
//!
//! Each file is parsed once to read its schema. The source text is kept, so activation
//! builds a fresh instance from it. The folder is rescanned on a fixed interval; only
//! files whose size or modified time changed are parsed again. A file that fails to
//! parse is logged with its error and left out of the list until it changes.

use canvas_proto::props::{Props, PropertySchema};
use canvas_proto::traits::Effect;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
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
}

impl UserEffects {
    pub fn new(builder: Builder) -> Self {
        Self { builder, state: RwLock::new(State::default()) }
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

        assert!(effects.create("poison").is_none());
    }
}
