//! The update route (#43): the tray and the CLI hand a downloaded installer to the service
//! through the daemon, because the pipe admits only the daemon the service launched.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use canvas_proto::api::{error_codes, InstallUpdateRequest, InstallUpdateResponse};
use canvas_proto::pipe::{PipeRequest, PipeResponse};
use std::sync::Arc;

use crate::api::{ApiFailure, ApiResult};
use crate::state::App;

/// The longest `.sig` text the route accepts. A real minisign signature is a few hundred bytes.
pub const MAX_SIGNATURE_BYTES: usize = 4096;

/// Checks the body before the service sees it: an absolute installer path and a signature
/// that is present and of sane size. The service re-checks everything that matters.
pub fn validate(request: &InstallUpdateRequest) -> Result<(), String> {
    if request.installer.trim().is_empty() || !std::path::Path::new(&request.installer).is_absolute() {
        return Err(format!("the installer path must be absolute: {}", request.installer));
    }
    if request.signature.trim().is_empty() {
        return Err("the signature is empty".to_string());
    }
    if request.signature.len() > MAX_SIGNATURE_BYTES {
        return Err(format!("the signature is longer than {MAX_SIGNATURE_BYTES} bytes"));
    }
    Ok(())
}

/// `POST /api/v1/update/install`.
///
/// 202 once the service runs setup; 422 for a refused file, signature or version; 409 while an
/// install runs; 503 when the service is down.
pub async fn install(
    State(app): State<Arc<App>>,
    Json(request): Json<InstallUpdateRequest>,
) -> ApiResult<(StatusCode, Json<InstallUpdateResponse>)> {
    validate(&request).map_err(ApiFailure::validation)?;

    let answer = crate::pipe_client::send(
        &app,
        &PipeRequest::InstallUpdate {
            installer: request.installer.clone(),
            signature: request.signature.clone(),
        },
    )
    .await
    .map_err(|e| ApiFailure::service_unavailable(format!("the service is not reachable: {e}")))?;

    match answer {
        PipeResponse::UpdateStarted { version } => {
            tracing::info!(%version, installer = %request.installer, "the service started the update");
            Ok((StatusCode::ACCEPTED, Json(InstallUpdateResponse { version })))
        }
        PipeResponse::Error { code, message } => {
            tracing::warn!(%code, %message, installer = %request.installer, "the service refused the update");
            Err(match code.as_str() {
                error_codes::VALIDATION => ApiFailure::validation(message),
                error_codes::CONFLICT => ApiFailure::conflict(message),
                error_codes::SERVICE_UNAVAILABLE => ApiFailure::service_unavailable(message),
                _ => ApiFailure::internal(message),
            })
        }
        _ => Err(ApiFailure::internal("the service gave an unexpected answer")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(installer: &str, signature: &str) -> InstallUpdateRequest {
        InstallUpdateRequest { installer: installer.into(), signature: signature.into() }
    }

    #[test]
    fn an_absolute_path_and_a_signature_pass() {
        assert!(validate(&req(r"C:\Users\x\setup.exe", "c2ln")).is_ok());
    }

    #[test]
    fn a_relative_path_is_refused() {
        assert!(validate(&req(r"setup.exe", "c2ln")).is_err());
        assert!(validate(&req("", "c2ln")).is_err());
    }

    #[test]
    fn an_empty_or_huge_signature_is_refused() {
        assert!(validate(&req(r"C:\setup.exe", "  \n")).is_err());
        assert!(validate(&req(r"C:\setup.exe", &"a".repeat(MAX_SIGNATURE_BYTES + 1))).is_err());
        assert!(validate(&req(r"C:\setup.exe", &"a".repeat(MAX_SIGNATURE_BYTES))).is_ok());
    }
}
