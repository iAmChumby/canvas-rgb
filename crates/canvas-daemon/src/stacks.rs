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
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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
    // `PUT /api/v1/stacks/preview` is the builder's live preview, so a stack may not take that
    // bare name: the save route and the preview route would be the same URL.
    if name.eq_ignore_ascii_case("preview") {
        return Err(format!(
            "{name:?} is reserved for the stack preview route; give the stack another name"
        ));
    }
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

/// The canonical id of the effect a layer names, or `None` when nothing builds it: a built-in,
/// a user effect or a hidden onboarding one, matched by id or by display name. A layer builder
/// takes ids, so a layer saved with "Fire" is stored as `fire`; a stack is never a layer's
/// effect, and a name that resolves only to a stack is `None` (D95.4.8).
pub fn layer_effect_id<'a>(known: &'a KnownEffects, effect: &str) -> Option<&'a str> {
    if effect.starts_with(STACK_ID_PREFIX) {
        return None;
    }
    known.takes(effect)
}

/// How a refusal names a layer: by the name the person gave it, which the editor shows, and by
/// its id only when it has no name (the id is internal: `l1` means nothing on screen).
fn who(layer: &StackLayer) -> String {
    if layer.name.trim().is_empty() {
        format!("layer {}", layer.id)
    } else {
        format!("layer \"{}\"", layer.name.trim())
    }
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
            "{}: {} is a stack, and a stack cannot contain a stack",
            who(layer), layer.effect
        ));
    }
    Ok(())
}

/// The part of a layer that is only a value: its opacity and its rectangle. A PUT is refused for
/// either (422), and a file loaded from disk keeps the layer and lets the compositor clamp it
/// (D95.3.6), so a hand-written file never takes the render thread down. `index` is the layer's
/// position in the stack, named in the refusal so the editor can point at the offending layer.
fn check_layer_values(layer: &StackLayer, index: usize) -> Result<(), String> {
    if layer.opacity > 100 {
        return Err(format!(
            "{}: opacity must be 0..=100, and it is {}",
            who(layer), layer.opacity
        ));
    }
    if let Some(mask) = &layer.mask {
        if mask.amount > 100 {
            return Err(format!(
                "{}: the mask's amount must be 0..=100, and it is {}",
                who(layer), mask.amount
            ));
        }
    }
    if let canvas_proto::stack::LayerArea::Rect { x, y, width, height } = layer.area {
        let finite = x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite();
        if !finite {
            return Err(format!(
                "{}: a rectangle's numbers must be finite, and x is {x}, y is {y}, width is {width}, height is {height}",
                who(layer)
            ));
        }
        if !(0.0..=320.0).contains(&width) || !(0.0..=200.0).contains(&height) {
            return Err(format!(
                "{}: a rectangle is at most 320 wide and 200 tall, and it is {width} by {height}",
                who(layer)
            ));
        }
        if width < 0.0 || height < 0.0 {
            return Err(format!(
                "{}: a rectangle's width and height are 0 or more",
                who(layer)
            ));
        }
        let canvas_w = canvas_proto::CANVAS_WIDTH as f32;
        let canvas_h = canvas_proto::CANVAS_HEIGHT as f32;
        // The rectangle's right and bottom edges are computed in f32: a huge x or width
        // overflows to infinity here, and infinity is past the canvas, so the check refuses
        // it as an edge failure instead of panicking.
        if x < 0.0 {
            return Err(format!(
                "{} ({}) at index {index}: the left edge x = {x} is past the canvas; x must be 0 or more in the {canvas_w}-wide canvas",
                who(layer), layer.effect
            ));
        }
        if y < 0.0 {
            return Err(format!(
                "{} ({}) at index {index}: the top edge y = {y} is past the canvas; y must be 0 or more in the {canvas_h}-tall canvas",
                who(layer), layer.effect
            ));
        }
        if x + width > canvas_w {
            return Err(format!(
                "{} ({}) at index {index}: the right edge x + width = {} is past the {canvas_w}-wide canvas",
                who(layer),
                layer.effect,
                x + width
            ));
        }
        if y + height > canvas_h {
            return Err(format!(
                "{} ({}) at index {index}: the bottom edge y + height = {} is past the {canvas_h}-tall canvas",
                who(layer),
                layer.effect,
                y + height
            ));
        }
    }
    Ok(())
}

/// A layer's mask has to be an effect the daemon can run (#256): never a stack, and never an id
/// that resolves to nothing. An HTML mask is allowed and counts toward the HTML cap.
fn check_mask(layer: &StackLayer, known: &KnownEffects) -> Result<(), String> {
    let Some(mask) = &layer.mask else { return Ok(()) };
    if mask.effect.starts_with(STACK_ID_PREFIX) {
        return Err(format!(
            "{}: the mask effect {} is a stack, and a stack cannot contain a stack",
            who(layer), mask.effect
        ));
    }
    if layer_effect_id(known, &mask.effect).is_none() {
        return Err(format!(
            "{}: the mask effect {:?} is not an effect",
            who(layer), mask.effect
        ));
    }
    Ok(())
}

/// The HTML slots one layer takes: one for its own effect and one for its mask (#256). A stack
/// may hold `STACK_HTML_MAX` slots in all, and save and load both count them with this. A mask
/// whose effect does not resolve takes no slot: it cannot build, so the layer draws unmasked.
pub fn html_slots(layer: &StackLayer, known: &KnownEffects) -> usize {
    let mask = layer.mask.as_ref().is_some_and(|mask| {
        is_html_effect(&mask.effect) && layer_effect_id(known, &mask.effect).is_some()
    });
    usize::from(is_html_effect(&layer.effect)) + usize::from(mask)
}

/// True for a layer that takes at least one HTML slot: the capped, slow kind.
pub fn layer_counts_as_html(layer: &StackLayer, known: &KnownEffects) -> bool {
    html_slots(layer, known) > 0
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
    let html: usize = stack.layers.iter().map(|layer| html_slots(layer, known)).sum();
    if html > STACK_HTML_MAX {
        return Err(format!(
            "a stack holds at most {STACK_HTML_MAX} HTML layers, and this one has {html}"
        ));
    }
    let mut seen = Vec::new();
    for (index, layer) in stack.layers.iter().enumerate() {
        check_layer_structure(layer, &mut seen)?;
        check_mask(layer, known)?;
        check_layer_values(layer, index)?;
    }
    Ok(())
}

/// Why a stack was refused, as the route answers it: the two kinds a Save already separates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StackRefusal {
    /// The name is another stack's, ignoring case: a 409, as `PUT /stacks/{id}` answers.
    Conflict(String),
    /// A name, a layer, a cap or a value breaks a rule: a 422 everywhere.
    Invalid(String),
}

impl StackRefusal {
    pub fn message(&self) -> &str {
        match self {
            StackRefusal::Conflict(message) | StackRefusal::Invalid(message) => message,
        }
    }
}

/// The one check every route that accepts a stack goes through: canonicalise every layer's
/// effect to the id the builder takes, then apply the rules a Save enforces (D95.4.1, D95.4.4,
/// D95.4.5).
///
/// Save, the preview and a stack draft all call it, so the three refuse the same stacks: the HTML
/// cap counts a layer written as a display name ("Spark") as the `html:` layer it becomes, and the
/// name rule leaves out exactly the stack being saved, by id, as `Stacks::put` does. A stack spelled
/// with another case is another stack, and answers 409 here as it does at Save.
///
/// The caller builds and hashes the *canonical* stack, so a preview of a stack saved from the same
/// layers has the same structure hash.
pub fn canonicalise_and_validate(
    stack: &mut Stack,
    stacks: &Stacks,
    known: &KnownEffects,
) -> Result<(), StackRefusal> {
    canonicalise(stack, known);
    let name = stack.name().to_string();
    let others: Vec<Stack> = stacks
        .all()
        .into_iter()
        .filter(|existing| existing.id != stack.id)
        .collect();
    if others
        .iter()
        .any(|other| other.name().eq_ignore_ascii_case(&name))
    {
        return Err(StackRefusal::Conflict(format!(
            "{name:?} is already another stack's name, ignoring case"
        )));
    }
    validate_stack(stack, known, &others).map_err(StackRefusal::Invalid)
}

/// Every layer's and mask's effect id, as the builder takes it: a display name ("Fire") becomes
/// its id, and a name that resolves to nothing is left as it was.
fn canonicalise(stack: &mut Stack, known: &KnownEffects) {
    for layer in &mut stack.layers {
        if let Some(id) = layer_effect_id(known, &layer.effect) {
            layer.effect = id.to_string();
        }
        if let Some(mask) = &mut layer.mask {
            if let Some(id) = layer_effect_id(known, &mask.effect) {
                mask.effect = id.to_string();
            }
        }
    }
}

/// A config bundle's stack: canonicalised the way a save does, then checked the way a load does
/// (#256). A mask whose effect does not resolve is kept — the layer draws unmasked — so a bundle
/// that a file could hold imports instead of refusing the whole config for a part it can report.
pub fn canonicalise_and_validate_for_import(
    stack: &mut Stack,
    known: &KnownEffects,
    others: &[Stack],
) -> Result<(), String> {
    canonicalise(stack, known);
    let checked = checked_for_load(stack, stack.layers.clone(), known);
    validate_stack(&checked, known, others)
}

/// The layers of a stack loaded from a file: the ones the compositor cannot run are dropped with
/// a logged reason, and the rest of the stack runs (D95.1.6, D95.4.8). A layer's effect is
/// stored as the canonical id the builder takes, so a file that spelled a display name runs.
///
/// A mask is **never** dropped here, even when its effect does not resolve (#256): the file is the
/// source of truth, so the mask stays in memory and is written back as saved. The compositor
/// reports the missing mask and draws the layer unmasked (`checked_for_load` drops it only from
/// the copy that is validated). Only the HTML cap can leave a layer out: a layer whose own effect
/// and mask together do not fit the `STACK_HTML_MAX` slots is dropped whole, which is never more
/// than the save rule would refuse, and the file is kept aside before the next write.
fn loaded_layers(stack: &Stack, known: &KnownEffects) -> Vec<StackLayer> {
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<StackLayer> = Vec::new();
    let mut html = 0usize;
    for (index, layer) in stack.layers.iter().enumerate() {
        if out.len() >= STACK_MAX_LAYERS {
            tracing::warn!(
                stack = %stack.id,
                layer = %layer.id,
                "the stack has more layers than a stack may hold; the rest are dropped"
            );
            break;
        }
        if let Err(reason) = check_layer_structure(layer, &mut seen) {
            tracing::warn!(stack = %stack.id, %reason, "a layer was dropped from the stack");
            continue;
        }
        let mut layer = layer.clone();
        if let Some(id) = layer_effect_id(known, &layer.effect) {
            layer.effect = id.to_string();
        }
        // A mask's effect is canonicalised like a layer's. One that does not resolve is kept as
        // it was saved: the layer runs unmasked and its status names the missing mask (#256).
        if let Some(mask) = &mut layer.mask {
            if let Some(id) = layer_effect_id(known, &mask.effect) {
                mask.effect = id.to_string();
            } else {
                tracing::warn!(
                    stack = %stack.id,
                    layer = %layer.id,
                    mask = %mask.effect,
                    "a layer's mask does not resolve to an effect; the layer draws unmasked and the mask is kept as saved"
                );
            }
        }
        // One rule, shared with the save: a layer's own effect and its mask take one HTML slot
        // each. A layer that does not fit the cap is dropped whole, never a mask alone.
        let slots = html_slots(&layer, known);
        if html + slots > STACK_HTML_MAX {
            tracing::warn!(
                stack = %stack.id,
                layer = %layer.id,
                slots,
                "the stack has more HTML layers than a stack may hold; this layer is dropped"
            );
            continue;
        }
        html += slots;
        if let Err(reason) = check_layer_values(&layer, index) {
            // A value is not a reason to drop the layer: the compositor clamps it (D95.3.6).
            tracing::warn!(stack = %stack.id, %reason, "a layer's value is out of range; the compositor neutralises it");
        }
        out.push(layer);
    }
    out
}

/// The stack a load validates: the layers that were kept, with the values the compositor would
/// neutralise clamped first. The stored stack keeps the file's own values; only a structural
/// problem, a bad name or a broken cap drops the whole stack (D95.3.6).
///
/// A mask whose effect does not resolve is dropped from **this copy only**: it cannot be built, so
/// validating it as a mask would drop the whole stack for a part the file may still fix. The kept
/// stack keeps it, and the compositor reports it missing (#256).
fn checked_for_load(stack: &Stack, layers: Vec<StackLayer>, known: &KnownEffects) -> Stack {
    let layers = layers
        .into_iter()
        .map(|mut layer| {
            layer.opacity = layer.opacity.min(100);
            // A mask that cannot build is left out of the copy the compositor will run; the kept
            // stack still holds it, as saved (#256).
            let mask_cannot_build = layer.mask.as_ref().is_some_and(|mask| {
                mask.effect.starts_with(STACK_ID_PREFIX)
                    || layer_effect_id(known, &mask.effect).is_none()
            });
            if mask_cannot_build {
                layer.mask = None;
            } else if let Some(mask) = &mut layer.mask {
                mask.amount = mask.amount.min(100);
            }
            if let canvas_proto::stack::LayerArea::Rect { x, y, width, height } = layer.area {
                let finite =
                    x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite();
                let canvas_w = canvas_proto::CANVAS_WIDTH as f32;
                let canvas_h = canvas_proto::CANVAS_HEIGHT as f32;
                // This is the copy `reload` validates, not the stack it keeps: the stored
                // layers are kept as they are. An edge past the canvas (refused on save,
                // preview and import since #118) is neutralised here like a bad width or
                // height, so the new edge check cannot drop a stack saved before it. The
                // compositor draws the kept rectangle into its on-canvas box when it renders it
                // (`stack.rs` `rect_px`), so the stack looks as it did (D95.3.6).
                let in_range = (0.0..=canvas_w).contains(&width)
                    && (0.0..=canvas_h).contains(&height)
                    && x >= 0.0
                    && y >= 0.0
                    && x + width <= canvas_w
                    && y + height <= canvas_h;
                if !finite || !in_range {
                    // A rectangle the compositor paints nothing for (D95.3.6).
                    layer.area = canvas_proto::stack::LayerArea::Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    };
                }
            }
            layer
        })
        .collect();
    Stack { id: stack.id.clone(), layers }
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
    /// True once a load left a stack or a layer out: the file still holds it, so the first write
    /// keeps the file aside before the write erases it.
    dropped_on_load: AtomicBool,
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
            dropped_on_load: AtomicBool::new(false),
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
        let mut dropped = false;
        for stack in file.stacks.drain(..) {
            let layers = loaded_layers(&stack, known);
            // A layer the compositor cannot run is out of memory but still in the file.
            if layers.len() != stack.layers.len() {
                dropped = true;
            }
            // The stack is checked as the compositor will run it: a value a hand-written file
            // got wrong is clamped first, so only the structure, the name and the caps can cost
            // the whole stack (D95.3.6).
            let checked = checked_for_load(&stack, layers.clone(), known);
            if let Err(reason) = validate_stack(&checked, known, &accepted) {
                tracing::warn!(stack = %stack.id, %reason, "a stack was dropped as it was loaded");
                dropped = true;
                continue;
            }
            accepted.push(Stack { id: stack.id.clone(), layers });
        }
        if dropped {
            // The stack that was dropped is still in the file; the first write must not erase it
            // without a copy (see `keep_aside_after_a_dropped_load`).
            self.dropped_on_load.store(true, Ordering::SeqCst);
        }
        let mut guard = write_recovering(&self.file);
        *guard = StackFile { schema_version: STACKS_SCHEMA, stacks: accepted };
        drop(guard);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Writes what the store holds to `stacks.json`.
    pub fn save(&self) -> Result<(), std::io::Error> {
        let file = read_recovering(&self.file).clone();
        self.keep_aside_after_a_dropped_load()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        crate::storage::write_json(&self.path(), &file)
    }

    /// Keeps the file as it was loaded aside before the first write after a load left a stack or
    /// a layer out: what is not in memory is still in the file, and the write would erase it.
    ///
    /// The copy is kept as `read_parsed` keeps an unparseable file: renamed to
    /// `stacks.json.<stamp>.corrupt` beside it. A copy that cannot be made fails the write, so the
    /// file is never overwritten without it.
    fn keep_aside_after_a_dropped_load(&self) -> Result<(), StacksError> {
        if !self.dropped_on_load.load(Ordering::SeqCst) {
            return Ok(());
        }
        let path = self.path();
        match crate::storage::quarantine(&path) {
            Ok(()) => {
                self.dropped_on_load.store(false, Ordering::SeqCst);
                tracing::warn!(
                    path = %path.display(),
                    "a stack this build does not run was left out as stacks.json loaded; the file is kept aside before the first save"
                );
                Ok(())
            }
            Err(error) => {
                tracing::error!(
                    path = %path.display(),
                    %error,
                    "stacks.json could not be kept aside; nothing is written over it"
                );
                Err(StacksError::Io(format!(
                    "stacks.json could not be kept aside: {error}"
                )))
            }
        }
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
    ///
    /// The file is written first: a save that cannot be written leaves the running store as it
    /// was, rather than holding a stack the next start will not see.
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
        let mut file = guard.clone();
        match file.stacks.iter_mut().find(|existing| existing.id == parsed.id) {
            Some(existing) => *existing = parsed.clone(),
            None => file.stacks.push(parsed.clone()),
        }
        self.keep_aside_after_a_dropped_load()?;
        crate::storage::write_json(&self.path(), &file)
            .map_err(|e| StacksError::Io(e.to_string()))?;
        *guard = file;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(parsed)
    }

    /// Removes a stack. `Ok(None)` when there was none.
    ///
    /// The file is written first, so a delete that cannot be written is an error to the caller
    /// and the stack is still there.
    pub fn delete(&self, id: &str) -> Result<Option<Stack>, StacksError> {
        let mut guard = write_recovering(&self.file);
        let Some(index) = guard
            .stacks
            .iter()
            .position(|stack| stack.id.eq_ignore_ascii_case(id) || stack.name().eq_ignore_ascii_case(id))
        else {
            return Ok(None);
        };
        let mut file = guard.clone();
        let removed = file.stacks.remove(index);
        self.keep_aside_after_a_dropped_load()?;
        crate::storage::write_json(&self.path(), &file)
            .map_err(|e| StacksError::Io(e.to_string()))?;
        *guard = file;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(Some(removed))
    }

    // ------------------------------------------------------------ runtime state

    /// The area book every stack effect resolves its layers against.
    pub fn areas(&self) -> AreaBook {
        Arc::clone(&self.areas)
    }

    /// Publishes a new area book: the layout, the render scale or the sampling mode changed.
    ///
    /// The version moves, so every running stack re-resolves its layers on its next frame and
    /// rebuilds nothing. The map is built outside the lock; the version is read and bumped under
    /// one write guard, so two publishes cannot both start from the same number.
    pub fn publish_areas(&self, layout: &canvas_proto::layout::Layout, devices: &[canvas_proto::DeviceInfo]) {
        let mut map = AreaMap::build(layout, devices, 0);
        let mut guard = write_recovering(&self.areas);
        map.version = guard.version.wrapping_add(1);
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
                    // The map is built outside the lock; the version is taken and the map
                    // swapped in under one write guard, so a build is never under it.
                    let mut next = LiveMap::from_stack(stack, 0);
                    let mut live = write_recovering(&running.live);
                    next.version = live.version.wrapping_add(1);
                    *live = next;
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
                // Built outside the guard; the version is taken and the map swapped in under it.
                let mut next = LiveMap::from_stack(stack, 0);
                let mut live = write_recovering(&running.live);
                next.version = live.version.wrapping_add(1);
                *live = next;
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

    #[test]
    fn a_refusal_names_the_layer_the_person_sees_not_its_id() {
        let mut named = layer("l1", "solid");
        named.name = "Warm white".into();
        named.opacity = 150;
        let named_err = check_layer_values(&named, 0).unwrap_err();
        assert!(named_err.starts_with("layer \"Warm white\":"), "{named_err}");
        assert!(!named_err.contains("l1"), "the internal id stays out of it: {named_err}");

        let mut unnamed = layer("l2", "solid");
        unnamed.name = String::new();
        unnamed.opacity = 150;
        let unnamed_err = check_layer_values(&unnamed, 0).unwrap_err();
        assert!(unnamed_err.starts_with("layer l2:"), "with no name, the id is all there is: {unnamed_err}");
    }

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
            mask: None,
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
    fn a_rect_off_the_left_edge_is_refused() {
        let known = known();
        let mut layer = layer("l1", "solid");
        layer.area = LayerArea::Rect { x: -3.0, y: 0.0, width: 10.0, height: 10.0 };
        let error = validate_stack(&stack("stack:A", vec![layer]), &known, &[]).unwrap_err();
        assert!(error.contains("l1"), "{error}");
        assert!(error.contains('0'), "{error}");
        assert!(error.contains("solid"), "{error}");
        assert!(error.contains("left"), "{error}");
    }

    #[test]
    fn a_rect_off_the_top_edge_is_refused() {
        let known = known();
        let mut layer = layer("l1", "solid");
        layer.area = LayerArea::Rect { x: 0.0, y: -1.0, width: 10.0, height: 10.0 };
        let error = validate_stack(&stack("stack:A", vec![layer]), &known, &[]).unwrap_err();
        assert!(error.contains("l1"), "{error}");
        assert!(error.contains("solid"), "{error}");
        assert!(error.contains("top"), "{error}");
    }

    #[test]
    fn a_rect_off_the_right_edge_is_refused() {
        let known = known();
        let mut layer = layer("l1", "solid");
        layer.area = LayerArea::Rect { x: 300.0, y: 0.0, width: 40.0, height: 10.0 };
        let error = validate_stack(&stack("stack:A", vec![layer]), &known, &[]).unwrap_err();
        assert!(error.contains("l1"), "{error}");
        assert!(error.contains("solid"), "{error}");
        assert!(error.contains("right"), "{error}");
    }

    #[test]
    fn a_rect_off_the_bottom_edge_is_refused() {
        let known = known();
        let mut layer = layer("l1", "solid");
        layer.area = LayerArea::Rect { x: 0.0, y: 190.0, width: 10.0, height: 20.0 };
        let error = validate_stack(&stack("stack:A", vec![layer]), &known, &[]).unwrap_err();
        assert!(error.contains("l1"), "{error}");
        assert!(error.contains("solid"), "{error}");
        assert!(error.contains("bottom"), "{error}");
    }

    #[test]
    fn a_rect_exactly_touching_the_edges_is_accepted() {
        let known = known();
        let mut corner = layer("l1", "solid");
        corner.area = LayerArea::Rect { x: 310.0, y: 190.0, width: 10.0, height: 10.0 };
        assert!(
            validate_stack(&stack("stack:A", vec![corner]), &known, &[]).is_ok(),
            "x + width == 320 and y + height == 200 touch the edges and are accepted"
        );
        let mut full = layer("l1", "solid");
        full.area = LayerArea::Rect { x: 0.0, y: 0.0, width: 320.0, height: 200.0 };
        assert!(validate_stack(&stack("stack:A", vec![full]), &known, &[]).is_ok());
    }

    #[test]
    fn a_rect_with_huge_numbers_is_refused_without_panicking() {
        let known = known();
        let mut far = layer("l1", "solid");
        far.area = LayerArea::Rect { x: 1.0e30, y: 0.0, width: 10.0, height: 10.0 };
        let error = validate_stack(&stack("stack:A", vec![far]), &known, &[]).unwrap_err();
        assert!(error.contains("l1"), "{error}");
        assert!(error.contains("right"), "{error}");

        let mut maxed = layer("l1", "solid");
        maxed.area = LayerArea::Rect { x: f32::MAX, y: 0.0, width: 10.0, height: 10.0 };
        let error = validate_stack(&stack("stack:A", vec![maxed]), &known, &[]).unwrap_err();
        assert!(error.contains("l1"), "{error}");
        assert!(error.contains("right"), "{error}");
    }

    #[test]
    fn a_file_with_an_out_of_canvas_area_still_loads_and_resolves_to_its_on_canvas_box() {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-stacks-oob-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(STACKS_FILE),
            r#"{"schema_version":1,"stacks":[
                {"id":"stack:Hand","layers":[
                  {"id":"l1","name":"Base","effect":"solid",
                   "area":{"kind":"rect","x":-3.0,"y":5.0,"width":10.0,"height":10.0}}]}]}"#,
        )
        .unwrap();
        let store = Stacks::new(&dir);
        store.reload(&UserEffects::new(crate::user_effects::html_builder()));
        let hands = store.all();
        assert_eq!(hands.len(), 1, "the stack with an out-of-canvas area still loads");
        assert_eq!(hands[0].layers.len(), 1, "its layer is kept");
        assert_eq!(
            hands[0].layers[0].area,
            LayerArea::Rect { x: -3.0, y: 5.0, width: 10.0, height: 10.0 },
            "a stored area that is only partly off the canvas is kept as it is, not emptied"
        );

        let layout = canvas_proto::layout::Layout {
            id: "l".into(),
            name: "l".into(),
            render_scale: canvas_proto::layout::RenderScale::P100,
            sampling: canvas_proto::layout::SamplingMode::Nearest,
            zones: Vec::new(),
            effect: None,
            case_form: canvas_proto::layout::CaseForm::None,
            landmarks: Vec::new(),
        };
        let map = crate::stack::AreaMap::build(&layout, &[], 1);
        let resolved = map
            .resolve(&hands[0].layers[0].area)
            .expect("the clipped area covers the canvas edge, not nothing");
        assert_eq!(resolved.dest, [0.0, 5.0, 7.0, 15.0], "the area resolves to its on-canvas box (the compositor draws the layer into it)");
        assert!(!map.is_empty_area(&hands[0].layers[0].area));
        assert!(map
            .resolve(&LayerArea::Rect { x: 400.0, y: 0.0, width: 10.0, height: 10.0 })
            .is_none(), "a wholly out-of-canvas rect paints nothing");
        let _ = std::fs::remove_dir_all(&dir);
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

        assert!(again.delete("stack:Sunset glow").expect("the delete is written").is_some());
        assert!(again.all().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder where `stacks.json` is a directory, so no write can land on it.
    fn unwritable(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-stacks-unwritable-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(STACKS_FILE)).unwrap();
        dir
    }

    #[test]
    fn a_save_that_cannot_be_written_leaves_the_store_as_it_was() {
        let dir = unwritable("put");
        let store = Stacks::new(&dir);
        let outcome = store.put(stack("stack:Fail", vec![layer("l1", "solid")]), &known());
        assert!(matches!(outcome, Err(StacksError::Io(_))), "{outcome:?}");
        assert!(store.all().is_empty(), "a failed save holds no stack");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_delete_that_cannot_be_written_keeps_the_stack() {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-stacks-unwritable-delete-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Stacks::new(&dir);
        let saved = store
            .put(stack("stack:Kept", vec![layer("l1", "solid")]), &known())
            .expect("the first save is written");
        // The file is replaced by a directory, so the delete cannot be written.
        std::fs::remove_file(dir.join(STACKS_FILE)).unwrap();
        std::fs::create_dir(dir.join(STACKS_FILE)).unwrap();
        let outcome = store.delete("stack:Kept");
        assert!(matches!(outcome, Err(StacksError::Io(_))), "{outcome:?}");
        assert_eq!(store.all(), vec![saved], "the stack is still in the store");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_stack_is_kept_aside_before_the_first_save_erases_it() {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-stacks-kept-aside-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(STACKS_FILE),
            r#"{"schema_version":1,"stacks":[
                {"id":"stack:Good","layers":[{"id":"l1","name":"Base","effect":"solid"}]},
                {"id":"Glow","layers":[{"id":"l1","name":"Base","effect":"solid"}]}]}"#,
        )
        .unwrap();
        let store = Stacks::new(&dir);
        store.reload(&UserEffects::new(crate::user_effects::html_builder()));
        assert_eq!(store.all().len(), 1, "the stack with no prefix is dropped as it loads");

        store
            .put(stack("stack:New", vec![layer("l1", "solid")]), &known())
            .expect("the save is written");

        // The refused stack is out of memory; the file as it loaded is kept beside the new one.
        let kept = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .find(|name| name.starts_with(STACKS_FILE) && name.ends_with(".corrupt"))
            .expect("the file as it loaded is kept aside");
        let text = std::fs::read_to_string(dir.join(&kept)).unwrap();
        assert!(text.contains("Glow"), "the kept file still holds the refused stack: {text}");

        let after = std::fs::read_to_string(dir.join(STACKS_FILE)).unwrap();
        assert!(after.contains("stack:New"), "{after}");
        assert!(after.contains("stack:Good"), "{after}");
        assert!(!after.contains("stack:Glow"), "{after}");
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
