//! The effect import and delete routes.
//!
//! Every test runs against a temporary effects folder, so nothing is written to the
//! owner's real one.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::state::App;
use canvas_proto::api::routes;
use canvas_proto::props::Props;
use common::{FakeAdapter, FakeEngine};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A valid effect: it parses, it draws, and it moves.
const VALID: &str = r##"<head>
  <title>Spark</title>
  <meta description="A test effect." />
  <meta property="speed" label="Speed" type="number" min="0" max="100" default="10" />
</head>
<body><canvas id="stage" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("stage");
  var ctx = c.getContext("2d");
  var x = 0;
  function draw() {
    ctx.fillStyle = "#101020";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = "#ffcc00";
    ctx.fillRect(x, 40, 30, 30);
    x = (x + 3) % 300;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

/// A file with no script: the parser refuses it.
const NO_SCRIPT: &str = "<head><title>Empty</title></head><body></body>";

/// A temporary directory that is removed when it is dropped.
struct TempHome {
    path: PathBuf,
}

impl TempHome {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-import-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
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

/// The router, the state, the home and the effects folder.
fn build() -> (axum::Router, Arc<App>, TempHome, PathBuf) {
    build_with(None)
}

/// Like [`build`], with the library's builder replaced: the seam that lets a test say which
/// of the builder's calls run out of time.
fn build_with(
    builder: Option<canvas_daemon::user_effects::Builder>,
) -> (axum::Router, Arc<App>, TempHome, PathBuf) {
    let home = TempHome::new();
    let folder = home.path.join("effects");
    std::fs::create_dir_all(&folder).expect("the effects folder is created");
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        engine,
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    // The import writes into this folder, never into the owner's.
    app.effects_dir = folder.clone();
    if let Some(builder) = builder {
        app.user_effects = Arc::new(canvas_daemon::user_effects::UserEffects::with_stock(builder, &[]));
    }
    let app = Arc::new(app);
    (api::router(app.clone()), app, home, folder)
}

/// A request with the token.
fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

/// A request with a body.
fn request_with_body(method: &str, path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("the request builds")
}

/// The body of a response, as JSON.
async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("the body is bounded");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// The import body.
fn import_body(file_name: &str, html: &str, replace: bool) -> String {
    serde_json::json!({ "file_name": file_name, "html": html, "replace": replace }).to_string()
}

/// The ids in the effects list.
async fn effect_ids(router: &axum::Router) -> Vec<String> {
    let response = router
        .clone()
        .oneshot(request("GET", routes::EFFECTS))
        .await
        .expect("the router answers");
    let body = json(response).await;
    body.get("effects")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect()
}

/// The files that did not load, with the reason.
async fn failed_effects(router: &axum::Router) -> Vec<(String, String)> {
    let response = router
        .clone()
        .oneshot(request("GET", routes::EFFECTS))
        .await
        .expect("the router answers");
    let body = json(response).await;
    body.get("failed")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|e| {
            let file = e.get("file")?.as_str()?.to_string();
            let error = e.get("error")?.as_str()?.to_string();
            Some((file, error))
        })
        .collect()
}

#[tokio::test]
async fn an_imported_effect_lands_in_the_folder_and_in_the_library() {
    let (router, app, _home, folder) = build();

    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("spark.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let effect = body.get("effect").expect("the effect");
    assert_eq!(effect.get("id").and_then(|v| v.as_str()), Some("html:spark"));
    assert_eq!(effect.get("name").and_then(|v| v.as_str()), Some("Spark"));
    assert_eq!(body.get("replaced").and_then(|v| v.as_bool()), Some(false));

    // The file is in the folder, with the contents that were sent.
    let written = std::fs::read_to_string(folder.join("spark.html")).expect("the file is there");
    assert_eq!(written, VALID);

    // And the effect is in the library.
    assert!(effect_ids(&router).await.contains(&"html:spark".to_string()));

    // And it runs: the library builds it from the file it wrote, and its setup runs without
    // a load timeout.
    let mut built = canvas_daemon::create_effect(
        &app.user_effects,
        &app.stacks,
        &app.patterns,
        "html:spark",
        None,
    )
    .expect("the imported effect builds");
    built.init(&Props::new());
    assert_eq!(built.schema().id, "html:spark");
}

#[tokio::test]
async fn a_file_name_with_a_separator_is_refused() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("sub/spark.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!folder.join("sub").exists(), "nothing is written");
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
}

#[tokio::test]
async fn a_file_name_without_the_html_suffix_is_refused() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("spark.txt", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
}

#[tokio::test]
async fn a_file_name_with_two_dots_is_refused() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("..spark.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
}

#[tokio::test]
async fn a_reserved_windows_name_is_refused() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("con.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
}

#[tokio::test]
async fn a_file_name_that_would_carry_a_built_in_id_is_refused() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("solid.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
}

#[tokio::test]
async fn a_file_that_is_too_big_is_refused() {
    let (router, _app, _home, folder) = build();
    let big = "x".repeat(canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES + 1);
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("big.html", &big, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
}

#[tokio::test]
async fn a_file_that_does_not_parse_is_refused_and_nothing_is_written() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("broken.html", NO_SCRIPT, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    assert!(message.contains("parse:"), "the parser's message is kept: {message}");
    // Nothing was written.
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
    assert!(!effect_ids(&router).await.contains(&"html:broken".to_string()));
}

#[tokio::test]
async fn a_clash_without_replace_is_a_conflict() {
    let (router, _app, _home, folder) = build();
    let body = import_body("spark.html", VALID, false);
    let response = router
        .clone()
        .oneshot(request_with_body("POST", routes::EFFECTS_IMPORT, &body))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(request_with_body("POST", routes::EFFECTS_IMPORT, &body))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    // The file that is there is the first one.
    let written = std::fs::read_to_string(folder.join("spark.html")).expect("the file is there");
    assert_eq!(written, VALID);
}

#[tokio::test]
async fn a_clash_with_replace_overwrites_the_file() {
    let (router, _app, _home, folder) = build();
    let body = import_body("spark.html", VALID, false);
    let response = router
        .clone()
        .oneshot(request_with_body("POST", routes::EFFECTS_IMPORT, &body))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    let other = VALID.replace("Spark", "Spark Two");
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("spark.html", &other, true),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert_eq!(body.get("replaced").and_then(|v| v.as_bool()), Some(true));
    let written = std::fs::read_to_string(folder.join("spark.html")).expect("the file is there");
    assert_eq!(written, other);
    // The library has the new name.
    let ids = effect_ids(&router).await;
    assert!(ids.contains(&"html:spark".to_string()));
}

#[tokio::test]
async fn a_broken_file_in_the_folder_is_reported_in_the_failed_list() {
    let (router, app, _home, folder) = build();
    std::fs::write(folder.join("broken.html"), NO_SCRIPT).expect("the file is written");
    app.user_effects.rescan(&folder);

    let response = router
        .clone()
        .oneshot(request("GET", routes::EFFECTS))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let failed = body.get("failed").and_then(|v| v.as_array()).expect("the failed list");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].get("file").and_then(|v| v.as_str()), Some("broken.html"));
    let error = failed[0].get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(error.contains("parse:"), "the parser's message is kept: {error}");
}

#[tokio::test]
async fn a_user_effect_can_be_deleted() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("spark.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(request("DELETE", "/api/v1/effects/html:spark"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    // The file is gone and the effect has left the library.
    assert!(!folder.join("spark.html").exists());
    assert!(!effect_ids(&router).await.contains(&"html:spark".to_string()));
}

#[tokio::test]
async fn a_built_in_effect_cannot_be_deleted() {
    let (router, _app, _home, _folder) = build();
    let response = router
        .clone()
        .oneshot(request("DELETE", "/api/v1/effects/solid"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // The built-in is still there.
    assert!(effect_ids(&router).await.contains(&"solid".to_string()));
}

#[tokio::test]
async fn deleting_an_effect_that_is_not_there_is_not_found() {
    let (router, _app, _home, _folder) = build();
    let response = router
        .clone()
        .oneshot(request("DELETE", "/api/v1/effects/html:missing"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_reserved_name_is_refused_whatever_comes_after_the_first_dot() {
    let (router, _app, _home, folder) = build();
    // Windows reserves the name with any extension, so the part before the first dot
    // is what counts.
    for name in ["con.x.html", "nul.a.html", "COM1.foo.html"] {
        let response = router
            .clone()
            .oneshot(request_with_body(
                "POST",
                routes::EFFECTS_IMPORT,
                &import_body(name, VALID, false),
            ))
            .await
            .expect("the router answers");
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{name} must be refused"
        );
    }
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
}

/// A file whose script never ends. It does not fit the normal load budget or the import's
/// longer one.
const RUNAWAY: &str =
    "<html><head><title>Loop</title></head><body></body><script>while (true) {}</script></html>";

/// A script that does not finish loading inside the import's longer load is refused, and
/// nothing is written: saving it would only buy the same load again on every retry (#75).
#[tokio::test]
async fn an_effect_whose_script_never_ends_is_refused_and_nothing_is_saved() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("loop.html", RUNAWAY, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        message.contains("did not finish loading within 10 s")
            && message.contains("nothing was saved")
            && !message.contains("timeout:"),
        "the answer says what happened in plain words: {message}"
    );
    // Nothing was written, and the file is not waiting to be retried.
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
    assert!(!effect_ids(&router).await.contains(&"html:loop".to_string()));
    assert!(
        failed_effects(&router).await.is_empty(),
        "a refused import leaves nothing to retry: {:?}",
        failed_effects(&router).await
    );
}

#[tokio::test]
async fn an_oversized_effect_is_refused_with_the_api_error_shape() {
    let (router, _app, _home, folder) = build();
    let big = "x".repeat(canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES + 1);
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("big.html", &big, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // The answer is the API's own error shape.
    let body = json(response).await;
    assert_eq!(
        body.get("error").and_then(|v| v.as_str()),
        Some(canvas_proto::api::error_codes::VALIDATION)
    );
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    assert!(message.contains("the limit is"), "the message names the limit: {message}");
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_with_the_api_error_shape() {
    let (router, _app, _home, folder) = build();
    // The body is over the route's limit, so it is refused before it is read.
    let huge = "x".repeat(canvas_daemon::api::effects::IMPORT_BODY_LIMIT + 1024);
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("huge.html", &huge, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // The answer is the API's own error shape, not axum's plain text.
    let body = json(response).await;
    assert!(
        body.get("error").and_then(|v| v.as_str()).is_some(),
        "the answer has the API's error shape: {body}"
    );
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        message.contains("over the limit"),
        "the message says the body is too big: {message}"
    );
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 0);
}

#[tokio::test]
async fn a_clash_is_found_without_regard_to_case() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("Foo.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert_eq!(body.get("effect").and_then(|e| e.get("id")).and_then(|v| v.as_str()), Some("html:Foo"));

    // The same file in another case is a clash, not a second file.
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("foo.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 1);

    // A replace overwrites the file that is there, and it keeps its name.
    let other = VALID.replace("Spark", "Spark Two");
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("foo.html", &other, true),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert_eq!(body.get("replaced").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(body.get("effect").and_then(|e| e.get("id")).and_then(|v| v.as_str()), Some("html:Foo"));
    // There is still one file, and it holds the new text.
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 1);
    let written = std::fs::read_to_string(folder.join("Foo.html")).expect("the file is there");
    assert_eq!(written, other);
}

#[tokio::test]
async fn deleting_an_effect_finds_it_without_regard_to_case() {
    let (router, _app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("Foo.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    // The id is in another case than the file's name.
    let response = router
        .clone()
        .oneshot(request("DELETE", "/api/v1/effects/html:foo"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!folder.join("Foo.html").exists());
}

#[tokio::test]
async fn deleting_the_active_effect_falls_back_like_a_missing_file() {
    let (router, app, _home, folder) = build();
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("spark.html", VALID, false),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    // The effect is the one the active scene's group plays.
    {
        let mut store = app.store.write().await;
        let active = store.scenes.active.clone();
        if let Some(scene) = store.scenes.scenes.iter_mut().find(|s| s.id == active) {
            if let Some(group) = scene.groups.first_mut() {
                group.plays = canvas_proto::scene::Plays::Effect {
                    effect: "html:spark".to_string(),
                    props: Props::new(),
                };
            }
        }
        store.save_scenes().expect("the scenes are saved");
    }

    let response = router
        .clone()
        .oneshot(request("DELETE", "/api/v1/effects/html:spark"))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    // The effect has left the library, and the file is gone.
    assert!(!folder.join("spark.html").exists());
    assert!(!effect_ids(&router).await.contains(&"html:spark".to_string()));

    // The daemon is still answering, and the effect can no longer be built: the same
    // answer the daemon gives when the file went missing by hand.
    let made = canvas_daemon::create_effect(&app.user_effects, &app.stacks, &app.patterns, "html:spark", None);
    assert!(made.is_err(), "the deleted effect cannot be built any more");
}

/// A builder whose calls, counted from zero, run out of time when their number is in
/// `times_out`; the others build the real effect.
///
/// An import makes its normal load first (call 0 for a file that is not there yet) and, only
/// because that ran out of time, one longer load (call 1). The rescan that follows the write
/// parses the file again at the normal budget (call 2).
fn timing_out_on(times_out: &'static [usize]) -> canvas_daemon::user_effects::Builder {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let real = canvas_daemon::user_effects::html_builder();
    Arc::new(move |id: &str, html: &str| {
        let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if times_out.contains(&call) {
            return Err("timeout: the effect's script ran longer than 2000 ms".to_string());
        }
        real(id, html)
    })
}

async fn import(router: &axum::Router, file: &str, html: &str, replace: bool) -> serde_json::Value {
    let response = router
        .clone()
        .oneshot(request_with_body("POST", routes::EFFECTS_IMPORT, &import_body(file, html, replace)))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);
    json(response).await
}

/// A new version whose normal load runs out of time and whose longer one does too: the
/// replace is refused before anything is written, so the version that loads keeps running
/// and the file is untouched.
#[tokio::test]
async fn a_replace_whose_new_version_never_loads_is_refused_and_the_old_one_stays() {
    // Calls: 0 v1's import load, 1 v1's rescan; 2 v2's normal load, 3 v2's longer load. A
    // replace whose new version does not finish inside the longer load is refused.
    let (router, _app, _home, folder) = build_with(Some(timing_out_on(&[2, 3])));
    import(&router, "spark.html", VALID, false).await;
    let v2 = VALID.replace("<title>Spark</title>", "<title>Spark Two</title>");
    let response = router
        .clone()
        .oneshot(request_with_body(
            "POST",
            routes::EFFECTS_IMPORT,
            &import_body("spark.html", &v2, true),
        ))
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json(response).await;
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    assert!(message.contains("did not finish loading within 10 s"), "{message}");
    // The file that was there is the version that loads, and it is still in the library.
    let written = std::fs::read_to_string(folder.join("spark.html")).expect("the file is there");
    assert_eq!(written, VALID);
    assert!(effect_ids(&router).await.contains(&"html:spark".to_string()), "v1 keeps running");
}

/// The normal load runs out of time but the import's one longer load succeeds: the file is
/// saved and the rescan loads it, so the reply is the normal one, not "will be tried again".
#[tokio::test]
async fn an_import_whose_normal_load_timed_out_but_the_longer_one_loaded_is_a_normal_reply() {
    let (router, _app, _home, _folder) = build_with(Some(timing_out_on(&[0])));
    let body = import(&router, "spark.html", VALID, false).await;
    assert_eq!(body["effect"]["name"], "Spark");
    assert_eq!(body["effect"]["description"], "A test effect.");
    assert!(effect_ids(&router).await.contains(&"html:spark".to_string()));
}

/// A page whose top-level script burns `iterations` loop iterations.
fn loop_page(iterations: u64) -> String {
    format!(
        "<head><title>Slow</title></head><body><canvas id=\"c\" width=\"4\" height=\"4\"></canvas></body>\
         <script>for (var i = 0; i < {iterations}; i++) {{}}</script>"
    )
}

/// How long a load of `iterations` takes, in seconds, using the given builder.
///
/// The probes run at the import's longer load budget, so a probe fails the test only if a
/// load stalls for ten seconds; a machine that is merely slow makes the probes shorter, not
/// the test red.
fn load_seconds(builder: &canvas_daemon::user_effects::Builder, iterations: u64) -> f64 {
    let started = std::time::Instant::now();
    builder("html:probe", &loop_page(iterations))
        .expect("the probe loads inside the import's 10 s budget");
    started.elapsed().as_secs_f64()
}

/// A script that needs longer than the normal load budget, but less than the import's longer
/// one, is saved exactly as it was before this change: the rescan at the normal budget puts it
/// in the failed list, in today's words, and the retry picks it up. It is not installed as
/// loaded, because the lights and the tiles run every effect at the normal budget (#75).
///
/// The script's cost is calibrated on this machine (a warm-up, the fixed cost of a load taken
/// off, and the budget as the yardstick), so the test does not depend on how fast it is
/// (class `cause:timing-test`, #51).
#[tokio::test]
async fn an_effect_a_little_slower_than_the_normal_budget_is_saved_and_waiting_to_be_retried() {
    let longer = canvas_daemon::user_effects::html_builder_with_import_load();
    // What a page costs before its script runs, and a warm-up.
    let overhead = load_seconds(&longer, 0);
    let _ = load_seconds(&longer, 1);
    // The cost of one iteration, measured over a probe long enough that the fixed cost is a
    // small share of it.
    let mut probe = 100u64;
    let per_iter = loop {
        let took = load_seconds(&longer, probe);
        let script = (took - overhead).max(0.0);
        if script >= 0.5 {
            break script / probe as f64;
        }
        probe *= 2;
    };
    // A script of about 3.2 s: past the 2 s normal budget, well inside the 10 s import one.
    let iterations = ((((3.2 - overhead).max(2.5)) / per_iter).round() as u64).max(1);
    let html = loop_page(iterations);

    let (router, _app, _home, folder) = build();
    let body = import(&router, "slow.html", &html, false).await;

    // The answer says what today's does: saved, took too long just now, tried again.
    let description = body["effect"]["description"].as_str().unwrap_or_default();
    assert!(
        description.contains("was saved")
            && description.contains("took too long to load just now")
            && description.contains("will be tried again"),
        "the answer says what is known: {description}"
    );
    assert_eq!(body["effect"]["name"], "slow");
    // The save was not lost: the file is in the folder.
    assert_eq!(std::fs::read_dir(&folder).expect("the folder").count(), 1);
    assert_eq!(
        std::fs::read_to_string(folder.join("slow.html")).expect("the file is there"),
        html
    );
    // The rescan runs at the normal budget, so the file is in the failed list, not the library.
    assert!(
        !effect_ids(&router).await.contains(&"html:slow".to_string()),
        "a file the lights cannot load is not installed as loaded"
    );
    let failed = failed_effects(&router).await;
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].0.ends_with("slow.html"), "{failed:?}");
    assert!(
        failed[0].1.contains("took too long to load just now") && failed[0].1.contains("will be tried again"),
        "the failed list says why, in today's words: {failed:?}"
    );
}
