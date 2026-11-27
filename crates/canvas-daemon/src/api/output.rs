//! `GET/PATCH /api/v1/output`: the global power state, brightness and warmth (#50).

use axum::extract::State;
use axum::Json;
use canvas_proto::api::{OutputState, PatchOutput, Power};
use std::sync::Arc;

use crate::api::ApiResult;
use crate::state::App;

/// `GET /api/v1/output`.
pub async fn get_output(State(app): State<Arc<App>>) -> Json<OutputState> {
    let state = app.store.read().await;
    let (power, brightness) = app.effective_output(&state.config.settings);
    let state = OutputState {
        power,
        brightness,
        warmth: state.config.settings.warmth,
    };
    Json(state)
}

/// `PATCH /api/v1/output`.
///
/// `power: "paused"` writes black once and then stops the render loop; `"running"`
/// resumes it. `brightness` is 0..100. `warmth` is kelvin,
/// [`canvas_proto::consts::WARMTH_MIN`]..=[`canvas_proto::consts::WARMTH_OFF`].
#[axum::debug_handler]
pub async fn patch_output(
    State(app): State<Arc<App>>,
    Json(patch): Json<PatchOutput>,
) -> ApiResult<Json<OutputState>> {
    // Everything is validated before anything is applied: a refused PATCH changes nothing.
    if let Some(brightness) = patch.brightness {
        if brightness > 100 {
            return Err(crate::api::ApiFailure::validation(format!(
                "brightness must be 0..100, got {brightness}"
            )));
        }
    }
    if let Some(warmth) = patch.warmth {
        validate_warmth(warmth)?;
    }
    app.owner_chose_output(patch.power.is_some(), patch.brightness.is_some());
    if let Some(brightness) = patch.brightness {
        app.engine.set_brightness(brightness);
    }
    if let Some(warmth) = patch.warmth {
        app.engine.set_warmth(warmth);
    }
    if let Some(power) = patch.power {
        apply_power(&app, power)?;
    }

    // Persist. The memory change and the disk write move together off the runtime worker;
    // the store lock is free while the disk is busy (#241).
    crate::store_io::store_save(
        &app,
        |store| {
            if let Some(brightness) = patch.brightness {
                store.config.settings.brightness = brightness;
            }
            if let Some(warmth) = patch.warmth {
                store.config.settings.warmth = warmth;
            }
            if let Some(power) = patch.power {
                store.config.settings.power = power;
            }
            Ok(())
        },
        |store| {
            store
                .save_config()
                .map_err(|e| crate::api::ApiFailure::internal(format!("could not save the config: {e}")))
        },
    )
    .await?;

    Ok(Json(get_output(State(app)).await.0))
}

/// #50: the warmth stage takes [`canvas_proto::consts::WARMTH_MIN`]..=[`canvas_proto::consts::WARMTH_OFF`] kelvin.
pub fn validate_warmth(warmth: u16) -> ApiResult<()> {
    use canvas_proto::consts::{WARMTH_MIN, WARMTH_OFF};
    if !(WARMTH_MIN..=WARMTH_OFF).contains(&warmth) {
        return Err(crate::api::ApiFailure::validation(format!(
            "warmth must be {WARMTH_MIN}..={WARMTH_OFF} K, got {warmth}"
        )));
    }
    Ok(())
}

/// Applies a power state to the engine and logs why.
pub fn apply_power(app: &Arc<App>, power: Power) -> ApiResult<()> {
    match power {
        Power::Paused => {
            // Paused writes black once, then stops the render loop.
            app.identify.stop();
            app.identify.paint_solid(app.engine.as_ref(), canvas_proto::Rgb8::BLACK);
            app.engine.set_power(Power::Paused);
            tracing::info!(reason = "requested", "output paused");
        }
        Power::Running => {
            app.engine.set_power(Power::Running);
            tracing::info!(reason = "requested", "output resumed");
        }
    }
    Ok(())
}
