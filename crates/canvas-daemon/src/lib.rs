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
pub mod thumbnail;
pub mod token;
pub mod user_effects;

use canvas_proto::props::Props;
use std::sync::Arc;

use crate::state::App;

/// A fresh instance of the effect with this id. User HTML effects (`html:<stem>`) come from
/// the effects folder; everything else from the built-in registry.
///
/// Returns an error when the id is unknown or the effect fails to build.
pub fn create_effect(
    user: &user_effects::UserEffects,
    id: &str,
) -> Result<Box<dyn canvas_proto::traits::Effect>, String> {
    match user.create(id) {
        Some(made) => made,
        None => canvas_effects::create(id).ok_or_else(|| format!("no effect with the id {id}")),
    }
}

/// Creates the effect and hands it to the engine, starting and stopping the captures to
/// match what the effect needs.
///
/// Returns an error when the effect id is unknown.
pub async fn apply_effect(app: &Arc<App>, id: &str, props: &Props) -> Result<(), String> {
    let mut effect = create_effect(&app.user_effects, id)?;
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
