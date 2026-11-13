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
//! engine parks when the editor window is closed. A preview engine has one sink, so only one
//! `?stack=1` socket streams at a time: a second one is closed with code 1013 and a reason the
//! page can show, rather than taking the frames of the socket already open.

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
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
    let count = subscribers.subscribe();
    let _session = Session { engine: &engine, subscribers };
    // The preview engine has one sink. A second `?stack=1` socket would take the frames of the
    // socket already streaming, so it is refused with a close the page can show, and the first
    // one keeps its sink. (The plain socket has the same limit; it is left as it was so an
    // editor window that reconnects before its old socket closes still works.)
    if stack_engine.is_some() && count > 1 {
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: 1013,
                reason: "another stack preview is open; close it first".into(),
            })))
            .await;
        tracing::info!("a second stack preview socket was refused");
        return;
    }
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

// ---------------------------------------------------------------- the per-light feed

/// The query the lights socket carries: the token, as for [`preview`].
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct LightsQuery {
    #[serde(default)]
    pub token: Option<String>,
}

/// `GET /api/v1/preview/lights`.
///
/// The first message is JSON mapping a `u16` index to a light id; it is sent again whenever the
/// scene or the lights change. Then binary messages: per light a `u16 LE` index, a `u16 LE` LED
/// count and that many RGB triples, at most 15 a second (the engine's rate).
pub async fn preview_lights(
    State(app): State<Arc<App>>,
    Query(query): Query<LightsQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Some(presented) = query.token.as_deref() else {
        return crate::api::ApiFailure::new(
            axum::http::StatusCode::UNAUTHORIZED,
            canvas_proto::api::error_codes::UNAUTHORIZED,
            "the light preview needs a token",
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
    upgrade.on_upgrade(move |socket| run_lights(socket, app))
}

/// The light ids by index, in the order of `lights.json`.
async fn light_index(app: &App) -> Vec<String> {
    app.store.read().await.lights.lights.iter().map(|l| l.id.clone()).collect()
}

/// The index message: `{"0":"<light id>","1":...}`.
pub fn index_message(ids: &[String]) -> String {
    let map: std::collections::BTreeMap<String, &String> =
        ids.iter().enumerate().map(|(i, id)| (i.to_string(), id)).collect();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
}

/// One binary message for `frame`. A light the index does not hold is left out; the second value
/// says one was, so the index is sent again first.
pub fn encode_light_frame(
    frame: &canvas_proto::traits::LightFrame,
    ids: &[String],
) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    let mut missing = false;
    for (id, colors) in &frame.lights {
        let Some(index) = ids.iter().position(|known| known == id) else {
            missing = true;
            continue;
        };
        let count = colors.len().min(usize::from(u16::MAX));
        out.extend_from_slice(&(index as u16).to_le_bytes());
        out.extend_from_slice(&(count as u16).to_le_bytes());
        for color in &colors[..count] {
            out.extend_from_slice(&[color.r, color.g, color.b]);
        }
    }
    (out, missing)
}

async fn run_lights(mut socket: WebSocket, app: Arc<App>) {
    let mut frames = app.scene.feed.join(app.engine.as_ref());
    let mut changed = app.scene.subscribe();
    let mut ids = light_index(&app).await;
    tracing::info!("the light preview connected");
    let mut resend = true;
    loop {
        if resend {
            resend = false;
            if socket.send(Message::Text(index_message(&ids).into())).await.is_err() {
                break;
            }
        }
        tokio::select! {
            frame = frames.recv() => match frame {
                Ok(frame) => {
                    let (mut payload, missing) = encode_light_frame(&frame, &ids);
                    if missing {
                        // A light the index lacks: a new one. Say so before the frame that names it.
                        ids = light_index(&app).await;
                        if socket.send(Message::Text(index_message(&ids).into())).await.is_err() {
                            break;
                        }
                        payload = encode_light_frame(&frame, &ids).0;
                    }
                    if !payload.is_empty()
                        && socket.send(Message::Binary(payload.into())).await.is_err()
                    {
                        break;
                    }
                }
                // A slow socket skips ahead; the engine's next frame is the newest one.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            woke = changed.changed() => {
                if woke.is_err() {
                    break;
                }
                ids = light_index(&app).await;
                resend = true;
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
    }
    app.scene.feed.leave(app.engine.as_ref());
    tracing::info!("the light preview disconnected");
}

#[cfg(test)]
mod light_feed_tests {
    use super::*;
    use canvas_proto::traits::LightFrame;
    use canvas_proto::Rgb8;

    #[test]
    fn the_index_maps_numbers_to_light_ids() {
        let ids = vec!["a#z0".to_string(), "b#z0".to_string()];
        let json: serde_json::Value = serde_json::from_str(&index_message(&ids)).unwrap();
        assert_eq!(json, serde_json::json!({ "0": "a#z0", "1": "b#z0" }));
    }

    #[test]
    fn a_frame_is_index_count_then_rgb_per_light() {
        let ids = vec!["a#z0".to_string(), "b#z0".to_string()];
        let frame = LightFrame {
            lights: vec![
                ("b#z0".into(), vec![Rgb8::new(1, 2, 3), Rgb8::new(4, 5, 6)]),
                ("a#z0".into(), vec![Rgb8::new(9, 8, 7)]),
            ],
        };
        let (bytes, missing) = encode_light_frame(&frame, &ids);
        assert!(!missing);
        assert_eq!(bytes, vec![1, 0, 2, 0, 1, 2, 3, 4, 5, 6, 0, 0, 1, 0, 9, 8, 7]);
    }

    #[test]
    fn a_light_the_index_lacks_is_left_out_and_reported() {
        let frame = LightFrame { lights: vec![("new#z0".into(), vec![Rgb8::WHITE])] };
        let (bytes, missing) = encode_light_frame(&frame, &["a#z0".to_string()]);
        assert!(bytes.is_empty() && missing);
    }
}
