//! What the daemon does with a device list: the one path every list takes (#233).
//!
//! The OpenRGB supervisor reads the devices, at connect, every 5 s while the list changes, and
//! when the owner asks for a rescan. Each list goes through [`apply_devices`], which keeps the
//! list, marks the link connected, places the zones a new device brought, hands the engine the new
//! layout, and gives each driven device a frame writer. The supervisor awaits it, so the lists
//! are applied one at a time and in the order they were read: a rescan never races a refresh, and
//! an answer to a rescan means the state already changed.
//!
//! It lives in the library, and not in the daemon's binary, so a test can run the real path.

use std::collections::HashMap;
use std::sync::Arc;

use canvas_proto::api::LinkState;
use canvas_proto::device::DeviceInfo;
use canvas_proto::DeviceId;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::state::App;

/// The frame writer of every device that has a sink in the engine.
pub type DeviceSinks = tokio::sync::Mutex<HashMap<DeviceId, JoinHandle<()>>>;

/// Keeps `devices` as the device list and brings everything that depends on it up to date.
pub async fn apply_devices(app: &Arc<App>, devices: Vec<DeviceInfo>) {
    *app.devices.write().await = devices.clone();
    *app.openrgb.write().await = LinkState::Connected;
    // After the first run, a zone that appears (a late device, a header that got its LEDs) is
    // placed instead of staying black.
    {
        let mut store = app.store.write().await;
        // Scenes: every zone with LEDs is a light. A new one joins no group; one whose device
        // went away stays, so it keeps its place when the device returns (the id is built from
        // the device id, never the adapter index).
        let store = &mut *store;
        // The first device list settles a held scenes migration (spec "Storage and migration"):
        // arrangements and zero-LED zones come from the real devices, not from a guess at load.
        if store.finish_scenes(&devices) {
            tracing::info!("scenes.json and lights.json were written from the first device list");
        }
        store.sync_lights_from(&devices);
        if store.config.first_run_done {
            let added = store
                .active_layout_mut()
                .map_or(0, |layout| crate::first_run::place_missing_zones(layout, &devices));
            if added > 0 {
                tracing::info!(added, "new zones were placed on the canvas");
                if let Err(e) = store.save_layouts() {
                    tracing::warn!(error = %e, "could not save the layouts");
                }
            }
        }
    }
    // The engine needs the new devices and the zone sizes.
    if let Some(layout) = app.active_layout().await {
        app.set_layout(&layout, &devices);
        app.stacks.publish_areas(&layout, &devices);
    }
    sync_device_sinks(app, &devices).await;
    // The first run waits for the devices. It can take a while, so it runs on its own task and
    // the supervisor goes on reading.
    let first_run = Arc::clone(app);
    tokio::spawn(async move { crate::first_run::maybe_run(&first_run).await });
}

/// Gives every connected device a frame sink in the engine: a bounded(1) channel into a writer
/// task that sends each frame to the adapter. Devices that went away lose theirs. Without this the
/// engine has no sinks, stays parked, and nothing reaches the lights.
pub async fn sync_device_sinks(app: &Arc<App>, devices: &[DeviceInfo]) {
    let mut sinks = app.device_sinks.lock().await;
    let live: std::collections::HashSet<_> = devices
        .iter()
        .filter(|d| d.connected)
        .filter(|d| {
            let driven = crate::is_driven(&d.id);
            if !driven && !sinks.contains_key(&d.id) {
                tracing::info!(device = %d.id, "this device is never driven; it gets no frames");
            }
            driven
        })
        .map(|d| d.id.clone())
        .collect();
    sinks.retain(|id, writer| {
        if live.contains(id) {
            return true;
        }
        app.engine.remove_sink(id);
        writer.abort();
        tracing::info!(device = %id, "device gone; its frame sink is removed");
        false
    });
    for id in live {
        if sinks.contains_key(&id) {
            continue;
        }
        let (tx, rx) = tokio::sync::mpsc::channel::<Vec<canvas_proto::Rgb8>>(1);
        // The writer task records the time of every frame that went out (#81). It lives in the
        // library, so a test can run it and prove that it records.
        let writer = tokio::spawn(crate::writer::writer_loop_with_lane(
            app.adapter.clone(),
            app.frame_lane.clone(),
            id.clone(),
            rx,
            Arc::clone(&app.last_frame),
        ));
        app.engine.add_sink(
            id.clone(),
            Arc::new(crate::state::DeviceSink::with_gate(id.clone(), tx, app.identify.clone())),
        );
        tracing::info!(device = %id, "device frame sink added");
        sinks.insert(id, writer);
    }
}

/// Runs the OpenRGB supervisor for `app` until `stop` is signalled: the daemon's own wiring, and
/// the one a test runs. It serves the rescans the API sends (see [`App::rescan_requests`]).
pub async fn supervise_app(app: Arc<App>, stop: watch::Receiver<bool>) {
    let inbox = app.take_rescan_inbox();
    let zones = Arc::clone(&app.zones);
    let adapter = Arc::clone(&app.adapter);
    let for_devices = Arc::clone(&app);
    let for_link = Arc::clone(&app);
    crate::openrgb_supervisor::supervise_with_rescans(
        adapter,
        // The zone sizes are shared with the app, so a resize shows up here.
        zones,
        move |devices| {
            let app = Arc::clone(&for_devices);
            async move { apply_devices(&app, devices).await }
        },
        // The link state follows every attempt. `Connected` is set with the devices.
        move |link| {
            let app = Arc::clone(&for_link);
            async move {
                if link != LinkState::Connected {
                    *app.openrgb.write().await = link;
                }
            }
        },
        inbox,
        stop,
    )
    .await;
}
