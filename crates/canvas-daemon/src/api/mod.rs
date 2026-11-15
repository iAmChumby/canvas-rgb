//! The loopback HTTP API on 127.0.0.1:9430.
//!
//! Every route is under `/api/v1`. The G9 guards (bearer token, Origin, Host) run as
//! middleware on the whole router.

pub mod audio;
pub mod diag;
pub mod devices;
pub mod displays;
pub mod editor;
pub mod effects;
pub mod fans;
pub mod lights;
pub mod output;
pub mod peripherals;
pub mod preview;
pub mod scenes;
pub mod settings;
pub mod stack_preview;
pub mod stacks;
pub mod status;
pub mod update;

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
        .route(routes::DEVICES, get(devices::list_devices))
        .route(
            routes::DEVICE_ZONE,
            patch(devices::patch_zone),
        )
        .route(routes::DEVICE_IDENTIFY, post(devices::identify))
        .route(routes::WHITE_BALANCE, get(devices::get_white_balance))
        .route(
            routes::DEVICE_WHITE_BALANCE,
            put(devices::put_white_balance).delete(devices::delete_white_balance),
        )
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
        .route(routes::EFFECT_PRESETS, post(effects::create_preset))
        .route(
            routes::EFFECT_PRESET,
            put(effects::replace_preset).delete(effects::delete_preset),
        )
        .route(routes::EFFECT_ACTIVATE, post(effects::activate))
        .route(routes::EFFECT_DEFAULTS, patch(effects::patch_defaults))
        .route(routes::EFFECT_THUMBNAIL, get(effects::thumbnail))
        .route(routes::EFFECT_FAVORITE, put(effects::put_favorite))
        .route(routes::EFFECT_SOURCE, get(editor::effect_source))
        .route(
            routes::EFFECT_PREVIEW,
            post(editor::preview_effect)
                .layer(axum::extract::DefaultBodyLimit::max(effects::IMPORT_BODY_LIMIT)),
        )
        .route(
            routes::EFFECT_DRAFT,
            get(editor::get_draft)
                .put(editor::put_draft)
                .delete(editor::delete_draft)
                .layer(axum::extract::DefaultBodyLimit::max(effects::IMPORT_BODY_LIMIT)),
        )
        .route(routes::SCENES, get(scenes::list_scenes).post(scenes::create_scene))
        .route(routes::SCENE_ACTIVE, put(scenes::put_active))
        .route(routes::SCENE, put(scenes::put_scene).delete(scenes::delete_scene))
        .route(routes::GROUP_PROPS, patch(scenes::patch_group_props))
        .route(routes::GROUP_BRIGHTNESS, put(scenes::put_group_brightness))
        .route(routes::LIGHTS_FLASH, post(scenes::flash_lights))
        .route(routes::LIGHTS, get(lights::list_lights))
        .route(routes::LIGHT, patch(lights::patch_light))
        .route(routes::STACKS, get(stacks::list_stacks))
        .route(
            routes::STACK_PREVIEW,
            put(stack_preview::put_stack_preview)
                .delete(stack_preview::delete_stack_preview)
                .layer(axum::extract::DefaultBodyLimit::max(effects::IMPORT_BODY_LIMIT)),
        )
        .route(
            routes::STACK,
            get(stacks::get_stack)
                .put(stacks::put_stack)
                .delete(stacks::delete_stack)
                .layer(axum::extract::DefaultBodyLimit::max(effects::IMPORT_BODY_LIMIT)),
        )
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
        .route(routes::PERIPHERAL_READ, get(peripherals::get_peripheral_read))
        .route(routes::FANS, get(fans::get_fans))
        .route(routes::FAN_HOT, put(fans::put_fan_hot))
        .route(routes::FAN_HEADER, patch(fans::patch_header))
        .route(routes::FAN_HEADER_AVAILABILITY, put(fans::put_header_availability))
        .route(routes::FAN_PROFILE, get(fans::get_profile).patch(fans::patch_profile))
        .route(routes::FAN_PROFILES, put(fans::put_profile).delete(fans::delete_profile))
        .route(routes::SETTINGS, get(settings::get_settings).put(settings::put_settings))
        .route(routes::AUDIO_SOURCES, get(audio::get_sources))
        .route(routes::AUDIO_SOURCE, put(audio::put_source))
        .route(routes::AUDIO_TEST, put(audio::put_test).get(audio::get_test).delete(audio::delete_test))
        .route(routes::DISMISS_ZONE_PANEL, post(settings::dismiss_zone_panel))
        .route(routes::ONBOARDING_COMPLETE, post(settings::complete_onboarding))
        .route(routes::ONBOARDING_SESSION, post(devices::onboarding_session))
        .route(routes::CONFIG_EXPORT, get(settings::config_export))
        .route(routes::CONFIG_IMPORT, post(settings::config_import))
        .route(routes::QUIT, post(status::quit))
        .route(routes::UPDATE_INSTALL, post(update::install))
        .route(routes::DIAG_READBACK, get(diag::readback))
        .route("/api/v1/diag/fail-sensor", post(diag::fail_sensor))
        .route(routes::PREVIEW, get(preview::preview))
        .route(routes::PREVIEW_LIGHTS, get(preview::preview_lights));

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
    /// A `Retry-After` header, in seconds, when the answer says to try again later.
    pub retry_after: Option<u64>,
    /// The line in a draft's script, when the failure is the runtime refusing it.
    pub line: Option<u32>,
    /// The column, when the runtime gave one.
    pub column: Option<u32>,
}

impl ApiFailure {
    pub fn new(status: axum::http::StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), retry_after: None, line: None, column: None }
    }

    /// Adds `Retry-After: <seconds>` to the answer.
    pub fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after = Some(seconds);
        self
    }

    /// Adds the runtime's position to the answer, for the editor's error gutter.
    pub fn at(mut self, line: Option<u32>, column: Option<u32>) -> Self {
        self.line = line;
        self.column = column;
        self
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
            line: self.line,
            column: self.column,
        };
        let mut response = (self.status, axum::Json(body)).into_response();
        if let Some(seconds) = self.retry_after {
            if let Ok(value) = axum::http::HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert(axum::http::header::RETRY_AFTER, value);
            }
        }
        response
    }
}

/// The result type every handler returns.
pub type ApiResult<T> = Result<T, ApiFailure>;

/// Reads the boolean query parameter `key` from the raw query: `true` or `1` is on; `false`, `0` or
/// no `key` is off. Any other value, or `key` given twice, is a validation error in the API's JSON
/// shape (a typed extractor would answer with axum's plain-text 400). One rule for every flag
/// (`?fresh=`, `?rescan=`), so a typo is an error and never a silent plain read.
pub fn parse_flag(query: Option<&str>, key: &str) -> Result<bool, ApiFailure> {
    let mut seen: Option<&str> = None;
    for pair in query.unwrap_or("").split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name != key {
            continue;
        }
        if seen.is_some() {
            return Err(ApiFailure::validation(format!("{key} is given more than once")));
        }
        seen = Some(value);
    }
    match seen {
        None | Some("false") | Some("0") => Ok(false),
        Some("true") | Some("1") => Ok(true),
        Some(other) => Err(ApiFailure::validation(format!("{key} is {other}; use true, false, 1 or 0"))),
    }
}

/// A request body, within `limit` bytes, or the API's own error shape.
///
/// The body is read as bytes so an oversized one is answered with [`ApiFailure`] and not with
/// axum's plain-text rejection. The import, the preview and the draft all use this, so the
/// three carry the same cap and say the same thing when a body is over it.
pub fn limited_body(
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
    limit: usize,
) -> ApiResult<axum::body::Bytes> {
    body.map_err(|rejection| match rejection {
        axum::extract::rejection::BytesRejection::FailedToBufferBody(
            axum::extract::rejection::FailedToBufferBody::LengthLimitError(_),
        ) => ApiFailure::new(
            axum::http::StatusCode::PAYLOAD_TOO_LARGE,
            canvas_proto::api::error_codes::VALIDATION,
            format!("the request body is over the limit of {limit} bytes"),
        ),
        other => ApiFailure::validation(format!("the request body could not be read: {other}")),
    })
}
