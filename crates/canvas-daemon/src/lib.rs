//! canvas-rgbd internals, shared by the daemon and CLI binaries.

pub mod api;
pub mod auth;
pub mod bundle;
pub mod capture;
pub mod cli;
pub mod composite;
pub mod conflict;
pub mod displays;
pub mod draft;
pub mod elevated;
pub mod exit_log;
pub mod first_run;
pub mod identify;
pub mod last_frame;
pub mod logging;
pub mod migration;
pub mod openrgb_supervisor;
pub mod paths;
pub mod peer;
pub mod peripherals;
pub mod pipe_client;
pub mod power;
pub mod state;
pub mod storage;
pub mod thumbnail;
pub mod transient;
pub mod token;
pub mod user_effects;
pub mod writer;

#[cfg(test)]
pub(crate) mod test_logs;

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

/// A fresh instance of the effect with this id, for a library tile.
///
/// The same effect the lights would run, but with a fixed seed: a tile is rendered again
/// and again, so it has to be the same picture every time. An HTML effect is built with
/// the seeded builder; a built-in one reads the seed from the properties the tile path
/// hands it (see [`crate::thumbnail::render`]).
pub fn create_effect_for_tile(
    user: &user_effects::UserEffects,
    id: &str,
) -> Result<Box<dyn canvas_proto::traits::Effect>, String> {
    match user.create_for_tile(id) {
        Some(made) => made,
        None => canvas_effects::create(id).ok_or_else(|| format!("no effect with the id {id}")),
    }
}

/// Creates the effect and hands it to the engine, starting and stopping the captures to
/// match what the effect needs.
///
/// Returns an error when the effect id is unknown.
pub async fn apply_effect(app: &Arc<App>, id: &str, props: &Props) -> Result<(), String> {
    // Loading an HTML effect runs its scripts (up to 2 s of CPU), so it must not run on an
    // async worker.
    let effect = {
        let app = Arc::clone(app);
        let id = id.to_string();
        let props = props.clone();
        tokio::task::spawn_blocking(move || {
            let mut effect = create_effect(&app.user_effects, &id)?;
            effect.init(&props);
            Ok::<_, String>(effect)
        })
        .await
        .map_err(|e| format!("the effect load task failed: {e}"))??
    };

    apply_built(app, effect, props).await
}

/// Hands an effect that is already built to the engine, starting and stopping the captures to
/// match what it needs.
///
/// [`apply_effect`] builds an effect from its id and comes here; the editor's draft
/// ([`crate::draft`]) builds its own HTML and comes here too. Nothing is saved: the caller
/// owns what, if anything, the effect means to the store.
pub async fn apply_built(
    app: &Arc<App>,
    effect: Box<dyn canvas_proto::traits::Effect>,
    props: &Props,
) -> Result<(), String> {
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
///   cannot be turned off in its config, so the daemon drops OpenRGB's copy and drives the
///   keyboard through the native `royuan` adapter instead (see `composite`).
pub const NOT_DRIVEN: &[&str] = &["VID_3151&PID_4015"];

/// True when the daemon may send frames to this device.
pub fn is_driven(id: &canvas_proto::DeviceId) -> bool {
    let id = id.0.to_ascii_uppercase();
    !NOT_DRIVEN.iter().any(|path| id.contains(path))
}

/// How many of `devices` the engine's `write_solid` reaches: connected, driven, and with LEDs.
/// The engine skips a sink with no LEDs, so counting every sink would report a write to a device
/// that got none (class `cause:unchecked-state`, #18).
pub fn solid_write_targets(devices: &[canvas_proto::DeviceInfo]) -> usize {
    devices.iter().filter(|d| d.connected && is_driven(&d.id) && d.led_count() > 0).count()
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
        assert!(super::is_driven(&DeviceId("openrgb:A1B2C3D4E5F6".into())));
    }

    fn device(id: &str, leds: u32, connected: bool) -> canvas_proto::DeviceInfo {
        use canvas_proto::device::{DeviceKind, ZoneInfo, ZoneKind};
        canvas_proto::DeviceInfo {
            id: DeviceId(id.into()),
            name: id.into(),
            vendor: String::new(),
            kind: DeviceKind::Motherboard,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones: vec![ZoneInfo {
                index: 0,
                name: "Zone".into(),
                kind: ZoneKind::Linear,
                led_count: leds,
                min_leds: leds,
                max_leds: leds,
                resizable: false,
            }],
            connected,
        }
    }

    /// `write_solid` skips a sink with no LEDs, so the "queued black for N devices" line counts
    /// only the devices it writes: not a zero-LED header, a disconnected device or a device that
    /// is never driven.
    #[test]
    fn the_solid_write_count_skips_devices_the_engine_skips() {
        let devices = vec![
            device("openrgb:a", 4, true),
            device("openrgb:empty-header", 0, true),
            device("openrgb:gone", 4, false),
            device(r"openrgb:HID#VID_3151&PID_4015&MI_02", 4, true),
            device("openrgb:b", 9, true),
        ];
        assert_eq!(super::solid_write_targets(&devices), 2);
        assert_eq!(super::solid_write_targets(&[device("openrgb:empty-header", 0, true)]), 0);
    }
}
