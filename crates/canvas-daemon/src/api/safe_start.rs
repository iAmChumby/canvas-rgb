//! `PUT /api/v1/safe-start` (#577).
//!
//! "Start normally" and "Start in safe start" ask the service to restart the daemon in the
//! chosen mode. The HTTP answer is built and returned first: the pipe message is sent on a task
//! that yields once, so a slow pipe never holds the reply. The service does a planned kill and
//! relaunch; the daemon itself does not run `quit_sequence`.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use canvas_proto::api::routes;
use canvas_proto::pipe::PipeRequest;
use std::sync::Arc;

use crate::state::App;

pub async fn put_safe_start(
    State(app): State<Arc<App>>,
    Json(body): Json<canvas_proto::api::SafeStartPut>,
) -> impl IntoResponse {
    let mode = body.mode;
    let request = match mode {
        canvas_proto::api::SafeStartMode::Safe => PipeRequest::EnterSafeStart,
        canvas_proto::api::SafeStartMode::Normal => PipeRequest::LeaveSafeStart,
    };
    // The answer exists before the pipe task starts. Axum writes it as soon as this handler
    // returns; the task yields once first, so it cannot hold up the response.
    let answer = Json(serde_json::json!({ "mode": mode, "restarting": true }));
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        if let Err(e) = crate::pipe_client::send(&app, &request).await {
            tracing::warn!(error = %e, "the service could not be told to change the start mode");
        }
    });
    let _ = routes::SAFE_START;
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_route_names_the_spine_path() {
        assert_eq!(routes::SAFE_START, "/api/v1/safe-start");
    }
}
