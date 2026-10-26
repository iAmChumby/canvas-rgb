//! G9: the API guards.
//!
//! - Every request needs `Authorization: Bearer <token>`, except `GET /api/v1/status`.
//! - A request with an `Origin` header that is not one of the Tauri origins gets 403
//!   `forbidden_origin`. The allowed origins also get CORS headers, and `OPTIONS`
//!   preflights are answered for them only.
//! - A `Host` header that is not in `consts::API_ALLOWED_HOSTS` gets 403 `bad_host`.
//!   This is the DNS-rebinding guard.
//!
//! The token is compared in constant time.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use canvas_proto::api::{error_codes, routes, ApiError};
use canvas_proto::consts;
use std::sync::Arc;

use crate::state::App;

/// The error body for a rejected request.
fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    let body = ApiError { error: code.to_string(), message: message.to_string() };
    let mut response = (status, axum::Json(body)).into_response();
    // A 403 must never be cached by anything.
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// True when the request's `Origin` is one the editor may use.
fn origin_allowed(origin: &str) -> bool {
    consts::API_ALLOWED_ORIGINS.iter().any(|allowed| allowed.eq_ignore_ascii_case(origin))
}

/// True when the request's `Host` is one of ours.
fn host_allowed(host: &str) -> bool {
    consts::API_ALLOWED_HOSTS.iter().any(|allowed| allowed.eq_ignore_ascii_case(host))
}

/// The guard middleware. Runs on every request.
pub async fn guard(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();

    // Host check first: a rebinding attack gets no further than this.
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !host_allowed(host) {
        return error_response(
            StatusCode::FORBIDDEN,
            error_codes::BAD_HOST,
            &format!("host {host:?} is not an allowed API host"),
        );
    }

    // Origin check: browsers send `Origin`; the CLI and the Tauri shell do not.
    let origin = request.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok()).map(str::to_string);
    let origin_allowed_here = origin.as_deref().is_some_and(origin_allowed);
    if let Some(origin) = &origin {
        if !origin_allowed_here {
            return error_response(
                StatusCode::FORBIDDEN,
                error_codes::FORBIDDEN_ORIGIN,
                &format!("origin {origin:?} is not allowed"),
            );
        }
    }

    // Preflight for the allowed origins only.
    if method == Method::OPTIONS {
        if !origin_allowed_here {
            return error_response(
                StatusCode::FORBIDDEN,
                error_codes::FORBIDDEN_ORIGIN,
                "preflight from an unknown origin is not allowed",
            );
        }
        return preflight_response(origin.as_deref());
    }

    // The status endpoint is open: the tray shows it without a token.
    let is_status = method == Method::GET && uri.path() == routes::STATUS;
    if !is_status {
        let presented = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(crate::token::bearer);
        match presented {
            None => {
                return error_response(
                    StatusCode::UNAUTHORIZED,
                    error_codes::UNAUTHORIZED,
                    "the API needs an Authorization: Bearer token",
                );
            }
            Some(presented) => {
                if !crate::token::constant_time_eq(presented, &app.token) {
                    return error_response(
                        StatusCode::UNAUTHORIZED,
                        error_codes::UNAUTHORIZED,
                        "the API token is not valid",
                    );
                }
            }
        }
    }

    let mut response = next.run(request).await;
    if origin_allowed_here {
        let origin = origin.unwrap_or_default();
        if let Ok(value) = HeaderValue::from_str(&origin) {
            response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
            response.headers_mut().insert(header::VARY, HeaderValue::from_static("Origin"));
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("authorization, content-type"),
            );
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
            );
        }
    }
    response
}

/// The preflight answer for an allowed origin.
fn preflight_response(origin: Option<&str>) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Some(origin) = origin {
        if let Ok(value) = HeaderValue::from_str(origin) {
            response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
            response.headers_mut().insert(header::VARY, HeaderValue::from_static("Origin"));
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("authorization, content-type"),
            );
            response.headers_mut().insert(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
            );
            response.headers_mut().insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
        }
    }
    response
}

/// The token the preview WebSocket takes as `?token=`.
pub fn preview_token(query: Option<&str>) -> Option<&str> {
    let query = query?;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == consts::PREVIEW_TOKEN_QUERY {
            return Some(value);
        }
    }
    None
}

/// A body that is only ever used for an error.
pub type EmptyBody = Body;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_check_follows_the_spine_list() {
        assert!(origin_allowed("http://tauri.localhost"));
        assert!(origin_allowed("tauri://localhost"));
        assert!(!origin_allowed("https://evil.example"));
        assert!(!origin_allowed(""));
    }

    #[test]
    fn host_check_follows_the_spine_list() {
        assert!(host_allowed("127.0.0.1:9430"));
        assert!(host_allowed("localhost:9430"));
        assert!(!host_allowed("evil.example"));
        assert!(!host_allowed("127.0.0.1:9999"));
    }

    #[test]
    fn preview_token_is_read_from_the_query() {
        assert_eq!(preview_token(Some("token=abc")), Some("abc"));
        assert_eq!(preview_token(Some("x=1&token=abc&y=2")), Some("abc"));
        assert_eq!(preview_token(Some("x=1")), None);
        assert_eq!(preview_token(None), None);
    }
}
