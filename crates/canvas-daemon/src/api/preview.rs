//! `GET /api/v1/preview`: the editor's WebSocket.
//!
//! Binary messages: `u16 LE width, u16 LE height, width*height*3 RGB bytes`, at most
//! `PREVIEW_MAX_FPS`. The WebSocket takes the token as `?token=` because a browser
//! cannot set headers on a WebSocket handshake.
//!
//! Without a query the socket streams the running effect from the main engine. With
//! `?stack=1` it streams the stack preview engine instead (D95.4.4): a second engine with no
//! device sink, so what the builder shows never reaches a light.
//!
//! Each preview sink is registered only while at least one subscriber is connected, so the
//! engine parks when the editor window is closed.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use canvas_proto::consts::PREVIEW_MAX_FPS;
use std::sync::Arc;

use crate::state::{App, WebsocketPreview};

/// The query a preview socket carries: the token (a browser cannot set a header on the
/// handshake) and `stack=1` for the preview engine.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PreviewQuery {
    #[serde(default)]
    pub token: Option<String>,
    /// Any of `1` or `true` streams the stack preview engine.
    #[serde(default)]
    pub stack: Option<String>,
}

/// `GET /api/v1/preview`.
pub async fn preview(
    State(app): State<Arc<App>>,
    Query(query): Query<PreviewQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Some(presented) = query.token.as_deref() else {
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
    let stack = query
        .stack
        .as_deref()
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    // The stack preview's engine is made now when the builder opens its socket first.
    let engine = if stack {
        Some(app.stack_preview_engine().await)
    } else {
        None
    };
    upgrade.on_upgrade(move |socket| run(socket, app, engine))
}

/// The preview session.
async fn run(mut socket: WebSocket, app: Arc<App>, stack_engine: Option<Arc<dyn canvas_proto::traits::Engine>>) {
    let (engine, subscribers): (Arc<dyn canvas_proto::traits::Engine>, &crate::state::PreviewRegistry) =
        match &stack_engine {
            Some(engine) => (Arc::clone(engine), &app.stack_subs),
            None => (Arc::clone(&app.engine), &app.preview_subs),
        };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(u32, u32, Vec<u8>)>(4);
    let sink = Arc::new(WebsocketPreview::new(tx));
    engine.set_preview(Some(sink));
    subscribers.subscribe();
    tracing::info!(stack = stack_engine.is_some(), "the editor preview connected");

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

    subscribers.unsubscribe();
    if subscribers.subscribers() == 0 {
        // The last subscriber left: the engine parks again.
        engine.set_preview(None);
    }
    tracing::info!(stack = stack_engine.is_some(), "the editor preview disconnected");
}
