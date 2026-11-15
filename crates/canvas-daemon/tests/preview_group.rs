//! `GET /api/v1/preview/group` (#378): the picture of one group, as a WebSocket.
//!
//! The engine is the recording fake: a test submits pictures through the sink the route gave it,
//! as the engine's render thread would, and reads them off the socket. No device, no port 9430:
//! the router is served over in-memory connections.

mod common;

use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::api::routes;
use canvas_proto::scene::{Group, GroupMode, Plays, Scene, ScenesFile, SCENES_SCHEMA};
use common::{FakeAdapter, FakeEngine};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct TempHome {
    path: std::path::PathBuf,
}

impl TempHome {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("canvas-rgb-group-preview-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp directory is created");
        Self { path }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct Rig {
    server: common::MemoryServer,
    app: Arc<App>,
    engine: Arc<FakeEngine>,
    _home: TempHome,
}

fn group(id: &str) -> Group {
    Group {
        id: id.into(),
        name: id.to_uppercase(),
        members: Vec::new(),
        plays: Plays::Off,
        mode: GroupMode::FlowAcross,
        brightness: 100,
    }
}

/// The daemon around the recording engine, with an active scene of two groups, `left` and
/// `right`.
async fn rig(name: &str) -> Rig {
    let home = TempHome::new(name);
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    app.store.write().await.scenes = ScenesFile {
        schema_version: SCENES_SCHEMA,
        active: "main".into(),
        scenes: vec![Scene {
            id: "main".into(),
            name: "Main".into(),
            rev: 1,
            groups: vec![group("left"), group("right")],
        }],
    };
    let server = common::serve_in_memory(api::router(app.clone()));
    Rig { server, app, engine, _home: home }
}

/// Sends a GET upgrade for `target` with `host` and reads the answer's head. The token rides in
/// the query, as a browser's must; no Authorization header.
async fn upgrade(
    server: &common::MemoryServer,
    target: &str,
    host: &str,
) -> (String, tokio::io::DuplexStream) {
    upgrade_with_buffer(server, target, host, None).await
}

async fn upgrade_with_buffer(
    server: &common::MemoryServer,
    target: &str,
    host: &str,
    buffer: Option<usize>,
) -> (String, tokio::io::DuplexStream) {
    let mut stream = match buffer {
        Some(bytes) => server.connect_with_buffer(bytes),
        None => server.connect(),
    };
    let request = format!(
        "GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.expect("the handshake is written");
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.expect("the answer is read");
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    (String::from_utf8_lossy(&head).into_owned(), stream)
}

const HOST: &str = "127.0.0.1:9430";

/// The status code of an answer's head.
fn status_of(head: &str) -> u16 {
    head.lines().next().and_then(|l| l.split(' ').nth(1)).and_then(|c| c.parse().ok()).expect("a status line")
}

async fn open_group(rig: &Rig, group: &str) -> tokio::io::DuplexStream {
    let target = format!("{}?token={TOKEN}&group={group}", routes::PREVIEW_GROUP);
    let (head, stream) = upgrade(&rig.server, &target, HOST).await;
    assert!(head.contains("101"), "the group socket opened: {head}");
    stream
}

/// One server frame: its opcode and payload.
async fn read_frame(stream: &mut tokio::io::DuplexStream) -> (u8, Vec<u8>) {
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.expect("a frame header is read");
    let opcode = header[0] & 0x0f;
    assert_eq!(header[1] & 0x80, 0, "a server frame is unmasked");
    let mut len = u64::from(header[1] & 0x7f);
    if len == 126 {
        let mut extended = [0u8; 2];
        stream.read_exact(&mut extended).await.expect("the length");
        len = u64::from(u16::from_be_bytes(extended));
    } else if len == 127 {
        let mut extended = [0u8; 8];
        stream.read_exact(&mut extended).await.expect("the length");
        len = u64::from_be_bytes(extended);
    }
    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload).await.expect("the payload is read");
    (opcode, payload)
}

async fn next_frame(stream: &mut tokio::io::DuplexStream) -> (u8, Vec<u8>) {
    tokio::time::timeout(Duration::from_secs(10), read_frame(stream))
        .await
        .expect("the socket answers, not left hanging")
}

/// Waits (no sleep) until `check` holds.
async fn eventually(what: &str, check: impl Fn() -> bool) {
    for _ in 0..5_000 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("never happened: {what}");
}

async fn group_sink(rig: &Rig) -> Arc<dyn canvas_proto::traits::PreviewSink> {
    eventually("the engine was given a group sink", || rig.engine.group_preview_sink.lock().unwrap().is_some()).await;
    rig.engine.group_preview_sink.lock().unwrap().clone().expect("a sink")
}

// ---------------------------------------------------------------- auth: the same as the existing sockets

/// The status and error code the route answers for `target` with `host`, as JSON.
async fn refusal(rig: &Rig, target: &str, host: &str) -> (u16, String) {
    let (head, mut stream) = upgrade(&rig.server, target, host).await;
    let status = status_of(&head);
    let length: usize = head
        .lines()
        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.expect("the body is read");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("a JSON error");
    (status, json["error"].as_str().unwrap_or_default().to_string())
}

#[tokio::test]
async fn no_token_is_refused_with_the_status_the_existing_previews_give() {
    let rig = rig("no-token").await;
    let existing = refusal(&rig, routes::PREVIEW, HOST).await;
    let lights = refusal(&rig, routes::PREVIEW_LIGHTS, HOST).await;
    let group = refusal(&rig, &format!("{}?group=left", routes::PREVIEW_GROUP), HOST).await;
    assert_eq!(existing.0, 401);
    assert_eq!(group, existing, "the group socket refuses as /preview does");
    assert_eq!(group, lights, "and as /preview/lights does");
    assert!(rig.engine.group_preview_calls().is_empty(), "a refused socket touches no engine");
}

#[tokio::test]
async fn a_wrong_token_is_refused_with_the_status_the_existing_previews_give() {
    let rig = rig("wrong-token").await;
    let existing = refusal(&rig, &format!("{}?token=not-the-token", routes::PREVIEW), HOST).await;
    let lights = refusal(&rig, &format!("{}?token=not-the-token", routes::PREVIEW_LIGHTS), HOST).await;
    let group = refusal(&rig, &format!("{}?token=not-the-token&group=left", routes::PREVIEW_GROUP), HOST).await;
    assert_eq!(existing.0, 401);
    assert_eq!(group, existing);
    assert_eq!(group, lights);
    assert!(rig.engine.group_preview_calls().is_empty());
}

#[tokio::test]
async fn a_wrong_host_is_refused_with_the_status_the_existing_previews_give() {
    let rig = rig("wrong-host").await;
    let existing = refusal(&rig, &format!("{}?token={TOKEN}", routes::PREVIEW), "evil.example").await;
    let lights = refusal(&rig, &format!("{}?token={TOKEN}", routes::PREVIEW_LIGHTS), "evil.example").await;
    let group =
        refusal(&rig, &format!("{}?token={TOKEN}&group=left", routes::PREVIEW_GROUP), "evil.example").await;
    assert_eq!(existing.0, 403);
    assert_eq!(existing.1, canvas_proto::api::error_codes::BAD_HOST);
    assert_eq!(group, existing);
    assert_eq!(group, lights);
    assert!(rig.engine.group_preview_calls().is_empty());
}

#[tokio::test]
async fn a_foreign_origin_is_refused_with_the_status_the_existing_previews_give() {
    let rig = rig("foreign-origin").await;
    // The helper writes the Host line; a second header line rides in with it. A page on another
    // origin that opens this socket sends its own Origin, and the right token does not help it.
    let from_elsewhere = format!("{HOST}
Origin: https://evil.example");
    let existing = refusal(&rig, &format!("{}?token={TOKEN}", routes::PREVIEW), &from_elsewhere).await;
    let lights = refusal(&rig, &format!("{}?token={TOKEN}", routes::PREVIEW_LIGHTS), &from_elsewhere).await;
    let group =
        refusal(&rig, &format!("{}?token={TOKEN}&group=left", routes::PREVIEW_GROUP), &from_elsewhere).await;
    assert_eq!(existing.0, 403);
    assert_eq!(existing.1, canvas_proto::api::error_codes::FORBIDDEN_ORIGIN);
    assert_eq!(group, existing);
    assert_eq!(group, lights);
    assert!(rig.engine.group_preview_calls().is_empty());
}

#[tokio::test]
async fn the_query_token_exemption_covers_the_group_path_and_no_other_path() {
    let rig = rig("exemption").await;
    // The group socket opens on the query token alone (no Authorization header).
    let _socket = open_group(&rig, "left").await;
    // A path beside it is not exempt: the guard wants the bearer header and answers 401.
    for path in ["/api/v1/preview/groups", "/api/v1/preview/group/left", "/api/v1/preview/other", "/api/v1/devices"] {
        let (status, code) = refusal(&rig, &format!("{path}?token={TOKEN}&group=left"), HOST).await;
        assert_eq!((status, code.as_str()), (401, "unauthorized"), "{path}");
    }
}

// ---------------------------------------------------------------- the group id

#[tokio::test]
async fn an_unknown_group_is_a_404_before_the_upgrade_and_after_the_token_check() {
    let rig = rig("unknown").await;
    let (status, code) =
        refusal(&rig, &format!("{}?token={TOKEN}&group=no-such-group", routes::PREVIEW_GROUP), HOST).await;
    assert_eq!((status, code.as_str()), (404, canvas_proto::api::error_codes::NOT_FOUND));
    // The token comes first: without it the answer is 401 even for an unknown group.
    let (status, _) = refusal(&rig, &format!("{}?group=no-such-group", routes::PREVIEW_GROUP), HOST).await;
    assert_eq!(status, 401, "401 before 404: the scene's group ids are not told to a caller without the token");
    assert!(rig.engine.group_preview_calls().is_empty(), "no selection was made for an unknown group");
}

#[tokio::test]
async fn a_missing_group_id_is_a_422() {
    let rig = rig("no-group").await;
    let (status, code) = refusal(&rig, &format!("{}?token={TOKEN}", routes::PREVIEW_GROUP), HOST).await;
    assert_eq!((status, code.as_str()), (422, canvas_proto::api::error_codes::VALIDATION));
    let (status, _) = refusal(&rig, &format!("{}?token={TOKEN}&group=", routes::PREVIEW_GROUP), HOST).await;
    assert_eq!(status, 422);
}

// ---------------------------------------------------------------- the stream

#[tokio::test]
async fn a_picture_arrives_in_the_format_of_the_effects_preview() {
    let rig = rig("format").await;
    let mut socket = open_group(&rig, "left").await;
    assert_eq!(rig.engine.group_preview_calls(), vec![(Some("left".to_string()), true)]);
    let sink = group_sink(&rig).await;
    sink.submit(2, 1, vec![1, 2, 3, 4, 5, 6]);
    let (opcode, payload) = next_frame(&mut socket).await;
    assert_eq!(opcode, 0x2, "a binary message");
    assert_eq!(payload, vec![2, 0, 1, 0, 1, 2, 3, 4, 5, 6], "u16 LE width, u16 LE height, RGB bytes");

    // The same bytes the effects preview socket sends for the same submit.
    let (head, mut preview) = upgrade(&rig.server, &format!("{}?token={TOKEN}", routes::PREVIEW), HOST).await;
    assert!(head.contains("101"));
    eventually("the effects preview has a sink", || rig.engine.preview.lock().unwrap().is_some()).await;
    let effects_sink = rig.engine.preview.lock().unwrap().clone().unwrap();
    effects_sink.submit(2, 1, vec![1, 2, 3, 4, 5, 6]);
    let (_, effects_payload) = next_frame(&mut preview).await;
    assert_eq!(effects_payload, payload, "byte for byte the existing preview frame");
}

#[tokio::test]
async fn the_effects_preview_socket_is_untouched_by_a_group_socket() {
    let rig = rig("editor-untouched").await;
    let (_, mut preview) = upgrade(&rig.server, &format!("{}?token={TOKEN}", routes::PREVIEW), HOST).await;
    eventually("the effects preview has a sink", || rig.engine.preview.lock().unwrap().is_some()).await;
    let mut group_socket = open_group(&rig, "left").await;
    let effects_sink = rig.engine.preview.lock().unwrap().clone().expect("the effects sink is still set");
    let group_sink = group_sink(&rig).await;
    effects_sink.submit(1, 1, vec![9, 9, 9]);
    group_sink.submit(1, 1, vec![7, 7, 7]);
    assert_eq!(next_frame(&mut preview).await.1, vec![1, 0, 1, 0, 9, 9, 9]);
    assert_eq!(next_frame(&mut group_socket).await.1, vec![1, 0, 1, 0, 7, 7, 7]);
}

// ---------------------------------------------------------------- one place, the newest wins

#[tokio::test]
async fn a_second_socket_takes_over_and_the_first_is_closed_with_1013() {
    let rig = rig("takeover").await;
    let mut first = open_group(&rig, "left").await;
    let first_sink = group_sink(&rig).await;
    let mut second = open_group(&rig, "right").await;

    // The first socket is told, with the code the stack preview socket uses for "not yours".
    let (opcode, payload) = next_frame(&mut first).await;
    assert_eq!(opcode, 0x8, "a close frame");
    assert_eq!(u16::from_be_bytes([payload[0], payload[1]]), 1013);
    assert_eq!(String::from_utf8_lossy(&payload[2..]), "another group preview took over");

    // The engine now follows the newest socket, and the older socket's release did not clear it.
    let calls = rig.engine.group_preview_calls();
    assert_eq!(calls, vec![(Some("left".to_string()), true), (Some("right".to_string()), true)]);
    let second_sink = group_sink(&rig).await;
    assert!(!Arc::ptr_eq(&first_sink, &second_sink), "the newest socket's own sink");
    second_sink.submit(1, 1, vec![5, 6, 7]);
    assert_eq!(next_frame(&mut second).await.1, vec![1, 0, 1, 0, 5, 6, 7]);
    assert_eq!(rig.engine.group_preview_calls().len(), 2, "the taken-over socket did not stop the tap");
    assert!(rig.app.group_preview.is_open());
}

#[tokio::test]
async fn closing_the_last_socket_clears_the_selection() {
    let rig = rig("close-last").await;
    let first = open_group(&rig, "left").await;
    let second = open_group(&rig, "right").await;
    // The first was taken over; closing it changes nothing.
    drop(first);
    tokio::task::yield_now().await;
    assert!(rig.engine.group_preview_calls().iter().all(|(group, _)| group.is_some()));
    // Closing the newest one stops the engine.
    drop(second);
    eventually("the tap was stopped", || rig.engine.group_preview_calls().last() == Some(&(None, false))).await;
    assert!(!rig.app.group_preview.is_open());
    assert_eq!(
        rig.engine.group_preview_calls(),
        vec![(Some("left".to_string()), true), (Some("right".to_string()), true), (None, false)]
    );
}

#[tokio::test]
async fn a_socket_that_closes_by_itself_clears_the_selection() {
    let rig = rig("close-one").await;
    let socket = open_group(&rig, "left").await;
    drop(socket);
    eventually("the tap was stopped", || rig.engine.group_preview_calls().last() == Some(&(None, false))).await;
}

// ---------------------------------------------------------------- backpressure

#[tokio::test]
async fn a_slow_reader_never_grows_a_queue_and_is_dropped_after_the_send_timeout() {
    let rig = rig("slow-reader").await;
    rig.app.scene.feed.set_send_timeout(Duration::from_millis(200));
    // A pipe that holds 512 bytes each way, and a client that never reads: the 12,000-byte pictures
    // block the socket's send.
    let target = format!("{}?token={TOKEN}&group=left", routes::PREVIEW_GROUP);
    let (head, _silent_client) = upgrade_with_buffer(&rig.server, &target, HOST, Some(512)).await;
    assert!(head.contains("101"), "{head}");
    let sink = group_sink(&rig).await;
    // The engine's side never waits: a thousand submits return at once, the queue (4 pictures)
    // holds the first few, the rest are dropped.
    let started = std::time::Instant::now();
    for _ in 0..1_000 {
        sink.submit(80, 50, vec![0; 12_000]);
    }
    assert!(started.elapsed() < Duration::from_secs(5), "submit blocked on a reader that does not read");
    // The reader is given the send timeout and then let go, and the place goes with it.
    eventually("the silent socket was dropped and the tap stopped", || {
        rig.engine.group_preview_calls().last() == Some(&(None, false))
    })
    .await;
    assert!(!rig.app.group_preview.is_open());
}
