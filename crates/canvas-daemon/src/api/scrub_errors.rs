//! The one choke point for error text that leaves the daemon (R2).
//!
//! An error is built in many places — an `ApiFailure`, the auth guard, and axum's plain-text
//! extractor rejections — and none of them has the `App` that owns the scrubber. This layer
//! holds `Arc<App>` and rewrites every error response body at the edge: a JSON `ApiError` has
//! its `message` and `hint` scrubbed, and a plain-text rejection has its whole body scrubbed.
//! Nothing touches a successful response, so a preview frame or a config file is never copied.

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use canvas_proto::api::ApiError;
use std::sync::Arc;

use crate::state::App;

/// The largest error body we will rewrite. A rejection is a sentence, never a file.
const MAX_ERROR_BODY: usize = 64 * 1024;

pub async fn scrub_errors(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    if !response.status().is_client_error() && !response.status().is_server_error() {
        return response;
    }
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("json"));
    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_ERROR_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };
    let scrubber = app.scrubber.lock().unwrap_or_else(|e| e.into_inner());
    let rewritten = if is_json {
        // Parse as a `Value`, not as `ApiError`: some conflict bodies carry extra fields (the
        // current scene, the write order) that the editor reads. Scrub only the two text fields
        // and keep every other field exactly as it was.
        match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(mut value) => {
                if let Some(object) = value.as_object_mut() {
                    for key in ["message", "hint", "problem"] {
                        if let Some(text) = object.get(key).and_then(|v| v.as_str()) {
                            let scrubbed = scrubber.scrub(text);
                            object.insert(key.to_string(), serde_json::Value::String(scrubbed));
                        }
                    }
                }
                serde_json::to_vec(&value).unwrap_or_else(|_| bytes.to_vec())
            }
            Err(_) => scrub_plain(&bytes, &scrubber),
        }
    } else {
        scrub_plain(&bytes, &scrubber)
    };
    Response::from_parts(parts, Body::from(rewritten))
}

/// A plain-text body (an extractor rejection) is one sentence: scrub it and keep its bytes.
fn scrub_plain(bytes: &Bytes, scrubber: &crate::scrub::Scrubber) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    scrubber.scrub(&text).into_bytes()
}

/// The next step for each error code. `message` says what happened; `hint` says what to do.
/// Both are scrubbed at the edge, so a hint that names a path is safe too.
pub fn hint_for(code: &str) -> &'static str {
    match code {
        "unauthorized" => "Check that the tray app is running, then try again.",
        "forbidden_origin" | "bad_host" => "Open the editor from the tray app, then try again.",
        "not_found" => "The item may have been removed. Refresh the list and try again.",
        "validation" => "Change the value shown and try again.",
        "service_unavailable" => "The fan service is not answering. Check it in the tray, then try again.",
        "conflict" => "The item is in use. Close the other editor and try again.",
        "gone" => "Refresh the list and try again.",
        "internal" => "Save diagnostics in Help & About, then report the problem.",
        _ => "Try again. If it keeps happening, save diagnostics and report it.",
    }
}

/// A JSON body for a status, in the same shape as every other API error. Used by the guard.
pub fn error_body(status: StatusCode, code: &str, message: &str) -> Response {
    use axum::response::IntoResponse;
    let body = ApiError {
        error: code.to_string(),
        message: message.to_string(),
        line: None,
        column: None,
        hint: Some(hint_for(code).to_string()),
    };
    (status, axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_code_has_a_next_step() {
        for code in [
            "unauthorized", "forbidden_origin", "bad_host", "not_found", "validation",
            "service_unavailable", "conflict", "gone", "internal", "other",
        ] {
            assert!(!hint_for(code).is_empty(), "{code}");
        }
    }

    #[test]
    fn a_plain_rejection_is_scrubbed() {
        let scrubber = crate::scrub::Scrubber::new(
            Some(concat!("C:\\", "Use", "rs\\Alice").into()),
            Some("Alice".into()),
            None,
        );
        let bytes = Bytes::from(concat!("failed to read C:\\", "Use", "rs\\Alice\\x.json").to_string());
        let out = String::from_utf8(scrub_plain(&bytes, &scrubber)).unwrap();
        assert!(!out.to_ascii_lowercase().contains("alice"), "{out}");
        assert!(out.contains("<profile>"), "{out}");
    }
}
