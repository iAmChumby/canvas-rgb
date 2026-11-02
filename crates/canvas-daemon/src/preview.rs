//! The stack preview engine and its slot (D95.4.4).
//!
//! The builder tunes a stack continuously, so the preview is not a render loop of its own: a
//! second engine runs the stack at [`canvas_proto::consts::PREVIEW_MAX_FPS`], gets the same
//! layout and input source as the lights' engine at the same call sites, and never has a device
//! sink, so a preview cannot touch a light. The engine is made the first time the builder opens
//! a preview or its WebSocket, and parks while nothing watches it.
//!
//! A PUT carries a session epoch and a sequence. The newest epoch owns the preview, and within
//! an epoch only a higher sequence is applied; anything else is superseded and answers 200 with
//! `applied: false`. The build runs outside the slot's gate, so Stop, DELETE and a newer PUT do
//! not wait for a slow HTML layer; the gate is taken to read the slot, to swap it and to apply.

use canvas_proto::api::{
    StackLayerState, StackLayerStatus, StackPreviewLayer, StackPreviewResponse,
};
use canvas_proto::props::Props;
use canvas_proto::stack::{Stack, STACK_ID_PREFIX};
use canvas_proto::traits::{Effect, Engine, EngineSettings};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::stack::{read_recovering, write_recovering, LiveMap, LiveStack};
use crate::state::{App, PreviewRegistry};

/// The preview engine's settings: 15 fps is its render rate and the WebSocket sink's cap, and
/// it runs at full brightness because the builder shows the stack, not the room.
pub fn engine_settings() -> EngineSettings {
    EngineSettings {
        fps: canvas_proto::consts::PREVIEW_MAX_FPS,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
    }
}

/// The preview engine the daemon makes: a second render thread, with no sink of its own.
pub fn real_engine() -> Arc<dyn Engine> {
    canvas_core::spawn(engine_settings())
}

/// The stack a preview is showing, with what it was built from.
pub struct PreviewSlot {
    /// `stack:` + the name, as the builder gave it.
    pub stack_id: String,
    /// The structure hash the effect was built from (D95.4.6).
    pub structure: String,
    /// The live map the effect holds, so a value change re-applies without a rebuild.
    pub live: LiveStack,
    /// The merged props the effect was initialised with.
    pub props: Props,
    /// What each layer was doing at the last PUT.
    pub layers: Vec<StackPreviewLayer>,
    /// The build errors by effect id, so a later value-only PUT still reports a missing one.
    pub errors: BTreeMap<String, String>,
    /// Where the effect reports a held or faulted HTML layer.
    pub statuses: Arc<Mutex<Vec<StackLayerStatus>>>,
}

/// The preview engine and the slot it is showing.
pub struct StackPreview {
    /// The engine, once something has asked for it. `None` while the builder is closed.
    engine: tokio::sync::Mutex<Option<Arc<dyn Engine>>>,
    /// How the daemon makes the engine. The tests install a fake instead.
    pub make_engine: Box<dyn Fn() -> Arc<dyn Engine> + Send + Sync>,
    /// True once the engine has been given its rate, layout and input source.
    configured: AtomicBool,
    /// The WebSocket subscribers of the preview engine (`?stack=1`).
    pub subs: PreviewRegistry,
    /// The stack the preview engine runs, and what it was built from.
    slot: Mutex<Option<PreviewSlot>>,
    /// The newest `(epoch, seq)` accepted. Cleared by DELETE.
    watermark: Mutex<Option<(String, u64)>>,
    /// Epochs that owned the preview and were replaced by another one. A PUT from one of
    /// these is an older tab, never a new page (the epochs are random, so this is how the
    /// daemon tells them apart).
    replaced_epochs: Mutex<Vec<String>>,
    /// Orders the slot's transitions. The build itself runs outside it.
    gate: tokio::sync::Mutex<()>,
}

impl StackPreview {
    /// A preview with no engine yet; the daemon's real engine is made on first use.
    pub fn new() -> Self {
        Self::with_maker(Box::new(real_engine))
    }

    /// A preview that makes its engine with `make_engine`: the tests hand in a fake.
    pub fn with_maker(make_engine: Box<dyn Fn() -> Arc<dyn Engine> + Send + Sync>) -> Self {
        Self {
            engine: tokio::sync::Mutex::new(None),
            make_engine,
            configured: AtomicBool::new(false),
            subs: PreviewRegistry::default(),
            slot: Mutex::new(None),
            watermark: Mutex::new(None),
            replaced_epochs: Mutex::new(Vec::new()),
            gate: tokio::sync::Mutex::new(()),
        }
    }

    /// Installs an engine a test built, instead of making one on first use.
    pub fn install_engine(&mut self, engine: Arc<dyn Engine>) {
        *self.engine.get_mut() = Some(engine);
    }

    /// The engine, made and configured on first use.
    pub async fn engine(&self, app: &App) -> Arc<dyn Engine> {
        let engine = {
            let mut guard = self.engine.lock().await;
            match guard.as_ref() {
                Some(engine) => Arc::clone(engine),
                None => {
                    let made = (self.make_engine)();
                    *guard = Some(Arc::clone(&made));
                    made
                }
            }
        };
        if !self.configured.swap(true, Ordering::SeqCst) {
            engine.set_fps(canvas_proto::consts::PREVIEW_MAX_FPS);
            if let Some(layout) = app.active_layout().await {
                let devices = app.devices.read().await.clone();
                engine.set_layout(layout, devices);
            }
            // The main engine has no input source wired on this commit either, so the preview
            // gets the same one it has (D95.4.4).
            engine.set_input_source(None);
        }
        engine
    }

    /// A layout change reaches the preview engine when it exists. Before it does, the engine
    /// is given the active layout when it is made.
    pub fn set_layout(&self, layout: &canvas_proto::layout::Layout, devices: &[canvas_proto::device::DeviceInfo]) {
        if let Ok(guard) = self.engine.try_lock() {
            if let Some(engine) = guard.as_ref() {
                engine.set_layout(layout.clone(), devices.to_vec());
            }
        }
    }

    /// An input-source change reaches the preview engine when it exists; otherwise the engine
    /// gets the source it is made with.
    pub fn set_input_source(&self, source: Option<Arc<dyn canvas_proto::traits::InputSource>>) {
        if let Ok(guard) = self.engine.try_lock() {
            if let Some(engine) = guard.as_ref() {
                engine.set_input_source(source);
            }
        }
    }

    /// The slot, copied, so a reader never holds the lock while it waits on anything.
    pub fn snapshot(&self) -> Option<PreviewSlot> {
        self.slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|slot| PreviewSlot {
                stack_id: slot.stack_id.clone(),
                structure: slot.structure.clone(),
                live: Arc::clone(&slot.live),
                props: slot.props.clone(),
                layers: slot.layers.clone(),
                errors: slot.errors.clone(),
                statuses: Arc::clone(&slot.statuses),
            })
    }

    /// The newest accepted `(epoch, seq)`.
    pub fn watermark(&self) -> Option<(String, u64)> {
        self.watermark
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn accept(&self, epoch: &str, seq: u64) {
        {
            let mut epochs = self
                .replaced_epochs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(current) = &*self
                .watermark
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
            {
                if current.0 != epoch && !epochs.iter().any(|seen| seen == &current.0) {
                    epochs.push(current.0.clone());
                }
            }
            if epochs.len() > 32 {
                epochs.remove(0);
            }
        }
        *self
            .watermark
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some((epoch.to_string(), seq));
    }

    /// True when this epoch owned the preview before another one replaced it: an older tab.
    fn was_replaced(&self, epoch: &str) -> bool {
        self.replaced_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|seen| seen == epoch)
    }

    fn clear_watermark(&self) {
        *self
            .watermark
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        self.replaced_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

impl Default for StackPreview {
    fn default() -> Self {
        Self::new()
    }
}

/// Why a preview PUT did not apply.
#[derive(Debug)]
pub enum PreviewError {
    /// Two previews already render: the answer is a 503 with `Retry-After`.
    Busy,
    /// The daemon could not hand the effect over.
    Internal(String),
}

/// The structure identity of a stack (D95.4.6): layer ids, order, effect ids and each HTML
/// layer's source, through the hash the repository already uses.
pub fn structure_of(app: &App, stack: &Stack) -> String {
    let parts = crate::stack::structure(stack, &|id| {
        app.user_effects
            .source_and_file_name(id)
            .map(|(source, _)| source.to_string())
    });
    crate::stack::structure_hash(&parts)
}

/// The merged props a stack's layers start with (D95.4.3).
pub fn props_of(app: &App, stack: &Stack) -> Props {
    crate::stack::stack_props(stack, &|id| {
        crate::api::effects::find_schema(&app.user_effects, &app.stacks, id)
    })
}

/// `PUT /api/v1/stacks/preview` (D95.4.4).
///
/// The gate is taken to check the watermark and the slot, then the build runs on a blocking
/// thread outside it, and the gate is taken again to swap the slot. A PUT whose epoch is the
/// accepted one and whose seq is not above it, or whose build finishes after a newer PUT, lands
/// nowhere and answers 200 with `applied: false`.
pub async fn put(
    app: &Arc<App>,
    stack: &Stack,
    epoch: &str,
    seq: u64,
) -> Result<StackPreviewResponse, PreviewError> {
    let Ok(_slot) = app.preview_slots.clone().try_acquire_owned() else {
        return Err(PreviewError::Busy);
    };

    let structure = structure_of(app, stack);
    let props = props_of(app, stack);

    // The gate is held while the slot is read and, when the structure is unchanged, while the
    // values are applied: no build happens on this path (D95.4.6).
    let gate = app.stack_preview.gate.lock().await;
    if let Some((stored, highest)) = app.stack_preview.watermark() {
        if stored == epoch {
            if seq <= highest {
                return Ok(superseded(app, epoch, seq));
            }
        } else if app.stack_preview.was_replaced(epoch) {
            // An older tab's epoch, already replaced by a newer page: it learns it lost.
            return Ok(superseded(app, epoch, seq));
        }
    }
    app.stack_preview.accept(epoch, seq);
    if let Some(slot) = app.stack_preview.snapshot() {
        if slot.stack_id == stack.id && slot.structure == structure {
            {
                let mut live = write_recovering(&slot.live);
                live.replace_with(stack);
            }
            let engine = app.stack_preview.engine(app).await;
            // The lock is not held here: `set_props` reaches the effect through the engine's
            // own queue and must not nest with the live map's lock (D95.4.6).
            engine.set_props(&props);
            let layers = reports(app, stack, &slot.errors, &slot.statuses);
            {
                let mut guard = app
                    .stack_preview
                    .slot
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(slot) = guard.as_mut() {
                    slot.props = props;
                    slot.layers = layers.clone();
                }
            }
            tracing::info!(stack = %stack.id, seq, "the preview's values changed without a rebuild");
            return Ok(answer(app, true, epoch, seq, layers));
        }
    }
    drop(gate);

    // A new structure: build it outside the gate, so Stop, DELETE and a newer PUT answer.
    let build_app = Arc::clone(app);
    let build_stack = stack.clone();
    let live: LiveStack = Arc::new(std::sync::RwLock::new(LiveMap::from_stack(stack, 1)));
    let build_live = Arc::clone(&live);
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let build_statuses = Arc::clone(&statuses);
    let fps = Arc::new(AtomicU32::new(canvas_proto::consts::PREVIEW_MAX_FPS));
    let built = tokio::task::spawn_blocking(move || {
        let errors: Arc<Mutex<BTreeMap<String, String>>> = Arc::new(Mutex::new(BTreeMap::new()));
        let leaf = crate::leaf_builder(&build_app.user_effects);
        let recording = {
            let errors = Arc::clone(&errors);
            Arc::new(move |id: &str| {
                let result = leaf(id);
                if let Err(error) = &result {
                    errors
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(id.to_string(), error.clone());
                }
                result
            })
        };
        let effect: Box<dyn Effect> = Box::new(
            crate::stack::StackEffect::new(
                &build_stack,
                recording,
                build_live,
                build_app.stacks.areas(),
            )
            .with_clock(build_app.stacks.clock())
            .with_fps(fps)
            .with_status_sink(build_statuses),
        );
        (effect, errors)
    })
    .await
    .map_err(|e| PreviewError::Internal(format!("the preview build task failed: {e}")))?;
    let (effect, errors) = built;

    let _gate = app.stack_preview.gate.lock().await;
    // A newer PUT, a newer epoch, or a DELETE while the build ran: this one is stale.
    if app.stack_preview.watermark() != Some((epoch.to_string(), seq)) {
        return Ok(superseded(app, epoch, seq));
    }
    let engine = app.stack_preview.engine(app).await;
    engine.set_effect(effect, &props);

    let errors = errors
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let layers = reports(app, stack, &errors, &statuses);
    let slot = PreviewSlot {
        stack_id: stack.id.clone(),
        structure,
        live,
        props: props.clone(),
        layers: layers.clone(),
        errors,
        statuses,
    };
    *app.stack_preview
        .slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(slot);
    tracing::info!(stack = %stack.id, seq, "the preview was built");
    Ok(answer(app, true, epoch, seq, layers))
}

/// `DELETE /api/v1/stacks/preview`.
///
/// Drops the preview, clears the sequence watermark and answers 200. The preview engine, if it
/// exists, paints black instead of keeping the last stack on the channel.
pub async fn clear(app: &Arc<App>) -> bool {
    let _gate = app.stack_preview.gate.lock().await;
    app.stack_preview.clear_watermark();
    let had = app
        .stack_preview
        .slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .is_some();
    if had {
        if let Ok(guard) = app.stack_preview.engine.try_lock() {
            if let Some(engine) = guard.as_ref() {
                if let Some(effect) = canvas_effects::create("solid") {
                    engine.set_effect(effect, &crate::draft::black());
                }
            }
        }
        tracing::info!("the preview was dropped");
    }
    had
}

/// The answer a superseded PUT gives: 200, `applied: false`, and the epoch/seq that own the
/// preview so the page can stop resending the stale one.
fn superseded(app: &App, epoch: &str, seq: u64) -> StackPreviewResponse {
    let (owned_epoch, owned_seq) = app
        .stack_preview
        .watermark()
        .unwrap_or_else(|| (epoch.to_string(), seq));
    let layers = app
        .stack_preview
        .snapshot()
        .map(|slot| slot.layers)
        .unwrap_or_default();
    answer(app, false, &owned_epoch, owned_seq, layers)
}

/// The answer for a PUT that applied (or did not).
fn answer(
    app: &App,
    applied: bool,
    epoch: &str,
    seq: u64,
    layers: Vec<StackPreviewLayer>,
) -> StackPreviewResponse {
    StackPreviewResponse {
        applied,
        epoch: epoch.to_string(),
        seq,
        layers,
        running: app.stack_preview.snapshot().is_some(),
    }
}

/// What each layer is doing: nested, missing (with the build's reason), off the desk, or the
/// runtime's own held/faulted report.
fn reports(
    app: &App,
    stack: &Stack,
    errors: &BTreeMap<String, String>,
    statuses: &Arc<Mutex<Vec<StackLayerStatus>>>,
) -> Vec<StackPreviewLayer> {
    let runtime = statuses
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let book = app.stacks.areas();
    let areas = read_recovering(&book);
    stack
        .layers
        .iter()
        .map(|layer| {
            let (status, error) = if layer.effect.starts_with(STACK_ID_PREFIX) {
                (
                    StackLayerState::NestedStack,
                    Some(format!(
                        "{} is a stack, and a stack cannot contain a stack",
                        layer.effect
                    )),
                )
            } else if let Some(error) = errors.get(&layer.effect) {
                (StackLayerState::MissingEffect, Some(error.clone()))
            } else if areas.is_empty_area(&layer.area) {
                (StackLayerState::NoArea, None)
            } else if let Some(reported) = runtime.iter().find(|status| status.id == layer.id) {
                (reported.status, None)
            } else {
                (StackLayerState::Ok, None)
            };
            StackPreviewLayer { id: layer.id.clone(), status, error }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_engine_runs_at_fifteen_fps() {
        let settings = engine_settings();
        assert_eq!(settings.fps, canvas_proto::consts::PREVIEW_MAX_FPS);
        assert_eq!(settings.fps, 15);
        assert_eq!(settings.brightness, 100);
        assert_eq!(settings.power, canvas_proto::api::Power::Running);
    }
}
