//! `POST /api/v1/quit` makes the built daemon exit promptly with code 0 (G3). Run elevated, the
//! test instead checks that the daemon refuses to start.
//!
//! The test runs the real `canvas-rgbd` binary against a temporary home. The API port comes
//! from the temporary config (`exit_lines.rs` does the same), so the test never needs
//! 127.0.0.1:9430 and never touches a running daemon. The Host check wants 9430, so
//! requests carry an explicit `Host: 127.0.0.1:9430` header while connecting to the free
//! port. The OpenRGB port is likewise a free port, so the test never reaches a real
//! OpenRGB server.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const HOST_HEADER: &str = "127.0.0.1:9430";

/// The longest the daemon may take to exit after it answers the quit request.
const EXIT_DEADLINE: Duration = Duration::from_secs(3);

/// Kills the daemon when the test ends, even when an assertion fails first.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A temporary `CANVAS_RGB_HOME`, removed when the test ends.
struct TempHome(PathBuf);

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("an ephemeral port is free");
    listener.local_addr().expect("the listener has an address").port()
}

/// Every log file under the temporary home, for a failure message.
fn daemon_logs(home: &std::path::Path) -> String {
    let mut out = String::new();
    let mut dirs = vec![home.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "log") {
                out.push_str(&format!("{}:
{}
", path.display(), std::fs::read_to_string(&path).unwrap_or_default()));
            }
        }
    }
    out
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .build()
        .into()
}

/// How a start ended.
enum Start {
    /// The daemon's API answers on this port.
    Up(KillOnDrop, u16),
    /// An elevated run cannot start the daemon at all, and stderr says why.
    Elevated(String),
    /// The daemon ended before its API answered; this says what it left behind.
    Ended(String),
}

/// Starts the daemon on a fresh free port and waits for its API to answer.
///
/// The port is picked by binding `:0` and letting it go (`free_port`), so another process on
/// a loaded PC can take it before the daemon binds it (#328). That is why a start that ends
/// early is reported as [`Start::Ended`] and tried again on a fresh port: the verdict this
/// test reads is about the quit route, not about that race. The wait itself is bounded, so a
/// daemon that neither answers nor exits fails the test instead of holding it.
fn start(home: &Path) -> Start {
    let api_port = free_port();
    let config = canvas_proto::config::ConfigFile {
        api_port,
        openrgb_port: free_port(),
        ..Default::default()
    };
    std::fs::write(
        home.join(canvas_proto::config::CONFIG_FILE),
        toml::to_string(&config).expect("the config serialises"),
    )
    .expect("the config is written");

    let child = Command::new(env!("CARGO_BIN_EXE_canvas-rgbd"))
        .env("CANVAS_RGB_HOME", home)
        .env("CANVAS_RGB_OPENRGB", home.join("no-such-OpenRGB.exe"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        // stderr goes to a file in the temporary home, so a failed start says why.
        .stderr(std::fs::File::create(home.join("stderr.log")).expect("the stderr file is created"))
        .spawn()
        .expect("the daemon starts");
    let mut daemon = KillOnDrop(child);

    // Wait for the API to answer.
    let api_base = format!("http://127.0.0.1:{api_port}");
    let agent = agent();
    let started = Instant::now();
    loop {
        if agent
            .get(&format!("{api_base}/api/v1/status"))
            .header("Host", HOST_HEADER)
            .call()
            .is_ok()
        {
            return Start::Up(daemon, api_port);
        }
        if let Some(status) = daemon.0.try_wait().expect("the child can be polled") {
            let stderr = std::fs::read_to_string(home.join("stderr.log")).unwrap_or_default();
            // An elevated test run (a CI runner is an elevated admin) cannot start the daemon: it
            // refuses an elevated token by design. Then the refusal itself is what is checked.
            if canvas_daemon::elevated::is_elevated() {
                assert!(!status.success(), "an elevated daemon must exit with an error");
                return Start::Elevated(stderr);
            }
            let logs = daemon_logs(home) + &daemon_logs(&canvas_proto::consts::user_log_dir());
            return Start::Ended(format!(
                "{status}
--- stderr ---
{stderr}
--- logs ---
{logs}"
            ));
        }
        assert!(started.elapsed() < Duration::from_secs(15), "the API did not start in 15 s");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn quit_makes_the_daemon_exit_with_code_zero_within_three_seconds() {
    let home = std::env::temp_dir().join(format!("canvas-rgb-quit-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temporary home is created");
    let home = TempHome(home);

    // The API and OpenRGB ports are free ports nothing listens on, so no real daemon or
    // server is touched (#88). A free port is taken by the daemon a moment later, so one that
    // another process took first is not this test's verdict (#328).
    let mut attempt = 1;
    let (mut daemon, api_port) = loop {
        match start(&home.0) {
            Start::Up(daemon, port) => break (daemon, port),
            Start::Elevated(stderr) => {
                let expected = canvas_daemon::elevated::ElevatedError.to_string();
                assert!(stderr.contains(&expected), "expected the elevation refusal, got: {stderr}");
                return;
            }
            Start::Ended(why) if attempt < 3 => {
                eprintln!("quit: start {attempt} ended before its API answered; a fresh port:");
                eprintln!("{why}");
                attempt += 1;
            }
            Start::Ended(why) => panic!("the daemon did not start on three free ports:\n{why}"),
        }
    };
    let api_base = format!("http://127.0.0.1:{api_port}");

    let token = std::fs::read_to_string(home.0.join(canvas_proto::consts::TOKEN_FILE))
        .expect("the daemon wrote the token file")
        .trim()
        .to_string();

    let agent = agent();
    let response = agent
        .post(&format!("{api_base}/api/v1/quit"))
        .header("Host", HOST_HEADER)
        .header("Authorization", &format!("Bearer {token}"))
        .send_empty()
        .expect("the quit request is answered");
    assert_eq!(response.status(), 200, "the quit request is accepted");

    // The process must exit on its own, and promptly.
    let answered = Instant::now();
    loop {
        if let Some(status) = daemon.0.try_wait().expect("the child can be polled") {
            assert!(status.success(), "the daemon exits with code 0, got {status}");
            let took = answered.elapsed();
            assert!(took <= EXIT_DEADLINE, "the daemon took {took:?} to exit");
            break;
        }
        if answered.elapsed() > EXIT_DEADLINE {
            panic!("the daemon is still running {EXIT_DEADLINE:?} after the quit request");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
