//! `GET /api/v1/preview`: the editor's WebSocket.
//!
//! Binary messages: `u16 LE width, u16 LE height, width*height*3 RGB bytes`, at most
//! `PREVIEW_MAX_FPS`. The WebSocket takes the token as `?token=` because a browser
//! cannot set headers on a WebSocket handshake.
//!
//! The preview sink is registered only while at least one subscriber is connected, so
//! the engine parks when the editor window is closed.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use canvas_proto::consts::PREVIEW_MAX_FPS;
use std::collections::HashMap;
use std::sync::Arc;

use crate::state::{App, WebsocketPreview};

/// The token the client passes in the query string.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PreviewQuery {
    #[serde(default)]
    pub token: Option<String>,
}

/// `GET /api/v1/preview`.
pub async fn preview(
    State(app): State<Arc<App>>,
    Query(query): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let presented = query
        .get(canvas_proto::consts::PREVIEW_TOKEN_QUERY)
        .map(String::as_str)
        .or(query.get("token").map(String::as_str));
    let Some(presented) = presented else {
        return crate::api::ApiFailure::new(
            axum::http::StatusCode::UNAUTHORIZED,
            canvas_proto::api::error_codes::UNAUTHORIZED,
            "the preview needs a token",
        )
        .into_response();
    };
    if !crate::token::constant_time_eq(presented, &app.token) {
        return crate::api::ApiFailure::new(
            axum::http::StatusCode::UNAUTHORIZED,
            canvas_proto::api::error_codes::UNAUTHORIZED,
            "the API token is not valid",
        )
        .into_response();
    }

    upgrade.on_upgrade(move |socket| run(socket, app))
}

use axum::response::IntoResponse;

/// The preview session.
async fn run(mut socket: WebSocket, app: Arc<App>) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(u32, u32, Vec<u8>)>(4);
    let sink = Arc::new(WebsocketPreview::new(tx));
    app.engine.set_preview(Some(sink));
    app.preview.subscribe();
    tracing::info!("the editor preview connected");

    let mut last = std::time::Instant::now();
    let interval = std::time::Duration::from_millis(1000 / PREVIEW_MAX_FPS.max(1) as u64);

    loop {
        tokio::select! {
            // Forward the frames to the editor.
            frame = rx.recv() => {
                let Some((width, height, rgb)) = frame else {
                    break;
                };
                let mut payload = bytes::BytesMut::with_capacity(4 + rgb.len());
                payload.extend_from_slice(&(width as u16).to_le_bytes());
                payload.extend_from_slice(&(height as u16).to_le_bytes());
                payload.extend_from_slice(&rgb);
                if socket.send(Message::Binary(payload.freeze())).await.is_err() {
                    break;
                }
            }
            // The client may close or ping.
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(_))) => {}
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            // The rate limit.
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(last + interval)) => {
                last = std::time::Instant::now();
            }
        }
    }

    app.preview.unsubscribe();
    if app.preview.subscribers() == 0 {
        // The last subscriber left: the engine parks again.
        app.engine.set_preview(None);
    }
    tracing::info!("the editor preview disconnected");
}
