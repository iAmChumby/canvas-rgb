//! The `stacks.json` store: saving, loading, validating and deleting effect stacks.
//!
//! A stack is a spine type ([`canvas_proto::stack`]); this module owns the file it lives in and
//! the rules a stack must keep, and it holds the two pieces of runtime state the compositor
//! shares: the area book (one per daemon) and the identity of the stack that is on the engine.

use canvas_proto::props::PropertySchema;
use canvas_proto::stack::{
    Stack, StackFile, StackLayer, LAYER_NAME_MAX, STACKS_FILE, STACKS_SCHEMA, STACK_HTML_MAX,
    STACK_ID_PREFIX, STACK_MAX_LAYERS, STACK_NAME_MAX,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use crate::stack::{
    read_recovering, write_recovering, AreaBook, AreaMap, Clock, LiveMap, LiveStack, real_clock,
};
use crate::user_effects::UserEffects;

/// Every id and display name `find_schema` can resolve, so a stack's name never takes another
/// effect's name (D95.1.4 rule 6).
#[derive(Debug, Clone, Default)]
pub struct KnownEffects {
    entries: Vec<(String, String)>,
}

impl KnownEffects {
    /// The names built from effect schemas: each one's id and its display name.
    pub fn from_schemas(schemas: &[PropertySchema]) -> Self {
        let mut entries: Vec<(String, String)> = schemas
            .iter()
            .map(|schema| (schema.id.clone(), schema.name.clone()))
            .collect();
        // The hidden onboarding effects are found by `find_schema` too, so their names are
        // taken even though they are never listed.
        for schema in canvas_effects::onboard::hidden_schemas() {
            entries.push((schema.id, schema.name));
        }
        Self { entries }
    }

    /// The effects a stack's own layers can name: everything but the stacks themselves.
    pub fn from_user(user: &UserEffects) -> Self {
        let mut schemas: Vec<PropertySchema> =
            canvas_effects::registry().into_iter().map(|f| f().schema()).collect();
        schemas.extend(user.schemas());
        Self::from_schemas(&schemas)
    }

    /// True when `name` is an id or a display name of a known effect, ignoring ASCII case.
    pub fn takes(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(id, display)| id.eq_ignore_ascii_case(name) || display.eq_ignore_ascii_case(name))
            .map(|(id, _)| id.as_str())
    }
}

/// A stack name or a layer's name as it may be saved. The rules are the file-name rules with the
/// `.html` extension removed, plus this document's own (D95.1.4).
pub fn validate_name(name: &str, max: usize, what: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(format!("a {what} needs a name"));
    }
    if trimmed != name {
        return Err(format!("{name:?} has a space at either end"));
    }
    for c in name.chars() {
        if !(c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '.') {
            return Err(format!(
                "{name:?} may only hold letters, digits, spaces, '-', '_' and '.'"
            ));
        }
    }
    if name.contains("..") {
        return Err(format!("{name:?} must not hold '..'"));
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err(format!("{name:?} must not end in a space or a dot"));
    }
    let base = name.split('.').next().unwrap_or(name);
    if crate::user_effects::is_reserved_name(base) {
        return Err(format!("{base:?} is a name Windows reserves"));
    }
    let count = name.chars().count();
    if count > max {
        return Err(format!(
            "a {what} is at most {max} characters, and {name:?} is {count}"
        ));
    }
    Ok(())
}

/// [`validate_name`] for a stack, plus the rule that its name is not another effect's: a stack
/// called "Fire" would take the built-in's name wherever a person types one.
pub fn validate_stack_name(name: &str, known: &KnownEffects, others: &[Stack]) -> Result<(), String> {
    validate_name(name, STACK_NAME_MAX, "stack name")?;
    if let Some(id) = known.takes(name) {
        return Err(format!(
            "{name:?} is already the name of the effect {id}; give the stack another name"
        ));
    }
    if others
        .iter()
        .any(|other| other.name().eq_ignore_ascii_case(name))
    {
        return Err(format!("{name:?} is already another stack's name"));
    }
    Ok(())
}

/// True for a layer id: `l` and one or two digits (D95.1.7).
pub fn layer_id_ok(id: &str) -> bool {
    let Some(digits) = id.strip_prefix('l') else { return false };
    !digits.is_empty() && digits.len() <= 2 && digits.chars().all(|c| c.is_ascii_digit())
}

/// True for an effect id that is an HTML effect (the capped, slow kind).
pub fn is_html_effect(id: &str) -> bool {
    id.starts_with(crate::user_effects::USER_EFFECT_PREFIX)
}

/// The part of a layer that decides whether it can exist at all: its id, its name and its
/// effect. A layer that fails this is a different stack, so it is dropped at load and refused at
/// PUT.
fn check_layer_structure(layer: &StackLayer, seen: &mut Vec<String>) -> Result<(), String> {
    if !layer_id_ok(&layer.id) {
        return Err(format!(
            "layer {}: an id is `l` and one or two digits (l1, l12)",
            layer.id
        ));
    }
    if seen.contains(&layer.id) {
        return Err(format!("layer {}: two layers share this id", layer.id));
    }
    seen.push(layer.id.clone());
    validate_name(&layer.name, LAYER_NAME_MAX, "layer name")
        .map_err(|e| format!("layer {}: {e}", layer.id))?;
    if layer.effect.starts_with(STACK_ID_PREFIX) {
        return Err(format!(
            "layer {}: {} is a stack, and a stack cannot contain a stack",
            layer.id, layer.effect
        ));
    }
    Ok(())
}

/// The part of a layer that is only a value: its opacity and its rectangle. A PUT is refused for
/// either (422), and a file loaded from disk keeps the layer and lets the compositor clamp it
/// (D95.3.6), so a hand-written file never takes the render thread down.
fn check_layer_values(layer: &StackLayer) -> Result<(), String> {
    if layer.opacity > 100 {
        return Err(format!(
            "layer {}: opacity must be 0..=100, and it is {}",
            layer.id, layer.opacity
        ));
    }
    if let canvas_proto::stack::LayerArea::Rect { x, y, width, height } = layer.area {
        let finite = x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite();
        if !finite {
            return Err(format!(
                "layer {}: a rectangle's numbers must be finite, and x is {x}, y is {y}, width is {width}, height is {height}",
                layer.id
            ));
        }
        if !(0.0..=320.0).contains(&width) || !(0.0..=200.0).contains(&height) {
            return Err(format!(
                "layer {}: a rectangle is at most 320 wide and 200 tall, and it is {width} by {height}",
                layer.id
            ));
        }
        if width < 0.0 || height < 0.0 {
            return Err(format!(
                "layer {}: a rectangle's width and height are 0 or more",
                layer.id
            ));
        }
    }
    Ok(())
}

/// Checks one stack before it is saved (the D95.1.1 table).
///
/// `known` is what `find_schema` can resolve and `others` the stacks that are not this one:
/// both are what the name rules are checked against.
pub fn validate_stack(stack: &Stack, known: &KnownEffects, others: &[Stack]) -> Result<(), String> {
    let name = stack.name();
    if !stack.id.starts_with(STACK_ID_PREFIX) {
        return Err(format!(
            "a stack's id is {STACK_ID_PREFIX} and its name, and it is {:?}",
            stack.id
        ));
    }
    validate_stack_name(name, known, others)?;
    if stack.layers.len() > STACK_MAX_LAYERS {
        return Err(format!(
            "a stack holds at most {STACK_MAX_LAYERS} layers, and this one has {}",
            stack.layers.len()
        ));
    }
    let html = stack.layers.iter().filter(|l| is_html_effect(&l.effect)).count();
    if html > STACK_HTML_MAX {
        return Err(format!(
            "a stack holds at most {STACK_HTML_MAX} HTML layers, and this one has {html}"
        ));
    }
    let mut seen = Vec::new();
    for layer in &stack.layers {
        check_layer_structure(layer, &mut seen)?;
        check_layer_values(layer)?;
    }
    Ok(())
}

/// The layers of a stack loaded from a file: the ones the compositor cannot run are dropped with
/// a logged reason, and the rest of the stack runs (D95.1.6, D95.4.8).
fn loaded_layers(stack: &Stack) -> Vec<StackLayer> {
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<StackLayer> = Vec::new();
    let mut html = 0usize;
    for layer in &stack.layers {
        if out.len() >= STACK_MAX_LAYERS {
            tracing::warn!(
                stack = %stack.id,
                layer = %layer.id,
                "the stack has more layers than a stack may hold; the rest are dropped"
            );
            break;
        }
        if is_html_effect(&layer.effect) {
            if html >= STACK_HTML_MAX {
                tracing::warn!(
                    stack = %stack.id,
                    layer = %layer.id,
                    "the stack has more HTML layers than a stack may hold; the rest are dropped"
                );
                continue;
            }
            html += 1;
        }
        if let Err(reason) = check_layer_structure(layer, &mut seen) {
            tracing::warn!(stack = %stack.id, %reason, "a layer was dropped from the stack");
            continue;
        }
        if let Err(reason) = check_layer_values(layer) {
            // A value is not a reason to drop the layer: the compositor clamps it (D95.3.6).
            tracing::warn!(stack = %stack.id, %reason, "a layer's value is out of range; the compositor neutralises it");
        }
        out.push(layer.clone());
    }
    out
}

/// Why a stack could not be saved.
#[derive(Debug, thiserror::Error)]
pub enum StacksError {
    /// A save that breaks a rule (422).
    #[error("{0}")]
    Invalid(String),
    /// A name that is already another stack's, ignoring case (409).
    #[error("{0}")]
    Conflict(String),
    /// The file could not be written (500).
    #[error("{0}")]
    Io(String),
}

/// What one stack on the engine is: the structure it was built from, so a save or a save-then-
/// activate with the same structure rebuilds nothing.
#[derive(Debug, Clone)]
struct Running {
    id: String,
    structure: String,
    live: LiveStack,
}

/// The saved stacks, with the runtime state the compositor shares.
pub struct Stacks {
    home: PathBuf,
    file: RwLock<StackFile>,
    generation: AtomicU64,
    areas: AreaBook,
    fps: Arc<AtomicU32>,
    clock: Clock,
    running: Mutex<Option<Running>>,
    /// What the running stack's layers last reported (held, faulted), for `GET /stacks/{id}`.
    statuses: Arc<Mutex<Vec<canvas_proto::api::StackLayerStatus>>>,
}

impl Stacks {
    /// An empty store rooted at `home`. Nothing is read until [`Stacks::reload`].
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            file: RwLock::new(StackFile::default()),
            generation: AtomicU64::new(0),
            areas: Arc::new(RwLock::new(AreaMap::default())),
            fps: Arc::new(AtomicU32::new(canvas_proto::consts::FPS_DEFAULT)),
            clock: real_clock(),
            running: Mutex::new(None),
            statuses: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The clock the HTML hold is measured on. The daemon uses the real one; the tests move
    /// theirs by hand.
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// The store's folder.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Reads `stacks.json`. A missing file is no stacks; a file that does not parse is
    /// quarantined and the store starts empty; a stack that breaks a rule is dropped, or its
    /// bad layers are, with the reason logged.
    pub fn reload(&self, user: &UserEffects) {
        self.reload_with(&KnownEffects::from_user(user));
    }

    /// [`Stacks::reload`] with the effect names already known (the tests and the API's own
    /// schema list use this).
    pub fn reload_with(&self, known: &KnownEffects) {
        let path = self.path();
        let parsed: Option<StackFile> = match crate::storage::read_parsed(&path, STACKS_SCHEMA, "stacks.json") {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(error = %error, "stacks.json could not be read; starting with no stacks");
                None
            }
        };
        let Some(mut file) = parsed else {
            let mut guard = write_recovering(&self.file);
            *guard = StackFile::default();
            drop(guard);
            self.generation.fetch_add(1, Ordering::SeqCst);
            return;
        };
        let mut accepted: Vec<Stack> = Vec::new();
        for stack in file.stacks.drain(..) {
            let mut candidates: Vec<Stack> = accepted.clone();
            let name = stack.name().to_string();
            if let Err(reason) = validate_stack_name(&name, known, &candidates) {
                tracing::warn!(stack = %stack.id, %reason, "a stack was dropped as it was loaded");
                continue;
            }
            let layers = loaded_layers(&stack);
            let kept = Stack { id: stack.id.clone(), layers };
            candidates.push(kept.clone());
            accepted.push(kept);
        }
        let mut guard = write_recovering(&self.file);
        *guard = StackFile { schema_version: STACKS_SCHEMA, stacks: accepted };
        drop(guard);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Writes what the store holds to `stacks.json`.
    pub fn save(&self) -> Result<(), std::io::Error> {
        let file = read_recovering(&self.file).clone();
        crate::storage::write_json(&self.path(), &file)
    }

    /// Where `stacks.json` lives.
    pub fn path(&self) -> PathBuf {
        self.home.join(STACKS_FILE)
    }

    /// Every saved stack, in the order they are stored.
    pub fn all(&self) -> Vec<Stack> {
        read_recovering(&self.file).stacks.clone()
    }

    /// The stack with this id (`stack:<name>`) or this name, ignoring ASCII case.
    pub fn get(&self, id: &str) -> Option<Stack> {
        let wanted = id.trim();
        read_recovering(&self.file)
            .stacks
            .iter()
            .find(|stack| {
                stack.id.eq_ignore_ascii_case(wanted)
                    || stack.name().eq_ignore_ascii_case(wanted)
            })
            .cloned()
    }

    /// The generation: it moves on every read of the file and every write, so caches keyed on it
    /// (the tile cache) drop their entries when a stack changes.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Saves `stack`, replacing a stack of the same id (its case as stored) and refusing a name
    /// that is another stack's, ignoring case.
    pub fn put(&self, stack: Stack, known: &KnownEffects) -> Result<Stack, StacksError> {
        let name = stack.name().to_string();
        let mut guard = write_recovering(&self.file);
        let others: Vec<Stack> = guard
            .stacks
            .iter()
            .filter(|existing| existing.id != stack.id)
            .cloned()
            .collect();
        // A name another stack has, ignoring case, is a conflict before it is a validation
        // problem: the editor offers to replace it (D95.4.1).
        if others
            .iter()
            .any(|other| other.name().eq_ignore_ascii_case(&name))
        {
            return Err(StacksError::Conflict(format!(
                "{name:?} is already another stack's name, ignoring case"
            )));
        }
        validate_stack(&stack, known, &others).map_err(StacksError::Invalid)?;
        let parsed = Stack { id: stack.id.clone(), layers: stack.layers.clone() };
        match guard.stacks.iter_mut().find(|existing| existing.id == parsed.id) {
            Some(existing) => *existing = parsed.clone(),
            None => guard.stacks.push(parsed.clone()),
        }
        let file = guard.clone();
        drop(guard);
        crate::storage::write_json(&self.path(), &file).map_err(|e| StacksError::Io(e.to_string()))?;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(parsed)
    }

    /// Removes a stack. `None` when there was none.
    pub fn delete(&self, id: &str) -> Option<Stack> {
        let mut guard = write_recovering(&self.file);
        let index = guard
            .stacks
            .iter()
            .position(|stack| stack.id.eq_ignore_ascii_case(id) || stack.name().eq_ignore_ascii_case(id))?;
        let removed = guard.stacks.remove(index);
        let file = guard.clone();
        drop(guard);
        if let Err(error) = crate::storage::write_json(&self.path(), &file) {
            tracing::error!(error = %error, "stacks.json could not be written after a delete");
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        Some(removed)
    }

    // ------------------------------------------------------------ runtime state

    /// The area book every stack effect resolves its layers against.
    pub fn areas(&self) -> AreaBook {
        Arc::clone(&self.areas)
    }

    /// Publishes a new area book: the layout, the render scale or the sampling mode changed.
    ///
    /// The version moves, so every running stack re-resolves its layers on its next frame and
    /// rebuilds nothing.
    pub fn publish_areas(&self, layout: &canvas_proto::layout::Layout, devices: &[canvas_proto::DeviceInfo]) {
        let version = {
            let current = read_recovering(&self.areas);
            current.version.wrapping_add(1)
        };
        let map = AreaMap::build(layout, devices, version);
        let mut guard = write_recovering(&self.areas);
        *guard = map;
    }

    /// The frame rate the HTML budget is derived from.
    pub fn fps_handle(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.fps)
    }

    /// The frame rate the daemon is configured for.
    pub fn set_fps(&self, fps: u32) {
        self.fps.store(fps.clamp(1, 1000), Ordering::SeqCst);
    }

    /// The clock a stack's hold is measured on.
    pub fn clock(&self) -> Clock {
        Arc::clone(&self.clock)
    }

    /// The live map a build of `stack` uses: the running one when its structure is unchanged, so
    /// a save that changes only values does not rebuild, and a fresh one otherwise.
    pub fn live_for(&self, stack: &Stack, structure: &str) -> LiveStack {
        {
            let running = self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(running) = running.as_ref() {
                if running.id == stack.id && running.structure == structure {
                    {
                        let mut live = write_recovering(&running.live);
                        live.replace_with(stack);
                    }
                    return Arc::clone(&running.live);
                }
            }
        }
        Arc::new(RwLock::new(LiveMap::from_stack(stack, 1)))
    }

    /// Records that `stack` is what the engine is running, with the live map the effect holds.
    pub fn note_running(&self, id: &str, structure: &str, live: LiveStack) {
        *self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Running {
            id: id.to_string(),
            structure: structure.to_string(),
            live,
        });
        *self
            .statuses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Vec::new();
    }

    /// Nothing that is not a stack is on the engine any more.
    pub fn clear_running(&self) {
        *self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        *self
            .statuses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Vec::new();
    }

    /// The stack on the engine, when it is this one with this structure.
    pub fn running_is(&self, id: &str, structure: &str) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .is_some_and(|running| running.id == id && running.structure == structure)
    }

    /// The stack on the engine, if one is.
    pub fn running_id(&self) -> Option<String> {
        self.running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|running| running.id.clone())
    }

    /// Updates the running stack's live map from the file, when it is the stack on the engine.
    pub fn update_live(&self, stack: &Stack) {
        let running = self.running.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(running) = running.as_ref() {
            if running.id == stack.id {
                let mut live = write_recovering(&running.live);
                live.replace_with(stack);
            }
        }
    }

    /// The held and faulted layers of the running stack.
    pub fn runtime_statuses(&self) -> Vec<canvas_proto::api::StackLayerStatus> {
        self.statuses
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Where a stack effect reports its held and faulted layers.
    pub fn status_sink(&self) -> Arc<Mutex<Vec<canvas_proto::api::StackLayerStatus>>> {
        Arc::clone(&self.statuses)
    }
}

/// The instance of `stacks.json` in a folder, as a bundle export sees it. A missing file is no
/// stacks.
pub fn read_file(home: &Path) -> StackFile {
    crate::storage::read_parsed::<StackFile>(
        &home.join(STACKS_FILE),
        STACKS_SCHEMA,
        STACKS_FILE,
    )
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// Writes a stack file into `home`; `None` for the schema version of the file's own.
pub fn write_file(home: &Path, stacks: &[Stack]) -> std::io::Result<()> {
    let file = StackFile { schema_version: STACKS_SCHEMA, stacks: stacks.to_vec() };
    crate::storage::write_json(&home.join(STACKS_FILE), &file)
}

/// The instant a just-started store measures a hold against, for the tests that move a clock.
pub fn now() -> Instant {
    Instant::now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::stack::{BlendMode, LayerArea};

    fn stack(id: &str, layers: Vec<StackLayer>) -> Stack {
        Stack { id: id.to_string(), layers }
    }

    fn layer(id: &str, effect: &str) -> StackLayer {
        StackLayer {
            id: id.to_string(),
            name: format!("Layer {id}"),
            effect: effect.to_string(),
            props: Default::default(),
            area: LayerArea::Canvas,
            blend: BlendMode::Normal,
            opacity: 100,
            visible: true,
        }
    }

    fn known() -> KnownEffects {
        KnownEffects::from_schemas(
            &canvas_effects::registry().into_iter().map(|f| f().schema()).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn a_layer_id_is_l_and_one_or_two_digits() {
        for id in ["l1", "l9", "l10", "l99"] {
            assert!(layer_id_ok(id), "{id}");
        }
        for id in ["l", "l0x", "l100", "L1", "x1", "l.1", "l-1", ""] {
            assert!(!layer_id_ok(id), "{id}");
        }
    }

    #[test]
    fn a_stack_name_follows_the_file_name_rules() {
        let known = known();
        assert!(validate_stack_name("Sunset glow", &known, &[]).is_ok());
        assert!(validate_stack_name("a.b-c_1", &known, &[]).is_ok());
        for bad in ["", " lead", "trail ", "a:b", "con", "nul.a", "..", "x.", "a\u{1}"] {
            assert!(
                validate_stack_name(bad, &known, &[]).is_err(),
                "{bad:?} should be refused"
            );
        }
        let sixty = "a".repeat(60);
        assert!(validate_stack_name(&sixty, &known, &[]).is_ok());
        assert!(validate_stack_name(&"a".repeat(61), &known, &[]).is_err());
    }

    #[test]
    fn a_stack_name_may_not_take_an_effects_name() {
        let known = known();
        for taken in ["Fire", "fire", "Solid", "Hello", "onboard-hello", "Sweep"] {
            assert!(
                validate_stack_name(taken, &known, &[]).is_err(),
                "{taken} should be refused"
            );
        }
        let other = stack("stack:Sunset", vec![layer("l1", "solid")]);
        assert!(validate_stack_name("sunset", &known, &[other]).is_err());
    }

    #[test]
    fn the_layer_and_html_caps_are_enforced() {
        let known = known();
        let nine: Vec<StackLayer> = (1..=9).map(|i| layer(&format!("l{i}"), "solid")).collect();
        let error = validate_stack(&stack("stack:Nine", nine), &known, &[]).unwrap_err();
        assert!(error.contains("at most 8 layers"), "{error}");

        let three_html: Vec<StackLayer> = (1..=3)
            .map(|i| layer(&format!("l{i}"), "html:ink-drop"))
            .collect();
        let error = validate_stack(&stack("stack:Html", three_html), &known, &[]).unwrap_err();
        assert!(error.contains("at most 2 HTML layers"), "{error}");
    }

    #[test]
    fn a_nested_stack_and_a_bad_id_are_refused() {
        let known = known();
        let nested = stack("stack:A", vec![layer("l1", "stack:B")]);
        let error = validate_stack(&nested, &known, &[]).unwrap_err();
        assert!(error.contains("cannot contain a stack"), "{error}");

        let dotted = stack("stack:A", vec![layer("l1.1", "solid")]);
        assert!(validate_stack(&dotted, &known, &[]).is_err());

        let duplicate = stack("stack:A", vec![layer("l1", "solid"), layer("l1", "fire")]);
        let error = validate_stack(&duplicate, &known, &[]).unwrap_err();
        assert!(error.contains("share this id"), "{error}");
    }

    #[test]
    fn the_value_ranges_are_refused_with_their_range_named() {
        let known = known();
        let mut over = layer("l1", "solid");
        over.opacity = 101;
        let error = validate_stack(&stack("stack:A", vec![over]), &known, &[]).unwrap_err();
        assert!(error.contains("0..=100"), "{error}");

        let mut nan = layer("l1", "solid");
        nan.area = LayerArea::Rect { x: f32::NAN, y: 0.0, width: 10.0, height: 10.0 };
        let error = validate_stack(&stack("stack:A", vec![nan]), &known, &[]).unwrap_err();
        assert!(error.contains("finite"), "{error}");

        let mut wide = layer("l1", "solid");
        wide.area = LayerArea::Rect { x: 0.0, y: 0.0, width: 400.0, height: 10.0 };
        let error = validate_stack(&stack("stack:A", vec![wide]), &known, &[]).unwrap_err();
        assert!(error.contains("at most 320 wide"), "{error}");
    }

    #[test]
    fn the_store_round_trips_a_stack() {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-stacks-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Stacks::new(&dir);
        let known = known();
        let saved = store.put(stack("stack:Sunset glow", vec![layer("l1", "solid")]), &known).unwrap();
        assert_eq!(saved.name(), "Sunset glow");
        assert!(store.path().exists());

        let again = Stacks::new(&dir);
        let user = UserEffects::new(crate::user_effects::html_builder());
        again.reload(&user);
        assert_eq!(again.all(), vec![saved]);
        assert_eq!(again.generation(), 1);

        // A name that is another stack's, ignoring case, is a conflict.
        let taken = again.put(stack("stack:Sunset Glow", vec![layer("l1", "solid")]), &known);
        assert!(matches!(taken, Err(StacksError::Conflict(_))));

        assert!(again.delete("stack:Sunset glow").is_some());
        assert!(again.all().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hand_written_file_keeps_its_layers_and_drops_the_nested_one() {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-stacks-load-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(STACKS_FILE),
            r#"{"schema_version":1,"stacks":[
                {"id":"stack:Hand","layers":[
                  {"id":"l1","name":"Base","effect":"solid","opacity":255,
                   "area":{"kind":"rect","x":0.0,"y":0.0,"width":-4.0,"height":10.0}},
                  {"id":"l2","name":"Nested","effect":"stack:Other"},
                  {"id":"l3","name":"Gone","effect":"no-such-effect"}]}]}"#,
        )
        .unwrap();
        let store = Stacks::new(&dir);
        store.reload(&UserEffects::new(crate::user_effects::html_builder()));
        let hands = store.all();
        assert_eq!(hands.len(), 1);
        let ids: Vec<&str> = hands[0].layers.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, vec!["l1", "l3"], "the nested layer is dropped, the rest stay");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_quarantined_and_the_store_starts_empty() {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-stacks-corrupt-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(STACKS_FILE), b"{ not json {{").unwrap();
        let store = Stacks::new(&dir);
        store.reload(&UserEffects::new(crate::user_effects::html_builder()));
        assert!(store.all().is_empty());
        assert!(!dir.join(STACKS_FILE).exists(), "the corrupt file is moved aside");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_missing_file_is_no_stacks() {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-stacks-missing-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Stacks::new(&dir);
        store.reload(&UserEffects::new(crate::user_effects::html_builder()));
        assert!(store.all().is_empty());
        assert!(store.get("stack:Anything").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
