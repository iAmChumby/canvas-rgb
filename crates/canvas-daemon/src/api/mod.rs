//! The loopback HTTP API on 127.0.0.1:9430.
//!
//! Every route is under `/api/v1`. The G9 guards (bearer token, Origin, Host) run as
//! middleware on the whole router.

pub mod diag;
pub mod devices;
pub mod displays;
pub mod effects;
pub mod fans;
pub mod layouts;
pub mod output;
pub mod peripherals;
pub mod preview;
pub mod render;
pub mod settings;
pub mod status;

use axum::routing::{get, patch, post, put};
use axum::Router;
use canvas_proto::api::routes;
use std::sync::Arc;

use crate::state::App;

/// Builds the API router.
pub fn router(app: Arc<App>) -> Router {
    let api = Router::new()
        .route(routes::STATUS, get(status::status))
        .route(routes::OUTPUT, get(output::get_output).patch(output::patch_output))
        .route(routes::RENDER, get(render::get_render).patch(render::patch_render))
        .route(routes::DEVICES, get(devices::list_devices))
        .route(
            routes::DEVICE_ZONE,
            patch(devices::patch_zone),
        )
        .route(routes::DEVICE_IDENTIFY, post(devices::identify))
        .route(routes::EFFECTS, get(effects::list_effects))
        .route(
            routes::EFFECT,
            get(effects::get_effect)
                .put(effects::put_effect)
                .delete(effects::delete_effect),
        )
        .route(
            routes::EFFECTS_IMPORT,
            post(effects::import_effect)
                .layer(axum::extract::DefaultBodyLimit::max(effects::IMPORT_BODY_LIMIT)),
        )
        .route(routes::EFFECT_ACTIVATE, post(effects::activate))
        .route(routes::EFFECT_THUMBNAIL, get(effects::thumbnail))
        .route(routes::EFFECT_ACTIVE_PROPS, patch(effects::patch_active_props))
        .route(routes::EFFECT_FAVORITE, put(effects::put_favorite))
        .route(routes::LAYOUTS, get(layouts::list_layouts))
        .route(routes::LAYOUT, get(layouts::get_layout).put(layouts::put_layout))
        .route(routes::LAYOUT_ACTIVE, put(layouts::put_active_layout))
        .route(routes::DISPLAYS, get(displays::list_displays))
        .route(
            routes::DISPLAY,
            put(displays::put_display).delete(displays::delete_display),
        )
        .route(routes::DISPLAY_VALUE, put(displays::put_display_value))
        .route(routes::PERIPHERALS, get(peripherals::list_peripherals))
        .route(
            routes::PERIPHERAL,
            get(peripherals::get_peripheral).patch(peripherals::patch_peripheral),
        )
        .route(routes::PERIPHERAL_ACCESS, put(peripherals::put_peripheral_access))
        .route(routes::FANS, get(fans::get_fans))
        .route(routes::FAN_HEADER, patch(fans::patch_header))
        .route(routes::FAN_PROFILE, get(fans::get_profile).patch(fans::patch_profile))
        .route(routes::FAN_PROFILES, put(fans::put_profile).delete(fans::delete_profile))
        .route(routes::SETTINGS, get(settings::get_settings).put(settings::put_settings))
        .route(routes::DISMISS_ZONE_PANEL, post(settings::dismiss_zone_panel))
        .route(routes::ONBOARDING_COMPLETE, post(settings::complete_onboarding))
        .route(routes::CONFIG_EXPORT, get(settings::config_export))
        .route(routes::CONFIG_IMPORT, post(settings::config_import))
        .route(routes::QUIT, post(status::quit))
        .route(routes::DIAG_READBACK, get(diag::readback))
        .route("/api/v1/diag/fail-sensor", post(diag::fail_sensor))
        .route(routes::PREVIEW, get(preview::preview));

    api.layer(axum::middleware::from_fn_with_state(
        app.clone(),
        crate::auth::guard,
    ))
    .with_state(app)
}

/// An API error with its HTTP status.
#[derive(Debug)]
pub struct ApiFailure {
    pub status: axum::http::StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiFailure {
    pub fn new(status: axum::http::StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(
            axum::http::StatusCode::NOT_FOUND,
            canvas_proto::api::error_codes::NOT_FOUND,
            message,
        )
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            canvas_proto::api::error_codes::VALIDATION,
            message,
        )
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(
            axum::http::StatusCode::CONFLICT,
            canvas_proto::api::error_codes::CONFLICT,
            message,
        )
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            canvas_proto::api::error_codes::SERVICE_UNAVAILABLE,
            message,
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            canvas_proto::api::error_codes::INTERNAL,
            message,
        )
    }
}

impl axum::response::IntoResponse for ApiFailure {
    fn into_response(self) -> axum::response::Response {
        let body = canvas_proto::api::ApiError {
            error: self.code.to_string(),
            message: self.message,
        };
        (self.status, axum::Json(body)).into_response()
    }
}

/// The result type every handler returns.
pub type ApiResult<T> = Result<T, ApiFailure>;
