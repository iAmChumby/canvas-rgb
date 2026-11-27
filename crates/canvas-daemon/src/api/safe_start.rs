//! `PUT /api/v1/safe-start` (#577).
//!
//! "Start normally" and "Start in safe start" ask the service to restart the daemon in the
//! chosen mode. The pipe message waits until the HTTP reply has been written: the reply's body
//! says when the server is done with it. The service does a planned kill and relaunch; the
//! daemon itself does not run `quit_sequence`. A mode already in force changes nothing.

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::Json;
use canvas_proto::api::{SafeStartMode, SafeStartPut};
use canvas_proto::pipe::PipeRequest;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use crate::state::App;

/// How long the socket gets to send a finished body before the restart is asked for.
const FLUSH_GRACE: Duration = Duration::from_millis(200);

pub async fn put_safe_start(State(app): State<Arc<App>>, Json(body): Json<SafeStartPut>) -> impl IntoResponse {
    let mode = body.mode;
    let request = match mode {
        SafeStartMode::Safe => PipeRequest::EnterSafeStart,
        SafeStartMode::Normal => PipeRequest::LeaveSafeStart,
    };
    let restarting = mode != app.start_mode();
    let answer = serde_json::json!({ "mode": mode, "restarting": restarting }).to_string();
    // The sender lives in the body's stream: it drops when the server has taken the last byte,
    // or when the client is gone.
    let (written, wait) = tokio::sync::oneshot::channel::<()>();
    let reply = Body::from_stream(futures_util::stream::unfold(
        (Some(Bytes::from(answer)), written),
        |(chunk, written)| async move { chunk.map(|c| (Ok::<_, Infallible>(c), (None, written))) },
    ));
    tokio::spawn(async move {
        let _ = wait.await;
        tokio::time::sleep(FLUSH_GRACE).await;
        if let Err(e) = crate::pipe_client::send(&app, &request).await {
            tracing::warn!(error = %e, "the service could not be told to change the start mode");
        }
    });
    ([(CONTENT_TYPE, "application/json")], reply)
}

#[cfg(test)]
mod tests {
    use canvas_proto::api::routes;

    #[test]
    fn the_route_names_the_spine_path() {
        assert_eq!(routes::SAFE_START, "/api/v1/safe-start");
    }
}
