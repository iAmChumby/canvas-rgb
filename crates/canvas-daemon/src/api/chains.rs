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
    ChainSetupState, ChainView, ChainsResponse, LightView, SetupCommit, SetupCommitted, SetupEmpty,
    SetupLight, SetupStarted,
};
use canvas_proto::device::{DeviceInfo, ZoneInfo};
use canvas_proto::scene::{ChainState, Light};
use canvas_proto::DeviceId;
use std::path::Path as FsPath;
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
    let _step = app.chain_setup.lock().await;
    if app.identify.setup_session().is_some() {
        return Err(ApiFailure::conflict(
            "a chain setup is already open; finish or cancel it first",
        ));
    }
    // A record still on disk with no setup open is an earlier header that has not gone back yet
    // (its resize failed, or the start-up replay has not run) or a commit that never finished. A
    // new record would overwrite it, so the start finishes it first, the way start-up does, and
    // is refused only when that fails again.
    let home = app.store.read().await.home.clone();
    if crate::chain_records::read_restore(&home).is_some()
        || crate::chain_records::read_intent(&home).is_some()
    {
        replay_leftovers(&app).await;
        if crate::chain_records::read_restore(&home).is_some()
            || crate::chain_records::read_intent(&home).is_some()
        {
            return Err(ApiFailure::conflict(
                "an earlier chain setup's header could not be put back yet; check that the device is connected and try again",
            ));
        }
    }
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
        let _ = finish_setup_locked(&app).await;
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
    // The session is checked under the step lock: a commit that finished while this waited has
    // closed it, and its parts must stay.
    let _step = app.chain_setup.lock().await;
    check_session(&app, &id, zone, &request.session).await?;
    let leds = finish_setup_locked(&app).await?;
    Ok(Json(serde_json::json!({ "ok": true, "leds": leds })))
}

/// `POST /api/v1/chains/{device}/{zone}/setup/commit`: the parts, their groups and the header's
/// final length, all or nothing.
///
/// The order is the spec's "Commit order": check the session, take the scene order and the draft
/// gate, and plan (422 when the rows break a rule); snapshot the three files and write the intent
/// that names the snapshot; resize to the parts' sum (the second and last resize of the setup);
/// write `scenes.json`, `lights.json` and `zones.json` in one store call; re-discover, put the
/// sizes the supervisor reads in step, drop the hold, end the session, delete both records, and
/// put the scene on the new parts unless a takeover holds the lights.
///
/// The failure rules are the spec's: a resize the device refuses changes nothing, puts the header
/// back and keeps the session (hold, record, gate session) for a retry, and the answer carries the
/// device's own error; a store write that fails after the resize puts memory, the three files and
/// the header back the same way and answers 503. When that rollback fails too, both records stay
/// and the next start replays them ([`replay_leftovers`]).
pub async fn commit_setup(
    State(app): State<Arc<App>>,
    Path((id, zone)): Path<(String, u32)>,
    Json(request): Json<SetupCommit>,
) -> ApiResult<Json<SetupCommitted>> {
    let device_id = DeviceId(id.clone());
    let _step = app.chain_setup.lock().await;
    check_session(&app, &id, zone, &request.session).await?;
    let max_leds = app
        .device(&device_id)
        .await
        .and_then(|device| device.zones.iter().find(|info| info.index == zone).map(|info| info.max_leds))
        .ok_or_else(|| ApiFailure::not_found(format!("{id} has no zone {zone}")))?;
    let home = app.store.read().await.home.clone();
    let Some(restore) = crate::chain_records::read_restore(&home) else {
        // The record is what a failed commit puts the header back to. Without it the commit
        // cannot be undone, so it is refused; a cancel ends the setup without one.
        return Err(ApiFailure::internal(
            "the setup's restore record is missing; cancel the setup and start it again",
        ));
    };
    if restore.device_id != device_id || restore.zone_index != zone {
        return Err(ApiFailure::internal(
            "the setup's restore record names another header; cancel the setup and start it again",
        ));
    }

    // The order every scene path takes, held across the read, the resize, the write and the
    // apply: a commit is one operation, and nothing else may build the scene over it while it
    // runs. The apply at the end is skipped while a takeover holds the lights; its own end puts
    // the scene, and these parts, back.
    let lock = app.scene.lock().await;
    let _gate = app.draft.gate().lock().await;
    let plan = {
        let store = app.store.read().await;
        crate::chain_plan::plan_commit(
            &store.lights,
            &store.scenes,
            &device_id,
            zone,
            &request.parts,
            max_leds,
        )
        .map_err(|e| ApiFailure::validation(e.to_string()))?
    };

    // A commit takes longer than the setup's quiet timeout allows between light requests. Renew
    // it before the work, with no runs (the commit's own frames are the last ones): otherwise a
    // slow resize would let the ticker end the session mid-commit and put the header back under
    // it. A stale session is caught here even when it went away since the check above.
    if !app
        .identify
        .set_runs(&request.session, Vec::new(), std::time::Instant::now())
    {
        return Err(ApiFailure::conflict(
            "this chain setup session is not open any more; start it again",
        ));
    }

    // The snapshot comes first, and the intent names it before the resize: a crash from here on
    // is a commit the next start undoes.
    let snapshot = crate::storage::snapshot(&home, "before-chain-setup").map_err(|e| {
        ApiFailure::internal(format!("could not take the snapshot before the commit: {e}"))
    })?;
    let Some(snapshot) = snapshot.file_name().and_then(|name| name.to_str()).map(str::to_string)
    else {
        return Err(ApiFailure::internal("the snapshot before the commit has no folder name"));
    };
    let intent = crate::chain_records::IntentRecord {
        device_id: device_id.clone(),
        zone_index: zone,
        restore_leds: restore.leds,
        final_leds: plan.final_leds,
        snapshot,
    };
    if let Err(e) = crate::chain_records::write_intent(&home, &intent) {
        return Err(ApiFailure::internal(format!(
            "could not write the commit's intent record: {e}"
        )));
    }

    // The hold follows the header to its final length before the resize, so the supervisor reads
    // "as wanted" throughout. No guard on the sizes or the holds is kept across an adapter call:
    // the supervisor reads both under the adapter lock, so holding one here while waiting for the
    // adapter would deadlock.
    let key = (id.clone(), zone);
    let raised = app.zone_holds.read().await.get(&key).copied().unwrap_or(max_leds);
    app.zone_holds.write().await.insert(key.clone(), plan.final_leds);

    // The second and last resize of the setup.
    let resized = {
        let mut adapter = app.adapter.lock().await;
        adapter.set_zone_size(&device_id, zone, plan.final_leds).await
    };
    if let Err(e) = resized {
        // Nothing was written. The session (hold, record, gate session) stays open for a retry,
        // so the header goes back to the raised length the setup holds.
        app.zone_holds.write().await.insert(key.clone(), raised);
        let back = {
            let mut adapter = app.adapter.lock().await;
            adapter.set_zone_size(&device_id, zone, raised).await
        };
        if let Err(back) = back {
            tracing::warn!(
                device = %id, zone, leds = raised, error = %back,
                "the header could not be put back after the failed resize; the session still holds it"
            );
        }
        crate::chain_records::delete_intent(&home);
        return Err(ApiFailure::internal(format!(
            "could not set the header to {} LEDs: {e}",
            plan.final_leds
        )));
    }

    let saved = {
        // `zones.json` is written whole from the store's copy, and the sizes the first run and a
        // zone PATCH saved live in `app.zones`: the copy takes them first, so the commit keeps
        // every other header's saved length.
        let sizes = app.zones.read().await.clone();
        let mut store = app.store.write().await;
        store.zones.sizes = sizes;
        store.apply_chain_plan(&plan, Some((&device_id, zone, plan.final_leds)))
    };
    if let Err(e) = saved {
        // Memory is back already: the store puts it back itself. Put the three files and the
        // header back too. When either fails, both records stay and start-up replays them.
        // The session stays open for a retry or a cancel, as after a refused resize: the header
        // goes back to the raised length the setup holds, and the restore record still names the
        // earlier length for the cancel (or the next start).
        let files = crate::chain_records::rollback_files(&home, &intent);
        app.zone_holds.write().await.insert(key.clone(), raised);
        let back = {
            let mut adapter = app.adapter.lock().await;
            adapter.set_zone_size(&device_id, zone, raised).await
        };
        let mut rolled_back = true;
        if let Err(files) = &files {
            rolled_back = false;
            tracing::warn!(
                error = %files,
                "the commit's files could not be put back; the intent stays for the next start"
            );
        }
        if let Err(back) = &back {
            rolled_back = false;
            tracing::warn!(
                device = %id, zone, leds = raised, error = %back,
                "the header could not be put back to its raised length after the failed write"
            );
        }
        if rolled_back {
            crate::chain_records::delete_intent(&home);
            tracing::info!(device = %id, zone, "the commit could not be saved: its files are back and the setup is still open");
            return Err(ApiFailure::service_unavailable(format!("the chain was not saved: {e}")));
        }
        return Err(ApiFailure::service_unavailable(format!(
            "the chain was not saved, and its rollback did not finish; the next start finishes it: {e}"
        )));
    }

    // The saved size first, then the hold goes: the supervisor never sees the header unwanted.
    app.zones.write().await.entry(id.clone()).or_default().insert(zone, plan.final_leds);
    app.zone_holds.write().await.remove(&key);

    if let Err(e) = rediscover(&app).await {
        // The files landed; the list catches up on the next refresh. Say so rather than fail a
        // commit the disk already holds.
        tracing::warn!(error = %e.message, "the committed header's list could not be read");
    }
    if let Some(session) = app.identify.setup_session() {
        app.identify.end_setup(&session);
    }
    crate::chain_records::delete_intent(&home);
    crate::chain_records::delete_restore(&home);
    crate::identify::restore(&app).await;
    if !app.scene.is_taken_over() {
        crate::scenes::apply_with(&app, &lock, false).await;
    }

    let lights = {
        let devices = app.devices.read().await.clone();
        let store = app.store.read().await;
        let mut parts: Vec<LightView> = store
            .lights
            .lights
            .iter()
            .filter(|light| light.device_id == device_id && light.zone_index == zone)
            .map(|light| light_view(light, &devices))
            .collect();
        parts.sort_by_key(|part| part.light.leds.map_or(0, |slice| slice.start));
        parts
    };
    tracing::info!(
        device = %id, zone, parts = lights.len(), final_leds = plan.final_leds,
        "a chain was set up: the parts, their groups and the header's length landed together"
    );
    Ok(Json(SetupCommitted { lights, final_leds: plan.final_leds }))
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
    let _step = app.chain_setup.lock().await;
    check_session(&app, &id, zone, &request.session).await?;
    let leds = finish_setup_locked(&app).await?;
    let device_id = DeviceId(id.clone());
    let plan = {
        let store = app.store.read().await;
        crate::chain_plan::plan_empty(&store.lights, &store.scenes, &device_id, zone)
    };
    {
        let mut store = app.store.write().await;
        store.apply_chain_plan(&plan, None).map_err(|e| {
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
    let _step = app.chain_setup.lock().await;
    finish_setup_locked(app).await
}

/// [`finish_setup`] for a caller that already holds `App::chain_setup`. A second end (a cancel
/// after the timeout) finds no record and resizes nothing.
async fn finish_setup_locked(app: &Arc<App>) -> Result<u32, ApiFailure> {
    // Whatever the reason, the setup is over: the gate must not keep holding the lights.
    if let Some(session) = app.identify.setup_session() {
        app.identify.end_setup(&session);
    }
    let home = app.store.read().await.home.clone();
    // A commit whose rollback failed left its intent: the setup ends the way start-up would end
    // it, now, rather than at a start that may come much later and would undo every edit since.
    if let Some(intent) = crate::chain_records::read_intent(&home) {
        app.zone_holds
            .write()
            .await
            .remove(&(intent.device_id.0.clone(), intent.zone_index));
        roll_back_intent(app, &home, &intent).await;
        if crate::chain_records::read_intent(&home).is_some() {
            crate::identify::restore(app).await;
            crate::scenes::apply_for_devices(app).await;
            return Err(ApiFailure::internal(
                "the unfinished commit could not be undone; the next start finishes it",
            ));
        }
        crate::identify::restore(app).await;
        return Ok(intent.restore_leds);
    }
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

/// What start-up replays once, on the first non-empty device list: the records a crash mid-setup
/// or mid-commit left.
///
/// An intent record is a commit that never finished, and it owns the start: the files go back to
/// its snapshot, the store reloads them, and the header returns to its earlier length. A restore
/// record with no intent is a setup that was open when the daemon stopped, and it puts the header
/// back on its own.
pub async fn replay_leftovers(app: &Arc<App>) {
    // An open setup's records are its own, and a start finished any earlier ones before it
    // opened: a replay that waited for the step lock behind that start leaves them alone.
    if app.identify.setup_session().is_some() {
        return;
    }
    let home = app.store.read().await.home.clone();
    if let Some(intent) = crate::chain_records::read_intent(&home) {
        roll_back_intent(app, &home, &intent).await;
        return;
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

/// Undoes the commit an intent record names: the three files come back from the snapshot, the
/// store reloads them (the start's own reader), and the header returns to `restore_leds`. Both
/// records go only when every step worked; any failure leaves them for the next start.
async fn roll_back_intent(
    app: &Arc<App>,
    home: &FsPath,
    intent: &crate::chain_records::IntentRecord,
) {
    if let Err(e) = crate::chain_records::rollback_files(home, intent) {
        tracing::warn!(
            device = %intent.device_id, zone = intent.zone_index, error = %e,
            "the interrupted commit's files could not be put back; both records stay for the next start"
        );
        return;
    }
    // The store was loaded from the files the interrupted commit wrote. What the snapshot holds
    // is what it must run on now, and the supervisor's sizes must be those.
    let reloaded = crate::storage::Store::load_from(home);
    let sizes = reloaded.zones.sizes.clone();
    {
        let mut store = app.store.write().await;
        store.lights = reloaded.lights;
        store.scenes = reloaded.scenes;
        store.zones = reloaded.zones;
    }
    *app.zones.write().await = sizes;

    let resized = {
        let mut adapter = app.adapter.lock().await;
        adapter
            .set_zone_size(&intent.device_id, intent.zone_index, intent.restore_leds)
            .await
    };
    match resized {
        Ok(()) => {
            crate::chain_records::delete_intent(home);
            // The restore record is the same header's (one setup at a time); one that names
            // another header is that header's to put back, so it stays.
            if crate::chain_records::read_restore(home).is_none_or(|record| {
                record.device_id == intent.device_id && record.zone_index == intent.zone_index
            }) {
                crate::chain_records::delete_restore(home);
            }
            if let Err(e) = rediscover(app).await {
                tracing::warn!(error = %e.message, "the rolled-back header's list could not be read");
            }
            crate::scenes::apply_for_devices(app).await;
            tracing::info!(
                device = %intent.device_id, zone = intent.zone_index, leds = intent.restore_leds,
                "an interrupted chain commit was undone: its files and the header's earlier length are back"
            );
        }
        Err(e) => tracing::warn!(
            device = %intent.device_id, zone = intent.zone_index, leds = intent.restore_leds, error = %e,
            "the interrupted commit's header could not be put back; both records stay for the next start"
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
