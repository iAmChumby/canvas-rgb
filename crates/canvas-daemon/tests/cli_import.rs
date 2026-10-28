//! The `canvas-rgb effects import` command, end to end.
//!
//! The command talks to the daemon's API over the loopback, and the token only goes to
//! the process that really is the daemon: the CLI checks the holder of the port first.
//! So the fake API here runs in a child process whose image is named `canvas-rgbd.exe`,
//! which is what that check looks for.
//!
//! Nothing here touches the real daemon, the real lights or the owner's configuration:
//! the token file lives in a temporary home, and the fake API writes down what it was
//! sent.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// The env var that tells the child process to be the fake API.
const FAKE_ENV: &str = "CANVAS_RGB_TEST_FAKE_API";
/// The env var that tells the child process where to write down the request.
const LOG_ENV: &str = "CANVAS_RGB_TEST_FAKE_API_LOG";
/// The token the test uses.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// The name of the test that becomes the fake API in the child process.
const SERVER_TEST: &str = "the_fake_api_answers_the_import_route";

/// A temporary directory that is removed when it is dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-cli-import-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp directory is created");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The fake API. It only serves in the child process; in the parent it does nothing.
#[test]
fn the_fake_api_answers_the_import_route() {
    let Ok(port) = std::env::var(FAKE_ENV) else {
        return;
    };
    let port: u16 = port.parse().expect("the port is a number");
    let log = std::env::var(LOG_ENV).expect("the log path");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async move {
        let router = axum::Router::new().route(
            canvas_proto::api::routes::EFFECTS_IMPORT,
            axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
                let log = log.clone();
                async move {
                    let authorization = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let record = format!("{authorization}\n{body}");
                    let _ = std::fs::write(log, record);
                    axum::Json(serde_json::json!({
                        "effect": {
                            "id": "html:spark",
                            "name": "Spark",
                            "description": "",
                            "needs": { "screen": false, "audio": false },
                            "favorite": false,
                        },
                        "replaced": false,
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("the fake API binds");
        axum::serve(listener, router).await.expect("the fake API serves");
    });
}

/// The fake API, running in a child process.
struct FakeApi {
    child: Child,
    #[allow(dead_code)]
    dir: TempDir,
}

impl FakeApi {
    /// Starts the fake API on a free port and waits until it answers.
    fn start() -> (Self, u16, PathBuf) {
        let dir = TempDir::new("api");
        // The port is free now; the child takes it a moment later.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().expect("the local address").port();
        drop(listener);

        let log = dir.0.join("request.txt");
        // The image name is what the CLI's port check looks for.
        let image = dir.0.join("canvas-rgbd.exe");
        let exe = std::env::current_exe().expect("the test binary");
        std::fs::copy(&exe, &image).expect("the test binary is copied");

        let child = Command::new(&image)
            .arg("--exact")
            .arg(SERVER_TEST)
            .arg("--nocapture")
            .env(FAKE_ENV, port.to_string())
            .env(LOG_ENV, &log)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the fake API starts");

        let api = Self { child, dir };
        // Wait for the port to answer.
        for _ in 0..200 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return (api, port, log);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("the fake API never answered on the port");
    }
}

impl Drop for FakeApi {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The CLI's exit code and output for `effects import`.
fn run_import(home: &std::path::Path, port: u16, file: &std::path::Path) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_canvas-rgb-cli"))
        .arg("effects")
        .arg("import")
        .arg(file)
        .arg("--api")
        .arg(format!("http://127.0.0.1:{port}"))
        .env("CANVAS_RGB_HOME", home)
        .output()
        .expect("the CLI runs");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_import_command_sends_the_file_to_the_api() {
    let (api, port, log) = FakeApi::start();

    // The home holds the token file the CLI reads.
    let home = TempDir::new("home");
    std::fs::write(home.0.join("api-token"), TOKEN).expect("the token is written");

    // The file to import.
    let effect = home.0.join("spark.html");
    std::fs::write(&effect, "<html><title>Spark</title></html>").expect("the effect is written");

    let (code, stdout, stderr) = run_import(&home.0, port, &effect);
    assert_eq!(code, 0, "the command succeeds: {stdout} {stderr}");
    assert!(stdout.contains("imported html:spark"), "the answer is shown: {stdout}");

    // The API got the file, and the token.
    let record = std::fs::read_to_string(&log).expect("the request was written down");
    let (authorization, body) = record.split_once('\n').expect("the record has two parts");
    assert_eq!(authorization, format!("Bearer {TOKEN}"), "the token was sent");
    let request: canvas_proto::api::ImportEffect =
        serde_json::from_str(body).expect("the body is the import request");
    assert_eq!(request.file_name, "spark.html");
    assert_eq!(request.html, "<html><title>Spark</title></html>");
    assert!(!request.replace);
    drop(api);
}

#[test]
fn the_import_command_sends_the_replace_flag() {
    let (api, port, log) = FakeApi::start();

    let home = TempDir::new("home-replaced");
    std::fs::write(home.0.join("api-token"), TOKEN).expect("the token is written");
    let effect = home.0.join("spark.html");
    std::fs::write(&effect, "<html></html>").expect("the effect is written");

    let output = Command::new(env!("CARGO_BIN_EXE_canvas-rgb-cli"))
        .arg("effects")
        .arg("import")
        .arg(&effect)
        .arg("--replace")
        .arg("--api")
        .arg(format!("http://127.0.0.1:{port}"))
        .env("CANVAS_RGB_HOME", &home.0)
        .output()
        .expect("the CLI runs");
    assert_eq!(output.status.code().unwrap_or(-1), 0);

    let record = std::fs::read_to_string(&log).expect("the request was written down");
    let (_, body) = record.split_once('\n').expect("the record has two parts");
    let request: canvas_proto::api::ImportEffect =
        serde_json::from_str(body).expect("the body is the import request");
    assert!(request.replace, "the replace flag is sent");
    drop(api);
}

#[test]
fn the_import_command_refuses_a_file_over_the_limit() {
    let (api, port, log) = FakeApi::start();

    let home = TempDir::new("home-big");
    std::fs::write(home.0.join("api-token"), TOKEN).expect("the token is written");
    let effect = home.0.join("big.html");
    std::fs::write(
        &effect,
        "x".repeat(canvas_proto::consts::EFFECT_IMPORT_MAX_BYTES + 1),
    )
    .expect("the effect is written");

    let output = Command::new(env!("CARGO_BIN_EXE_canvas-rgb-cli"))
        .arg("effects")
        .arg("import")
        .arg(&effect)
        .arg("--api")
        .arg(format!("http://127.0.0.1:{port}"))
        .env("CANVAS_RGB_HOME", &home.0)
        .output()
        .expect("the CLI runs");
    assert_ne!(output.status.code().unwrap_or(-1), 0);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("the limit is"),
        "the message names the limit: {stderr}"
    );
    // The file was never posted.
    assert!(
        !log.exists(),
        "the API must not have been asked to take the file"
    );
    drop(api);
}

#[test]
fn the_import_command_refuses_a_file_it_cannot_read() {
    // No API is needed for this one: the file is read before anything is sent.
    let home = TempDir::new("home-missing");
    std::fs::write(home.0.join("api-token"), TOKEN).expect("the token is written");
    let missing = home.0.join("nope.html");

    let output = Command::new(env!("CARGO_BIN_EXE_canvas-rgb-cli"))
        .arg("effects")
        .arg("import")
        .arg(&missing)
        .arg("--api")
        .arg("http://127.0.0.1:1")
        .env("CANVAS_RGB_HOME", &home.0)
        .output()
        .expect("the CLI runs");
    assert_ne!(output.status.code().unwrap_or(-1), 0);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not read"),
        "the message says the file is missing: {stderr}"
    );
}
