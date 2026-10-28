//! `POST /api/v1/quit` makes the built daemon exit promptly with code 0 (G3).
//!
//! The test runs the real `canvas-rgbd` binary against a temporary home. The API must
//! use 127.0.0.1:9430 (the Host check allows only that), so the test refuses to run when
//! something already holds that port: it never talks to the user's running daemon. The
//! OpenRGB port comes from the temporary config, so the test never reaches a real
//! OpenRGB server.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const API_BASE: &str = "http://127.0.0.1:9430";

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

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(2)))
        .build()
        .into()
}

#[test]
fn quit_makes_the_daemon_exit_with_code_zero_within_three_seconds() {
    assert!(
        TcpListener::bind("127.0.0.1:9430").is_ok(),
        "127.0.0.1:9430 is busy: stop canvas-rgbd before running this test"
    );

    let home = std::env::temp_dir().join(format!("canvas-rgb-quit-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the temporary home is created");
    let home = TempHome(home);

    // The OpenRGB port is a free port nothing listens on, so no real server is touched.
    let config = canvas_proto::config::ConfigFile {
        openrgb_port: free_port(),
        ..Default::default()
    };
    std::fs::write(
        home.0.join(canvas_proto::config::CONFIG_FILE),
        toml::to_string(&config).expect("the config serialises"),
    )
    .expect("the config is written");

    let child = Command::new(env!("CARGO_BIN_EXE_canvas-rgbd"))
        .env("CANVAS_RGB_HOME", &home.0)
        .env("CANVAS_RGB_OPENRGB", home.0.join("no-such-OpenRGB.exe"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the daemon starts");
    let mut daemon = KillOnDrop(child);

    // Wait for the API to answer.
    let agent = agent();
    let started = Instant::now();
    loop {
        if agent.get(&format!("{API_BASE}/api/v1/status")).call().is_ok() {
            break;
        }
        if let Some(status) = daemon.0.try_wait().expect("the child can be polled") {
            panic!("the daemon exited before the API answered: {status}");
        }
        assert!(started.elapsed() < Duration::from_secs(15), "the API did not start in 15 s");
        std::thread::sleep(Duration::from_millis(100));
    }

    let token = std::fs::read_to_string(home.0.join(canvas_proto::consts::TOKEN_FILE))
        .expect("the daemon wrote the token file")
        .trim()
        .to_string();

    let response = agent
        .post(&format!("{API_BASE}/api/v1/quit"))
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
