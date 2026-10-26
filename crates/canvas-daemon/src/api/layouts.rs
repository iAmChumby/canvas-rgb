//! The layout routes: list, read, replace and activate.

use axum::extract::{Path, State};
use axum::Json;
use canvas_proto::api::{LayoutsResponse, SetActiveLayout};
use canvas_proto::layout::Layout;
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// `GET /api/v1/layouts`.
pub async fn list_layouts(State(app): State<Arc<App>>) -> Json<LayoutsResponse> {
    let store = app.store.read().await;
    let layouts = store.layouts.layouts.clone();
    let active = store.layouts.active.clone();
    Json(LayoutsResponse { active, layouts })
}

/// `GET /api/v1/layouts/{id}`.
pub async fn get_layout(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
) -> ApiResult<Json<Layout>> {
    let store = app.store.read().await;
    let layout = store
        .layouts
        .layouts
        .iter()
        .find(|l| l.id == id)
        .cloned()
        .ok_or_else(|| ApiFailure::not_found(format!("no layout with the id {id}")))?;
    Ok(Json(layout))
}

/// `PUT /api/v1/layouts/{id}`.
///
/// Replaces a layout. When it is the active one, the engine gets it.
pub async fn put_layout(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(layout): Json<Layout>,
) -> ApiResult<Json<Layout>> {
    if layout.id != id {
        return Err(ApiFailure::validation(format!(
            "the layout id in the body ({}) does not match the path ({id})",
            layout.id
        )));
    }
    {
        let mut store = app
            .store
            .write()
            .await;
        match store.layouts.layouts.iter_mut().find(|l| l.id == id) {
            Some(existing) => *existing = layout.clone(),
            None => store.layouts.layouts.push(layout.clone()),
        }
        store
            .save_layouts()
            .map_err(|e| ApiFailure::internal(format!("could not save the layouts: {e}")))?;
    }

    // The active layout goes to the engine straight away.
    let active = app.store.read().await.layouts.active.clone();
    if active == id {
        let devices = app.devices.read().await.clone();
        app.engine.set_layout(layout.clone(), devices);
        if let Some(effect) = &layout.effect {
            let _ = crate::apply_effect(&app, &effect.id, &effect.props).await;
        }
    }

    Ok(Json(layout))
}

/// `PUT /api/v1/layouts/active`.
///
/// Switches to another layout and re-sets it on the engine.
pub async fn put_active_layout(
    State(app): State<Arc<App>>,
    Json(request): Json<SetActiveLayout>,
) -> ApiResult<Json<LayoutsResponse>> {
    let layout = {
        let mut store = app
            .store
            .write()
            .await;
        let layout = store
            .layouts
            .layouts
            .iter()
            .find(|l| l.id == request.id)
            .cloned()
            .ok_or_else(|| ApiFailure::not_found(format!("no layout with the id {}", request.id)))?;
        store.layouts.active = request.id.clone();
        store
            .save_layouts()
            .map_err(|e| ApiFailure::internal(format!("could not save the layouts: {e}")))?;
        layout
    };

    let devices = app.devices.read().await.clone();
    app.engine.set_layout(layout.clone(), devices);
    if let Some(effect) = &layout.effect {
        let _ = crate::apply_effect(&app, &effect.id, &effect.props).await;
    }
    tracing::info!(layout = %layout.id, "the active layout changed");

    Ok(list_layouts(State(app)).await)
}
