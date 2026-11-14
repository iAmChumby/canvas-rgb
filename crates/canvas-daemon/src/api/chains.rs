//! The chain routes (#338): `GET /api/v1/chains`, and one setup session per header.
//!
//! `POST /api/v1/chains/{device}/{zone}/setup` raises a header to its longest length and opens a
//! session that holds the lights ([`crate::identify`]): the owner answers what is plugged into it,
//! and the app lights coloured runs on it. The session ends by a commit (Task 7b), by cancel, by a
//! quiet timeout, or by the daemon quitting, and every end but a commit puts the earlier length
//! back.
//!
//! The earlier length is on disk before the raise (`chain-restore.json`), so a crash between the
//! two cannot leave a header raised for good: the first device list after a start replays the
//! record ([`replay_leftovers`]). The raise is never written to `zones.json`; while the setup is
//! open it lives in `App::zone_holds`, which the OpenRGB supervisor overlays on the saved sizes so
//! its refresh leaves the raise alone.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{
    ChainSetupState, ChainView, ChainsResponse, LightView, SetupEmpty, SetupLight, SetupStarted,
};
use canvas_proto::device::{DeviceInfo, ZoneInfo};
use canvas_proto::scene::{ChainState, Light};
use canvas_proto::DeviceId;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::chain_records::RestoreRecord;
use crate::state::App;

/// `GET /api/v1/chains`.
///
/// One view per resizable zone of every device the daemon holds. Its state comes from the hardware
/// map: a chain record saying `empty` or `set_up`; no record at all is `not_set_up`, which is what
/// first run's guessed 120-LED sizing reads as. A guessed length is never shown as a known one.
pub async fn list_chains(State(app): State<Arc<App>>) -> Json<ChainsResponse> {
    let devices = app.devices.read().await.clone();
    let store = app.store.read().await;
    let mut chains = Vec::new();
    for device in &devices {
        for zone in device.zones.iter().filter(|zone| zone.resizable) {
            let state = store
                .lights
                .chains
                .iter()
                .find(|record| record.device_id == device.id && record.zone_index == zone.index)
                .map_or(ChainSetupState::NotSetUp, |record| match record.state {
                    ChainState::Empty => ChainSetupState::Empty,
                    ChainState::SetUp => ChainSetupState::SetUp,
                });
            let mut parts: Vec<LightView> = store
                .lights
                .lights
                .iter()
                .filter(|light| light.device_id == device.id && light.zone_index == zone.index)
                .map(|light| light_view(light, &devices))
                .collect();
            // Chain order: a part's run starts where it sits along the header, and the whole-zone
            // light of a chain that was never split owns it from LED 0.
            parts.sort_by_key(|part| part.light.leds.map_or(0, |slice| slice.start));
            chains.push(ChainView {
                device_id: device.id.clone(),
                device_name: device.name.clone(),
                zone_index: zone.index,
                zone_name: zone.name.clone(),
                state,
                leds: zone.led_count,
                min_leds: zone.min_leds,
                max_leds: zone.max_leds,
                parts,
            });
        }
    }
    Json(ChainsResponse { chains })
}

/// `POST /api/v1/chains/{device}/{zone}/setup`: opens a setup on one header.
///
/// 404 for an unknown device or zone, 422 for a zone that cannot be resized, 409 while a setup is
/// open (on any header: one window at a time). The gate session is opened first, so the 409 is
/// settled before anything is touched and two starts cannot undo each other's raise. The restore
/// record is written before the raise, so a crash from here on is replayed at start; the raise
/// itself goes in `App::zone_holds`, never in `zones.json`. A raise the device refuses deletes
/// the record and the hold, ends the session and answers the device's own error.
pub async fn start_setup(
    State(app): State<Arc<App>>,
    Path((id, zone)): Path<(String, u32)>,
) -> ApiResult<Json<SetupStarted>> {
    let device_id = DeviceId(id.clone());
    let device = app
        .device(&device_id)
        .await
        .ok_or_else(|| ApiFailure::not_found(format!("no device with the id {id}")))?;
    let Some(zone_info) = device.zones.iter().find(|info| info.index == zone) else {
        return Err(ApiFailure::not_found(format!("{id} has no zone {zone}")));
    };
    if !zone_info.resizable {
        return Err(ApiFailure::validation(format!("{id} zone {zone} cannot be resized")));
    }

    let restore_to = zone_info.led_count;
    let raised_to = zone_info.max_leds;
    let session = session_id();
    let Some(start_ticker) =
        app.identify
            .start_setup(device_id.clone(), zone, session.clone(), std::time::Instant::now())
    else {
        return Err(ApiFailure::conflict(
            "a chain setup is already open; finish or cancel it first",
        ));
    };
    // From here the gate holds the lights: every path below either completes the start or ends
    // the session before it answers.
    //
    // The gate counts a ticker as running from here, so one is spawned at once: a failure below
    // ends the setup and the ticker then finds nothing and stops, instead of the gate believing in
    // a ticker that never started (which would leave every later identify and flash dark).
    if start_ticker {
        crate::identify::spawn_ticker(app.clone());
    }
    let home = app.store.read().await.home.clone();
    let record = RestoreRecord { device_id: device_id.clone(), zone_index: zone, leds: restore_to };
    // Before the raise: a crash after this point is a header the next start puts back.
    if let Err(e) = crate::chain_records::write_restore(&home, &record) {
        app.identify.end_setup(&session);
        return Err(ApiFailure::internal(format!(
            "could not write the chain restore record: {e}"
        )));
    }
    app.zone_holds.write().await.insert((id.clone(), zone), raised_to);

    let raised = {
        let mut adapter = app.adapter.lock().await;
        adapter.set_zone_size(&device_id, zone, raised_to).await
    };
    if let Err(e) = raised {
        // Nothing was raised, so nothing may be replayed.
        app.zone_holds.write().await.remove(&(id.clone(), zone));
        crate::chain_records::delete_restore(&home);
        app.identify.end_setup(&session);
        return Err(ApiFailure::internal(format!(
            "could not raise the header to {raised_to} LEDs: {e}"
        )));
    }

    // The list carries the raised length. The scene is deliberately not re-applied: the setup's
    // gate holds the lights dark, and the raised list would be a frame of the old scene.
    if let Err(e) = rediscover(&app).await {
        tracing::warn!(device = %id, zone, error = %e.message, "the raised header's list could not be read; putting the length back");
        let _ = finish_setup(&app).await;
        return Err(e);
    }

    tracing::info!(
        device = %id, zone, restore_to, raised_to, session = %session,
        "a chain setup opened: the header is raised and held"
    );
    Ok(Json(SetupStarted { session, restore_to, raised_to }))
}

/// `POST /api/v1/chains/{device}/{zone}/setup/light`: the runs to show now, also the heartbeat.
///
/// The runs are LED indexes inside the session's zone, each in its own colour; every other LED of
/// every device is black. The first frame is pushed before the answer, so the hardware changes
/// with the request. 409 when the session is no longer open.
pub async fn light_setup(
    State(app): State<Arc<App>>,
    Path((id, zone)): Path<(String, u32)>,
    Json(request): Json<SetupLight>,
) -> ApiResult<Json<serde_json::Value>> {
    let device_id = DeviceId(id.clone());
    let Some(device) = app.device(&device_id).await else {
        return Err(ApiFailure::not_found(format!("no device with the id {id}")));
    };
    if !device.zones.iter().any(|info| info.index == zone) {
        return Err(ApiFailure::not_found(format!("{id} has no zone {zone}")));
    }
    if !app
        .identify
        .set_runs(&request.session, request.runs, std::time::Instant::now())
    {
        return Err(ApiFailure::conflict(
            "this chain setup session is not open any more; start it again",
        ));
    }
    // The first frames now; the ticker repeats them until the timeout.
    let devices = app.devices.read().await.clone();
    if let crate::identify::Tick::Frames(frames) =
        app.identify.tick(std::time::Instant::now(), &devices)
    {
        app.identify.push(frames);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `DELETE /api/v1/chains/{device}/{zone}/setup`: ends the setup and puts the earlier length and
/// the scene back. The body names the session; 409 when it is stale. A resize back that fails
/// leaves the restore record on disk for the next start and answers the device's error.
pub async fn cancel_setup(
    State(app): State<Arc<App>>,
    Path((id, zone)): Path<(String, u32)>,
    Json(request): Json<SetupEmpty>,
) -> ApiResult<Json<serde_json::Value>> {
    check_session(&app, &id, zone, &request.session).await?;
    let leds = finish_setup(&app).await?;
    Ok(Json(serde_json::json!({ "ok": true, "leds": leds })))
}

/// `POST /api/v1/chains/{device}/{zone}/setup/empty`: nothing is plugged into this header.
///
/// The setup ends as a cancel does (the earlier length, the scene back), and then the zone's
/// lights leave the hardware map and every group, and the chain is recorded empty. The header
/// keeps its earlier length, is not a light, and the scene no longer names it, so it is driven
/// black.
pub async fn empty_setup(
    State(app): State<Arc<App>>,
    Path((id, zone)): Path<(String, u32)>,
    Json(request): Json<SetupEmpty>,
) -> ApiResult<Json<serde_json::Value>> {
    check_session(&app, &id, zone, &request.session).await?;
    let leds = finish_setup(&app).await?;
    let device_id = DeviceId(id.clone());
    let plan = {
        let store = app.store.read().await;
        crate::chain_plan::plan_empty(&store.lights, &store.scenes, &device_id, zone)
    };
    {
        let mut store = app.store.write().await;
        store.apply_chain_plan(&plan).map_err(|e| {
            ApiFailure::service_unavailable(format!("the empty header was not saved: {e}"))
        })?;
    }
    crate::scenes::apply_for_devices(&app).await;
    tracing::info!(device = %id, zone, "a chain is empty: its lights left the hardware map");
    Ok(Json(serde_json::json!({ "ok": true, "leds": leds })))
}

/// The restore every end of a setup runs, but a commit: the session ends, the header goes back to
/// the length the restore record holds, the hold and then the record go, and the scene plays
/// again. One path for cancel, empty, the setup's timeout and the daemon's quit.
///
/// A resize back the device refuses leaves the record where it is, for the next start to replay,
/// and answers the device's own error. The hold is dropped either way: the setup is not holding
/// anything any more. Returns the length the header went back to (0 when there was nothing to
/// restore).
pub async fn finish_setup(app: &Arc<App>) -> Result<u32, ApiFailure> {
    // Whatever the reason, the setup is over: the gate must not keep holding the lights.
    if let Some(session) = app.identify.setup_session() {
        app.identify.end_setup(&session);
    }
    let home = app.store.read().await.home.clone();
    let Some(record) = crate::chain_records::read_restore(&home) else {
        // No readable record: nothing says which length to put back, but nothing may keep the
        // header raised either. One setup at a time, so every hold is this setup's. The scene goes
        // back on the lights all the same.
        app.zone_holds.write().await.clear();
        crate::scenes::apply_for_devices(app).await;
        return Ok(0);
    };

    let resized = {
        let mut adapter = app.adapter.lock().await;
        adapter
            .set_zone_size(&record.device_id, record.zone_index, record.leds)
            .await
    };
    app.zone_holds
        .write()
        .await
        .remove(&(record.device_id.0.clone(), record.zone_index));
    match resized {
        Ok(()) => {
            crate::chain_records::delete_restore(&home);
            if let Err(e) = rediscover(app).await {
                // The length is back and the record's job is done; the list catches up on the
                // next refresh. Say so rather than fail the restore over a stale list.
                tracing::warn!(error = %e.message, "the header went back but the device list still carries the raised length");
            }
            crate::identify::restore(app).await;
            crate::scenes::apply_for_devices(app).await;
            tracing::info!(
                device = %record.device_id, zone = record.zone_index, leds = record.leds,
                "the chain setup ended: the header's earlier length is back"
            );
            Ok(record.leds)
        }
        Err(e) => {
            // The header is still raised. The record stays, and the next start replays it.
            crate::identify::restore(app).await;
            crate::scenes::apply_for_devices(app).await;
            Err(ApiFailure::internal(format!(
                "could not put the header back to {} LEDs: {e}",
                record.leds
            )))
        }
    }
}

/// What start-up replays once, on the first non-empty device list: the record a crash mid-setup
/// left. An intent record is a commit that never finished, and Task 7b's rollback owns it.
pub async fn replay_leftovers(app: &Arc<App>) {
    let home = app.store.read().await.home.clone();
    if let Some(intent) = crate::chain_records::read_intent(&home) {
        tracing::info!(
            device = %intent.device_id, zone = intent.zone_index,
            "a chain commit was interrupted; its rollback is left for the commit's own replay"
        );
    }
    let Some(record) = crate::chain_records::read_restore(&home) else {
        return;
    };
    let resized = {
        let mut adapter = app.adapter.lock().await;
        adapter
            .set_zone_size(&record.device_id, record.zone_index, record.leds)
            .await
    };
    match resized {
        Ok(()) => {
            crate::chain_records::delete_restore(&home);
            if let Err(e) = rediscover(app).await {
                tracing::warn!(error = %e.message, "the header went back but the device list could not be read");
            }
            crate::scenes::apply_for_devices(app).await;
            tracing::info!(
                device = %record.device_id, zone = record.zone_index, leds = record.leds,
                "a chain setup was interrupted: the header's earlier length is back"
            );
        }
        Err(e) => tracing::warn!(
            device = %record.device_id, zone = record.zone_index, leds = record.leds, error = %e,
            "the header's earlier length could not be put back; the record stays for the next start"
        ),
    }
}

/// Ends nothing and restores nothing unless `session` is the setup that is open on `id`/`zone`:
/// a window that was closed and reopened answers a 409, as does one for another header.
async fn check_session(
    app: &Arc<App>,
    id: &str,
    zone: u32,
    session: &str,
) -> ApiResult<()> {
    let device_id = DeviceId(id.to_string());
    let Some(device) = app.device(&device_id).await else {
        return Err(ApiFailure::not_found(format!("no device with the id {id}")));
    };
    if !device.zones.iter().any(|info| info.index == zone) {
        return Err(ApiFailure::not_found(format!("{id} has no zone {zone}")));
    }
    if app.identify.setup_session().as_deref() != Some(session) {
        return Err(ApiFailure::conflict(
            "this chain setup session is not open any more; start it again",
        ));
    }
    Ok(())
}

/// Reads the devices again and keeps the new list, as a zone resize does (`api/devices.rs`): the
/// next frames and the next scene build need the length the header really has.
async fn rediscover(app: &Arc<App>) -> Result<(), ApiFailure> {
    let devices = {
        let mut adapter = app.adapter.lock().await;
        adapter.discover().await.map_err(|e| {
            ApiFailure::internal(format!("could not re-discover the devices: {e}"))
        })?
    };
    *app.devices.write().await = devices;
    Ok(())
}

/// A session id: the clock mixed with a counter, in hex. Two setups opened in the same instant
/// must not answer to one id, so a stale window can never speak for another's setup.
fn session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("setup-{:016x}", nanos ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// What the editor shows beside a light: the same shape the lights route answers with.
fn light_view(light: &Light, devices: &[DeviceInfo]) -> LightView {
    let device = devices.iter().find(|device| device.id == light.device_id);
    let zone = device
        .and_then(|device| device.zones.iter().find(|zone| zone.index == light.zone_index));
    LightView {
        light: light.clone(),
        led_count: zone.map(|zone| led_count(light, zone)).unwrap_or(0),
        connected: device.is_some_and(|device| device.connected),
        label: label(light, device, zone),
    }
}

/// The LEDs the light has now: its run clamped to the zone, or the zone's whole count.
fn led_count(light: &Light, zone: &ZoneInfo) -> u32 {
    match light.leds {
        None => zone.led_count,
        Some(slice) => {
            let start = slice.start.min(zone.led_count);
            slice.count.min(zone.led_count - start)
        }
    }
}

/// The owner's name, else the device and zone name, else the light's id.
fn label(light: &Light, device: Option<&DeviceInfo>, zone: Option<&ZoneInfo>) -> String {
    if let Some(name) = light.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_string();
    }
    match (device, zone) {
        (Some(device), Some(zone)) => format!("{} {}", device.name, zone.name),
        (Some(device), None) => device.name.clone(),
        _ => light.id.clone(),
    }
}
