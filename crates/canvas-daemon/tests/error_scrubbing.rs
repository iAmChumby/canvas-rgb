//! #576/#575 regression tests, run entirely on the fakes.
//!
//! The first two tests would fail on the tree before the response layer: a fake profile path
//! came back in an error body, both from a JSON error and from a plain-text extractor rejection.
//! The last one drives the diagnostics route's list-then-write pair.

mod common;

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_diag::scrub::Scrubber;
use canvas_proto::api::routes;
use common::{FakeAdapter, FakeEngine};
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
// Spelled in parts so no test writes a real profile path into the repository.
const FAKE: &str = concat!("C:\\", "Use", "rs\\Alice");

/// One fake home for this test binary. Setting the override once keeps parallel tests from
/// racing on the environment and keeps every read off the real user and machine trees.
fn home() -> &'static std::path::PathBuf {
    static HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!(
            "canvas-rgb-scrub-{}-{}",
            std::process::id(),
            canvas_diag::paths::timestamp()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zones.json"), format!("{{\"{FAKE}/zones\":1}}")).unwrap();
        // The override also relocates the fake machine tree, so no test reads ProgramData.
        std::env::set_var(canvas_diag::paths::HOME_ENV, &dir);
        dir
    })
}

fn build() -> (axum::Router, Arc<App>, std::path::PathBuf) {
    let home = home().clone();
    let store = canvas_daemon::storage::Store::at(&home);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(App::new(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    app.set_scrubber(Scrubber::new(
        Some(FAKE.to_string()),
        Some("Alice".to_string()),
        Some("ALICE-PC".to_string()),
    ));
    (api::router(app.clone()), app, home)
}

async fn body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn raw_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec()
}

/// A JSON error's `message` and `hint` never carry the profile path, in any spelling.
#[tokio::test]
async fn a_fake_profile_path_in_an_error_route_is_scrubbed() {
    let (router, _app, _home) = build();
    let response = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::STATUS)
                .header("host", "127.0.0.1:9430")
                .header("origin", format!("http://{}/notes", FAKE.replace('\\', "/")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let text = body(response).await;
    assert!(!text.to_ascii_lowercase().contains("alice"), "{text}");
    assert!(!text.contains("Use\\rs") && !text.contains("Use/rs"), "{text}");
    assert!(text.contains("<profile>"), "{text}");
    assert!(text.contains("hint"), "the next step rides with every error: {text}");
}

/// An extractor rejection is plain text, not JSON. It is scrubbed by the same layer.
#[tokio::test]
async fn a_malformed_json_body_never_comes_back_unscrubbed() {
    let (router, _app, _home) = build();
    // Serialised so the body is valid JSON syntax with a wrong field type: serde quotes the
    // offending value in its rejection, which is the plain-text body the layer must scrub.
    let bad = serde_json::to_string(&serde_json::json!({ "source": "cpu", "failed": FAKE })).unwrap();
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/diag/fail-sensor")
                .header("host", "127.0.0.1:9430")
                .header("authorization", format!("Bearer {TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(bad))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_client_error(), "{:?}", response.status());
    let text = body(response).await;
    assert!(!text.to_ascii_lowercase().contains("alice"), "{text}");
    assert!(!text.contains("Use\\rs") && !text.contains("Use/rs"), "{text}");
}

/// `GET` lists before `POST` writes; the answer carries the file list and a plain reason for
/// service logs that are absent on the fakes' tree.
#[tokio::test]
async fn the_bundle_route_lists_then_writes() {
    let (router, _app, home) = build();
    let auth = format!("Bearer {TOKEN}");
    let list = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(routes::DIAG_BUNDLE)
                .header("host", "127.0.0.1:9430")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let text = body(list).await;
    let listed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(listed["files"].as_array().unwrap().iter().any(|f| f["name"] == "config/zones.json"));
    assert!(listed["path"].is_null(), "GET does not write: {text}");
    assert!(listed["service_logs"].as_str().unwrap().contains("not included"), "{text}");

    // A caller cannot choose the destination (P2).
    let refused = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(routes::DIAG_BUNDLE)
                .header("host", "127.0.0.1:9430")
                .header("authorization", &auth)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({ "path": home.join("out").to_string_lossy() })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    // The handler takes no body, so a caller-chosen path is ignored (P2).
    let ignored: serde_json::Value = serde_json::from_str(&body(refused).await).unwrap();
    assert_eq!(ignored["error"], serde_json::Value::Null);
    assert_eq!(
        std::path::PathBuf::from(ignored["path"].as_str().unwrap()).parent().unwrap(),
        canvas_diag::paths::diagnostics_dir()
    );

    let write = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(routes::DIAG_BUNDLE)
                .header("host", "127.0.0.1:9430")
                .header("authorization", &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = write.status();
    let text = body(write).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let written: serde_json::Value = serde_json::from_str(&text).unwrap();
    let path = std::path::PathBuf::from(written["path"].as_str().unwrap());
    assert!(path.exists());
    assert_eq!(path.parent().unwrap(), canvas_diag::paths::diagnostics_dir());
    // The home is not removed: every test in this binary shares it, and removing it while
    // another test saves into it fails that save.
}

/// The layer itself, through a real axum router: only a non-2xx body is rewritten (G4).
#[tokio::test]
async fn the_layer_leaves_2xx_user_data_byte_identical() {
    let (_router, app, _home) = build();
    async fn bad() -> (StatusCode, axum::Json<serde_json::Value>) {
        (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "detail": { "note": format!("failed at {FAKE}\\a.html: openrgb:SERIAL-42") }
            })),
        )
    }
    async fn fine() -> axum::Json<serde_json::Value> {
        axum::Json(serde_json::json!({ "note": "use cpu:fan" }))
    }
    let router = axum::Router::new()
        .route("/bad", axum::routing::get(bad))
        .route("/fine", axum::routing::get(fine))
        .layer(axum::middleware::from_fn_with_state(
            app.clone(),
            api::scrub_errors::scrub_errors,
        ));
    let rejected = router.clone().oneshot(Request::builder().uri("/bad").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    let text = body(rejected).await;
    assert!(!text.to_ascii_lowercase().contains("alice"), "{text}");
    assert!(!text.contains("SERIAL-42"), "{text}");

    let ok = router.oneshot(Request::builder().uri("/fine").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let text = body(ok).await;
    assert_eq!(text, serde_json::json!({ "note": "use cpu:fan" }).to_string());
}

/// One API call through the real router and layer, as JSON in and out.
async fn api_call(
    router: &axum::Router,
    method: &str,
    path: &str,
    auth: &str,
    payload: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", auth);
    if payload.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let text = payload.map(|v| v.to_string()).unwrap_or_default();
    let response = router
        .clone()
        .oneshot(builder.body(Body::from(text)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = raw_bytes(response).await;
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn url_part(text: &str) -> String {
    text.replace('%', "%25").replace(' ', "%20").replace(':', "%3A")
}

/// User-authored names that look like device ids are data, not error text: a read, a save and a
/// second read must return the same bytes (H5/G4).
#[tokio::test]
async fn user_names_with_device_like_text_survive_get_save_get() {
    let (router, app, _home) = build();
    // One light so a new scene really holds a group to rename. A create makes its default group
    // from the store's lights, not from the adapter's device list.
    *app.devices.write().await = vec![common::device(
        "desk",
        canvas_proto::device::DeviceKind::LedStrip,
        4,
    )];
    let light = canvas_proto::scene::Light {
        id: "fake:desk#z0".into(),
        device_id: canvas_proto::device::DeviceId("fake:desk".into()),
        zone_index: 0,
        leds: None,
        name: None,
        arrangement: canvas_proto::scene::Arrangement::Strip,
        reversed: false,
        kind: None,
        copies: None,
        first_led: None,
    };
    app.store.write().await.lights = canvas_proto::scene::LightsFile {
        chains: Vec::new(),
        schema_version: canvas_proto::scene::LIGHTS_SCHEMA,
        lights: vec![light],
    };
    let auth = format!("Bearer {TOKEN}");
    let scene_name = "use cpu:fan on openrgb:ABC";
    let group_name = "layer cpu:fan, device openrgb:ABC";

    let (status, created) = api_call(
        &router,
        "POST",
        routes::SCENES,
        &auth,
        Some(serde_json::json!({ "name": scene_name })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a scene was created: {created}");

    let (status, list) = api_call(&router, "GET", routes::SCENES, &auth, None).await;
    assert_eq!(status, StatusCode::OK);
    let mut scenes = list["scenes"].as_array().unwrap().clone();
    assert!(
        scenes.iter().any(|s| s["name"] == scene_name),
        "the read already changed the name: {list}"
    );
    // Put the second user string into a group name, then save the lot unchanged otherwise.
    for scene in scenes.iter_mut().filter(|s| s["name"] == scene_name) {
        for group in scene["groups"].as_array_mut().unwrap() {
            group["name"] = serde_json::Value::String(group_name.to_string());
        }
    }
    let order: Vec<_> = scenes.iter().map(|s| s["id"].clone()).collect();
    let writes: Vec<_> = scenes
        .iter()
        .map(|s| serde_json::json!({ "scene": s, "expected_rev": s["rev"] }))
        .collect();
    let (status, answer) = api_call(
        &router,
        "PUT",
        routes::SCENES,
        &auth,
        Some(serde_json::json!({
            "scenes": writes,
            "order": order,
            "expected_order": order,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the save was refused: {answer}");

    let (_, list) = api_call(&router, "GET", routes::SCENES, &auth, None).await;
    let saved = list["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == scene_name)
        .expect("the scene name survived");
    assert!(
        saved["groups"].as_array().unwrap().iter().any(|g| g["name"] == group_name),
        "the group name survived: {saved}"
    );

    // The same rule for an effect preset's name: save, read, rename, read.
    let (status, effects) = api_call(&router, "GET", routes::EFFECTS, &auth, None).await;
    assert_eq!(status, StatusCode::OK);
    let id = effects["effects"][0]["id"].as_str().unwrap().to_string();
    let presets = format!("/api/v1/effects/{id}/presets");
    let detail_path = format!("/api/v1/effects/{id}");
    let (status, _) = api_call(
        &router,
        "POST",
        &presets,
        &auth,
        Some(serde_json::json!({ "name": scene_name, "props": {} })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a preset was saved");
    let (_, detail) = api_call(&router, "GET", &detail_path, &auth, None).await;
    assert!(
        detail["user_presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == scene_name),
        "the preset name survived: {detail}"
    );
    let rename_path = format!("{presets}/{}", url_part(scene_name));
    let (status, answer) = api_call(
        &router,
        "PUT",
        &rename_path,
        &auth,
        Some(serde_json::json!({
            "props": {},
            "expected": {},
            "rename_to": group_name,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the rename was refused: {answer}");
    let (_, detail) = api_call(&router, "GET", &detail_path, &auth, None).await;
    assert!(
        detail["user_presets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == group_name),
        "the renamed preset name survived: {detail}"
    );
}

/// A 4xx whose stream breaks is forwarded as it came: the bytes read so far, unchanged, and then
/// the error, never a shorter body that looks complete (H2).
#[tokio::test]
async fn a_broken_4xx_stream_keeps_its_bytes_and_stays_broken() {
    use futures_util::StreamExt;
    let (_router, app, _home) = build();
    let sent = format!("failed at {FAKE}/notes: ").into_bytes();
    let handler = {
        let sent = sent.clone();
        move || {
            let chunk = Bytes::from(sent.clone());
            async move {
                let cut = axum::Error::new(std::io::Error::other("the stream was cut"));
                let stream = futures_util::stream::iter(vec![Ok::<Bytes, axum::Error>(chunk), Err(cut)]);
                (StatusCode::BAD_REQUEST, Body::from_stream(stream))
            }
        }
    };
    let router = axum::Router::new()
        .route("/broken", axum::routing::get(handler))
        .layer(axum::middleware::from_fn_with_state(app, api::scrub_errors::scrub_errors));
    let response = router
        .oneshot(Request::builder().uri("/broken").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.headers().get(axum::http::header::CONTENT_LENGTH).is_none());
    let mut stream = response.into_body().into_data_stream();
    let mut got = Vec::new();
    let mut broke = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => got.extend_from_slice(&chunk),
            Err(_) => {
                broke = true;
                break;
            }
        }
    }
    assert_eq!(got, sent, "the bytes before the break were changed");
    assert!(broke, "a cut body must not end as a complete one");
}

/// An oversized stream without Content-Length cannot be rewritten: every byte, including the
/// user path in its prefix and the bytes after the cap, comes back unchanged (H2).
#[tokio::test]
async fn an_oversized_4xx_body_without_content_length_is_byte_identical() {
    let (_router, app, _home) = build();
    let mut expected = format!("failed at {FAKE}/notes: ").into_bytes();
    expected.extend(std::iter::repeat_n(b'z', 1024 * 1024 + 64));
    let expected_len = expected.len();
    let handler = {
        let expected = expected.clone();
        move || {
            let bytes = expected.clone();
            async move {
                let first = Bytes::from(bytes[..64].to_vec());
                let rest = Bytes::from(bytes[64..].to_vec());
                let chunks = vec![Ok::<Bytes, axum::Error>(first), Ok::<Bytes, axum::Error>(rest)];
                let stream = futures_util::stream::iter(chunks);
                let mut response = (StatusCode::BAD_REQUEST, Body::from_stream(stream)).into_response();
                response.headers_mut().remove(axum::http::header::CONTENT_LENGTH);
                response
            }
        }
    };
    let router = axum::Router::new()
        .route("/huge", axum::routing::get(handler))
        .layer(axum::middleware::from_fn_with_state(app, api::scrub_errors::scrub_errors));
    let response = router
        .oneshot(Request::builder().uri("/huge").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.headers().get(axum::http::header::CONTENT_LENGTH).is_none());
    let actual = raw_bytes(response).await;
    assert_eq!(actual.len(), expected_len);
    assert_eq!(actual, expected, "the oversized body was rewritten");
}

/// A rewritten 4xx body keeps a Content-Length that matches the bytes actually sent (H2).
#[tokio::test]
async fn a_rewritten_4xx_body_has_a_correct_content_length() {
    let (_router, app, _home) = build();
    async fn sized() -> axum::response::Response {
        let original =
            serde_json::to_vec(&serde_json::json!({ "message": format!("failed at {FAKE}\\x: openrgb:SERIAL-42") }))
                .unwrap();
        let original_len = original.len();
        let mut response = (StatusCode::BAD_REQUEST, Body::from(original)).into_response();
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_TYPE, axum::http::HeaderValue::from_static("application/json"));
        response.headers_mut().insert(
            axum::http::header::CONTENT_LENGTH,
            axum::http::HeaderValue::from_str(&original_len.to_string()).unwrap(),
        );
        response
    }
    let router = axum::Router::new()
        .route("/sized", axum::routing::get(sized))
        .layer(axum::middleware::from_fn_with_state(app, api::scrub_errors::scrub_errors));
    let response = router
        .oneshot(Request::builder().uri("/sized").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let declared: usize = response
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    let bytes = raw_bytes(response).await;
    assert_eq!(declared, bytes.len(), "the length belongs to the rewritten body");
    assert!(!String::from_utf8_lossy(&bytes).contains("SERIAL-42"));
}

/// Non-JSON binary error bytes are not text: no lossy conversion (H2).
#[tokio::test]
async fn a_binary_4xx_body_is_byte_identical() {
    let (_router, app, _home) = build();
    async fn binary() -> (StatusCode, Body) {
        (StatusCode::BAD_REQUEST, Body::from(vec![0xff, 0xfe, 0x00, b'a', 0x80, 0x00]))
    }
    let router = axum::Router::new()
        .route("/binary", axum::routing::get(binary))
        .layer(axum::middleware::from_fn_with_state(app, api::scrub_errors::scrub_errors));
    let response = router
        .oneshot(Request::builder().uri("/binary").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        raw_bytes(response).await,
        vec![0xff, 0xfe, 0x00, b'a', 0x80, 0x00]
    );
}
