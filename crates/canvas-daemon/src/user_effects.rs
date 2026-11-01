//! User HTML effects: every `*.html` file in `%APPDATA%\canvas-rgb\effects\` is an effect
//! with the id `html:<file stem>`.
//!
//! Each file is parsed once to read its schema. The source text is kept, so activation
//! builds a fresh instance from it. The folder is rescanned on a fixed interval; only
//! files whose size or modified time changed are parsed again. A file that fails to
//! parse is logged with its error and left out of the list until it changes.
//!
//! A file whose *load* ran out of time is another case: it says how busy the machine was,
//! not what the file is, so it is parsed again after a back-off
//! ([`crate::thumbnail::TIMEOUT_RETRY_AFTER`]) instead of being left out for good. The
//! failure stays in the report while the file waits, so the editor still says what went
//! wrong.
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

/// The clock the retry back-off is measured against: a monotonic time since the effects
/// were loaded. The tests inject a clock they move by hand, so a back-off is tested
/// without waiting.
pub type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

/// The clock the daemon uses: the time since the effects were loaded. It never goes
/// backwards, so a back-off cannot be shortened by the clock being set.
fn monotonic_clock() -> Clock {
    let started = std::time::Instant::now();
    Arc::new(move || started.elapsed())
}

/// Turns an effect id and its HTML source into a runnable effect.
///
/// The daemon uses [`html_builder`]; the tests inject fakes.
pub type Builder = Arc<dyn Fn(&str, &str) -> Result<Box<dyn Effect>, String> + Send + Sync>;

/// The builder that runs real SignalRGB-format HTML effects through `canvas-html`.
pub fn html_builder() -> Builder {
    Arc::new(|id: &str, source: &str| {
        canvas_html::HtmlEffect::from_source(id, source, &Props::new())
            .map(|effect| {
                for warning in effect.warnings() {
                    tracing::warn!(id, %warning, "a user effect loaded with a part of its file ignored");
                }
                Box::new(effect) as Box<dyn Effect>
            })
            .map_err(|e| e.to_string())
    })
}

/// The builder that runs the same HTML effects for a library tile: a fixed seed, and no frame
/// dropped for being slow.
///
/// The lights get [`html_builder`], whose seed comes from the clock (every run of an effect
/// is a little different) and whose frames are dropped when they overrun the live deadline.
/// A tile is rendered again and again, on any machine and under any load, so it needs a seed
/// that does not move and frames that always finish. A frame that fails for any reason fails
/// the tile render ([`TileHtml`]), so a tile is never a quietly different picture.
pub fn html_builder_seeded(seed: u32) -> Builder {
    Arc::new(move |id: &str, source: &str| {
        canvas_html::HtmlEffect::from_source_seeded(id, source, &Props::new(), seed)
            .map(|effect| {
                for warning in effect.warnings() {
                    tracing::warn!(id, %warning, "a user effect loaded with a part of its file ignored");
                }
                Box::new(TileHtml(effect.tile_mode())) as Box<dyn Effect>
            })
            .map_err(|e| e.to_string())
    })
}

/// An HTML effect rendering a library tile. It runs as the lights would, except that a failed
/// frame ends the render: the tile path reads [`canvas_html::HtmlEffect::errors`] after every
/// step and stops with a [`crate::thumbnail::TileFailure`] when one appears.
struct TileHtml(canvas_html::HtmlEffect);

impl TileHtml {
    fn fail_on_error(&self) {
        if let Some(error) = self.0.errors().first() {
            // `resume_unwind` does not run the panic hook, so a bad effect is not logged as a
            // daemon panic; the render catches the payload and reports it.
            // A frame stopped for running past its deadline is a timing outcome: the render is
            // given up for now and tried again later, not for good.
            std::panic::resume_unwind(Box::new(crate::thumbnail::TileFailure {
                reason: format!("the script failed while the tile was drawn: {error}"),
                load_dependent: self.0.timed_out(),
            }));
        }
    }
}

impl Effect for TileHtml {
    fn schema(&self) -> PropertySchema {
        self.0.schema()
    }

    fn init(&mut self, props: &Props) {
        self.0.init(props);
        self.fail_on_error();
    }

    fn update(&mut self, dt: f32, canvas: &mut canvas_proto::traits::Canvas, inputs: &canvas_proto::traits::Inputs) {
        // Every frame of this update is held to the time left in the render's budget.
        self.0.set_give_up_at(crate::thumbnail::give_up_at());
        self.0.update(dt, canvas, inputs);
        self.fail_on_error();
    }
}

/// The HTML effects that ship inside the daemon, as (file stem, source). Each one has the id
/// `html:<stem>`, exactly as if its file sat in the effects folder, but it needs no file: it
/// is in the library on a fresh install and survives the folder being emptied. A file in the
/// effects folder with the same id replaces the shipped one until it is deleted.
pub const STOCK_EFFECTS: &[(&str, &str)] = &[
    ("ink-drop", include_str!("../stock-effects/ink-drop.html")),
    ("wet-smear", include_str!("../stock-effects/wet-smear.html")),
    ("palette-knife", include_str!("../stock-effects/palette-knife.html")),
];

/// A file's identity for change detection: modified time and size.
type Stamp = (Option<SystemTime>, u64);

#[derive(Clone)]
struct LoadedEffect {
    stamp: Stamp,
    schema: PropertySchema,
    source: Arc<str>,
}

#[derive(Clone)]
struct StockEffect {
    schema: PropertySchema,
    source: Arc<str>,
}

/// A file that did not parse.
struct Failure {
    /// The stamp the file failed at: a file that changes is parsed again at once.
    stamp: Stamp,
    /// The error, as the builder gave it.
    error: String,
    /// When the file may be parsed again: `Some` when the failure came from the machine's
    /// load (the effect's script ran past its load budget), which says how busy the
    /// machine was and not what the effect is. `None` when the file fails the same way
    /// every time, and only a changed file is worth another try.
    retry_at: Option<Duration>,
}

impl Failure {
    /// True when the file may be parsed again: the failure came from the machine's load
    /// and the back-off has passed.
    fn may_retry(&self, now: Duration) -> bool {
        self.retry_at.is_some_and(|at| now >= at)
    }
}

/// A stock effect that did not build.
#[derive(Clone)]
struct StockFailure {
    /// The source it is built from, so it can be tried again.
    source: Arc<str>,
    /// When it may be built again; see [`Failure::retry_at`].
    retry_at: Option<Duration>,
}

impl StockFailure {
    /// True when the effect may be built again; see [`Failure::may_retry`].
    fn may_retry(&self, now: Duration) -> bool {
        self.retry_at.is_some_and(|at| now >= at)
    }
}

/// A file that did not parse, and whether the machine was too busy for it.
struct ParseFailure {
    /// The error, as the builder gave it.
    error: String,
    /// True when the failure came from the machine's load: the effect's script ran past
    /// its load budget, so it says how busy the machine was and not what the effect is.
    /// The file is parsed again after a back-off.
    depends_on_load: bool,
}

impl ParseFailure {
    /// A failure of the file itself: it fails the same way every time.
    fn plain(error: String) -> Self {
        Self { error, depends_on_load: false }
    }

    /// A failure of the builder, told apart by what it says (see
    /// [`crate::thumbnail::build_error_depends_on_load`]).
    fn from_error(error: String) -> Self {
        let depends_on_load = crate::thumbnail::build_error_depends_on_load(&error);
        Self { error, depends_on_load }
    }
}

#[derive(Default)]
struct State {
    /// Files that parsed, by path.
    loaded: BTreeMap<PathBuf, LoadedEffect>,
    /// Files that failed to parse, by path, with the error and when they may be tried again.
    failed: BTreeMap<PathBuf, Failure>,
    /// The effects that ship inside the daemon, by id; see [`STOCK_EFFECTS`].
    stock: BTreeMap<String, StockEffect>,
    /// The stock effects that did not build, by id, with when they may be built again.
    stock_failed: BTreeMap<String, StockFailure>,
}

/// The user HTML effects found in the effects folder.
pub struct UserEffects {
    builder: Builder,
    /// The builder a library tile is rendered with: the same file, run with a fixed seed,
    /// so a tile is the same picture every time it is rendered.
    tile_builder: Builder,
    state: RwLock<State>,
    /// How long a file whose load ran out of time is left before it is parsed again.
    retry_after: Duration,
    /// The clock that back-off is measured against.
    clock: Clock,
    /// Bumped whenever a rescan changes the set of user effects, so caches keyed on it
    /// (the thumbnails) know to drop their entries.
    generation: AtomicU64,
}

impl UserEffects {
    pub fn new(builder: Builder) -> Self {
        Self::with_stock(builder, &[])
    }

    /// Like [`UserEffects::new`], with effects that ship inside the daemon. A stock effect
    /// that does not build is logged and left out.
    pub fn with_stock(builder: Builder, stock: &[(&str, &str)]) -> Self {
        Self::build(builder.clone(), builder, stock)
    }

    /// Like [`UserEffects::with_stock`], with a builder of its own for library tiles: the
    /// same file run with a fixed seed, so a tile is the same picture every time.
    pub fn with_stock_and_tiles(
        builder: Builder,
        tile_builder: Builder,
        stock: &[(&str, &str)],
    ) -> Self {
        Self::build(builder, tile_builder, stock)
    }

    /// Like [`UserEffects::with_stock`], with the back-off a file whose load ran out of
    /// time waits before it is parsed again, and the clock that back-off is measured
    /// against.
    ///
    /// The daemon uses [`crate::thumbnail::TIMEOUT_RETRY_AFTER`] and a monotonic clock; the
    /// tests shorten the back-off and move the clock, so the wait is tested without waiting.
    pub fn with_limits(
        builder: Builder,
        stock: &[(&str, &str)],
        retry_after: Duration,
        clock: Clock,
    ) -> Self {
        Self::build_with_limits(builder.clone(), builder, stock, retry_after, clock)
    }

    fn build(builder: Builder, tile_builder: Builder, stock: &[(&str, &str)]) -> Self {
        Self::build_with_limits(
            builder,
            tile_builder,
            stock,
            crate::thumbnail::TIMEOUT_RETRY_AFTER,
            monotonic_clock(),
        )
    }

    fn build_with_limits(
        builder: Builder,
        tile_builder: Builder,
        stock: &[(&str, &str)],
        retry_after: Duration,
        clock: Clock,
    ) -> Self {
        let now = clock();
        let mut state = State::default();
        for (stem, source) in stock {
            let id = format!("{USER_EFFECT_PREFIX}{stem}");
            match builder(&id, source) {
                Ok(effect) => {
                    let mut schema = effect.schema();
                    schema.id = id.clone();
                    state.stock.insert(id, StockEffect { schema, source: Arc::from(*source) });
                }
                Err(error) => {
                    tracing::error!(%id, %error, "a stock effect failed to build and was left out");
                    let depends_on_load = crate::thumbnail::build_error_depends_on_load(&error);
                    state.stock_failed.insert(
                        id,
                        StockFailure {
                            source: Arc::from(*source),
                            retry_at: depends_on_load.then(|| now + retry_after),
                        },
                    );
                }
            }
        }
        Self {
            builder,
            tile_builder,
            state: RwLock::new(state),
            retry_after,
            clock,
            generation: AtomicU64::new(0),
        }
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
        // A file with a stock effect's id replaces the stock one.
        for stock in state.stock.values() {
            if !out.iter().any(|s| s.id == stock.schema.id) {
                out.push(stock.schema.clone());
            }
        }
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
        let source = self.source_of(id)?;
        Some((self.builder)(id, &source))
    }

    /// Builds a fresh instance of a user effect for a library tile: the same file, run
    /// with the tile builder's fixed seed, so the tile is the same picture every time it
    /// is rendered. `None` when `id` is not a loaded user effect.
    pub fn create_for_tile(&self, id: &str) -> Option<Result<Box<dyn Effect>, String>> {
        let source = self.source_of(id)?;
        Some((self.tile_builder)(id, &source))
    }

    /// The source of a loaded or stock user effect.
    fn source_of(&self, id: &str) -> Option<Arc<str>> {
        let state = read_lock(&self.state);
        state
            .loaded
            .values()
            .find(|e| e.schema.id == id)
            .map(|e| e.source.clone())
            .or_else(|| state.stock.get(id).map(|e| e.source.clone()))
    }

    /// The files that failed to parse at their last scan, with the error.
    ///
    /// A file that is waiting to be parsed again (its load ran out of time) is in this list
    /// too: it did fail, and this says why.
    pub fn failures(&self) -> Vec<(PathBuf, String)> {
        let state = read_lock(&self.state);
        state
            .failed
            .iter()
            .map(|(path, failure)| (path.clone(), failure.error.clone()))
            .collect()
    }

    /// Brings the loaded list in line with `dir`. Returns true when anything changed.
    ///
    /// A missing folder is an empty folder. An unreadable folder keeps the last known
    /// list. Parsing happens outside the lock, so the API keeps answering while a file
    /// is read.
    ///
    /// A file whose load ran out of time is parsed again once the back-off has passed, so
    /// a busy machine does not lose an effect for good. A file that fails the same way
    /// every time is left out until it changes.
    pub fn rescan(&self, dir: &Path) -> bool {
        let now = (self.clock)();
        // A stock effect whose load ran out of time is built again after the back-off.
        let mut changed = self.retry_stock(now);
        let listing = match listing(dir) {
            Ok(listing) => listing,
            Err(error) => {
                tracing::warn!(dir = %dir.display(), %error, "could not read the user effects folder");
                if changed {
                    self.generation.fetch_add(1, Ordering::SeqCst);
                }
                return changed;
            }
        };

        // The files whose stamp differs from what is already known, and the files whose
        // load ran out of time and may be parsed again.
        let todo: Vec<(PathBuf, Stamp)> = {
            let state = read_lock(&self.state);
            listing
                .iter()
                .filter(|(path, stamp)| {
                    let known = state.loaded.get(*path).is_some_and(|e| e.stamp == **stamp);
                    let failed = state.failed.get(*path);
                    let known = known
                        || failed.is_some_and(|f| f.stamp == **stamp && !f.may_retry(now));
                    !known
                })
                .map(|(path, stamp)| (path.clone(), *stamp))
                .collect()
        };

        let parsed: Vec<(PathBuf, Stamp, Result<LoadedEffect, ParseFailure>)> = todo
            .into_iter()
            .map(|(path, stamp)| {
                let result = self.parse(&path, stamp);
                (path, stamp, result)
            })
            .collect();

        let mut state = write_lock(&self.state);

        for (path, stamp, result) in parsed {
            match result {
                Ok(effect) => {
                    let duplicate = state
                        .loaded
                        .iter()
                        .any(|(other, l)| other != &path && l.schema.id == effect.schema.id);
                    if duplicate {
                        let error = format!("duplicate effect id {}", effect.schema.id);
                        tracing::warn!(path = %path.display(), %error, "a user effect was skipped");
                        // The file was in the library before, so the set changed.
                        changed |= state.loaded.remove(&path).is_some();
                        state.failed.insert(
                            path,
                            Failure { stamp, error, retry_at: None },
                        );
                    } else {
                        tracing::info!(path = %path.display(), id = %effect.schema.id, "a user effect was loaded");
                        state.failed.remove(&path);
                        state.loaded.insert(path, effect);
                        changed = true;
                    }
                }
                Err(failure) => {
                    tracing::warn!(path = %path.display(), error = %failure.error, "a user effect failed to parse and was skipped");
                    // The file was in the library before, so the set changed.
                    changed |= state.loaded.remove(&path).is_some();
                    // A failure that came from the machine's load is tried again after the
                    // back-off; one that fails the same way every time is not.
                    let retry_at = failure.depends_on_load.then(|| now + self.retry_after);
                    state.failed.insert(
                        path,
                        Failure { stamp, error: failure.error, retry_at },
                    );
                }
            }
        }

        let loaded_before = state.loaded.len();
        state.loaded.retain(|path, _| listing.contains_key(path));
        state.failed.retain(|path, _| listing.contains_key(path));
        if state.loaded.len() != loaded_before {
            changed = true;
        }
        if changed {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        changed
    }

    /// Builds the stock effects whose load ran out of time and may be built again.
    ///
    /// A stock effect that fails the same way every time is left out until the daemon
    /// starts again; one that only ran out of time says how busy the machine was, so it is
    /// built again after the back-off.
    fn retry_stock(&self, now: Duration) -> bool {
        let due: Vec<(String, Arc<str>)> = {
            let state = read_lock(&self.state);
            state
                .stock_failed
                .iter()
                .filter(|(_, failure)| failure.may_retry(now))
                .map(|(id, failure)| (id.clone(), failure.source.clone()))
                .collect()
        };
        if due.is_empty() {
            return false;
        }

        // The builds run outside the lock, so the API keeps answering while they do.
        let built: Vec<(String, Arc<str>, Result<StockEffect, String>)> = due
            .into_iter()
            .map(|(id, source)| {
                let result = (self.builder)(&id, &source).map(|effect| {
                    let mut schema = effect.schema();
                    schema.id = id.clone();
                    StockEffect { schema, source: source.clone() }
                });
                (id, source, result)
            })
            .collect();

        let mut changed = false;
        let mut state = write_lock(&self.state);
        for (id, source, result) in built {
            match result {
                Ok(stock) => {
                    tracing::info!(%id, "a stock effect was loaded");
                    state.stock_failed.remove(&id);
                    state.stock.insert(id, stock);
                    changed = true;
                }
                Err(error) => {
                    tracing::error!(%id, %error, "a stock effect failed to build and was left out");
                    let depends_on_load = crate::thumbnail::build_error_depends_on_load(&error);
                    state.stock_failed.insert(
                        id,
                        StockFailure {
                            source,
                            retry_at: depends_on_load.then(|| now + self.retry_after),
                        },
                    );
                }
            }
        }
        changed
    }

    /// Reads and parses one file. The id comes from the file stem.
    fn parse(&self, path: &Path, stamp: Stamp) -> Result<LoadedEffect, ParseFailure> {
        let stem = path
            .file_stem()
            .ok_or_else(|| ParseFailure::plain("the file has no name".to_string()))?
            .to_string_lossy()
            .into_owned();
        let id = format!("{USER_EFFECT_PREFIX}{stem}");
        let source = std::fs::read_to_string(path)
            .map_err(|e| ParseFailure::plain(format!("could not read the file: {e}")))?;
        let effect = (self.builder)(&id, &source).map_err(ParseFailure::from_error)?;
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
            Ok(Box::new(FakeHtml { schema: fake_schema(id, source) }) as Box<dyn Effect>)
        })
    }

    /// The schema the fake builds: its name is the source's first line.
    fn fake_schema(id: &str, source: &str) -> PropertySchema {
        PropertySchema {
            id: id.to_string(),
            name: source.lines().next().unwrap_or("").trim().to_string(),
            description: String::new(),
            needs: Default::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    /// The error a load that ran out of time gives: the runtime's own text, which says
    /// how busy the machine was and not what the effect is.
    const LOAD_TIMEOUT: &str = "timeout: the effect's script ran longer than 2000 ms";

    /// A builder whose first `times_out` calls fail the way a load that ran out of time
    /// does, and that builds the effect after that.
    fn timing_out_builder(parses: Arc<AtomicUsize>, times_out: usize) -> Builder {
        Arc::new(move |id: &str, source: &str| {
            let count = parses.fetch_add(1, Ordering::SeqCst);
            if count < times_out {
                return Err(LOAD_TIMEOUT.to_string());
            }
            Ok(Box::new(FakeHtml { schema: fake_schema(id, source) }) as Box<dyn Effect>)
        })
    }

    /// A clock the test moves by hand: the back-off is tested without waiting.
    #[derive(Clone)]
    struct TestClock {
        now: Arc<std::sync::Mutex<Duration>>,
    }

    impl TestClock {
        fn new() -> Self {
            Self { now: Arc::new(std::sync::Mutex::new(Duration::ZERO)) }
        }

        /// The clock the effects read.
        fn clock(&self) -> Clock {
            let now = self.now.clone();
            Arc::new(move || *now.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
        }

        /// Moves the clock on.
        fn advance(&self, by: Duration) {
            let mut now = self.now.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            *now += by;
        }
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

    /// The back-off the tests use: long enough that a scan inside it does not parse the
    /// file again, and the test moves the clock past it.
    const TEST_RETRY: Duration = Duration::from_secs(60);

    /// A file whose load ran out of time is not left out of the library for good: a later
    /// scan parses it again, and it is in the library when it loads.
    #[test]
    fn a_file_whose_load_timed_out_is_in_the_library_after_a_later_scan() {
        let dir = TempDir::new("timeout-retry");
        dir.write("slow.html", "Slow\n<canvas></canvas>");
        let parses = Arc::new(AtomicUsize::new(0));
        let clock = TestClock::new();
        let effects = UserEffects::with_limits(
            timing_out_builder(parses.clone(), 1),
            &[],
            TEST_RETRY,
            clock.clock(),
        );

        // The first scan: the load runs out of time.
        effects.rescan(&dir.0);
        assert_eq!(parses.load(Ordering::SeqCst), 1);
        assert!(effects.schemas().is_empty(), "the effect is not in the library yet");
        // The failure is reported, and it says why.
        let failures = effects.failures();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].0.ends_with("slow.html"), "{failures:?}");
        assert!(failures[0].1.contains("ran longer than"), "the reason is kept: {failures:?}");

        // Inside the back-off the file is left alone.
        effects.rescan(&dir.0);
        assert_eq!(
            parses.load(Ordering::SeqCst),
            1,
            "the file was parsed again inside the back-off"
        );

        // After the back-off it is parsed again, and now it loads.
        clock.advance(TEST_RETRY);
        effects.rescan(&dir.0);
        assert_eq!(
            parses.load(Ordering::SeqCst),
            2,
            "the file was not parsed again after the back-off"
        );
        assert_eq!(names(&effects), vec![("html:slow".to_string(), "Slow".to_string())]);
        assert!(effects.failures().is_empty(), "the failure is gone: the file loads now");
    }

    /// A file that does not parse fails the same way every time: it is left out until it
    /// changes, however long the daemon has been running.
    #[test]
    fn a_real_parse_error_is_skipped_until_the_file_changes() {
        let dir = TempDir::new("parse-error");
        dir.write("bad.html", "BROKEN");
        let parses = Arc::new(AtomicUsize::new(0));
        let clock = TestClock::new();
        let effects = UserEffects::with_limits(
            fake_builder(parses.clone()),
            &[],
            TEST_RETRY,
            clock.clock(),
        );

        effects.rescan(&dir.0);
        assert_eq!(parses.load(Ordering::SeqCst), 1);
        assert_eq!(effects.failures().len(), 1);

        // A later scan does not parse it again, however far the clock has moved.
        clock.advance(TEST_RETRY * 10);
        effects.rescan(&dir.0);
        assert_eq!(
            parses.load(Ordering::SeqCst),
            1,
            "a file that does not parse was parsed again"
        );
        assert!(effects.schemas().is_empty());

        // The file changes: it is parsed again.
        dir.write("bad.html", "Fixed\n<canvas></canvas>");
        effects.rescan(&dir.0);
        assert_eq!(parses.load(Ordering::SeqCst), 2, "a changed file is parsed again");
        assert_eq!(names(&effects), vec![("html:bad".to_string(), "Fixed".to_string())]);
    }

    /// A script that is always too slow is not parsed on every scan: it waits for the
    /// back-off, and it stays in the report while it waits.
    #[test]
    fn a_file_that_always_times_out_is_not_parsed_again_before_the_back_off() {
        let dir = TempDir::new("always-slow");
        dir.write("slow.html", "Slow\n<canvas></canvas>");
        let parses = Arc::new(AtomicUsize::new(0));
        let clock = TestClock::new();
        let effects = UserEffects::with_limits(
            timing_out_builder(parses.clone(), usize::MAX),
            &[],
            TEST_RETRY,
            clock.clock(),
        );

        effects.rescan(&dir.0);
        assert_eq!(parses.load(Ordering::SeqCst), 1);

        // Every scan inside the back-off leaves the file alone.
        for _ in 0..3 {
            clock.advance(Duration::from_secs(10));
            effects.rescan(&dir.0);
        }
        assert_eq!(
            parses.load(Ordering::SeqCst),
            1,
            "the file was parsed again inside the back-off"
        );

        // Past the back-off it is tried again: the machine may be free now.
        clock.advance(TEST_RETRY);
        effects.rescan(&dir.0);
        assert_eq!(
            parses.load(Ordering::SeqCst),
            2,
            "the file was not parsed again after the back-off"
        );
        // It is still reported as failed: it did fail, and why.
        let failures = effects.failures();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].1.contains("ran longer than"), "{failures:?}");
        assert!(effects.schemas().is_empty());
    }

    /// A stock effect is built when the daemon starts; if its load runs out of time then,
    /// it is built again after the back-off instead of being left out for good.
    #[test]
    fn a_stock_effect_whose_load_timed_out_is_built_after_the_back_off() {
        let dir = TempDir::new("stock-timeout");
        let builds = Arc::new(AtomicUsize::new(0));
        let clock = TestClock::new();
        let effects = UserEffects::with_limits(
            timing_out_builder(builds.clone(), 1),
            &[("stock-one", "Stock One\n<canvas></canvas>")],
            TEST_RETRY,
            clock.clock(),
        );

        // The first build ran out of time: the effect is not in the library.
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(effects.schemas().is_empty(), "the stock effect is not in the library yet");

        // Inside the back-off it is not built again.
        clock.advance(Duration::from_secs(10));
        effects.rescan(&dir.0);
        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "the stock effect was built again inside the back-off"
        );

        // After the back-off it is built, and it is in the library.
        clock.advance(TEST_RETRY);
        effects.rescan(&dir.0);
        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "the stock effect was not built again after the back-off"
        );
        assert_eq!(
            names(&effects),
            vec![("html:stock-one".to_string(), "Stock One".to_string())]
        );
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
    /// An effect whose third frame throws.
    const THROWS_IN_FRAME_THREE: &str = r##"<head><title>Throws</title></head>
<body><canvas id="c" width="8" height="8"></canvas></body>
<script>
var n = 0;
function draw() {
  n++;
  if (n === 3) { throw new Error("boom in frame three"); }
  window.requestAnimationFrame(draw);
}
window.requestAnimationFrame(draw);
</script>"##;

    #[test]
    fn a_tile_frame_that_throws_fails_the_tile_and_the_lights_carry_on() {
        // The tile path: the render fails, and says why.
        let mut tile = html_builder_seeded(1)("html:throws", THROWS_IN_FRAME_THREE).expect("the effect loads");
        let error = crate::thumbnail::render(tile.as_mut(), &Props::new())
            .expect_err("a tile whose frame throws is not rendered");
        assert!(!error.depends_on_load(), "{error}");
        assert!(error.to_string().contains("boom in frame three"), "{error}");

        // The lights: the same frame is reported in the log and the effect carries on.
        let mut lights = html_builder()("html:throws", THROWS_IN_FRAME_THREE).expect("the effect loads");
        let mut canvas = canvas_proto::traits::Canvas::new(8, 8);
        for _ in 0..10 {
            lights.update(1.0 / 60.0, &mut canvas, &canvas_proto::traits::Inputs::default());
        }
    }

    /// An effect whose second frame is slow: a long loop, far over the 100 ms live budget on any
    /// machine. It paints red, then green when the slow frame finishes.
    const SLOW_SECOND_FRAME: &str = r##"<head><title>Slow</title></head>
<body><canvas id="c" width="8" height="8"></canvas></body>
<script>
var ctx = document.getElementById("c").getContext("2d");
var frames = 0;
function draw() {
  frames++;
  if (frames === 2) { var sum = 0; for (var i = 0; i < 20000000; i++) { sum += i % 7; } }
  ctx.fillStyle = frames >= 2 ? "#00ff00" : "#ff0000";
  ctx.fillRect(0, 0, 8, 8);
  window.requestAnimationFrame(draw);
}
window.requestAnimationFrame(draw);
</script>"##;

    #[test]
    fn a_library_tile_does_not_drop_a_slow_frame_and_the_lights_do() {
        let paint_after_two_frames = |mut effect: Box<dyn Effect>| {
            let mut canvas = Canvas::new(8, 8);
            for _ in 0..2 {
                effect.update(1.0 / 60.0, &mut canvas, &Inputs::default());
            }
            canvas.get(0, 0)
        };
        // The lights drop the slow frame (the live deadline) and keep the red one.
        let lights = paint_after_two_frames(html_builder()("html:slow", SLOW_SECOND_FRAME).expect("loads"));
        assert!(lights[0] > 0.9 && lights[1] < 0.1, "the lights kept the last good frame: {lights:?}");
        // A tile finishes every frame, whatever the machine's load: the slow frame is green.
        let tile = paint_after_two_frames(html_builder_seeded(1)("html:slow", SLOW_SECOND_FRAME).expect("loads"));
        assert!(tile[1] > 0.9 && tile[0] < 0.1, "the tile dropped a slow frame: {tile:?}");
    }

    /// An effect whose frame never returns.
    const NEVER_RETURNS: &str = r##"<head><title>Never</title></head>
<body><canvas id="c" width="8" height="8"></canvas></body>
<script>
function draw() { while (true) {} }
window.requestAnimationFrame(draw);
</script>"##;

    #[test]
    fn a_tile_frame_that_ran_out_of_time_is_over_budget_and_retried_not_failed_for_good() {
        let mut tile = html_builder_seeded(1)("html:never", NEVER_RETURNS).expect("the effect loads");
        let started = std::time::Instant::now();
        let error = crate::thumbnail::render_within(tile.as_mut(), &Props::new(), std::time::Duration::from_millis(400))
            .expect_err("a frame that never returns ends the render");
        let took = started.elapsed();
        assert!(error.depends_on_load(), "a timeout is a timing outcome, retried later: {error:?}");
        assert!(
            matches!(error, crate::thumbnail::RenderError::OverBudget(_)),
            "a timed-out tile frame is OverBudget, not Failed for good: {error:?}"
        );
        // The frame is held to the time the render has left, not to the 30 s frame budget.
        assert!(took < std::time::Duration::from_secs(5), "the render ran for {took:?} on a 400 ms budget");
    }

    /// An effect whose every frame burns `BURN` loop iterations.
    fn burning_frames(iterations: u64) -> String {
        format!(
            r##"<head><title>Burn</title></head>
<body><canvas id="c" width="8" height="8"></canvas></body>
<script>
function draw() {{ var s = 0; for (var i = 0; i < {iterations}; i++) {{ s += i % 7; }} window.requestAnimationFrame(draw); }}
window.requestAnimationFrame(draw);
</script>"##
        )
    }

    #[test]
    fn a_tile_render_does_not_hold_its_slot_much_past_its_budget() {
        // Calibrate: how long does one frame of a fixed loop take on this machine, right now?
        let probe = 200_000u64;
        let mut effect = html_builder_seeded(1)("html:burn", &burning_frames(probe)).expect("loads");
        let mut canvas = Canvas::new(8, 8);
        let started = std::time::Instant::now();
        effect.update(1.0 / 60.0, &mut canvas, &Inputs::default());
        let per_probe = started.elapsed().as_secs_f64().max(1e-4);
        // Frames of about half a second each. A step of 1/15 s runs four of them, two seconds.
        let frame_s = 0.5;
        let iterations = (probe as f64 * frame_s / per_probe) as u64;

        let budget = std::time::Duration::from_millis(700);
        let mut tile = html_builder_seeded(1)("html:burn", &burning_frames(iterations)).expect("loads");
        let started = std::time::Instant::now();
        let error = crate::thumbnail::render_within(tile.as_mut(), &Props::new(), budget)
            .expect_err("frames of half a second cannot finish a render in 700 ms");
        let took = started.elapsed();
        assert!(error.depends_on_load(), "{error:?}");
        // Without a bound per frame, the first step runs its four frames (two seconds) before the
        // budget is looked at.
        assert!(
            took < budget + std::time::Duration::from_millis(400),
            "the render held its slot for {took:?} on a {budget:?} budget"
        );
    }

    #[test]
    fn a_library_tile_is_built_with_the_tile_builder() {
        // The lights get the builder that seeds from the clock; a tile gets the seeded one,
        // so a tile is the same picture every time it is rendered.
        let calls: Arc<std::sync::Mutex<Vec<&'static str>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let make = |name: &'static str| {
            let calls = calls.clone();
            Arc::new(move |id: &str, _source: &str| {
                calls.lock().unwrap().push(name);
                Ok(Box::new(FakeHtml {
                    schema: PropertySchema {
                        id: id.to_string(),
                        name: id.to_string(),
                        description: String::new(),
                        needs: Default::default(),
                        properties: Vec::new(),
                        presets: Vec::new(),
                        thumbnail: None,
                    },
                }) as Box<dyn Effect>)
            }) as Builder
        };
        let dir = TempDir::new("tile-builder");
        dir.write("live.html", "Live\n<canvas></canvas>");
        let effects = UserEffects::with_stock_and_tiles(make("lights"), make("tiles"), &[]);
        effects.rescan(&dir.0);
        effects.create("html:live").expect("a loaded effect").expect("the effect builds");
        effects.create_for_tile("html:live").expect("a loaded effect").expect("the effect builds");
        let calls = calls.lock().unwrap().clone();
        assert!(calls.contains(&"lights"), "the effect on the lights was not built: {calls:?}");
        assert!(calls.contains(&"tiles"), "the tile was not built with the tile builder: {calls:?}");
    }

    #[test]
    fn the_stock_effects_are_listed_and_build_with_the_real_engine() {
        let effects = UserEffects::with_stock(html_builder(), STOCK_EFFECTS);
        let ids: Vec<String> = effects.schemas().into_iter().map(|s| s.id).collect();
        for stem in ["ink-drop", "wet-smear", "palette-knife"] {
            let id = format!("html:{stem}");
            assert!(ids.contains(&id), "{id} is missing from {ids:?}");
            let built = effects.create(&id).expect("a stock id is known").expect("it builds");
            assert!(!built.schema().name.is_empty());
        }
        assert_eq!(ids.len(), STOCK_EFFECTS.len());
    }

    #[test]
    fn a_file_with_a_stock_id_replaces_it_until_the_file_is_deleted() {
        let dir = TempDir::new("stock-override");
        let parses = Arc::new(AtomicUsize::new(0));
        let effects = UserEffects::with_stock(
            fake_builder(parses),
            &[("ink-drop", "Stock Ink
<canvas></canvas>")],
        );
        assert_eq!(names(&effects), vec![("html:ink-drop".to_string(), "Stock Ink".to_string())]);

        dir.write("ink-drop.html", "My Ink
<canvas></canvas>");
        effects.rescan(&dir.0);
        assert_eq!(names(&effects), vec![("html:ink-drop".to_string(), "My Ink".to_string())]);

        dir.remove("ink-drop.html");
        effects.rescan(&dir.0);
        assert_eq!(names(&effects), vec![("html:ink-drop".to_string(), "Stock Ink".to_string())]);
        assert!(effects.create("html:ink-drop").is_some());
    }

}
