//! The settings routes, the first-run panel and the config export/import (G10).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use canvas_proto::api::OutputState;
use canvas_proto::config::{ConfigFile, ExportBundle, Settings};
use canvas_proto::fans::{FanProfile, FansStatus, HotKind, HotLimits};
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// `GET /api/v1/settings`.
pub async fn get_settings(State(app): State<Arc<App>>) -> Json<Settings> {
    let settings = app.store.read().await.config.settings.clone();
    Json(settings)
}

/// `PUT /api/v1/settings`.
///
/// The editor's Settings page. The fps, brightness, warmth and power are applied to the
/// engine as well as saved. A body without `warmth` (a client older than #50) keeps the
/// stored warmth: a whole-object write from an old snapshot must never reset it to off.
pub async fn put_settings(
    State(app): State<Arc<App>>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<Settings>> {
    let has_warmth = body.get("warmth").is_some();
    let mut settings: Settings = serde_json::from_value(body)
        .map_err(|e| ApiFailure::validation(format!("the settings do not parse: {e}")))?;
    if !has_warmth {
        settings.warmth = app.store.read().await.config.settings.warmth;
    }
    crate::storage::validate_settings(&settings).map_err(ApiFailure::validation)?;

    let stored = app.store.read().await.config.settings.clone();
    // Safe start (#577): a value this save leaves as it was keeps what safe start shows.
    app.owner_chose_output(settings.power != stored.power, settings.brightness != stored.brightness);
    app.engine.set_fps(settings.fps);
    app.engine.set_brightness(app.effective_output(&settings).1);
    app.engine.set_warmth(settings.warmth);
    // The HTML budget a stack's layers are held to follows the frame rate.
    app.stacks.set_fps(settings.fps);
    if settings.power != stored.power {
        crate::api::output::apply_power(&app, settings.power)?;
    }

    let persisted = settings.clone();
    crate::store_io::store_save(
        &app,
        |store| {
            store.config.settings = persisted;
            Ok(())
        },
        |store| {
            store
                .save_config()
                .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))
        },
    )
    .await?;

    Ok(Json(settings))
}

/// `POST /api/v1/first-run/dismiss-zone-panel`.
///
/// The "size your ARGB headers" panel goes away and stays away.
pub async fn dismiss_zone_panel(State(app): State<Arc<App>>) -> ApiResult<Json<serde_json::Value>> {
    crate::store_io::store_save(
        &app,
        |store| {
            store.config.zone_panel_dismissed = true;
            Ok(())
        },
        |store| {
            store
                .save_config()
                .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))
        },
    )
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// The most dismissal keys the store keeps; the oldest goes when a new one is added (#580 C5).
pub const MAX_DISMISSED_FAULTS: usize = 32;

/// Only a note the current status actually carries may be dismissed or shown again (C5).
async fn check_active_fault(app: &Arc<App>, key: &str) -> ApiResult<()> {
    if key.is_empty() || key.len() > 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiFailure::validation("the fault key is not a dismissal key"));
    }
    let status = app
        .status()
        .await
        .ok_or_else(|| ApiFailure::service_unavailable("The daemon is not answering yet."))?;
    if !status.faults.iter().any(|f| f.key == key) {
        // The spine has no `unknown_fault` code; `not_found` is its 404 code and the message says
        // exactly what was not found. Adding a new code needs a spine change.
        return Err(ApiFailure::new(
            StatusCode::NOT_FOUND,
            canvas_proto::api::error_codes::NOT_FOUND,
            "That hardware note is not active, so there is nothing to change.",
        ));
    }
    Ok(())
}

/// `POST /api/v1/faults/{key}/dismiss` (#580).
///
/// The note goes away on this viewer and stays away across restarts. Its key is stored beside
/// `zone_panel_dismissed` in `config.toml` and comes back when the condition clears and returns.
pub async fn dismiss_fault(
    State(app): State<Arc<App>>,
    Path(key): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    check_active_fault(&app, &key).await?;
    crate::store_io::store_save(
        &app,
        |store| {
            if !store.config.dismissed_faults.iter().any(|k| k == &key) {
                store.config.dismissed_faults.push(key.clone());
                if store.config.dismissed_faults.len() > MAX_DISMISSED_FAULTS {
                    store.config.dismissed_faults.remove(0);
                }
            }
            Ok(())
        },
        |store| {
            store
                .save_config()
                .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))
        },
    )
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `DELETE /api/v1/faults/{key}/dismiss` (#580 C2): the strip's "Show again" puts the note back
/// on the page while its condition is still active.
pub async fn show_fault_again(
    State(app): State<Arc<App>>,
    Path(key): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    check_active_fault(&app, &key).await?;
    if !app.store.read().await.config.dismissed_faults.iter().any(|k| k == &key) {
        return Ok(Json(serde_json::json!({ "ok": true })));
    }
    crate::store_io::store_save(
        &app,
        |store| {
            store.config.dismissed_faults.retain(|k| k != &key);
            Ok(())
        },
        |store| {
            store
                .save_config()
                .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))
        },
    )
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/v1/onboarding/complete`.
///
/// The user reached Done in the onboarding. The window stops opening by itself.
pub async fn complete_onboarding(State(app): State<Arc<App>>) -> ApiResult<Json<serde_json::Value>> {
    crate::store_io::store_save(
        &app,
        |store| {
            store.config.onboarding_complete = true;
            Ok(())
        },
        |store| {
            store
                .save_config()
                .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))
        },
    )
    .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `GET /api/v1/config/export`.
///
/// The fans come from the service when it is up, and from the last config the daemon
/// pushed when it is down.
pub async fn config_export(State(app): State<Arc<App>>) -> ApiResult<Json<ExportBundle>> {
    let service_is_up = app.fans.read().await.is_some();
    let fans = if service_is_up {
        // The service is up: ask it for the whole fan config.
        match crate::pipe_client::send(
            &app,
            &canvas_proto::pipe::PipeRequest::FanStatus,
        )
        .await
        {
            Ok(canvas_proto::pipe::PipeResponse::FanStatus { status }) => {
                fans_file_from_status(&status)
            }
            Ok(other) => {
                tracing::debug!(response = ?other, "the service answered the fan status differently");
                last_fans(&app).await
            }
            Err(e) => {
                tracing::debug!(error = %e, "could not ask the service for the fan config");
                last_fans(&app).await
            }
        }
    } else {
        last_fans(&app).await
    };

    let store = app.store.read().await;
    // The scenes travel with the bundle (0.14.0), unless the migration has not run yet: then
    // only the layouts do, and the import migrates them with its own devices.
    let (lights, scenes) = crate::storage::scenes_for_export(&store);
    let bundle = ExportBundle {
        patterns: app.patterns.all(),
        schema_version: canvas_proto::config::EXPORT_SCHEMA,
        exported_at: crate::storage::export_timestamp(),
        app_version: crate::version(),
        settings: store.config.settings.clone(),
        layouts: canvas_proto::config::LayoutsFile { schema_version: canvas_proto::config::LAYOUTS_SCHEMA, active: String::new(), layouts: Vec::new() },
        zones: store.zones.clone(),
        effects: store.effects.clone(),
        fans,
        displays: store.displays.displays.clone(),
        stacks: app.stacks.all(),
        lights,
        scenes,
    };
    Ok(Json(bundle))
}

/// The last fan config the daemon pushed, or the default.
async fn last_fans(app: &Arc<App>) -> canvas_proto::config::FansFile {
    let home = app.store.read().await.home.clone();
    app.fans_last
        .read()
        .await
        .clone()
        .or_else(|| crate::storage::load_fans_last(&home))
        .unwrap_or_default()
}

/// Turns a fan status into the config file shape.
fn fans_file_from_status(status: &FansStatus) -> canvas_proto::config::FansFile {
    // Issue #130: the user's hot limits travel with the export. A row the user set
    // carries its value; an automatic row carries nothing.
    let mut hot_limits = HotLimits::default();
    for row in &status.hot {
        let value = row.user_set.then_some(row.limit_c);
        match row.kind {
            HotKind::Cpu => hot_limits.cpu_c = value,
            HotKind::Gpu => hot_limits.gpu_c = value,
            HotKind::Board => hot_limits.board_c = value,
        }
    }
    let mut fans = canvas_proto::config::FansFile {
        schema_version: canvas_proto::config::FANS_SCHEMA,
        floor_percent: status.floor_percent,
        active_profile: status.active_profile.clone(),
        profiles: Vec::new(),
        boot_color: None,
        hot_limits,
    };
    for name in &status.profiles {
        let headers = status
            .headers
            .iter()
            .filter(|h| &h.id == name)
            .map(|h| (h.id.clone(), h.mode.clone()))
            .collect();
        fans.profiles.push(FanProfile { name: name.clone(), headers });
    }
    fans
}

/// A seam in the config import, for the tests.
///
/// The import stops here, between the end of the draft and the swap of the store, so a test
/// can put a draft PUT in that gap and see what the import does with it (#94).
///
/// The daemon never sets one: [`crate::state::App::set_import_seam`] is how a test does.
#[derive(Clone, Default)]
pub struct ImportSeam {
    /// The import is at the seam.
    reached: Arc<tokio::sync::Notify>,
    /// The test lets the import go on.
    go: Arc<tokio::sync::Notify>,
}

impl ImportSeam {
    /// A seam nobody watches yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Says the import is at the seam.
    pub fn reached(&self) {
        self.reached.notify_one();
    }

    /// Waits until the import is at the seam.
    pub async fn wait_until_reached(&self) {
        self.reached.notified().await;
    }

    /// Lets the import go on.
    pub fn go(&self) {
        self.go.notify_one();
    }

    /// Waits until the test lets the import go on.
    pub async fn wait_for_go(&self) {
        self.go.notified().await;
    }
}

/// `POST /api/v1/config/import`.
///
/// Validates the bundle and refuses fan values below the floors with 422.
pub async fn config_import(
    State(app): State<Arc<App>>,
    Json(bundle): Json<ExportBundle>,
) -> ApiResult<Json<serde_json::Value>> {
    // Issue #130: the hot limits are validated against the rated maxima from the
    // last fans status when it is known (as `put_fan_hot` does); without a status
    // the fallbacks apply and the service validates again on receipt.
    let status = app.fans.read().await.clone();
    let rated_of = |kind: HotKind| {
        status
            .as_ref()
            .and_then(|s| s.hot.iter().find(|h| h.kind == kind).and_then(|h| h.rated_max_c))
    };
    let (rated_cpu, rated_gpu) = (rated_of(HotKind::Cpu), rated_of(HotKind::Gpu));
    // The machines's current devices, so a bundle with scenes and no lights rebuilds its lights
    // (and so a 0.13 bundle's layouts are migrated with this machine's zones: a matrix zone is a
    // Grid here, not a Strip).
    let devices = app.devices.read().await.clone();
    crate::storage::validate_bundle_rated(&bundle, &devices, rated_cpu, rated_gpu)
        .map_err(|e| ApiFailure::validation(e.to_string()))?;

    // The stacks in the bundle go through the rules a PUT does (D95.1.2). A bundle from before
    // stacks has none, and importing it replaces the saved stacks with none, as it does the
    // display configs.
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    let mut stacks: Vec<canvas_proto::stack::Stack> = Vec::new();
    for stack in &bundle.stacks {
        // Canonicalised as a save does, then checked as a load does (#256): a mask whose effect
        // does not resolve is kept, and the layer draws unmasked, instead of refusing the config.
        let mut stack = stack.clone();
        crate::stacks::canonicalise_and_validate_for_import(&mut stack, &known, &stacks)
            .map_err(ApiFailure::validation)?;
        stacks.push(stack);
    }

    // The patterns in the bundle keep the rules a PUT does: each one is checked, and two
    // patterns of the bundle may not share a name, ignoring case, exactly as a save refuses.
    let mut patterns: Vec<canvas_proto::pattern::Pattern> = Vec::new();
    for pattern in &bundle.patterns {
        canvas_proto::pattern::validate_pattern(pattern)
            .map_err(|error| ApiFailure::validation(format!("{}: {error}", pattern.id)))?;
        if patterns
            .iter()
            .any(|saved| saved.name().eq_ignore_ascii_case(pattern.name()))
        {
            return Err(ApiFailure::validation(format!(
                "{:?} is the name of two patterns in the bundle, ignoring case",
                pattern.name()
            )));
        }
        patterns.push(pattern.clone());
    }

    let home = app.store.read().await.home.clone();
    // The three writes run on a blocking thread, in the order they always had: each syncs to disk,
    // so on a stalled disk they must not hold a runtime worker (#241).
    let written = {
        let (home, bundle, devices) = (home.clone(), bundle.clone(), devices.clone());
        let (stacks, patterns) = (stacks.clone(), patterns.clone());
        tokio::task::spawn_blocking(move || -> Result<(), ApiFailure> {
            crate::storage::import_bundle_rated(&home, &bundle, &devices, rated_cpu, rated_gpu)
                .map_err(|e| match &e {
                    // A file this machine cannot read refuses the import (nothing was written),
                    // and a scene that cannot be made valid is the bundle's fault: both are the
                    // caller's to fix, not an internal failure.
                    crate::storage::ImportError::Blocked(_) => ApiFailure::conflict(e.to_string()),
                    crate::storage::ImportError::Scenes(_) => ApiFailure::validation(e.to_string()),
                    _ => ApiFailure::internal(format!("could not import the config: {e}")),
                })?;
            crate::stacks::write_file(&home, &stacks)
                .map_err(|e| ApiFailure::internal(format!("could not save the stacks: {e}")))?;
            // A bundle from before patterns has none, and importing it replaces the saved
            // patterns with none, as it does the stacks.
            crate::patterns::write_file(&home, &patterns)
                .map_err(|e| ApiFailure::internal(format!("could not save the patterns: {e}")))?;
            Ok(())
        })
        .await
    };
    written.map_err(|e| ApiFailure::internal(format!("the config import did not finish: {e}")))??;

    // A draft on the lights holds the layout's effect from before the import as its prior,
    // and the import brings a layout of its own. The draft ends here, before the store is
    // swapped: its prior effect goes back on the lights the normal way (the layout in
    // force now still names it), and the draft's slot, lease and epoch are gone, so the
    // draft cannot end later and put the pre-import effect back over the imported one
    // (#94).
    //
    // The draft gate is held from here across the swap of the store and the layout that
    // goes with it. A PUT that starts in the gap between the end of the draft and the swap
    // would read its prior from the store that is about to go, and its later restore would
    // put the pre-import effect back over the imported one (#94). Holding the gate makes
    // the end of the draft, the swap and the layout one step: the PUT waits for the import
    // and reads the layout the import brought in.
    //
    // The order is the gate, then the store, the order `put` uses (the gate, then
    // `store.read()`), so nothing inverts.
    {
        let _gate = app.draft.gate().lock().await;
        let ended = crate::draft::stop_locked(&app, None).await;
        if ended.ended.is_some() {
            tracing::info!("the config import ended the draft on the lights");
        }

        // The seam: the import waits here, between the end of the draft and the swap of the
        // store, so a test can put a draft PUT in that gap (#94).
        if let Some(seam) = app.import_seam() {
            seam.reached();
            seam.wait_for_go().await;
        }

        // Reload the store from disk and apply it.
        let reload_from = home.clone();
        let loaded = tokio::task::spawn_blocking(move || crate::storage::Store::load_from(&reload_from))
            .await
            .map_err(|e| ApiFailure::internal(format!("the imported config could not be read back: {e}")))?;
        crate::store_io::store_edit(&app, |guard| *guard = loaded).await;
        // The stacks are read again from the file the import wrote, so the library shows them at
        // once and the next frame resolves them against the new layout.
        app.stacks.reload_with(&known);
        // The patterns too: the file the import wrote is what every group builds from now.
        app.patterns.reload();
        // The import keeps this machine's audio-source pick (B1): the daemon
        // matches the kept-on-disk pick, like a restart does.
        let audio_source = app.store.read().await.config.audio_source.clone();
        app.inputs.set_requested(audio_source);
        // The display task reads the saved configs on its next pass; wake it so it does not wait.
        app.displays.changed();
        let settings = app.store.read().await.config.settings.clone();
        let (power, brightness) = if crate::scenes::safe_mode(&app) {
            app.raise_safe_output(&settings)
        } else {
            (settings.power, settings.brightness)
        };
        app.engine.set_fps(settings.fps);
        app.engine.set_brightness(brightness);
        app.engine.set_warmth(settings.warmth);
        app.engine.set_power(power);
        let balances = crate::api::devices::white_balance_map(&app.store.read().await.config);
        app.engine.set_white_balance(balances);
        app.stacks.set_fps(settings.fps);
        // The scene goes on again (Scenes, 0.14.0): the import brought a store of its own, and
        // the active scene is what plays now.
        crate::scenes::apply_active_ungated(&app, false).await;
    }

    tracing::info!("the config was imported");
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// The output state after an import, for the CLI.
pub fn output_state(settings: &Settings) -> OutputState {
    OutputState { power: settings.power, brightness: settings.brightness, warmth: settings.warmth }
}

/// A default config, for the tests.
pub fn default_config() -> ConfigFile {
    ConfigFile::default()
}
