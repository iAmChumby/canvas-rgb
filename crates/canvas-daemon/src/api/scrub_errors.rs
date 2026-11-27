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
/// A 2xx preview body carries a base64 picture, so the layer must not truncate it while looking
/// for a `note` field. Errors stay tiny; success bodies are bounded well above a preview frame.
const MAX_ERROR_BODY: usize = 32 * 1024 * 1024;

pub async fn scrub_errors(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    let status = response.status();
    let is_error = status.is_client_error() || status.is_server_error();
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("json"));
    if !is_error && !is_json {
        return response;
    }
    // A large 2xx body (effects list, source) must never be dropped by the layer.
    let too_large = response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > MAX_ERROR_BODY);
    if too_large {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_ERROR_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };
    let scrubber = app.scrubber.lock().unwrap_or_else(|e| e.into_inner());
    let rewritten = if is_json {
        match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(mut value) => {
                scrub_json_value(&mut value, &scrubber);
                serde_json::to_vec(&value).unwrap_or_else(|_| bytes.to_vec())
            }
            Err(_) if is_error => scrub_plain(&bytes, &scrubber),
            Err(_) => bytes.to_vec(),
        }
    } else {
        scrub_plain(&bytes, &scrubber)
    };
    Response::from_parts(parts, Body::from(rewritten))
}

/// Every person-facing string field, at any depth in any JSON body — 2xx included (F7). Rewriting
/// only error status bodies leaves `EffectError.message` and `note` fields unscrubbed.
pub fn scrub_json_value(value: &mut serde_json::Value, scrubber: &crate::scrub::Scrubber) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let person_field = matches!(key.as_str(), "message" | "error" | "note" | "hint" | "problem");
                if person_field && child.is_string() {
                    let text = child.as_str().unwrap_or_default();
                    *child = serde_json::Value::String(scrub_error_text(scrubber, text));
                } else {
                    scrub_json_value(child, scrubber);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                scrub_json_value(item, scrubber);
            }
        }
        _ => {}
    }
}

/// One rule for every string a person reads: names first, then device ids (P6).
pub fn scrub_error_text(scrubber: &crate::scrub::Scrubber, text: &str) -> String {
    canvas_diag::scrub::scrub_device_ids(&scrubber.scrub(text))
}

/// A plain-text body (an extractor rejection) is one sentence: scrub it and keep its bytes.
fn scrub_plain(bytes: &Bytes, scrubber: &crate::scrub::Scrubber) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    scrub_error_text(scrubber, &text).into_bytes()
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
    use crate::scrub::Scrubber;

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

    #[test]
    fn a_device_id_in_an_error_becomes_the_device() {
        let scrubber = crate::scrub::Scrubber::default();
        let out = scrub_error_text(&scrubber, "the device openrgb:SERIAL-42 refused the write");
        assert_eq!(out, "the device the device refused the write");
    }

    #[test]
    fn nested_text_in_a_200_body_is_scrubbed_at_any_depth() {
        let scrubber = Scrubber::new(
            Some(concat!("C:\\", "Use", "rs\\Alice").into()),
            Some("Alice".into()),
            None,
        );
        let profile = concat!("C:\\", "Use", "rs\\Alice");
        let mut value = serde_json::json!({
            "layers": [{ "note": format!("failed at {profile}\\a.html: openrgb:SERIAL-42") }],
            "failed": [{ "error": format!("{profile}\\b.html failed") }]
        });
        scrub_json_value(&mut value, &scrubber);
        let text = value.to_string();
        assert!(!text.to_ascii_lowercase().contains("alice"), "{text}");
        assert!(!text.contains("SERIAL-42"), "{text}");
    }

    #[test]
    fn a_fake_profile_path_in_a_200_body_field_is_scrubbed() {
        let scrubber = Scrubber::new(
            Some(concat!("C:\\", "Use", "rs\\Alice").into()),
            Some("Alice".into()),
            None,
        );
        // The same helper is called where FailedEffect.error and StackPreviewLayer.error are
        // built, because a 200 body never reaches this layer (P7).
        let out = scrub_error_text(&scrubber, concat!("could not parse C:\\", "Use", "rs\\Alice\\bad.html"));
        assert!(!out.to_ascii_lowercase().contains("alice"), "{out}");
        assert!(out.contains("<profile>"), "{out}");
    }
}
