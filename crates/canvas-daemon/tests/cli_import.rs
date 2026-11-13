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
//!
//! # Bounds (#328)
//!
//! Every process this file starts is bounded, so a test here can never hold a build slot
//! for minutes: one did, for the full 20 minutes of the lock. The fake API ends itself
//! after [`FAKE_LIFETIME`] if the test dies first, a CLI run is killed at [`CLI_LIMIT`],
//! and the wait for the fake's port gives up at [`FAKE_START_LIMIT`].
//!
//! The bounds are generous because a loaded PC is slow in two measured ways, both worse
//! than the work:
//!
//! - the fake's image is a copy of this test binary, and one took 36 s to start and bind
//!   its port while eight other threads kept the cores busy;
//! - a loopback connect can be dropped by the machine under that load. One CLI request
//!   in a 30-run loop spent 21.06 s in the connect and then failed with the operating
//!   system's own timeout (`timeout: connect`, WSAETIMEDOUT) while the fake was up and
//!   listening; another run of the same loop took 65.9 s to be answered and succeeded.
//!   The CLI bounds a request now (5 s to connect, 120 s in all) and reports a failed
//!   request as "nothing answers on", so a dropped connect is exactly that message.
//!
//! A run that ends that way is repeated, up to [`CLI_ATTEMPTS`] times, with each repeat
//! written to stderr. It is the machine's network stack, not the daemon and not a wrong
//! port: the CLI's own port check passed, the fake is still listening, and a repeat sends
//! the same file, which the daemon imports under the same name.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The env var that tells the child process to be the fake API. Its value is the path of
/// the file the child writes its port to, once its listener is bound.
const FAKE_ENV: &str = "CANVAS_RGB_TEST_FAKE_API";
/// The env var that tells the child process where to write down the request.
const LOG_ENV: &str = "CANVAS_RGB_TEST_FAKE_API_LOG";
/// The token the test uses.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// The name of the test that becomes the fake API in the child process.
const SERVER_TEST: &str = "the_fake_api_answers_the_import_route";
/// The most one CLI run may take. A run takes about 1.4 s; the slowest seen on a loaded PC
/// was 65.9 s, and it succeeded. This is a hang guard, not a performance bar.
const CLI_LIMIT: Duration = Duration::from_secs(120);
/// The most the fake API may take to publish its port and answer on it. Starting a copy of
/// this test binary took 36 s on a loaded PC, so this is a hang guard too.
const FAKE_START_LIMIT: Duration = Duration::from_secs(90);
/// How long the fake API keeps serving if the test that started it is killed before it can
/// stop it. Comfortably longer than the slowest test here, and short enough that a leaked
/// child cannot sit on a port for the session.
const FAKE_LIFETIME: Duration = Duration::from_secs(600);
/// How many times a CLI run is made when the machine drops its connection. The operating
/// system gives up on a dropped connect after about 21 s, and the CLI gives up after 5 s, so
/// eight attempts cover two such windows.
const CLI_ATTEMPTS: u32 = 8;
/// What the CLI prints, with exit code 2, when its request failed (`cli.rs`, `NotRunning`).
const CONNECT_DROPPED: &str = "nothing answers on";

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
///
/// The child binds its port itself (`:0`) and tells the parent which one it got, so no
/// port is ever picked and then given up before something takes it.
#[test]
fn the_fake_api_answers_the_import_route() {
    let Ok(port_file) = std::env::var(FAKE_ENV) else {
        return;
    };
    let log = std::env::var(LOG_ENV).expect("the log path");

    // If the test that started this process is killed before it can kill this one, the
    // fake must not outlive it.
    std::thread::spawn(|| {
        std::thread::sleep(FAKE_LIFETIME);
        std::process::exit(3);
    });

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
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("the fake API binds");
        let port = listener.local_addr().expect("the local address").port();
        // Written whole under another name, then renamed, so the parent never reads half.
        let pending = format!("{port_file}.part");
        std::fs::write(&pending, port.to_string()).expect("the port is written down");
        std::fs::rename(&pending, &port_file).expect("the port file is published");
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
    /// Starts the fake API and waits until it has bound a port and answers on it.
    fn start() -> (Self, u16, PathBuf) {
        let dir = TempDir::new("api");
        let log = dir.0.join("request.txt");
        let port_file = dir.0.join("port.txt");
        // The image name is what the CLI's port check looks for.
        let image = dir.0.join("canvas-rgbd.exe");
        let exe = std::env::current_exe().expect("the test binary");
        std::fs::copy(&exe, &image).expect("the test binary is copied");

        let child = Command::new(&image)
            .arg("--exact")
            .arg(SERVER_TEST)
            .arg("--nocapture")
            .env(FAKE_ENV, &port_file)
            .env(LOG_ENV, &log)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the fake API starts");

        let mut api = Self { child, dir };
        let port = api.wait_for_port(&port_file);
        (api, port, log)
    }

    /// Waits for the child to say which port it bound, and for that port to answer.
    fn wait_for_port(&mut self, port_file: &Path) -> u16 {
        let started = Instant::now();
        loop {
            if let Ok(text) = std::fs::read_to_string(port_file) {
                let port: u16 = text.trim().parse().expect("the port file holds a port");
                let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
                // A connect the machine drops costs the operating system's timeout, so this
                // waits a bounded one at a time.
                if std::net::TcpStream::connect_timeout(&address, Duration::from_secs(5)).is_ok() {
                    return port;
                }
            }
            if let Some(status) = self.child.try_wait().expect("the fake API can be asked") {
                panic!("the fake API ended before it answered ({status})");
            }
            if started.elapsed() >= FAKE_START_LIMIT {
                panic!("the fake API did not answer within {FAKE_START_LIMIT:?}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for FakeApi {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Runs the CLI with `args` and a temporary home, and returns its exit code, stdout and
/// stderr. A run that has not ended after [`CLI_LIMIT`] is killed and fails the test. A run
/// that ends because the machine dropped its connection (see the file's header) is made
/// again, up to [`CLI_ATTEMPTS`] times in all.
fn run_cli(home: &Path, args: &[&std::ffi::OsStr]) -> (i32, String, String) {
    let mut attempt = 1;
    loop {
        let (code, stdout, stderr) = run_cli_once(home, args);
        if code == 2 && stderr.contains(CONNECT_DROPPED) && attempt < CLI_ATTEMPTS {
            eprintln!(
                "cli_import: attempt {attempt} of {CLI_ATTEMPTS}: the machine dropped the connect: {}",
                stderr.trim()
            );
            attempt += 1;
            continue;
        }
        return (code, stdout, stderr);
    }
}

/// One CLI run, ended after [`CLI_LIMIT`] at the latest.
fn run_cli_once(home: &Path, args: &[&std::ffi::OsStr]) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_canvas-rgb-cli"))
        .args(args)
        .env("CANVAS_RGB_HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the CLI runs");
    // Read both pipes while the child runs, so a full pipe never stalls it.
    let mut out = child.stdout.take().expect("stdout is piped");
    let mut err = child.stderr.take().expect("stderr is piped");
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = out.read_to_string(&mut text);
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = err.read_to_string(&mut text);
        text
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("the CLI can be asked") {
            break status;
        }
        if started.elapsed() >= CLI_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            let stdout = out.join().unwrap_or_default();
            let stderr = err.join().unwrap_or_default();
            panic!("the CLI was still running after {CLI_LIMIT:?} and was killed: {stdout} {stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    (
        status.code().unwrap_or(-1),
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    )
}

/// The CLI's exit code and output for `effects import <file>` against `port`.
fn run_import(home: &Path, port: u16, file: &Path) -> (i32, String, String) {
    run_import_with(home, port, file, &[])
}

/// As [`run_import`], with extra flags before `--api`.
fn run_import_with(home: &Path, port: u16, file: &Path, flags: &[&str]) -> (i32, String, String) {
    let api = format!("http://127.0.0.1:{port}");
    let mut args: Vec<&std::ffi::OsStr> = vec!["effects".as_ref(), "import".as_ref(), file.as_os_str()];
    args.extend(flags.iter().map(|flag| -> &std::ffi::OsStr { flag.as_ref() }));
    args.push("--api".as_ref());
    args.push(api.as_ref());
    run_cli(home, &args)
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

    let (code, stdout, stderr) = run_import_with(&home.0, port, &effect, &["--replace"]);
    assert_eq!(code, 0, "the command succeeds: {stdout} {stderr}");

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

    let (code, _stdout, stderr) = run_import(&home.0, port, &effect);
    assert_ne!(code, 0);
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

    let (code, _stdout, stderr) = run_import(&home.0, 1, &missing);
    assert_ne!(code, 0);
    assert!(
        stderr.contains("could not read"),
        "the message says the file is missing: {stderr}"
    );
}
