//! canvas-rgbd internals, shared by the daemon and CLI binaries.

pub mod api;
pub mod auth;
pub mod bundle;
pub mod capture;
pub mod cli;
pub mod conflict;
pub mod elevated;
pub mod first_run;
pub mod logging;
pub mod openrgb_supervisor;
pub mod paths;
pub mod pipe_client;
pub mod power;
pub mod state;
pub mod storage;
pub mod token;
pub mod user_effects;

use canvas_proto::props::Props;
use std::sync::Arc;

use crate::state::App;

/// Creates the effect and hands it to the engine, starting and stopping the captures to
/// match what the effect needs.
///
/// User HTML effects (`html:<stem>`) come from the effects folder; everything else from
/// the built-in registry.
///
/// Returns an error when the effect id is unknown.
pub async fn apply_effect(app: &Arc<App>, id: &str, props: &Props) -> Result<(), String> {
    let mut effect = match app.user_effects.create(id) {
        Some(made) => made?,
        None => canvas_effects::create(id).ok_or_else(|| format!("no effect with the id {id}"))?,
    };
    effect.init(props);

    // The captures follow the effect's `needs`.
    let needs = effect.schema().needs;
    app.inputs.lock().map_err(|_| "the capture state is poisoned")?.apply_needs(needs);

    app.engine.set_effect(effect, props);
    Ok(())
}

/// The daemon's version, as reported by `/status`.
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
