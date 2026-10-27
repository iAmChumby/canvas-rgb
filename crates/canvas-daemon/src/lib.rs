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
pub mod peer;
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

/// Devices the daemon never sends frames to, matched on the HID path inside the device id.
/// OpenRGB's driver for each of these breaks the device when it is streamed to:
///
/// - `VID_3151&PID_4015`, the ROYUAN MonsGeek keyboard: frames through OpenRGB blank its
///   lights and stop it typing (lighting session 2, 2026-10-26). OpenRGB's detector for it
///   cannot be turned off in its config, so the daemon leaves it alone instead.
pub const NOT_DRIVEN: &[&str] = &["VID_3151&PID_4015"];

/// True when the daemon may send frames to this device.
pub fn is_driven(id: &canvas_proto::DeviceId) -> bool {
    let id = id.0.to_ascii_uppercase();
    !NOT_DRIVEN.iter().any(|path| id.contains(path))
}

/// The daemon's version, as reported by `/status`.
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    use canvas_proto::DeviceId;

    #[test]
    fn the_monsgeek_keyboard_is_never_driven_and_the_rest_are() {
        let keyboard = DeviceId(
            r"openrgb:HID: \\?\HID#VID_3151&PID_4015&MI_02#9&298b2e71&0&0000#{4d1e55b2}".into(),
        );
        let mic = DeviceId(r"openrgb:HID: \\?\HID#VID_03F0&PID_028C&MI_00#a&eb32b1c".into());
        assert!(!super::is_driven(&keyboard));
        assert!(!super::is_driven(&DeviceId(keyboard.0.to_lowercase())));
        assert!(super::is_driven(&mic));
        assert!(super::is_driven(&DeviceId("openrgb:SERIAL0000000".into())));
    }
}
