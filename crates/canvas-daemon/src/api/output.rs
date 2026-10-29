//! `GET/PATCH /api/v1/output`: the global power state and brightness.

use axum::extract::State;
use axum::Json;
use canvas_proto::api::{OutputState, PatchOutput, Power};
use std::sync::Arc;

use crate::api::ApiResult;
use crate::state::App;

/// `GET /api/v1/output`.
pub async fn get_output(State(app): State<Arc<App>>) -> Json<OutputState> {
    let state = app.store.read().await;
    let state = OutputState {
        power: state.config.settings.power,
        brightness: state.config.settings.brightness,
    };
    Json(state)
}

/// `PATCH /api/v1/output`.
///
/// `power: "paused"` writes black once and then stops the render loop; `"running"`
/// resumes it. `brightness` is 0..100.
#[axum::debug_handler]
pub async fn patch_output(
    State(app): State<Arc<App>>,
    Json(patch): Json<PatchOutput>,
) -> ApiResult<Json<OutputState>> {
    if let Some(brightness) = patch.brightness {
        if brightness > 100 {
            return Err(crate::api::ApiFailure::validation(format!(
                "brightness must be 0..100, got {brightness}"
            )));
        }
        app.engine.set_brightness(brightness);
    }
    if let Some(power) = patch.power {
        apply_power(&app, power)?;
    }

    // Persist.
    {
        let mut store = app
            .store
            .write()
            .await;
        if let Some(brightness) = patch.brightness {
            store.config.settings.brightness = brightness;
        }
        if let Some(power) = patch.power {
            store.config.settings.power = power;
        }
        store
            .save_config()
            .map_err(|e| crate::api::ApiFailure::internal(format!("could not save the config: {e}")))?;
    }

    Ok(Json(get_output(State(app)).await.0))
}

/// Applies a power state to the engine and logs why.
pub fn apply_power(app: &Arc<App>, power: Power) -> ApiResult<()> {
    match power {
        Power::Paused => {
            // Paused writes black once, then stops the render loop.
            app.identify.stop();
            app.engine.write_solid(canvas_proto::Rgb8::BLACK);
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
