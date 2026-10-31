//! Startup ordering: the machine home is secured before anything is read or spawned, and an
//! unsecured home means no config read, no chip, and no OpenRGB spawn.

use crate::app::{build_core, check_confirmation, secure_boot, Deps};
use crate::fakes::{FakeLauncher, FakeProcesses, FakeSpawner};
use crate::paths;
use crate::secure::SecureDir;
use crate::store;
use canvas_proto::config::FansFile;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Records each call so tests can see what ran, and when.
struct RecordingSecure {
    calls: Arc<Mutex<Vec<String>>>,
    fail: bool,
}

impl SecureDir for RecordingSecure {
    fn secure(&mut self, dir: &Path) -> io::Result<()> {
        self.calls.lock().unwrap().push(format!("secure {}", dir.display()));
        if self.fail {
            Err(io::Error::other("no privilege to set the owner"))
        } else {
            Ok(())
        }
    }

    fn trusted(&self, _path: &Path) -> bool {
        !self.fail
    }
}

fn deps(openrgb: &FakeProcesses, daemon: &FakeProcesses, install: &Path) -> Deps {
    Deps {
        spawner: Box::new(FakeSpawner(openrgb.clone())),
        launcher: Box::new(FakeLauncher(daemon.clone())),
        adapter: Box::new(|| Box::new(canvas_adapters::openrgb::OpenRgbAdapter::new("127.0.0.1", 1))),
        install_dir: install.to_path_buf(),
        updates: None,
    }
}

#[test]
fn the_secure_step_runs_first_and_its_failure_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let ok = secure_boot(dir.path(), Box::new(RecordingSecure { calls: Arc::clone(&calls), fail: false }));
    assert!(ok.secured());
    assert_eq!(calls.lock().unwrap().len(), 1, "exactly one secure call, before any Boot exists");

    let failed = secure_boot(dir.path(), Box::new(RecordingSecure { calls: Arc::clone(&calls), fail: true }));
    assert!(!failed.secured());
    assert!(failed.secure_error().unwrap().contains("no privilege"));
}

#[tokio::test]
async fn an_unsecured_home_is_not_read_and_no_openrgb_is_spawned() {
    let home = tempfile::tempdir().unwrap();
    // A saved profile that a read would apply. An unsecured boot must not see it.
    let file = FansFile { active_profile: "Quiet".into(), ..FansFile::default() };
    store::save_fans(&paths::fans_path(home.path()), &file).unwrap();

    let calls = Arc::new(Mutex::new(Vec::new()));
    let boot = secure_boot(home.path(), Box::new(RecordingSecure { calls, fail: true }));
    let openrgb = FakeProcesses::new();
    let daemon = FakeProcesses::new();
    let install = tempfile::tempdir().unwrap();
    let mut core = build_core(&boot, deps(&openrgb, &daemon, install.path()));

    assert_eq!(core.fans.status().active_profile, "Default", "fans.toml was not read");
    assert!(core.fans.status().chip.is_none(), "the chip was not opened");
    core.tick(Instant::now()).await;
    core.tick(Instant::now()).await;
    assert!(openrgb.spawns().is_empty(), "OpenRGB was never spawned");
}

#[test]
fn a_confirmation_must_name_exactly_the_detected_map() {
    assert!(check_confirmation("msi_alt1", "msi_alt1").is_ok());
    let err = check_confirmation("msi_alt1", "default").unwrap_err();
    assert!(err.contains("detected channel map is msi_alt1"), "{err}");
}

/// The run loop production uses (`run` builds the core, then calls `run_core`) must start the thread
/// that finishes a pending update, or an update whose setup stopped the old service never finishes.
#[tokio::test(flavor = "multi_thread")]
async fn the_production_run_loop_resumes_a_pending_update_on_start() {
    use crate::update_install_tests::{rig, wait_until};
    let rig = rig("0.1.1");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    rig.leave_pending(0, 0, now);
    assert!(rig.marker_exists());
    rig.write_result(true, "0.1.1");

    let fan_home = tempfile::tempdir().unwrap();
    let mut core = crate::pipe_server::tests::test_core(fan_home.path().to_path_buf());
    core.updates = Some(Arc::clone(&rig.installer));
    let name = format!(r"\\.\pipe\canvas-rgb-svc-test-run-core-{}", std::process::id());
    let first = crate::pipe_server::create_instance(&name, true).unwrap();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (_power_tx, power_rx) = tokio::sync::mpsc::unbounded_channel();
    let run = tokio::spawn(crate::app::run_core(
        Arc::new(tokio::sync::Mutex::new(core)),
        name,
        first,
        async move {
            let _ = stop_rx.await;
        },
        power_rx,
    ));

    wait_until("the pending update to be finished by the run loop", || rig.commands().len() == 2 && !rig.marker_exists());
    assert!(rig.staged_files().is_empty(), "{:?}", rig.staged_files());
    let _ = stop_tx.send(());
    tokio::time::timeout(std::time::Duration::from_secs(10), run).await.expect("the run loop stops").unwrap().unwrap();
}

#[test]
fn a_service_without_an_installer_starts_no_resume_thread() {
    assert!(crate::app::start_update_resume(None).is_none());
}
