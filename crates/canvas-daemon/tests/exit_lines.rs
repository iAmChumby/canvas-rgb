//! #19: the real `canvas-rgbd` binary, run against a temporary home, leaves its closing line when
//! its API port is taken, and an elevated run touches no log folder at all.
//!
//! The API port comes from the temporary config, so this test never needs 127.0.0.1:9430 and
//! never reaches the user's running daemon. The OpenRGB port is a free port nothing listens on.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct TempHome(PathBuf);

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn log_text(home: &Path) -> String {
    let mut out = String::new();
    if let Ok(entries) = std::fs::read_dir(home.join("logs")) {
        for entry in entries.flatten() {
            out.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
        }
    }
    out
}

#[test]
fn a_taken_api_port_leaves_an_exiting_line_that_names_the_holder() {
    let home = std::env::temp_dir().join(format!("canvas-rgb-bindfail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let home = TempHome(home);

    // The stand-in: this test process holds the API port first.
    let holder = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = holder.local_addr().unwrap().port();
    let config = canvas_proto::config::ConfigFile {
        api_port: port,
        openrgb_port: free_port(),
        ..Default::default()
    };
    std::fs::write(home.0.join(canvas_proto::config::CONFIG_FILE), toml::to_string(&config).unwrap()).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_canvas-rgbd"))
        .env("CANVAS_RGB_HOME", &home.0)
        .env("CANVAS_RGB_OPENRGB", home.0.join("no-such-OpenRGB.exe"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(home.0.join("stderr.txt")).unwrap())
        .spawn()
        .expect("the daemon starts");

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            panic!("the daemon did not exit within 20 s of a taken API port");
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    if canvas_daemon::elevated::is_elevated() {
        // An elevated run refuses before it logs: stderr says why, and no log folder was made.
        assert!(!status.success());
        let stderr = std::fs::read_to_string(home.0.join("stderr.txt")).unwrap_or_default();
        assert!(stderr.contains(&canvas_daemon::elevated::ElevatedError.to_string()), "{stderr}");
        assert!(!home.0.join("logs").exists(), "an elevated daemon created a log folder");
        return;
    }

    assert_eq!(status.code(), Some(1), "a taken API port is exit code 1");
    let logs = log_text(&home.0);
    let closing = logs
        .lines()
        .find(|l| l.contains("canvas-rgbd is exiting"))
        .unwrap_or_else(|| panic!("no closing line in the log:\n{logs}"));
    assert!(closing.contains("ERROR") && closing.contains("code=1"), "{closing}");
    assert!(closing.contains("could not bind the API") && closing.contains("exiting"), "{closing}");
    // The holder is named: this test process is the only listener on the port.
    assert!(
        closing.contains(&format!("pid {}", std::process::id())),
        "the closing line names the holder (pid {}): {closing}",
        std::process::id()
    );
    drop(holder);
}
