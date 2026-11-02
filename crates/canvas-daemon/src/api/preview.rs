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

/// The socket's place in the preview registry and the engine's sink, for as long as the session
/// lasts. Both are state tokens taken when the socket opens and given back at the end of
/// [`run`]; a session whose task is dropped early would otherwise leave the engine rendering for
/// nobody, so the drop is what releases them.
struct Session<'a> {
    engine: &'a Arc<dyn canvas_proto::traits::Engine>,
    subscribers: &'a crate::state::PreviewRegistry,
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        self.subscribers.unsubscribe();
        if self.subscribers.subscribers() == 0 {
            // The last subscriber left: the engine parks again.
            self.engine.set_preview(None);
        }
    }
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
    subscribers.subscribe();
    let _session = Session { engine: &engine, subscribers };
    engine.set_preview(Some(sink));
    tracing::info!(stack = stack_engine.is_some(), "the editor preview connected");

    // The engine's own rate is the cap: it submits at `PREVIEW_MAX_FPS` at most, which is the
    // same figure the socket promises, so the socket forwards every frame it is handed.
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
        }
    }

    // `_session` gives the registry place and the engine's sink back here: the loop's breaks,
    // an error, and a dropped task all release them the same way.
    tracing::info!(stack = stack_engine.is_some(), "the editor preview disconnected");
}
