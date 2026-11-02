//! `GET/PATCH /api/v1/render`: the frame rate (global) and the scale and sampling mode
//! (the active layout's).

use axum::extract::State;
use axum::Json;
use canvas_proto::api::{PatchRender, RenderState};
use canvas_proto::consts::{FPS_MAX, FPS_MIN};
use canvas_proto::layout::SamplingMode;
use std::sync::Arc;

use crate::api::ApiResult;
use crate::state::App;

/// `GET /api/v1/render`.
pub async fn get_render(State(app): State<Arc<App>>) -> Json<RenderState> {
    let store = app.store.read().await;
    let fps = store.config.settings.fps;
    let layout = store.active_layout().cloned();
    Json(RenderState {
        fps,
        scale: layout.as_ref().map(|l| l.render_scale).unwrap_or_default(),
        sampling: layout.as_ref().map(|l| l.sampling).unwrap_or_default(),
    })
}

/// `PATCH /api/v1/render`.
///
/// The fps changes live (no restart). The scale and sampling are written into the active
/// layout and the layout is re-set on the engine.
pub async fn patch_render(
    State(app): State<Arc<App>>,
    Json(patch): Json<PatchRender>,
) -> ApiResult<Json<RenderState>> {
    if let Some(fps) = patch.fps {
        if !(FPS_MIN..=FPS_MAX).contains(&fps) {
            return Err(crate::api::ApiFailure::validation(format!(
                "fps must be {FPS_MIN}..={FPS_MAX}, got {fps}"
            )));
        }
    }

    if let Some(fps) = patch.fps {
        app.engine.set_fps(fps);
        // The HTML budget a stack's layers are held to comes from the same rate.
        app.stacks.set_fps(fps);
        {
            let mut store = app
                .store
                .write()
                .await;
            store.config.settings.fps = fps;
            store
                .save_config()
                .map_err(|e| crate::api::ApiFailure::internal(format!("could not save the config: {e}")))?;
        }
    }

    if patch.scale.is_some() || patch.sampling.is_some() {
        let layout = {
            let mut store = app
                .store
                .write()
                .await;
            if let Some(layout) = store.active_layout_mut() {
                if let Some(scale) = patch.scale {
                    layout.render_scale = scale;
                }
                if let Some(sampling) = patch.sampling {
                    layout.sampling = sampling;
                }
            }
            store
                .save_layouts()
                .map_err(|e| crate::api::ApiFailure::internal(format!("could not save the layouts: {e}")))?;
            store.active_layout().cloned()
        };

        // Re-set the layout on the engine so the new scale and sampling take effect.
        if let Some(layout) = layout {
            let devices = app.devices.read().await.clone();
            app.engine.set_layout(layout.clone(), devices.clone());
            // The masks are built at the render scale and the sampling mode, so they are
            // rebuilt with the layout (D95.2.3).
            app.stacks.publish_areas(&layout, &devices);
        }
    }

    Ok(Json(get_render(State(app)).await.0))
}

/// The default sampling mode, used when a layout has none.
pub const DEFAULT_SAMPLING: SamplingMode = SamplingMode::AreaAverage;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fps_range_is_the_spine_range() {
        assert_eq!(FPS_MIN, 10);
        assert_eq!(FPS_MAX, 60);
    }
}
