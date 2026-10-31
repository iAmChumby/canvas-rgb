//! Update tests, all on fakes: the rules and files as plain functions, and the plugin against a fake
//! release server on 127.0.0.1 (tauri's mock runtime, no window, no installer, no real network).
//! The keys under `tests/fixtures/update` are test-only.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;

use canvas_proto::tauri::{CheckFailReason, UpdateInfo, UpdatePhase, UpdateState};
use semver::Version;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};
use time::OffsetDateTime;
use url::Url;

use super::launch::{self, LaunchError, RunError, Started};
use super::rules::{self, DownloadPolicy};
use super::state::{self, LastResult, Stored};
use super::*;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/update");

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(FIXTURES).join(name)).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

fn fixture_text(name: &str) -> String {
    String::from_utf8(fixture(name)).unwrap().trim().to_string()
}

fn v(s: &str) -> Version {
    s.parse().unwrap()
}

fn at(s: &str) -> OffsetDateTime {
    state::parse(s).unwrap()
}

// ---- a temp folder that cleans up after itself ----

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("canvas-rgb-update-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `log::append` writes under %LOCALAPPDATA%; point that at a temp folder for the whole test run.
fn isolate_log() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-update-test-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LOCALAPPDATA", dir);
    });
}

// ---- a fake release server ----

#[derive(Clone)]
struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// Send the body in pieces of this size, pausing between them.
    chunk: Option<usize>,
}

impl Resp {
    fn ok(body: impl Into<Vec<u8>>) -> Self {
        Resp { status: 200, headers: vec![], body: body.into(), chunk: None }
    }
    fn status(status: u16) -> Self {
        Resp { status, headers: vec![], body: b"{}".to_vec(), chunk: None }
    }
    fn chunked(mut self, size: usize) -> Self {
        self.chunk = Some(size);
        self
    }
    fn header(mut self, k: &str, val: &str) -> Self {
        self.headers.push((k.into(), val.into()));
        self
    }
}

struct FakeServer {
    port: u16,
    routes: Arc<Mutex<HashMap<String, Resp>>>,
    hits: Arc<Mutex<HashMap<String, u32>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Arc<Mutex<HashMap<String, Resp>>> = Arc::default();
        let hits: Arc<Mutex<HashMap<String, u32>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (r, h, s) = (routes.clone(), hits.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(stream) = stream {
                    let (r, h) = (r.clone(), h.clone());
                    std::thread::spawn(move || serve(stream, &r, &h));
                }
            }
        });
        FakeServer { port, routes, hits, stop, thread: Some(thread) }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn set(&self, path: &str, resp: Resp) {
        self.routes.lock().unwrap().insert(path.to_string(), resp);
    }

    fn hits(&self, path: &str) -> u32 {
        *self.hits.lock().unwrap().get(path).unwrap_or(&0)
    }

    /// Serves a manifest announcing `version` plus the installer at `/setup.exe`.
    fn release(&self, version: &str, signature: &str, payload: Vec<u8>) {
        let manifest = serde_json::json!({
            "version": version,
            "notes": "### Added\n\n- Something new.",
            "pub_date": "2026-10-29T12:00:00Z",
            "platforms": { rules::TARGET: { "signature": signature, "url": self.url("/setup.exe") } },
        });
        self.set("/latest.json", Resp::ok(manifest.to_string()));
        self.set("/setup.exe", Resp::ok(payload));
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(mut stream: TcpStream, routes: &Mutex<HashMap<String, Resp>>, hits: &Mutex<HashMap<String, u32>>) {
    let mut buf = [0u8; 4096];
    let mut request = Vec::new();
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let line = String::from_utf8_lossy(&request);
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    *hits.lock().unwrap().entry(path.clone()).or_insert(0) += 1;
    let resp = routes.lock().unwrap().get(&path).cloned().unwrap_or_else(|| Resp::status(404));
    let mut out = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", resp.status, resp.body.len());
    for (k, val) in &resp.headers {
        out.push_str(&format!("{k}: {val}\r\n"));
    }
    out.push_str("\r\n");
    let _ = stream.write_all(out.as_bytes());
    match resp.chunk {
        None => {
            let _ = stream.write_all(&resp.body);
        }
        Some(size) => {
            for piece in resp.body.chunks(size) {
                let _ = stream.write_all(piece);
                let _ = stream.flush();
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        }
    }
}

// ---- the controller on the mock runtime ----

#[derive(Clone, Default)]
struct LaunchSeen {
    calls: Arc<Mutex<Vec<Seen>>>,
}

#[derive(Debug, Clone)]
struct Seen {
    pending_in_state_file: Option<String>,
    write_blocked: bool,
    delete_blocked: bool,
}

#[derive(Clone, Copy)]
enum Launch {
    /// Reports a started installer (a handle the tests never wait on).
    Starts,
    /// Reports a started installer with nothing to follow.
    NoHandle,
    /// Starts a harmless real program and hands over its process handle.
    Process,
    Declined,
    Fails,
}

struct Harness {
    _app: tauri::App<MockRuntime>,
    handle: tauri::AppHandle<MockRuntime>,
    ctl: Arc<Controller>,
    server: FakeServer,
    dir: TempDir,
    states: Arc<Mutex<Vec<UpdateState>>>,
    seen: LaunchSeen,
}

/// Starts the stub (whoami prints its help and exits) and gives its process handle away.
fn spawn_stub(stub: &Path) -> Result<Started, RunError> {
    use std::os::windows::io::IntoRawHandle;
    let child = std::process::Command::new(stub)
        .arg("/?")
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|e| RunError::Failed(e.to_string()))?;
    Ok(Some(launch::ProcessHandle(child.into_raw_handle() as isize)))
}

fn harness(current: &str, pubkey_file: &str, how: Launch) -> Harness {
    isolate_log();
    let server = FakeServer::start();
    let dir = TempDir::new();
    let settings = Settings {
        endpoint: server.url("/latest.json").parse().unwrap(),
        pubkey: fixture_text(pubkey_file),
        policy: DownloadPolicy { https_only: false, hosts: vec!["127.0.0.1".into()] },
        updates_dir: dir.path().join("updates"),
        state_path: dir.path().join("update.json"),
        result_path: dir.path().join("update-result.json"),
        guard_root: dir.path().to_path_buf(),
        progress_every: Duration::ZERO,
    };

    let mut ctx = mock_context(noop_assets());
    ctx.package_info_mut().version = v(current);
    ctx.config_mut().plugins.0.insert(
        "updater".to_string(),
        serde_json::json!({
            "pubkey": settings.pubkey,
            "endpoints": [],
            "dangerousInsecureTransportProtocol": true,
            "requireSignedVersion": true,
        }),
    );
    let app = mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(ctx)
        .unwrap();
    let handle = app.handle().clone();

    let seen = LaunchSeen::default();
    let (seen2, state_path) = (seen.clone(), settings.state_path.clone());
    let stub = dir.path().join("stub.exe");
    #[cfg(windows)]
    {
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        std::fs::copy(system.join("System32").join("whoami.exe"), &stub).unwrap();
    }
    let launcher: Launcher = Arc::new(move |path: &Path| {
        // What the editor and the next start can rely on at the moment the installer starts.
        let pending = state::load(&state_path).pending.map(|p| p.version);
        let write_blocked = std::fs::OpenOptions::new().write(true).open(path).is_err();
        let delete_blocked = std::fs::remove_file(path).is_err();
        seen2.calls.lock().unwrap().push(Seen { pending_in_state_file: pending, write_blocked, delete_blocked });
        match how {
            Launch::Starts => Ok::<Started, RunError>(Some(launch::ProcessHandle(0))),
            Launch::NoHandle => Ok(None),
            Launch::Process => spawn_stub(&stub),
            Launch::Declined => Err(RunError::Declined),
            Launch::Fails => Err(RunError::Failed("boom".into())),
        }
    });
    let states: Arc<Mutex<Vec<UpdateState>>> = Arc::default();
    let sink = states.clone();
    let ctl = Arc::new(Controller::new(
        settings,
        v(current),
        launcher,
        Box::new(move |s: &UpdateState| sink.lock().unwrap().push(s.clone())),
    ));
    Harness { _app: app, handle, ctl, server, dir, states, seen }
}

impl Harness {
    fn check(&self, manual: bool) -> UpdateState {
        tauri::async_runtime::block_on(self.ctl.check(&self.handle, manual))
    }
    fn install(&self) -> UpdateState {
        tauri::async_runtime::block_on(self.ctl.install())
    }
    fn stored(&self) -> Stored {
        state::load(&self.dir.path().join("update.json"))
    }
    fn installer(&self) -> PathBuf {
        self.dir.path().join("updates").join("canvas-rgb-setup-0.1.1.exe")
    }
    fn phases(&self) -> Vec<UpdatePhase> {
        self.states.lock().unwrap().iter().map(|s| s.phase.clone()).collect()
    }
}

fn good_release(h: &Harness) {
    h.server.release("0.1.1", &fixture_text("payload.bin.sig"), fixture("payload.bin"));
}

// ---- version rules ----

#[test]
fn version_rule_needs_strictly_newer_stable() {
    let cur = v("0.1.0");
    assert!(!rules::is_update(&cur, &v("0.1.0")), "equal");
    assert!(!rules::is_update(&cur, &v("0.0.9")), "older");
    assert!(!rules::is_update(&cur, &v("0.2.0-beta.1")), "newer pre-release");
    assert!(!rules::is_update(&cur, &v("0.1.0-rc.1")), "pre-release of the running version");
    assert!(!rules::is_update(&cur, &v("0.1.0+build5")), "build metadata does not make it newer");
    assert!(rules::is_update(&cur, &v("0.1.1")), "patch");
    assert!(rules::is_update(&cur, &v("0.2.0")), "minor");
    assert!(rules::is_update(&cur, &v("1.0.0+abc")), "newer with build metadata");
}

#[test]
fn installer_file_name_uses_the_three_numbers_only() {
    assert_eq!(rules::installer_file_name(&v("0.1.1")), "canvas-rgb-setup-0.1.1.exe");
    assert_eq!(rules::installer_file_name(&v("1.2.3+evil")), "canvas-rgb-setup-1.2.3.exe");
}

// ---- download host rules ----

#[test]
fn production_policy_allows_the_three_hosts_over_https() {
    let p = DownloadPolicy::production();
    for host in ["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"] {
        let url: Url = format!("https://{host}/iAmChumby/canvas-rgb/releases/download/v0.1.1/canvas-rgb-setup-0.1.1.exe").parse().unwrap();
        assert!(p.check(&url).is_ok(), "{host}");
    }
}

#[test]
fn production_policy_refuses_everything_else() {
    let p = DownloadPolicy::production();
    for bad in [
        "http://github.com/x.exe",
        "https://example.com/x.exe",
        "https://github.com.evil.com/x.exe",
        "https://evilgithub.com/x.exe",
        "https://raw.githubusercontent.com/x.exe",
        "https://github.com:8443/x.exe",
        "https://github.com@evil.com/x.exe",
        "https://user:pw@github.com/x.exe",
        "file:///C:/x.exe",
        "ftp://github.com/x.exe",
    ] {
        let url: Url = bad.parse().unwrap();
        assert!(p.check(&url).is_err(), "{bad} should be refused");
    }
}

#[test]
fn injected_policy_takes_a_loopback_host() {
    let p = DownloadPolicy { https_only: false, hosts: vec!["127.0.0.1".into()] };
    assert!(p.check(&"http://127.0.0.1:9999/x".parse().unwrap()).is_ok());
    assert!(p.check(&"http://localhost:9999/x".parse().unwrap()).is_err());
}

// ---- the state file ----

#[test]
fn state_file_round_trips() {
    let dir = TempDir::new();
    let path = dir.path().join("tray").join("update.json");
    let stored = Stored {
        auto_check: false,
        last_check: Some("2026-10-29T12:00:00Z".into()),
        last_attempt: Some("2026-10-30T12:00:00Z".into()),
        last_result: Some(LastResult::Failed { reason: CheckFailReason::RateLimited }),
        skipped_version: Some("0.1.1".into()),
        snoozed_at: Some("2026-10-29T13:00:00Z".into()),
        pending: Some(info("0.1.2")),
        updated_to: Some(info("0.1.0")),
    };
    state::save(&path, &stored).unwrap();
    assert_eq!(state::load(&path), stored);
    assert!(!path.with_extension("json.tmp").exists(), "no temp file left behind");
}

#[test]
fn missing_corrupt_and_partial_state_files_are_defaults() {
    isolate_log();
    let dir = TempDir::new();
    let path = dir.path().join("update.json");
    assert_eq!(state::load(&path), Stored::default());
    assert!(Stored::default().auto_check, "automatic check is on by default");
    std::fs::write(&path, "{ not json").unwrap();
    assert_eq!(state::load(&path), Stored::default());
    std::fs::write(&path, r#"{"skipped_version":"0.3.0","future_field":1}"#).unwrap();
    let loaded = state::load(&path);
    assert_eq!(loaded.skipped_version.as_deref(), Some("0.3.0"));
    assert!(loaded.auto_check, "fields missing from the file take their defaults");
}

// ---- the schedule ----

#[test]
fn automatic_check_runs_once_a_day() {
    let mut s = Stored::default();
    let now = at("2026-10-29T12:00:00Z");
    assert!(state::auto_check_due(&s, now), "never checked");
    s.last_check = Some("2026-10-28T12:00:01Z".into());
    assert!(!state::auto_check_due(&s, now), "23 h 59 m 59 s ago");
    s.last_check = Some("2026-10-28T12:00:00Z".into());
    assert!(state::auto_check_due(&s, now), "exactly 24 h ago");
    s.last_check = Some("2026-10-29T11:59:00Z".into());
    assert!(!state::auto_check_due(&s, now), "a minute ago");
    s.last_check = Some("2026-10-30T00:00:00Z".into());
    assert!(state::auto_check_due(&s, now), "a clock that moved back");
    s.last_check = Some("garbage".into());
    assert!(state::auto_check_due(&s, now), "unreadable time");
}

#[test]
fn the_off_switch_stops_the_automatic_check_only() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.ctl.set_auto(false);
    assert!(!h.ctl.auto_due(OffsetDateTime::now_utc()), "never checked, but the switch is off");
    let state = h.check(true);
    assert_eq!(state.phase, UpdatePhase::Available, "Check now works with the switch off");
    assert!(!state.auto_check);
    assert_eq!(h.server.hits("/latest.json"), 1);
    h.ctl.set_auto(true);
    assert!(!h.ctl.auto_due(OffsetDateTime::now_utc()), "just checked");
}

#[test]
fn a_finished_check_and_a_failed_automatic_one_both_start_the_day() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    assert!(h.ctl.auto_due(OffsetDateTime::now_utc()));
    h.server.set("/latest.json", Resp::status(500));
    let s = h.check(false);
    assert!(matches!(s.phase, UpdatePhase::CheckFailed { .. }));
    assert!(s.last_check.is_none(), "a failed check is not a check");
    assert!(!h.ctl.auto_due(OffsetDateTime::now_utc()), "no automatic retry now");
    assert!(!h.ctl.auto_due(OffsetDateTime::now_utc() + time::Duration::hours(23)), "none within 23 h");
    assert!(h.ctl.auto_due(OffsetDateTime::now_utc() + time::Duration::hours(25)), "due again after the day");
    assert!(h.stored().last_attempt.is_some(), "the attempt is remembered across restarts");
    good_release(&h);
    assert_eq!(h.check(true).phase, UpdatePhase::Available, "Check now ignores the wait");
    assert!(h.stored().last_check.is_some());
    assert!(!h.ctl.auto_due(OffsetDateTime::now_utc()));

    // A failed Check now does not hold the automatic check back.
    let h2 = harness("0.1.0", "test.key.pub", Launch::Starts);
    h2.server.set("/latest.json", Resp::status(500));
    h2.check(true);
    assert!(h2.ctl.auto_due(OffsetDateTime::now_utc()), "Check now is unaffected by the daily limit");
}

#[test]
fn later_ends_at_the_next_automatic_check_or_after_a_day() {
    let mut s = Stored::default();
    let t0 = at("2026-10-29T12:00:00Z");
    assert!(!state::snoozed(&s, t0));
    s.snoozed_at = Some(state::rfc3339(t0));
    assert!(state::snoozed(&s, at("2026-10-30T11:59:59Z")));
    assert!(!state::snoozed(&s, at("2026-10-30T12:00:00Z")), "24 h later it ends by itself");

    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    assert!(h.check(true).phase == UpdatePhase::Available);
    assert!(h.ctl.later().snoozed);
    assert!(!h.check(false).snoozed, "the next automatic check ends it");
    h.ctl.later();
    assert!(!h.check(true).snoozed, "a Check now that finds a version ends it");
}

#[test]
fn up_to_date_check_now_keeps_later() {
    let h = harness("0.1.1", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.ctl.later();
    let s = h.check(true);
    assert_eq!(s.phase, UpdatePhase::UpToDate);
    assert!(s.snoozed);
}

#[test]
fn skip_hides_one_version_and_a_newer_one_shows_again() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    let s = h.check(true);
    assert_eq!(tray_version(&s).as_deref(), Some("0.1.1"));
    let s = h.ctl.skip();
    assert_eq!(s.skipped_version.as_deref(), Some("0.1.1"));
    assert_eq!(tray_version(&s), None, "skipped: no tray row, no tooltip line");
    assert!(s.available.is_some(), "the editor still knows it");
    let s = h.ctl.unskip();
    assert_eq!(tray_version(&s).as_deref(), Some("0.1.1"));
    h.ctl.skip();
    h.server.release("0.1.2", &fixture_text("payload.bin.sig"), fixture("payload.bin"));
    let s = h.check(true);
    assert_eq!(s.available.unwrap().version, "0.1.2");
    assert_eq!(tray_version(&h.ctl.state()).as_deref(), Some("0.1.2"), "a newer version shows again");
}

// ---- check against the fake release server ----

#[test]
fn check_reads_the_manifest() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    let s = h.check(true);
    assert_eq!(s.phase, UpdatePhase::Available);
    let info = s.available.unwrap();
    assert_eq!(info.version, "0.1.1");
    assert_eq!(info.notes, "### Added\n\n- Something new.");
    assert_eq!(info.pub_date.as_deref(), Some("2026-10-29T12:00:00Z"));
    assert_eq!(s.current_version, "0.1.0");
    assert!(s.last_check.is_some());
    assert_eq!(h.phases().first(), Some(&UpdatePhase::Checking));
    assert_eq!(h.stored().last_result, Some(LastResult::Available { version: "0.1.1".into() }));
}

#[test]
fn equal_older_and_pre_release_manifests_are_up_to_date() {
    for (current, announced) in [("0.1.1", "0.1.1"), ("0.1.1", "0.1.0"), ("0.1.0", "0.2.0-beta.1")] {
        let h = harness(current, "test.key.pub", Launch::Starts);
        h.server.release(announced, &fixture_text("payload.bin.sig"), fixture("payload.bin"));
        let s = h.check(true);
        assert_eq!(s.phase, UpdatePhase::UpToDate, "running {current}, manifest {announced}");
        assert!(s.available.is_none());
    }
}

#[test]
fn check_failures_are_told_apart() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    h.server.set("/latest.json", Resp::status(404));
    assert_eq!(h.check(true).phase, UpdatePhase::CheckFailed { reason: CheckFailReason::Other }, "404");

    h.server.set("/latest.json", Resp::status(403).header("x-ratelimit-remaining", "0"));
    assert_eq!(h.check(true).phase, UpdatePhase::CheckFailed { reason: CheckFailReason::RateLimited }, "403 rate limit");

    h.server.set("/latest.json", Resp::status(429).header("x-ratelimit-remaining", "0"));
    assert_eq!(h.check(true).phase, UpdatePhase::CheckFailed { reason: CheckFailReason::RateLimited }, "429 rate limit");

    h.server.set("/latest.json", Resp::status(403));
    assert_eq!(h.check(true).phase, UpdatePhase::CheckFailed { reason: CheckFailReason::Other }, "403 without the header");

    h.server.set("/latest.json", Resp::ok("this is not json"));
    assert_eq!(h.check(true).phase, UpdatePhase::CheckFailed { reason: CheckFailReason::Other }, "garbage manifest");

    h.server.set("/latest.json", Resp::ok(r#"{"version":"0.1.1","platforms":{}}"#));
    assert_eq!(h.check(true).phase, UpdatePhase::CheckFailed { reason: CheckFailReason::Other }, "no windows entry");
    assert_eq!(h.stored().last_check, None, "no failure counts as a check");
}

#[test]
fn a_dead_server_is_offline() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let ctl = Controller::new(
        Settings {
            endpoint: format!("http://127.0.0.1:{port}/latest.json").parse().unwrap(),
            pubkey: fixture_text("test.key.pub"),
            policy: DownloadPolicy { https_only: false, hosts: vec!["127.0.0.1".into()] },
            updates_dir: h.dir.path().join("u2"),
            state_path: h.dir.path().join("s2.json"),
            result_path: h.dir.path().join("r2.json"),
            guard_root: h.dir.path().to_path_buf(),
            progress_every: Duration::ZERO,
        },
        v("0.1.0"),
        Arc::new(|_: &Path| Ok(None)),
        Box::new(|_| {}),
    );
    let s = tauri::async_runtime::block_on(ctl.check(&h.handle, true));
    assert_eq!(s.phase, UpdatePhase::CheckFailed { reason: CheckFailReason::Offline });
}

#[test]
fn a_manifest_pointing_at_a_foreign_host_is_not_offered() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    let manifest = serde_json::json!({
        "version": "0.1.1", "notes": "", "pub_date": "2026-10-29T12:00:00Z",
        "platforms": { rules::TARGET: { "signature": fixture_text("payload.bin.sig"), "url": "http://evil.example/setup.exe" } },
    });
    h.server.set("/latest.json", Resp::ok(manifest.to_string()));
    assert_eq!(h.check(true).phase, UpdatePhase::CheckFailed { reason: CheckFailReason::Other });
}

#[test]
fn a_redirect_to_a_foreign_host_is_refused_and_one_to_an_allowed_host_is_followed() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.server.set("/hop", Resp::status(302).header("Location", &h.server.url("/latest.json")));
    let moved = h.server.url("/hop");
    // The endpoint is /hop, which redirects to the manifest on an allowed host.
    let ctl = Controller::new(
        Settings {
            endpoint: moved.parse().unwrap(),
            pubkey: fixture_text("test.key.pub"),
            policy: DownloadPolicy { https_only: false, hosts: vec!["127.0.0.1".into()] },
            updates_dir: h.dir.path().join("u3"),
            state_path: h.dir.path().join("s3.json"),
            result_path: h.dir.path().join("r3.json"),
            guard_root: h.dir.path().to_path_buf(),
            progress_every: Duration::ZERO,
        },
        v("0.1.0"),
        Arc::new(|_: &Path| Ok(None)),
        Box::new(|_| {}),
    );
    assert_eq!(tauri::async_runtime::block_on(ctl.check(&h.handle, true)).phase, UpdatePhase::Available);

    h.server.set("/hop", Resp::status(302).header("Location", "http://evil.example/latest.json"));
    let s = tauri::async_runtime::block_on(ctl.check(&h.handle, true));
    assert_eq!(s.phase, UpdatePhase::CheckFailed { reason: CheckFailReason::Other }, "foreign redirect target");
}

// ---- download and signature ----

#[test]
fn a_good_download_is_written_checked_and_started() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    let s = h.install();
    assert_eq!(s.phase, UpdatePhase::Installing);
    assert_eq!(std::fs::read(h.installer()).unwrap(), fixture("payload.bin"));
    let phases = h.phases();
    for want in [UpdatePhase::Verifying, UpdatePhase::WaitingForUac, UpdatePhase::Installing] {
        assert!(phases.contains(&want), "{want:?} in {phases:?}");
    }
    let seen = h.seen.calls.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "the installer was started once");
    assert_eq!(seen[0].pending_in_state_file.as_deref(), Some("0.1.1"), "pending is on disk before the start");
    assert!(seen[0].write_blocked, "the file cannot be rewritten while the tray holds it");
    assert!(seen[0].delete_blocked, "the file cannot be deleted while the tray holds it");
    assert_eq!(h.stored().pending.map(|p| p.version).as_deref(), Some("0.1.1"));
}

#[test]
fn a_tampered_byte_fails_the_check_and_leaves_no_file() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    let mut bytes = fixture("payload.bin");
    bytes[3] ^= 0x01;
    h.server.release("0.1.1", &fixture_text("payload.bin.sig"), bytes);
    h.check(true);
    let s = h.install();
    assert_eq!(s.phase, UpdatePhase::VerifyFailed);
    assert!(!h.installer().exists());
    assert!(h.seen.calls.lock().unwrap().is_empty(), "nothing was started");
    assert_eq!(h.stored().pending, None);
}

#[test]
fn a_signature_from_another_key_fails_the_check() {
    let h = harness("0.1.0", "other.key.pub", Launch::Starts);
    good_release(&h); // signed with test.key, but this controller trusts other.key
    h.check(true);
    assert_eq!(h.install().phase, UpdatePhase::VerifyFailed);
    assert!(!h.installer().exists());
    assert!(h.seen.calls.lock().unwrap().is_empty());
}

#[test]
fn a_signature_made_for_another_version_is_rejected() {
    // The replay: a manifest announcing 0.1.1 that points at 0.1.0's validly signed installer.
    let h = harness("0.0.9", "test.key.pub", Launch::Starts);
    h.server.release("0.1.1", &fixture_text("payload.v010.sig"), fixture("payload.bin"));
    h.check(true);
    assert_eq!(h.install().phase, UpdatePhase::VerifyFailed);
    assert!(!h.installer().exists());
    assert!(h.seen.calls.lock().unwrap().is_empty());
}

#[test]
fn a_signature_without_a_version_is_rejected() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    h.server.release("0.1.1", &fixture_text("payload.nover.sig"), fixture("payload.bin"));
    h.check(true);
    assert_eq!(h.install().phase, UpdatePhase::VerifyFailed);
    assert!(h.seen.calls.lock().unwrap().is_empty());
}

#[test]
fn a_missing_asset_is_an_install_failure_not_a_signature_failure() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    h.server.set("/setup.exe", Resp::status(404));
    assert!(matches!(h.install().phase, UpdatePhase::InstallFailed { .. }));
    assert!(h.seen.calls.lock().unwrap().is_empty());
    assert_eq!(h.stored().pending, None);
    assert_eq!(h.ctl.failed_version().as_deref(), Some("0.1.1"), "the tooltip names the version that failed");
}

#[test]
fn the_host_list_stops_a_download_before_any_request() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    // An update whose address was swapped for a foreign host after the check.
    let strict = Controller::new(
        Settings {
            endpoint: h.server.url("/latest.json").parse().unwrap(),
            pubkey: fixture_text("test.key.pub"),
            policy: DownloadPolicy { https_only: false, hosts: vec!["127.0.0.1".into()] },
            updates_dir: h.dir.path().join("u4"),
            state_path: h.dir.path().join("s4.json"),
            result_path: h.dir.path().join("r4.json"),
            guard_root: h.dir.path().to_path_buf(),
            progress_every: Duration::ZERO,
        },
        v("0.1.0"),
        Arc::new(|_: &Path| Ok(None)),
        Box::new(|_| {}),
    );
    tauri::async_runtime::block_on(strict.check(&h.handle, true));
    {
        let mut inner = strict.lock();
        inner.update.as_mut().unwrap().download_url = "http://evil.example/setup.exe".parse().unwrap();
    }
    let s = tauri::async_runtime::block_on(Arc::new(strict).install());
    assert!(matches!(s.phase, UpdatePhase::InstallFailed { .. }), "{:?}", s.phase);
    assert_eq!(h.server.hits("/setup.exe"), 0, "no request went out");
}

#[test]
fn declined_changes_nothing_and_clears_pending() {
    let h = harness("0.1.0", "test.key.pub", Launch::Declined);
    good_release(&h);
    h.check(true);
    let s = h.install();
    assert_eq!(s.phase, UpdatePhase::Declined);
    assert_eq!(h.stored().pending, None);
    assert_eq!(h.seen.calls.lock().unwrap().len(), 1);
    assert!(s.available.is_some(), "the update stays available to try again");
}

#[test]
fn a_launch_failure_clears_pending() {
    let h = harness("0.1.0", "test.key.pub", Launch::Fails);
    good_release(&h);
    h.check(true);
    let s = h.install();
    assert_eq!(s.phase, UpdatePhase::InstallFailed { message: "boom".into() });
    assert_eq!(h.stored().pending, None);
}

#[test]
fn a_file_changed_after_the_download_fails_the_second_check_and_is_deleted() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    let update = h.ctl.begin_install().unwrap();
    let path = tauri::async_runtime::block_on(h.ctl.fetch(&update)).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[0] ^= 0xff;
    std::fs::write(&path, bytes).unwrap();
    let (s, _) = h.ctl.launch_stage(&update, &path);
    assert_eq!(s.phase, UpdatePhase::VerifyFailed);
    assert!(!path.exists());
    assert!(h.seen.calls.lock().unwrap().is_empty(), "never started");
    assert_eq!(h.stored().pending, None);
}

// ---- one operation at a time ----

#[test]
fn install_and_check_are_refused_while_an_operation_runs() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    let update = h.ctl.begin_install();
    assert!(update.is_some());
    assert!(matches!(h.ctl.state().phase, UpdatePhase::Downloading { .. }));
    assert!(h.ctl.begin_install().is_none(), "a second install is refused");
    let hits = h.server.hits("/latest.json");
    let s = h.check(true);
    assert!(matches!(s.phase, UpdatePhase::Downloading { .. }), "Check now is refused and returns the state");
    assert_eq!(h.server.hits("/latest.json"), hits, "no request went out");
    assert!(!h.ctl.auto_due(OffsetDateTime::now_utc()), "the schedule waits too");
    for busy in [
        UpdatePhase::Checking,
        UpdatePhase::Verifying,
        UpdatePhase::WaitingForUac,
        UpdatePhase::Installing,
    ] {
        h.ctl.set_phase(busy.clone());
        assert!(h.ctl.begin_install().is_none(), "{busy:?}");
        assert_eq!(h.check(true).phase, busy);
    }
}

#[test]
fn install_with_nothing_available_does_nothing() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    assert_eq!(h.install().phase, UpdatePhase::Idle);
    assert!(h.seen.calls.lock().unwrap().is_empty());
}

// ---- after an update ----

fn info(version: &str) -> UpdateInfo {
    UpdateInfo { version: version.into(), notes: format!("### Added

- notes for {version}"), pub_date: None }
}

fn result(version: &str, ok: bool, message: &str) -> InstallResult {
    InstallResult { version: version.into(), ok, message: message.into() }
}

#[test]
fn settle_decides_from_the_running_version_and_the_installers_report() {
    assert_eq!(settle("0.1.1", "0.1.1", Some(&result("0.1.1", true, ""))), Settled::Updated);
    assert_eq!(settle("0.1.1", "0.1.1", None), Settled::Updated, "no report, but the new version runs");
    assert_eq!(
        settle("0.1.1", "0.1.1", Some(&result("0.1.1", false, "Setup could not set up your account."))),
        Settled::Failed("Setup could not set up your account.".into()),
        "the new exe runs, but the installer said it failed"
    );
    assert!(matches!(settle("0.1.1", "0.1.0", None), Settled::Failed(m) if m.contains("without finishing")));
    assert!(matches!(settle("0.1.1", "0.1.0", Some(&result("0.1.1", true, ""))), Settled::Failed(m) if m.contains("still running")));
    assert_eq!(
        settle("0.1.1", "0.1.0", Some(&result("0.1.1", false, "no space"))),
        Settled::Failed("no space".into())
    );
    assert!(
        matches!(settle("0.1.1", "0.1.0", Some(&result("0.0.9", true, ""))), Settled::Failed(_)),
        "a result for another version proves nothing"
    );
}

#[test]
fn start_after_update_turns_pending_into_updated_to() {
    let h = harness("0.1.1", "test.key.pub", Launch::Starts);
    state::save(&h.dir.path().join("update.json"), &Stored { pending: Some(info("0.1.1")), ..Stored::default() }).unwrap();
    std::fs::create_dir_all(h.dir.path().join("updates")).unwrap();
    std::fs::write(h.installer(), b"left over").unwrap();
    let ctl = Controller::new(
        Settings { ..Settings::production() }
            .with_dir(h.dir.path()),
        v("0.1.1"),
        Arc::new(|_: &Path| Ok(None)),
        Box::new(|_| {}),
    );
    ctl.reconcile_pending();
    let s = ctl.state();
    assert_eq!(s.updated_to, Some(info("0.1.1")), "the notes travel with it");
    assert_eq!(s.phase, UpdatePhase::Idle);
    let stored = state::load(&h.dir.path().join("update.json"));
    assert_eq!(stored.pending, None);
    assert_eq!(stored.updated_to, Some(info("0.1.1")));
    assert!(!h.installer().exists(), "the downloaded installer is swept away");
    assert!(ctl.dismiss_updated().updated_to.is_none());
    assert_eq!(state::load(&h.dir.path().join("update.json")).updated_to, None);
}

#[test]
fn start_with_a_failed_install_reports_the_installers_message() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    state::save(&h.dir.path().join("update.json"), &Stored { pending: Some(info("0.1.1")), ..Stored::default() }).unwrap();
    std::fs::write(
        h.dir.path().join("update-result.json"),
        r#"{"version":"0.1.1","ok":false,"code":10,"message":"Setup could not finish."}"#,
    )
    .unwrap();
    let ctl = Controller::new(
        Settings::production().with_dir(h.dir.path()),
        v("0.1.0"),
        Arc::new(|_: &Path| Ok(None)),
        Box::new(|_| {}),
    );
    ctl.reconcile_pending();
    let s = ctl.state();
    assert_eq!(s.phase, UpdatePhase::InstallFailed { message: "Setup could not finish.".into() });
    assert_eq!(s.updated_to, None);
    assert_eq!(state::load(&h.dir.path().join("update.json")).pending, None);
    assert_eq!(s.available, Some(info("0.1.1")), "the failed release is on offer again, so the editor can name it");
    assert_eq!(ctl.failed_version().as_deref(), Some("0.1.1"));
}

#[test]
fn a_start_with_nothing_pending_changes_nothing() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    h.ctl.reconcile_pending();
    assert_eq!(h.ctl.state().phase, UpdatePhase::Idle);
    assert_eq!(h.ctl.state().updated_to, None);
}

impl Settings {
    /// Test helper: production settings, with every path moved under `dir`.
    fn with_dir(mut self, dir: &Path) -> Self {
        self.updates_dir = dir.join("updates");
        self.state_path = dir.join("update.json");
        self.result_path = dir.join("update-result.json");
        self.guard_root = dir.to_path_buf();
        self.progress_every = Duration::ZERO;
        self
    }
}

#[test]
fn the_after_update_argument_is_found() {
    assert!(has_after_update_arg(&["canvas-rgb.exe", "--after-update"]));
    assert!(!has_after_update_arg(&["canvas-rgb.exe", "--onboard"]));
    assert!(!has_after_update_arg::<&str>(&[]));
}

// ---- starting the installer ----

#[test]
fn installer_arguments_have_update_and_no_suppressed_message_boxes() {
    let args: Vec<&str> = launch::INSTALLER_ARGS.split(' ').collect();
    assert_eq!(args, ["/SILENT", "/NORESTART", "/UPDATE"]);
}

#[test]
fn run_locked_holds_the_file_while_the_installer_starts() {
    let dir = TempDir::new();
    let path = dir.path().join("setup.exe");
    std::fs::write(&path, b"installer bytes").unwrap();
    let seen = Mutex::new(None);
    let out = launch::run_locked(
        &path,
        &|bytes| bytes == b"installer bytes",
        &|p| {
            *seen.lock().unwrap() = Some((
                std::fs::OpenOptions::new().write(true).open(p).is_err(),
                std::fs::remove_file(p).is_err(),
                std::fs::rename(p, p.with_extension("moved")).is_err(),
                std::fs::read(p).is_ok(),
            ));
            Ok(None)
        },
    );
    assert_eq!(out, Ok(None));
    assert_eq!(*seen.lock().unwrap(), Some((true, true, true, true)), "write, delete and rename blocked; reading allowed");
    assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok(), "released afterwards");
}

#[test]
fn run_locked_deletes_a_file_that_fails_its_check_and_never_runs_it() {
    let dir = TempDir::new();
    let path = dir.path().join("setup.exe");
    std::fs::write(&path, b"tampered").unwrap();
    let ran = AtomicBool::new(false);
    let out = launch::run_locked(&path, &|_| false, &|_| {
        ran.store(true, Ordering::SeqCst);
        Ok(None)
    });
    assert_eq!(out, Err(LaunchError::VerifyFailed));
    assert!(!ran.load(Ordering::SeqCst));
    assert!(!path.exists());
}

#[test]
fn run_locked_maps_declined_and_failures() {
    let dir = TempDir::new();
    let path = dir.path().join("setup.exe");
    std::fs::write(&path, b"x").unwrap();
    assert_eq!(launch::run_locked(&path, &|_| true, &|_| Err(RunError::Declined)), Err(LaunchError::Declined));
    assert_eq!(
        launch::run_locked(&path, &|_| true, &|_| Err(RunError::Failed("no".into()))),
        Err(LaunchError::Failed("no".into()))
    );
    assert!(path.exists(), "a refused or failed start keeps the file for another try");
    assert!(matches!(
        launch::run_locked(&dir.path().join("missing.exe"), &|_| true, &|_| Ok(None)),
        Err(LaunchError::Failed(_))
    ));
}

/// The share mode must not stop Windows from loading the file as a program: copy a harmless
/// system exe, hold it the way the tray does, and run it (it prints its help and exits).
#[cfg(windows)]
#[test]
fn a_held_file_can_still_be_run_as_a_program() {
    let dir = TempDir::new();
    let path = dir.path().join("whoami-copy.exe");
    let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    std::fs::copy(system.join("System32").join("whoami.exe"), &path).unwrap();
    let out = launch::run_locked(&path, &|_| true, &|p| {
        let status = std::process::Command::new(p)
            .arg("/?")
            .stdout(std::process::Stdio::null())
            .status()
            .map_err(|e| RunError::Failed(e.to_string()))?;
        if status.success() {
            Ok(None)
        } else {
            Err(RunError::Failed(format!("exit {status}")))
        }
    });
    assert_eq!(out, Ok(None));
}

// ---- the shipped config agrees with the code ----

#[test]
fn tauri_conf_repeats_the_compiled_key_and_feed_and_requires_a_signed_version() {
    let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    let u = &conf["plugins"]["updater"];
    assert_eq!(u["pubkey"], rules::PUBKEY);
    assert_eq!(u["endpoints"], serde_json::json!([rules::ENDPOINT]));
    assert_eq!(u["requireSignedVersion"], true);
    assert!(!u.to_string().contains("dangerous"), "no dangerous switch in the shipped config");
}

#[test]
fn the_editor_cannot_call_the_plugins_install_commands() {
    let caps = include_str!("../../capabilities/default.json");
    assert!(!caps.contains("updater"), "no updater permission for the editor");
}

#[test]
fn the_compiled_key_is_a_minisign_public_key() {
    // base64 of "untrusted comment: minisign public key: "
    assert!(rules::PUBKEY.starts_with("dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6"));
    assert!(!rules::PUBKEY.contains(char::is_whitespace));
}

// ---- hostile folders ----

/// `mklink /J`: a directory junction, the case a symlink check alone would miss.
fn make_junction(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J failed");
}

#[test]
fn a_junction_as_the_updates_folder_is_never_written_through_or_cleared() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    let target = h.dir.path().join("elsewhere");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("victim.txt"), b"keep me").unwrap();
    make_junction(&h.dir.path().join("updates"), &target);

    let s = h.install();
    match &s.phase {
        UpdatePhase::InstallFailed { message } => assert!(message.contains("junction"), "{message}"),
        other => panic!("expected InstallFailed, got {other:?}"),
    }
    let names: Vec<_> = std::fs::read_dir(&target).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(names, vec![std::ffi::OsString::from("victim.txt")], "nothing written into the target");
    assert_eq!(std::fs::read(target.join("victim.txt")).unwrap(), b"keep me", "nothing deleted from it");
    assert!(h.seen.calls.lock().unwrap().is_empty(), "no installer was started");

    // The sweep after an update leaves it alone too.
    h.ctl.reconcile_pending();
    assert!(target.join("victim.txt").exists());
    drop(h); // the junction goes first (remove_dir on a junction removes only the link)
}

#[test]
fn a_junction_higher_up_is_refused_and_named() {
    let dir = TempDir::new();
    let real = dir.path().join("real");
    std::fs::create_dir_all(real.join("updates")).unwrap();
    make_junction(&dir.path().join("tray"), &real);
    let err = state::refuse_reparse(dir.path(), &dir.path().join("tray").join("updates")).unwrap_err();
    assert!(err.contains("tray") && err.contains("junction"), "{err}");
    assert!(state::refuse_reparse(dir.path(), &dir.path().join("real").join("updates")).is_ok());
    assert!(state::refuse_reparse(dir.path(), &dir.path().join("not-there").join("updates")).is_ok());
    assert!(state::refuse_reparse(dir.path(), Path::new(r"C:\elsewhere")).is_err(), "outside the root");
}

// ---- no stale offer ----

#[test]
fn a_failed_recheck_withdraws_the_offer() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    assert_eq!(h.check(true).phase, UpdatePhase::Available);
    h.server.set("/latest.json", Resp::status(500));
    let s = h.check(true);
    assert!(matches!(s.phase, UpdatePhase::CheckFailed { .. }));
    assert!(s.available.is_none(), "nothing stale on offer");
    assert!(h.ctl.begin_install().is_none(), "nothing installable either");
    assert!(matches!(h.install().phase, UpdatePhase::CheckFailed { .. }));
    assert_eq!(h.server.hits("/setup.exe"), 0);
    assert!(h.seen.calls.lock().unwrap().is_empty());
}

// ---- after the installer starts ----

#[test]
fn a_started_installer_with_no_process_to_follow_does_not_stick_on_installing() {
    let h = harness("0.1.0", "test.key.pub", Launch::NoHandle);
    good_release(&h);
    h.check(true);
    let s = h.install();
    match &s.phase {
        UpdatePhase::InstallFailed { message } => assert!(message.contains("cannot follow"), "{message}"),
        other => panic!("expected InstallFailed, got {other:?}"),
    }
    assert_eq!(h.stored().pending.map(|p| p.version).as_deref(), Some("0.1.1"), "pending stays: the installer may be running");
    assert_eq!(h.ctl.failed_version().as_deref(), Some("0.1.1"));
}

#[test]
fn the_tray_waits_for_a_real_process_and_then_reads_the_installers_report() {
    for (report, want) in [
        (Some(r#"{"version":"0.1.1","ok":false,"code":10,"message":"Setup could not finish."}"#), "Setup could not finish."),
        (None, "The installer ended without finishing."),
        (
            Some(r#"{"version":"0.1.1","ok":true,"code":0,"message":""}"#),
            "Installed, but the old canvas-rgb is still running. Quit it and start it again.",
        ),
    ] {
        let h = harness("0.1.0", "test.key.pub", Launch::Process);
        good_release(&h);
        h.check(true);
        if let Some(report) = report {
            std::fs::write(h.dir.path().join("update-result.json"), report).unwrap();
        }
        let update = h.ctl.begin_install().unwrap();
        let s = tauri::async_runtime::block_on(h.ctl.run_install(&update));
        assert_eq!(s.phase, UpdatePhase::InstallFailed { message: want.into() });
        assert!(h.phases().contains(&UpdatePhase::Installing), "Installing was shown while the process ran");
        assert_eq!(h.stored().pending, None, "settled");
        assert_eq!(h.ctl.failed_version().as_deref(), Some("0.1.1"));
    }
}

// ---- hosts, https, redirects and progress for the download ----

#[test]
fn the_download_follows_a_redirect_to_an_allowed_host_and_refuses_a_foreign_one() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    let payload = h.server.routes.lock().unwrap().get("/setup.exe").unwrap().clone();
    h.server.set("/real.exe", payload);
    h.server.set("/setup.exe", Resp::status(302).header("Location", &h.server.url("/real.exe")));
    assert_eq!(h.install().phase, UpdatePhase::Installing, "allowed hop");
    assert_eq!(h.server.hits("/real.exe"), 1);

    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    h.server.set("/setup.exe", Resp::status(302).header("Location", "http://evil.example/setup.exe"));
    let s = h.install();
    assert!(matches!(s.phase, UpdatePhase::InstallFailed { .. }), "{:?}", s.phase);
    assert!(h.seen.calls.lock().unwrap().is_empty());
    assert!(!h.installer().exists());
}

#[test]
fn an_http_download_address_is_refused_before_any_request() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    let settings = Settings {
        endpoint: h.server.url("/latest.json").parse().unwrap(),
        pubkey: fixture_text("test.key.pub"),
        policy: DownloadPolicy { https_only: false, hosts: vec!["127.0.0.1".into()] },
        updates_dir: h.dir.path().join("u5"),
        state_path: h.dir.path().join("s5.json"),
        result_path: h.dir.path().join("r5.json"),
        guard_root: h.dir.path().to_path_buf(),
        progress_every: Duration::ZERO,
    };
    let mut ctl = Controller::new(settings, v("0.1.0"), Arc::new(|_: &Path| Ok(None)), Box::new(|_| {}));
    assert_eq!(tauri::async_runtime::block_on(ctl.check(&h.handle, true)).phase, UpdatePhase::Available);
    ctl.settings.policy = DownloadPolicy { https_only: true, hosts: vec!["127.0.0.1".into()] };
    let s = tauri::async_runtime::block_on(Arc::new(ctl).install());
    assert!(matches!(s.phase, UpdatePhase::InstallFailed { .. }), "{:?}", s.phase);
    assert_eq!(h.server.hits("/setup.exe"), 0);
}

#[test]
fn download_progress_is_sent_with_the_real_byte_counts() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    let payload = fixture("payload.bin");
    h.server.set("/setup.exe", Resp::ok(payload.clone()).chunked(8));
    h.check(true);
    assert_eq!(h.install().phase, UpdatePhase::Installing);
    let phases = h.phases();
    let counts: Vec<(u64, Option<u64>)> = phases
        .iter()
        .filter_map(|p| match p {
            UpdatePhase::Downloading { downloaded, total } if *downloaded > 0 => Some((*downloaded, *total)),
            _ => None,
        })
        .collect();
    assert!(counts.len() >= 2, "several progress events: {counts:?}");
    assert!(counts.windows(2).all(|w| w[0].0 <= w[1].0), "never goes backwards: {counts:?}");
    assert!(counts.iter().all(|c| c.1 == Some(payload.len() as u64)), "total from Content-Length: {counts:?}");
    assert_eq!(counts.last().unwrap().0, payload.len() as u64, "ends at the full size");
    let at = |want: &UpdatePhase| phases.iter().position(|p| p == want).unwrap();
    let last_download = phases.iter().rposition(|p| matches!(p, UpdatePhase::Downloading { .. })).unwrap();
    assert!(last_download < at(&UpdatePhase::Verifying) && at(&UpdatePhase::Verifying) < at(&UpdatePhase::WaitingForUac));
}

// ---- timeouts and a damaged state file ----

#[test]
fn a_timeout_after_connecting_is_not_offline() {
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();
    let keep = std::thread::spawn(move || {
        // Accept and say nothing, until the client gives up.
        let held: Vec<_> = silent.incoming().take(1).flatten().collect();
        std::thread::sleep(std::time::Duration::from_millis(800));
        drop(held);
    });
    let url = format!("http://127.0.0.1:{port}/latest.json");
    let err = tauri::async_runtime::block_on(async {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(200))
            .build()
            .unwrap()
            .get(&url)
            .send()
            .await
    })
    .unwrap_err();
    assert!(err.is_timeout() && !err.is_connect(), "{err:?}");
    let reason = tauri::async_runtime::block_on(classify(&tauri_plugin_updater::Error::Reqwest(err), &url.parse().unwrap()));
    assert_eq!(reason, CheckFailReason::Other);
    keep.join().unwrap();
}

#[test]
fn a_corrupt_state_file_is_kept_aside_not_overwritten() {
    isolate_log();
    let dir = TempDir::new();
    let path = dir.path().join("update.json");
    std::fs::write(&path, r#"{"pending": {"version": "0.1.1", "notes": "n""#).unwrap(); // cut off
    let loaded = state::load(&path);
    assert_eq!(loaded, Stored::default());
    let bad = dir.path().join("update.json.bad");
    assert_eq!(std::fs::read_to_string(&bad).unwrap(), r#"{"pending": {"version": "0.1.1", "notes": "n""#);
    assert!(!path.exists(), "the damaged file is not left in place to be overwritten unseen");
}

// ---- review fixes: restart after a failed install, off-thread start, probe, result file ----

/// A tray that restarted after a failed `/UPDATE` run: `pending` is stored, the installer's report is
/// on disk, and no plugin `Update` object exists. Returns the controller and a launch counter.
fn restarted(h: &Harness, current: &str, pending: &str) -> (Arc<Controller>, Arc<AtomicU32>) {
    state::save(&h.dir.path().join("update.json"), &Stored { pending: Some(info(pending)), ..Stored::default() }).unwrap();
    std::fs::write(
        h.dir.path().join("update-result.json"),
        format!(r#"{{"version":"{pending}","ok":false,"code":10,"message":"Setup could not finish."}}"#),
    )
    .unwrap();
    let launches = Arc::new(AtomicU32::new(0));
    let counter = launches.clone();
    let ctl = Arc::new(Controller::new(
        h.ctl.settings.clone(),
        v(current),
        Arc::new(move |_: &Path| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }),
        Box::new(|_| {}),
    ));
    ctl.reconcile_pending();
    (ctl, launches)
}

#[test]
fn try_again_after_a_restart_checks_first_and_installs_the_release() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    let (ctl, launches) = restarted(&h, "0.1.0", "0.1.1");
    assert!(matches!(ctl.state().phase, UpdatePhase::InstallFailed { .. }));
    assert!(ctl.begin_install().is_none(), "no live release object after a restart");
    assert!(ctl.needs_check_to_install());
    let s = tauri::async_runtime::block_on(ctl.check_and_install(&h.handle));
    assert_eq!(h.server.hits("/latest.json"), 1, "it checked first");
    assert_eq!(launches.load(Ordering::SeqCst), 1, "then started the installer");
    assert!(matches!(s.phase, UpdatePhase::InstallFailed { ref message } if message.contains("cannot follow")), "{:?}", s.phase);
}

#[test]
fn try_again_after_a_restart_says_so_when_the_check_fails_or_finds_nothing() {
    // The feed is down.
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    h.server.set("/latest.json", Resp::status(500));
    let (ctl, launches) = restarted(&h, "0.1.0", "0.1.1");
    let s = tauri::async_runtime::block_on(ctl.check_and_install(&h.handle));
    assert!(matches!(s.phase, UpdatePhase::CheckFailed { .. }), "{:?}", s.phase);
    assert_eq!(launches.load(Ordering::SeqCst), 0);

    // The feed has nothing newer than what runs.
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    h.server.release("0.1.0", &fixture_text("payload.bin.sig"), fixture("payload.bin"));
    let (ctl, launches) = restarted(&h, "0.1.0", "0.1.1");
    let s = tauri::async_runtime::block_on(ctl.check_and_install(&h.handle));
    assert_eq!(s.phase, UpdatePhase::UpToDate);
    assert_eq!(launches.load(Ordering::SeqCst), 0);

    // The feed now offers something older than the release that failed: shown, not installed.
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    let (ctl, launches) = restarted(&h, "0.1.0", "0.1.2");
    let s = tauri::async_runtime::block_on(ctl.check_and_install(&h.handle));
    assert_eq!(s.phase, UpdatePhase::Available);
    assert_eq!(s.available.map(|a| a.version).as_deref(), Some("0.1.1"));
    assert_eq!(launches.load(Ordering::SeqCst), 0);
}

#[test]
fn try_again_after_a_restart_is_single_flight() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    let (ctl, _) = restarted(&h, "0.1.0", "0.1.1");
    ctl.set_phase(UpdatePhase::Installing);
    assert!(!ctl.needs_check_to_install(), "an operation runs");
    let hits = h.server.hits("/latest.json");
    let s = tauri::async_runtime::block_on(ctl.check_and_install(&h.handle));
    assert_eq!(s.phase, UpdatePhase::Installing);
    assert_eq!(h.server.hits("/latest.json"), hits, "no request went out");
}

#[test]
fn the_installer_start_runs_off_the_async_thread() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    let seen: Arc<Mutex<Option<std::thread::ThreadId>>> = Arc::default();
    let sink = seen.clone();
    let ctl = Arc::new(Controller::new(
        h.ctl.settings.clone(),
        v("0.1.0"),
        Arc::new(move |_: &Path| {
            *sink.lock().unwrap() = Some(std::thread::current().id());
            Ok(None)
        }),
        Box::new(|_| {}),
    ));
    tauri::async_runtime::block_on(ctl.check(&h.handle, true));
    let update = ctl.begin_install().unwrap();
    let here = std::thread::current().id();
    tauri::async_runtime::block_on(ctl.start_install(&update));
    let ran_on = seen.lock().unwrap().expect("the launcher ran");
    assert_ne!(ran_on, here, "ShellExecuteExW must not wait on the calling (async) thread");
}

#[test]
fn the_probe_maps_a_timeout_after_connecting_to_other_and_a_refusal_to_offline() {
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let url: Url = format!("http://127.0.0.1:{}/latest.json", silent.local_addr().unwrap().port()).parse().unwrap();
    let keep = std::thread::spawn(move || {
        let held: Vec<_> = silent.incoming().take(1).flatten().collect();
        std::thread::sleep(Duration::from_millis(1200));
        drop(held);
    });
    assert_eq!(probe(&url, Duration::from_millis(300)), CheckFailReason::Other);
    keep.join().unwrap();

    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let url: Url = format!("http://127.0.0.1:{}/latest.json", dead.local_addr().unwrap().port()).parse().unwrap();
    drop(dead);
    assert_eq!(probe(&url, Duration::from_secs(5)), CheckFailReason::Offline);
}

#[test]
fn the_result_file_is_read_with_a_bom_and_with_non_ascii_or_bad_bytes() {
    let dir = TempDir::new();
    let path = dir.path().join("update-result.json");
    let body = r#"{"version":"0.1.1","ok":false,"code":10,"message":"Setup stopped"}"#;
    let mut with_bom = vec![0xEF, 0xBB, 0xBF];
    with_bom.extend_from_slice(body.as_bytes());
    std::fs::write(&path, &with_bom).unwrap();
    let r = read_result(&path).expect("a BOM does not hide the report");
    assert_eq!((r.version.as_str(), r.ok), ("0.1.1", false));

    std::fs::write(&path, r#"{"version":"0.1.1","ok":false,"message":"Zoë — fini"}"#.as_bytes()).unwrap();
    assert_eq!(read_result(&path).unwrap().message, "Zoë — fini", "UTF-8 round-trips");

    // An ANSI byte (0xE9) in the message: lossy, but still parsed.
    let mut ansi = br#"{"version":"0.1.1","ok":false,"message":"Caf"#.to_vec();
    ansi.push(0xE9);
    ansi.extend_from_slice(br#""}"#);
    std::fs::write(&path, &ansi).unwrap();
    let r = read_result(&path).expect("a stray byte does not hide the report");
    assert!(r.message.starts_with("Caf") && !r.ok);
}

// ---- which token starts the installer (#28) ----

/// Setup must be started with the tray's own non-elevated token and raise its own permission
/// prompt; started elevated, its `ExecAsOriginalUser` relaunch gave an elevated tray that refuses
/// to run. Fails on the old code: `shell_execute_runas` passed the elevating verb.
#[test]
fn the_installer_is_started_without_an_elevating_verb() {
    let request = launch::installer_request(launch::INSTALLER_ARGS);
    assert_eq!(request.verb, None);
    assert_eq!(request.args, launch::INSTALLER_ARGS);
    let source = include_str!("launch.rs").to_lowercase();
    assert!(!source.contains("runas"), "launch.rs must not name an elevating verb");
}

#[test]
fn a_setup_that_ends_cancelled_with_no_result_is_declined_not_failed() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    assert_eq!(h.install().phase, UpdatePhase::Installing);
    assert!(h.stored().pending.is_some());
    h.ctl.settle_after_installer(Some(launch::SETUP_CANCELLED_CODE));
    assert_eq!(h.ctl.state().phase, UpdatePhase::Declined);
    assert_eq!(h.stored().pending, None);
}

#[test]
fn a_setup_that_ends_any_other_way_is_settled_from_its_result() {
    let h = harness("0.1.0", "test.key.pub", Launch::Starts);
    good_release(&h);
    h.check(true);
    h.install();
    h.ctl.settle_after_installer(Some(4));
    assert!(matches!(h.ctl.state().phase, UpdatePhase::InstallFailed { .. }), "{:?}", h.ctl.state().phase);
}
