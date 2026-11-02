//! The settings routes, the first-run panel and the config export/import (G10).

use axum::extract::State;
use axum::Json;
use canvas_proto::api::OutputState;
use canvas_proto::config::{ConfigFile, ExportBundle, Settings};
use canvas_proto::fans::{FanProfile, FansStatus};
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
/// The editor's Settings page. The fps, brightness and power are applied to the engine
/// as well as saved.
pub async fn put_settings(
    State(app): State<Arc<App>>,
    Json(settings): Json<Settings>,
) -> ApiResult<Json<Settings>> {
    if !(canvas_proto::consts::FPS_MIN..=canvas_proto::consts::FPS_MAX).contains(&settings.fps) {
        return Err(ApiFailure::validation(format!(
            "fps must be {}..={}, got {}",
            canvas_proto::consts::FPS_MIN,
            canvas_proto::consts::FPS_MAX,
            settings.fps
        )));
    }
    if settings.brightness > 100 {
        return Err(ApiFailure::validation(format!(
            "brightness must be 0..100, got {}",
            settings.brightness
        )));
    }

    app.engine.set_fps(settings.fps);
    app.engine.set_brightness(settings.brightness);
    // The HTML budget a stack's layers are held to follows the frame rate.
    app.stacks.set_fps(settings.fps);
    let current_power = app.store.read().await.config.settings.power;
    if settings.power != current_power {
        crate::api::output::apply_power(&app, settings.power)?;
    }

    {
        let mut store = app
            .store
            .write()
            .await;
        store.config.settings = settings.clone();
        store
            .save_config()
            .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))?;
    }

    Ok(Json(settings))
}

/// `POST /api/v1/first-run/dismiss-zone-panel`.
///
/// The "size your ARGB headers" panel goes away and stays away.
pub async fn dismiss_zone_panel(State(app): State<Arc<App>>) -> ApiResult<Json<serde_json::Value>> {
    {
        let mut store = app
            .store
            .write()
            .await;
        store.config.zone_panel_dismissed = true;
        store
            .save_config()
            .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))?;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/v1/onboarding/complete`.
///
/// The user reached Done in the onboarding. The window stops opening by itself.
pub async fn complete_onboarding(State(app): State<Arc<App>>) -> ApiResult<Json<serde_json::Value>> {
    {
        let mut store = app.store.write().await;
        store.config.onboarding_complete = true;
        store
            .save_config()
            .map_err(|e| ApiFailure::internal(format!("could not save the config: {e}")))?;
    }
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
    let bundle = ExportBundle {
        schema_version: canvas_proto::config::EXPORT_SCHEMA,
        exported_at: crate::storage::export_timestamp(),
        app_version: crate::version(),
        settings: store.config.settings.clone(),
        layouts: store.layouts.clone(),
        zones: store.zones.clone(),
        effects: store.effects.clone(),
        fans,
        displays: store.displays.displays.clone(),
        stacks: app.stacks.all(),
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
    let mut fans = canvas_proto::config::FansFile {
        schema_version: canvas_proto::config::FANS_SCHEMA,
        floor_percent: status.floor_percent,
        active_profile: status.active_profile.clone(),
        profiles: Vec::new(),
        boot_color: None,
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

/// `POST /api/v1/config/import`.
///
/// Validates the bundle and refuses fan values below the floors with 422.
pub async fn config_import(
    State(app): State<Arc<App>>,
    Json(bundle): Json<ExportBundle>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::storage::validate_bundle(&bundle)
        .map_err(|e| ApiFailure::validation(e.to_string()))?;

    // The stacks in the bundle go through the rules a PUT does (D95.1.2). A bundle from before
    // stacks has none, and importing it replaces the saved stacks with none, as it does the
    // display configs.
    let known = crate::stacks::KnownEffects::from_user(&app.user_effects);
    let mut stacks: Vec<canvas_proto::stack::Stack> = Vec::new();
    for stack in &bundle.stacks {
        crate::stacks::validate_stack(stack, &known, &stacks)
            .map_err(ApiFailure::validation)?;
        stacks.push(stack.clone());
    }

    let home = app.store.read().await.home.clone();
    crate::storage::import_bundle(&home, &bundle)
        .map_err(|e| ApiFailure::internal(format!("could not import the config: {e}")))?;
    crate::stacks::write_file(&home, &stacks)
        .map_err(|e| ApiFailure::internal(format!("could not save the stacks: {e}")))?;

    // Reload the store from disk and apply it.
    let store = crate::storage::Store::load_from(&home);
    {
        let mut guard = app
            .store
            .write()
            .await;
        *guard = store;
    }
    // The stacks are read again from the file the import wrote, so the library shows them at
    // once and the next frame resolves them against the new layout.
    app.stacks.reload_with(&known);
    // The display task reads the saved configs on its next pass; wake it so it does not wait.
    app.displays.changed();
    let settings = app.store.read().await.config.settings.clone();
    app.engine.set_fps(settings.fps);
    app.engine.set_brightness(settings.brightness);
    app.engine.set_power(settings.power);
    app.stacks.set_fps(settings.fps);
    if let Some(layout) = app.active_layout().await {
        let devices = app.devices.read().await.clone();
        app.engine.set_layout(layout.clone(), devices.clone());
        app.stacks.publish_areas(&layout, &devices);
    }

    tracing::info!("the config was imported");
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// The output state after an import, for the CLI.
pub fn output_state(settings: &Settings) -> OutputState {
    OutputState { power: settings.power, brightness: settings.brightness }
}

/// A default config, for the tests.
pub fn default_config() -> ConfigFile {
    ConfigFile::default()
}
