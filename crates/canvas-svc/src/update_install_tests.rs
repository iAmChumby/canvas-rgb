//! Tests for `update_install` (#43). Everything runs on fakes: the installer is read from memory, the
//! filesystem checks run over a real temp folder with the owner check faked, setup is a recorder, and the
//! console user is a recorder. Nothing here starts setup or touches anything outside a temp folder.

use crate::fakes::FakeProcesses;
use crate::pipe_server::Admitted;
use crate::process::ChildProcess;
use crate::secure::{FileId, FsAcl};
use crate::update_install::*;
use canvas_proto::api::error_codes;
use canvas_proto::consts::SERVICE_SETUP_ARGS;
use canvas_proto::pipe::PipeResponse;
use std::collections::HashMap;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) const PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/update/payload.bin");
pub(crate) const SIG_011: &str = include_str!("../tests/fixtures/update/payload.bin.sig");
const SIG_010: &str = include_str!("../tests/fixtures/update/payload.v010.sig");
const SIG_NOVER: &str = include_str!("../tests/fixtures/update/payload.nover.sig");
const PUB_TEST: &str = include_str!("../tests/fixtures/update/test.key.pub");
const PUB_OTHER: &str = include_str!("../tests/fixtures/update/other.key.pub");
pub(crate) const WHO: Admitted = Admitted { pid: 77, session: 1 };
pub(crate) const SOURCE: &str = r"C:\Users\someone\Downloads\setup.exe";

/// Waits up to 5 s for `cond`.
pub(crate) fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < end, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

// ---- fakes ----

/// The real filesystem under a temp folder; only the owner and DACL answers are faked.
pub(crate) struct TreeFs {
    pub untrusted: Arc<Mutex<Vec<PathBuf>>>,
    /// Paths a non-admin principal may write.
    pub writable: Arc<Mutex<Vec<PathBuf>>>,
    /// Paths where a non-admin holds WRITE_DAC, WRITE_OWNER or FILE_DELETE_CHILD.
    pub controlled: Arc<Mutex<Vec<PathBuf>>>,
}

impl FsAcl for TreeFs {
    fn exists(&self, p: &Path) -> io::Result<bool> {
        Ok(std::fs::symlink_metadata(p).is_ok())
    }
    fn is_dir(&self, p: &Path) -> io::Result<bool> {
        Ok(std::fs::symlink_metadata(p)?.file_attributes() & 0x10 != 0)
    }
    fn is_reparse(&self, p: &Path) -> io::Result<bool> {
        Ok(std::fs::symlink_metadata(p)?.file_attributes() & 0x400 != 0)
    }
    fn list(&self, p: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(p)?.map(|e| e.map(|e| e.path())).collect()
    }
    fn owner_trusted(&self, p: &Path) -> io::Result<bool> {
        Ok(!self.untrusted.lock().unwrap().iter().any(|u| u == p))
    }
    fn file_id(&self, _p: &Path) -> io::Result<FileId> {
        Ok(FileId { volume: 0, index: 0 })
    }
    fn create_dir_protected(&mut self, p: &Path) -> io::Result<()> {
        std::fs::create_dir(p)
    }
    fn create_dir_private(&mut self, p: &Path) -> io::Result<()> {
        std::fs::create_dir(p)
    }
    fn apply_protected_dacl(&mut self, _p: &Path) -> io::Result<()> {
        Ok(())
    }
    fn has_nonadmin_write(&self, p: &Path) -> io::Result<bool> {
        Ok(self.writable.lock().unwrap().iter().any(|w| w == p))
    }
    fn owner_trusted_parent(&self, p: &Path) -> io::Result<bool> {
        self.owner_trusted(p)
    }
    fn has_nonadmin_control(&self, p: &Path, _is_drive_root: bool) -> io::Result<bool> {
        Ok(self.controlled.lock().unwrap().iter().any(|c| c == p))
    }
}

pub(crate) struct FakeReader {
    pub bytes: Vec<u8>,
    pub fail: Option<&'static str>,
    pub after_read: Option<Box<dyn FnOnce() + Send>>,
}

impl FakeReader {
    pub fn of(bytes: &[u8]) -> Box<Self> {
        Box::new(Self { bytes: bytes.to_vec(), fail: None, after_read: None })
    }
}

impl ClientFileReader for FakeReader {
    fn read(&mut self, _path: &Path, max: u64) -> io::Result<Vec<u8>> {
        if let Some(why) = self.fail {
            return Err(io::Error::other(why));
        }
        assert!(self.bytes.len() as u64 <= max);
        let out = self.bytes.clone();
        if let Some(after) = self.after_read.take() {
            after();
        }
        Ok(out)
    }
}

type LaunchHook = Box<dyn Fn(&Path) + Send>;

#[derive(Default)]
pub(crate) struct SetupState {
    pub launches: Mutex<Vec<(PathBuf, String, PathBuf)>>,
    /// The `tmp` argument of each launch.
    pub tmps: Mutex<Vec<PathBuf>>,
    pub exited: Arc<AtomicBool>,
    pub killed: AtomicBool,
    pub resumed: AtomicBool,
    pub fail: AtomicBool,
    pub fail_resume: AtomicBool,
    /// Runs inside `launch`, i.e. while the service holds the staged handle.
    pub on_launch: Mutex<Option<LaunchHook>>,
    /// Runs inside `resume`, i.e. at the moment setup would start running.
    pub on_resume: Mutex<Option<Box<dyn Fn() + Send>>>,
}

struct FakeSetup(Arc<SetupState>);

struct FakeSetupProcess {
    state: Arc<SetupState>,
}

impl ChildProcess for FakeSetupProcess {
    fn try_wait(&mut self) -> io::Result<Option<i32>> {
        Ok(self.state.exited.load(Ordering::SeqCst).then_some(0))
    }
    fn kill(&mut self) -> io::Result<()> {
        self.state.killed.store(true, Ordering::SeqCst);
        self.state.exited.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn id(&self) -> u32 {
        7000
    }
}

impl SetupProcess for FakeSetupProcess {
    fn created(&self) -> u64 {
        123_456
    }
    fn resume(&mut self) -> io::Result<()> {
        if let Some(hook) = self.state.on_resume.lock().unwrap().as_ref() {
            hook();
        }
        if self.state.fail_resume.load(Ordering::SeqCst) {
            return Err(io::Error::other("fake resume failure"));
        }
        self.state.resumed.store(true, Ordering::SeqCst);
        Ok(())
    }
}

impl SetupLauncher for FakeSetup {
    fn launch(&mut self, exe: &Path, raw_args: &str, dir: &Path, tmp: &Path) -> io::Result<Box<dyn SetupProcess>> {
        if self.0.fail.load(Ordering::SeqCst) {
            return Err(io::Error::other("fake setup failure"));
        }
        if let Some(hook) = self.0.on_launch.lock().unwrap().as_ref() {
            hook(exe);
        }
        assert!(tmp.is_dir(), "the private temp folder exists before setup is created");
        self.0.tmps.lock().unwrap().push(tmp.to_path_buf());
        self.0.launches.lock().unwrap().push((exe.to_path_buf(), raw_args.to_string(), dir.to_path_buf()));
        Ok(Box::new(FakeSetupProcess { state: Arc::clone(&self.0) }))
    }
}

#[derive(Default)]
pub(crate) struct UserState {
    pub commands: Mutex<Vec<String>>,
    pub no_user: AtomicBool,
    /// While set, started steps never exit.
    pub hang: AtomicBool,
    /// Set when the service ended a started step.
    pub killed: AtomicBool,
}

struct FakeUser(Arc<UserState>, FakeProcesses);

/// A user-step child that records being killed.
struct KillSpy {
    inner: Box<dyn ChildProcess>,
    state: Arc<UserState>,
}

impl ChildProcess for KillSpy {
    fn try_wait(&mut self) -> io::Result<Option<i32>> {
        self.inner.try_wait()
    }
    fn kill(&mut self) -> io::Result<()> {
        self.state.killed.store(true, Ordering::SeqCst);
        self.inner.kill()
    }
    fn id(&self) -> u32 {
        self.inner.id()
    }
}

impl UserCommands for FakeUser {
    fn start(&mut self, command_line: &str, _dir: &Path) -> io::Result<Box<dyn ChildProcess>> {
        if self.0.no_user.load(Ordering::SeqCst) {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "nobody is logged on (fake)"));
        }
        self.0.commands.lock().unwrap().push(command_line.to_string());
        // A child that has already exited.
        let child = crate::process::ProcessSpawner::spawn(&mut crate::fakes::FakeSpawner(self.1.clone()), Path::new("user-step.exe"), &[])?;
        if !self.0.hang.load(Ordering::SeqCst) {
            self.1.exit_last();
        }
        Ok(Box::new(KillSpy { inner: child, state: Arc::clone(&self.0) }))
    }
}

/// pid -> (creation time the process really has, polls it stays alive for; `u32::MAX` = forever).
#[derive(Default)]
pub(crate) struct ProbeState {
    pub procs: Mutex<HashMap<u32, (u64, u32)>>,
    pub asked: AtomicU32,
    /// pids the service ended (only when pid and creation time both matched).
    pub terminated: Mutex<Vec<u32>>,
}

struct FakeProbe(Arc<ProbeState>);

impl ProcessProbe for FakeProbe {
    fn alive(&mut self, pid: u32, created: u64) -> bool {
        self.0.asked.fetch_add(1, Ordering::SeqCst);
        let mut procs = self.0.procs.lock().unwrap();
        match procs.get_mut(&pid) {
            Some((real_created, left)) if *real_created == created && *left > 0 => {
                if *left != u32::MAX {
                    *left -= 1;
                }
                true
            }
            _ => false,
        }
    }

    fn terminate(&mut self, pid: u32, created: u64) -> bool {
        let mut procs = self.0.procs.lock().unwrap();
        if procs.get(&pid).is_some_and(|(real, _)| *real == created) {
            procs.remove(&pid);
            self.0.terminated.lock().unwrap().push(pid);
        }
        true
    }
}

pub(crate) struct Rig {
    pub dir: tempfile::TempDir,
    pub home: PathBuf,
    pub app: PathBuf,
    pub installer: Arc<UpdateInstaller>,
    pub setup: Arc<SetupState>,
    pub user: Arc<UserState>,
    pub probe: Arc<ProbeState>,
    pub untrusted: Arc<Mutex<Vec<PathBuf>>>,
    pub writable: Arc<Mutex<Vec<PathBuf>>>,
    pub controlled: Arc<Mutex<Vec<PathBuf>>>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        // Lets any watcher thread end.
        self.setup.exited.store(true, Ordering::SeqCst);
    }
}

pub(crate) fn rig(own_version: &str) -> Rig {
    rig_with(own_version, PUB_TEST, |_| {}, |i| i)
}

pub(crate) fn rig_with(
    own_version: &str,
    pubkey: &str,
    tweak: impl FnOnce(&mut UpdateConfig),
    wrap: impl FnOnce(UpdateInstaller) -> UpdateInstaller,
) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let app = dir.path().join("app");
    std::fs::create_dir_all(home.join("logs")).unwrap();
    // The install folder as the finish step needs it.
    std::fs::create_dir_all(app.join("installer")).unwrap();
    std::fs::write(app.join("installer").join("setup-user.ps1"), b"# fake").unwrap();
    std::fs::write(app.join("installer").join("lib.ps1"), b"# fake").unwrap();
    std::fs::write(app.join("canvas-rgb.exe"), b"MZ fake").unwrap();
    std::fs::write(app.join("canvas-rgb-svc.exe"), b"MZ fake").unwrap();
    let mut cfg = UpdateConfig {
        home: home.clone(),
        app_dir: app.clone(),
        own_version: own_version.to_string(),
        pubkey: pubkey.trim().to_string(),
        powershell: PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
        poll: Duration::from_millis(1),
        setup_wait_limit: Duration::from_secs(5),
        user_step_limit: Duration::from_secs(2),
        user_session_wait: Duration::from_millis(30),
        resume_retry: Duration::from_millis(2),
    };
    tweak(&mut cfg);
    let untrusted = Arc::new(Mutex::new(Vec::new()));
    let writable = Arc::new(Mutex::new(Vec::new()));
    let controlled = Arc::new(Mutex::new(Vec::new()));
    let setup = Arc::new(SetupState::default());
    let user = Arc::new(UserState::default());
    let probe = Arc::new(ProbeState::default());
    let installer = UpdateInstaller::new(
        cfg,
        Box::new(TreeFs { untrusted: Arc::clone(&untrusted), writable: Arc::clone(&writable), controlled: Arc::clone(&controlled) }),
        Box::new(FakeSetup(Arc::clone(&setup))),
        Box::new(FakeUser(Arc::clone(&user), FakeProcesses::new())),
        Box::new(FakeProbe(Arc::clone(&probe))),
    );
    Rig { dir, home, app, installer: Arc::new(wrap(installer)), setup, user, probe, untrusted, writable, controlled }
}

impl Rig {
    pub fn updates(&self) -> PathBuf {
        self.home.join("updates")
    }

    pub fn install(&self, bytes: &[u8], sig: &str) -> PipeResponse {
        self.installer.install(WHO, SOURCE, sig, FakeReader::of(bytes))
    }

    pub fn launches(&self) -> usize {
        self.setup.launches.lock().unwrap().len()
    }

    pub fn commands(&self) -> Vec<String> {
        self.user.commands.lock().unwrap().clone()
    }

    /// Files in the staging folder, by name.
    pub fn staged_files(&self) -> Vec<String> {
        match std::fs::read_dir(self.updates()) {
            Ok(rd) => rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// A staged file and a marker as a previous service life would have left them.
    pub fn leave_pending(&self, pid: u32, created: u64, started_unix: u64) -> PendingUpdate {
        let updates = self.installer.secure_updates_dir(true).unwrap().unwrap();
        let staged = updates.join("canvas-rgb-setup-1-1-1.exe");
        std::fs::write(&staged, PAYLOAD).unwrap();
        let pending = PendingUpdate {
            version: "0.1.1".into(),
            staged: staged.to_string_lossy().into_owned(),
            setup_pid: pid,
            setup_created: created,
            requester_pid: 77,
            requester_session: 1,
            started_unix,
            tmp_dir: String::new(),
        };
        self.installer.write_marker(&updates, &pending).unwrap();
        pending
    }

    /// What setup writes at the end of a /UPDATE run: `{app}\update-result.json`.
    pub fn write_result(&self, ok: bool, version: &str) {
        let json = format!("{{\"version\":\"{version}\",\"ok\":{ok},\"code\":0,\"message\":\"\"}}");
        std::fs::write(self.app.join("update-result.json"), json).unwrap();
    }

    pub fn marker_exists(&self) -> bool {
        self.installer.marker_path().exists()
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// Runs `f` on its own thread and fails the test if it has not finished within 10 s.
fn with_deadline<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(10)).expect("did not finish within 10 s")
}

fn code_of(r: &PipeResponse) -> String {
    match r {
        PipeResponse::Error { code, .. } => code.clone(),
        other => format!("not an error: {other:?}"),
    }
}

fn message_of(r: &PipeResponse) -> String {
    match r {
        PipeResponse::Error { message, .. } => message.clone(),
        other => format!("not an error: {other:?}"),
    }
}

fn mklink_junction(link: &Path, target: &Path) {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "mklink /J failed");
}

// ---- a good install ----

#[test]
fn a_good_install_stages_verifies_writes_the_marker_and_starts_setup() {
    let r = rig("0.1.0");
    let response = r.install(PAYLOAD, SIG_011);
    assert_eq!(response, PipeResponse::UpdateStarted { version: "0.1.1".into() });
    let launches = r.setup.launches.lock().unwrap().clone();
    assert_eq!(launches.len(), 1);
    let (exe, args, cwd) = &launches[0];
    assert_eq!(exe.parent().unwrap(), r.updates(), "setup runs from the staging folder");
    assert_eq!(std::fs::read(exe).unwrap(), PAYLOAD, "the staged bytes are the signed bytes");
    let log = r.home.join("logs").join("setup-0.1.1.log");
    assert_eq!(args, &format!("{SERVICE_SETUP_ARGS} /LOG=\"{}\"", log.display()));
    assert_eq!(cwd, &r.updates());
    let marker = r.installer.read_marker(&r.installer.marker_path()).unwrap();
    assert_eq!(marker.version, "0.1.1");
    assert_eq!(marker.staged, exe.to_string_lossy());
    assert_eq!((marker.setup_pid, marker.setup_created), (7000, 123_456));
    assert_eq!((marker.requester_pid, marker.requester_session), (77, 1));
}

// ---- refusals: nothing runs, nothing is left behind ----

fn assert_refused_clean(r: &Rig, response: &PipeResponse, code: &str, mention: &str) {
    assert_eq!(code_of(response), code, "{response:?}");
    assert!(message_of(response).contains(mention), "{response:?} should mention {mention}");
    assert_eq!(r.launches(), 0, "setup must not start");
    assert!(!r.marker_exists(), "no marker");
    assert!(r.staged_files().is_empty(), "the staged copy is removed: {:?}", r.staged_files());
    assert!(!r.installer.is_busy(), "the install slot is free again");
}

#[test]
fn a_tampered_file_is_refused() {
    let r = rig("0.1.0");
    let mut bad = PAYLOAD.to_vec();
    bad[3] ^= 0x01;
    let response = r.install(&bad, SIG_011);
    assert_refused_clean(&r, &response, error_codes::VALIDATION, "signature does not match");
}

#[test]
fn the_wrong_key_is_refused() {
    let r = rig_with("0.1.0", PUB_OTHER, |_| {}, |i| i);
    let response = r.install(PAYLOAD, SIG_011);
    assert_refused_clean(&r, &response, error_codes::VALIDATION, "signature does not match");
}

#[test]
fn an_older_signed_version_is_a_downgrade_and_is_refused() {
    let r = rig("0.2.0");
    let response = r.install(PAYLOAD, SIG_011);
    assert_refused_clean(&r, &response, error_codes::VALIDATION, "downgrade");
}

#[test]
fn the_same_signed_version_is_a_replay_and_is_refused() {
    let r = rig("0.1.0");
    let response = r.install(PAYLOAD, SIG_010);
    assert_refused_clean(&r, &response, error_codes::VALIDATION, "replay");
}

#[test]
fn a_signature_with_no_version_is_refused() {
    let r = rig("0.1.0");
    let response = r.install(PAYLOAD, SIG_NOVER);
    assert_refused_clean(&r, &response, error_codes::VALIDATION, "no version");
}

#[test]
fn garbage_signatures_and_bad_requests_are_refused() {
    let r = rig("0.1.0");
    let long = "A".repeat(MAX_SIGNATURE_BYTES + 1);
    for (installer, sig, mention) in [
        (SOURCE, "!!! not base64", "base64"),
        (SOURCE, "", "empty"),
        (SOURCE, long.as_str(), "bytes"),
        (r"relative\setup.exe", SIG_011, "absolute"),
        (r"\\server\share\setup.exe", SIG_011, "absolute"),
        (r"\\.\pipe\evil", SIG_011, "absolute"),
        (r"C:setup.exe", SIG_011, "absolute"),
    ] {
        let response = r.installer.install(WHO, installer, sig, FakeReader::of(PAYLOAD));
        assert_refused_clean(&r, &response, error_codes::VALIDATION, mention);
    }
}

#[test]
fn an_unreadable_or_empty_file_is_refused() {
    let r = rig("0.1.0");
    let mut reader = FakeReader::of(PAYLOAD);
    reader.fail = Some("access denied");
    let response = r.installer.install(WHO, SOURCE, SIG_011, reader);
    assert_refused_clean(&r, &response, error_codes::VALIDATION, "access denied");
    let response = r.install(b"", SIG_011);
    assert_refused_clean(&r, &response, error_codes::VALIDATION, "empty");
}

#[test]
fn a_client_that_cannot_be_impersonated_is_refused() {
    let r = rig("0.1.0");
    let response = r.installer.install(WHO, SOURCE, SIG_011, Box::new(RefusingReader));
    assert_refused_clean(&r, &response, error_codes::VALIDATION, "cannot be impersonated");
}

// ---- the version rule, on its own ----

#[test]
fn the_version_rule_is_strict_semver() {
    assert!(check_newer("0.1.1", "0.1.0").is_ok());
    assert!(check_newer("v0.2.0", "0.1.9").is_ok());
    assert!(check_newer("0.10.0", "0.9.0").is_ok(), "numeric, not textual");
    assert!(check_newer("1.0.0", "1.0.0-rc.1").is_ok(), "a release is newer than its pre-release");
    assert!(check_newer("0.2.1-dev.12", "0.2.0").is_ok(), "a test build is newer than the release before it");
    assert!(check_newer("0.2.1-dev.12", "0.2.1").unwrap_err().message.contains("downgrade"), "and older than the release after it");
    assert!(check_newer("0.2.1-dev.13", "0.2.1-dev.12").is_ok());
    assert!(check_newer("0.1.0", "0.1.0").unwrap_err().message.contains("replay"));
    assert!(check_newer("0.1.0+build5", "0.1.0+build4").unwrap_err().message.contains("replay"), "build metadata is not precedence");
    assert!(check_newer("0.0.9", "0.1.0").unwrap_err().message.contains("downgrade"));
    for bad in ["", "abc", "1.2", "1.2.3.4", "latest"] {
        let e = check_newer(bad, "0.1.0").unwrap_err();
        assert_eq!(e.code, error_codes::VALIDATION, "{bad}");
        assert!(e.message.contains("not valid semver"), "{bad}: {}", e.message);
    }
}

#[test]
fn the_signed_version_is_read_from_the_tab_separated_trusted_comment() {
    assert_eq!(signed_version("timestamp:1\tfile:a.exe\tversion:0.1.1"), Some("0.1.1"));
    assert_eq!(signed_version("timestamp:1\tfile:a.exe"), None);
    assert_eq!(signed_version("timestamp:1 version:9.9.9"), None, "spaces do not separate fields");
}

// ---- reading as the client: the file checks ----

#[test]
fn the_client_read_refuses_what_it_must() {
    use crate::update_install::win::read_regular_file;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.bin");
    std::fs::write(&file, b"12345678").unwrap();
    assert_eq!(read_regular_file(&file, 8).unwrap(), b"12345678");
    assert!(read_regular_file(&file, 7).unwrap_err().to_string().contains("limit"), "one byte over the limit");
    assert!(read_regular_file(dir.path(), 100).is_err(), "a directory");
    assert!(read_regular_file(&dir.path().join("missing"), 100).is_err());
    // A junction is a reparse point.
    let link = dir.path().join("junction");
    mklink_junction(&link, dir.path());
    assert!(read_regular_file(&link, 100).is_err(), "a junction");
}

#[test]
fn the_client_read_refuses_a_file_symlink() {
    use crate::update_install::win::read_regular_file;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.bin");
    std::fs::write(&file, b"x").unwrap();
    let link = dir.path().join("link.bin");
    match std::os::windows::fs::symlink_file(&file, &link) {
        Ok(()) => {
            let e = read_regular_file(&link, 100).unwrap_err();
            assert!(e.to_string().contains("reparse"), "{e}");
        }
        // Creating a symlink needs a privilege this account may lack. Then this is UNKNOWN, not a pass;
        // the junction case above covers the reparse check.
        Err(e) => eprintln!("UNKNOWN: file symlink not created here ({e})"),
    }
}

// ---- TOCTOU ----

#[test]
fn while_setup_runs_the_staged_file_cannot_be_written_replaced_renamed_or_deleted() {
    let r = rig("0.1.0");
    let checked = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&checked);
    *r.setup.on_launch.lock().unwrap() = Some(Box::new(move |staged: &Path| {
        assert!(std::fs::OpenOptions::new().write(true).open(staged).is_err(), "open for write");
        assert!(std::fs::write(staged, b"swapped").is_err(), "overwrite");
        assert!(std::fs::remove_file(staged).is_err(), "delete");
        assert!(std::fs::rename(staged, staged.with_extension("moved")).is_err(), "rename away");
        let other = staged.with_extension("other");
        std::fs::write(&other, b"swapped").unwrap();
        assert!(std::fs::rename(&other, staged).is_err(), "rename over it");
        std::fs::remove_file(&other).unwrap();
        assert_eq!(std::fs::read(staged).unwrap(), PAYLOAD, "still the checked bytes");
        flag.store(true, Ordering::SeqCst);
    }));
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    assert!(checked.load(Ordering::SeqCst), "the launch hook ran");
}

/// Reads a real file from disk, then runs a hook (which changes that file).
struct DiskReader {
    after_read: Option<Box<dyn FnOnce() + Send>>,
}

impl ClientFileReader for DiskReader {
    fn read(&mut self, path: &Path, _max: u64) -> io::Result<Vec<u8>> {
        let bytes = std::fs::read(path)?;
        if let Some(after) = self.after_read.take() {
            after();
        }
        Ok(bytes)
    }
}

#[test]
fn a_source_that_changes_after_the_read_does_not_change_what_runs() {
    let r = rig("0.1.0");
    let source = r.dir.path().join("source.exe");
    std::fs::write(&source, PAYLOAD).unwrap();
    let path = source.clone();
    let reader = DiskReader { after_read: Some(Box::new(move || std::fs::write(path, b"evil bytes after the read").unwrap())) };
    let response = r.installer.install(WHO, source.to_str().unwrap(), SIG_011, Box::new(reader));
    assert!(matches!(response, PipeResponse::UpdateStarted { .. }), "{response:?}");
    assert_eq!(std::fs::read(&source).unwrap(), b"evil bytes after the read", "the real source file did change after it was read");
    let staged = r.setup.launches.lock().unwrap()[0].0.clone();
    assert_eq!(std::fs::read(staged).unwrap(), PAYLOAD, "setup runs the copy that was checked");
}

#[test]
fn the_check_runs_on_what_is_on_disk_not_on_what_was_written() {
    // The staged file is swapped between the write and the reopen, as a racing writer would.
    let r = rig_with("0.1.0", PUB_TEST, |_| {}, |i| i.with_stage_seam(|p| std::fs::write(p, b"swapped on disk").unwrap()));
    let response = r.install(PAYLOAD, SIG_011);
    assert_refused_clean(&r, &response, error_codes::VALIDATION, "signature does not match");
}

// ---- the staging folder fails closed ----

#[test]
fn a_junction_as_the_staging_folder_is_refused_named_and_left_alone() {
    let r = rig("0.1.0");
    let elsewhere = r.dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("keep.txt"), b"keep").unwrap();
    mklink_junction(&r.updates(), &elsewhere);
    let response = r.install(PAYLOAD, SIG_011);
    assert_eq!(code_of(&response), error_codes::INTERNAL, "{response:?}");
    assert!(message_of(&response).contains("updates"), "names the path: {response:?}");
    assert!(message_of(&response).contains("junction"), "{response:?}");
    assert_eq!(r.launches(), 0);
    let left: Vec<_> = std::fs::read_dir(&elsewhere).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from("keep.txt")], "nothing written through the junction, nothing deleted");
    assert!(std::fs::symlink_metadata(r.updates()).is_ok(), "the junction itself is still there (never deleted)");
}

#[test]
fn a_junction_inside_the_staging_folder_is_refused() {
    let r = rig("0.1.0");
    std::fs::create_dir(r.updates()).unwrap();
    let elsewhere = r.dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let evil = r.updates().join("evil");
    mklink_junction(&evil, &elsewhere);
    let response = r.install(PAYLOAD, SIG_011);
    assert_eq!(code_of(&response), error_codes::INTERNAL, "{response:?}");
    assert!(message_of(&response).contains("evil"), "{response:?}");
    assert_eq!(r.launches(), 0);
    assert!(std::fs::symlink_metadata(&evil).is_ok(), "never deleted");
}

#[test]
fn an_untrusted_owner_in_the_staging_folder_is_refused_and_kept() {
    let r = rig("0.1.0");
    std::fs::create_dir(r.updates()).unwrap();
    let planted = r.updates().join("planted.exe");
    std::fs::write(&planted, b"x").unwrap();
    r.untrusted.lock().unwrap().push(planted.clone());
    let response = r.install(PAYLOAD, SIG_011);
    assert_eq!(code_of(&response), error_codes::INTERNAL, "{response:?}");
    assert!(message_of(&response).contains("planted.exe"), "{response:?}");
    assert!(planted.exists(), "never deleted");
    assert_eq!(r.launches(), 0);
}

#[test]
fn an_untrusted_staging_folder_or_home_is_refused() {
    for which in ["updates", "home"] {
        let r = rig("0.1.0");
        std::fs::create_dir(r.updates()).unwrap();
        let path = if which == "home" { r.home.clone() } else { r.updates() };
        r.untrusted.lock().unwrap().push(path);
        let response = r.install(PAYLOAD, SIG_011);
        assert_eq!(code_of(&response), error_codes::INTERNAL, "{which}: {response:?}");
        assert_eq!(r.launches(), 0, "{which}");
    }
}

#[test]
fn the_staging_folder_is_made_when_it_is_missing_and_matches_the_spine_path() {
    let r = rig("0.1.0");
    assert!(!r.updates().exists());
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    assert!(r.updates().is_dir());
    assert_eq!(canvas_proto::consts::update_staging_dir(), crate::paths::updates_dir(&canvas_proto::consts::machine_data_dir()));
}

// ---- one at a time ----

#[test]
fn a_second_request_while_one_runs_is_a_conflict_and_a_third_works_once_it_ends() {
    let r = rig("0.1.0");
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    assert!(r.installer.is_busy());
    let second = r.install(PAYLOAD, SIG_011);
    assert_eq!(code_of(&second), error_codes::CONFLICT, "{second:?}");
    assert_eq!(r.launches(), 1, "the second request started nothing");
    assert!(r.marker_exists(), "the first install's marker is untouched");
    // Setup ends; the service finishes the update and the slot frees up.
    r.setup.exited.store(true, Ordering::SeqCst);
    wait_until("the install slot to free", || !r.installer.is_busy());
    r.setup.exited.store(false, Ordering::SeqCst);
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
}

#[test]
fn a_leftover_marker_whose_setup_is_still_alive_blocks_a_new_install() {
    let r = rig("0.1.0");
    r.leave_pending(7000, 1, now());
    r.probe.procs.lock().unwrap().insert(7000, (1, u32::MAX));
    let response = r.install(PAYLOAD, SIG_011);
    assert_eq!(code_of(&response), error_codes::CONFLICT, "{response:?}");
    assert_eq!(r.launches(), 0);
    assert!(r.marker_exists());
}

#[test]
fn a_leftover_marker_whose_setup_has_ended_is_superseded_not_a_24_hour_lockout() {
    let r = rig("0.1.0");
    let old = r.leave_pending(7000, 1, now());
    // The probe knows no such process: setup ended. Also true for a reused pid with another creation time.
    let tmp_old = r.updates().join("tmp-old");
    std::fs::create_dir(&tmp_old).unwrap();
    let mut with_tmp = old.clone();
    with_tmp.tmp_dir = tmp_old.to_string_lossy().into_owned();
    r.installer.write_marker(&r.updates(), &with_tmp).unwrap();
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    assert!(!Path::new(&old.staged).exists(), "the old staged file is discarded");
    assert!(!tmp_old.exists(), "and its setup temp folder");
    assert_eq!(r.launches(), 1);
}

#[test]
fn a_reused_pid_in_a_leftover_marker_does_not_block_a_new_install() {
    let r = rig("0.1.0");
    r.leave_pending(7000, 1, now());
    r.probe.procs.lock().unwrap().insert(7000, (2, u32::MAX));
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
}

#[test]
fn an_unreadable_marker_is_removed_and_does_not_lock_installs_out() {
    let r = rig("0.1.0");
    r.installer.secure_updates_dir(true).unwrap();
    std::fs::write(r.installer.marker_path(), b"{ not json").unwrap();
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
}

#[test]
fn a_stale_marker_is_cleared_and_the_install_goes_ahead() {
    let r = rig("0.1.0");
    let old = r.leave_pending(7000, 1, 1);
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    assert!(!Path::new(&old.staged).exists(), "the old staged file is gone");
}

#[test]
fn a_failed_setup_start_leaves_nothing_behind() {
    let r = rig("0.1.0");
    r.setup.fail.store(true, Ordering::SeqCst);
    let response = r.install(PAYLOAD, SIG_011);
    assert_refused_clean(&r, &response, error_codes::INTERNAL, "could not start setup");
}

// ---- the marker handover ----

fn ps_line(r: &Rig) -> String {
    let app = r.app.display();
    format!(
        "\"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoProfile -ExecutionPolicy Bypass -File \"{app}\\installer\\setup-user.ps1\" -InstallDir \"{app}\" -SignalRgb N"
    )
}

#[test]
fn the_old_service_still_alive_when_setup_ends_finishes_the_update() {
    let r = rig("0.1.0");
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    let staged = r.setup.launches.lock().unwrap()[0].0.clone();
    assert!(r.commands().is_empty(), "nothing runs while setup runs");
    r.write_result(true, "0.1.1");
    r.setup.exited.store(true, Ordering::SeqCst);
    wait_until("the update to finish", || !r.installer.is_busy());
    assert_eq!(r.commands(), vec![ps_line(&r), format!("\"{}\\canvas-rgb.exe\" --after-update", r.app.display())]);
    assert!(!staged.exists(), "the staged file it made is deleted");
    assert!(!r.marker_exists(), "and the marker");
}

#[test]
fn the_new_service_finishes_on_start_after_waiting_for_the_setup_pid() {
    let r = rig("0.1.1");
    r.leave_pending(5000, 99, now());
    r.write_result(true, "0.1.1");
    r.probe.procs.lock().unwrap().insert(5000, (99, 3));
    let outcome = r.installer.resume_pending();
    assert_eq!(outcome, Some(Finish::Done));
    assert!(r.probe.asked.load(Ordering::SeqCst) >= 4, "it kept asking while setup ran");
    assert_eq!(r.commands().len(), 2);
    assert_eq!(r.commands()[0], ps_line(&r), "setup-user.ps1 first, then the tray");
    assert!(r.commands()[1].ends_with("canvas-rgb.exe\" --after-update"));
    assert!(r.staged_files().is_empty(), "staged file and marker are both gone: {:?}", r.staged_files());
    assert!(!r.installer.is_busy());
}

#[test]
fn a_reused_pid_is_not_mistaken_for_setup() {
    let r = rig("0.1.1");
    r.leave_pending(5000, 99, now());
    r.write_result(true, "0.1.1");
    // Something else now has pid 5000 (another creation time) and runs for a long time.
    r.probe.procs.lock().unwrap().insert(5000, (100, u32::MAX));
    assert_eq!(r.installer.resume_pending(), Some(Finish::Done));
    assert!(r.probe.terminated.lock().unwrap().is_empty(), "a stranger with a reused pid is never ended");
    assert_eq!(r.probe.asked.load(Ordering::SeqCst), 1, "one look, no waiting for a stranger");
    assert_eq!(r.commands().len(), 2);
}

#[test]
fn the_wait_for_setup_is_bounded() {
    let r = rig_with("0.1.1", PUB_TEST, |c| c.setup_wait_limit = Duration::from_millis(40), |i| i);
    r.leave_pending(5000, 99, now());
    r.probe.procs.lock().unwrap().insert(5000, (99, u32::MAX));
    assert_eq!(r.installer.resume_pending(), None);
    assert!(r.commands().is_empty(), "does not finish under a setup that is still running");
    assert!(r.marker_exists(), "the update stays pending for the next start");
    assert!(!r.installer.is_busy());
}

#[test]
fn with_nobody_logged_on_one_attempt_keeps_the_update_pending() {
    let r = rig("0.1.1");
    r.leave_pending(0, 0, now());
    r.user.no_user.store(true, Ordering::SeqCst);
    assert_eq!(r.installer.resume_once(), Some(Finish::NoUser));
    assert!(r.marker_exists());
    assert_eq!(r.staged_files().len(), 2, "marker and staged file both kept");
    assert!(!r.installer.is_busy(), "the install slot is free while it waits for a user");
}

#[test]
fn resume_keeps_trying_until_a_user_logs_on_well_past_two_minutes_worth_of_attempts() {
    let r = rig("0.1.1");
    r.leave_pending(0, 0, now());
    r.user.no_user.store(true, Ordering::SeqCst);
    let user = Arc::clone(&r.user);
    // A user logs on after many failed attempts (each attempt waits user_session_wait = 30 ms).
    let logon = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        user.no_user.store(false, Ordering::SeqCst);
    });
    r.write_result(true, "0.1.1");
    let installer = Arc::clone(&r.installer);
    let outcome = with_deadline(move || installer.resume_pending());
    logon.join().unwrap();
    assert_eq!(outcome, Some(Finish::Done));
    assert_eq!(r.commands().len(), 2, "setup-user.ps1 and the tray ran once a user was there");
    assert!(r.staged_files().is_empty());
}

#[test]
fn a_new_install_supersedes_a_marker_that_resume_is_still_waiting_on() {
    let r = rig("0.1.0");
    r.leave_pending(0, 0, now());
    r.user.no_user.store(true, Ordering::SeqCst);
    assert_eq!(r.installer.resume_once(), Some(Finish::NoUser));
    // The slot was free: a request can take it, and it clears the old marker.
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    assert_eq!(r.installer.resume_once(), None, "nothing left for resume to retry under the new install");
}

#[test]
fn a_stale_marker_on_start_is_dropped_without_running_anything() {
    let r = rig("0.1.1");
    let old = r.leave_pending(0, 0, 1);
    assert_eq!(r.installer.resume_pending(), None);
    assert!(r.commands().is_empty());
    assert!(!r.marker_exists());
    assert!(!Path::new(&old.staged).exists());
}

#[test]
fn no_marker_means_nothing_to_resume() {
    let r = rig("0.1.1");
    assert_eq!(r.installer.resume_pending(), None);
    let r = rig("0.1.1");
    std::fs::create_dir(r.updates()).unwrap();
    assert_eq!(r.installer.resume_pending(), None);
    assert!(r.commands().is_empty());
}

#[test]
fn a_marker_cannot_make_the_service_delete_a_file_outside_the_staging_folder() {
    let r = rig("0.1.1");
    let victim = r.dir.path().join("canvas-rgb-setup-victim.exe");
    std::fs::write(&victim, b"precious").unwrap();
    let mut pending = r.leave_pending(0, 0, now());
    pending.staged = victim.to_string_lossy().into_owned();
    let updates = r.updates();
    r.installer.write_marker(&updates, &pending).unwrap();
    assert_eq!(r.installer.resume_pending(), Some(Finish::Done));
    assert!(victim.exists(), "a path outside the staging folder is never deleted");
    assert!(!r.marker_exists());
    // A name that is not a staged installer, inside the folder, is also left alone.
    let r = rig("0.1.1");
    let mut pending = r.leave_pending(0, 0, now());
    let other = r.updates().join("something-else.dll");
    std::fs::write(&other, b"x").unwrap();
    pending.staged = other.to_string_lossy().into_owned();
    r.installer.write_marker(&r.updates(), &pending).unwrap();
    assert_eq!(r.installer.resume_pending(), Some(Finish::Done));
    assert!(other.exists());
}

// ---- setup's private TMP/TEMP, and the suspended start ----

#[test]
fn setup_gets_a_private_temp_folder_inside_the_staging_folder_and_it_is_removed_after() {
    let r = rig("0.1.0");
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    let tmps = r.setup.tmps.lock().unwrap().clone();
    assert_eq!(tmps.len(), 1);
    assert_eq!(tmps[0].parent().unwrap(), r.updates(), "inside the staging folder");
    assert!(tmps[0].file_name().unwrap().to_string_lossy().starts_with("tmp-"));
    assert!(tmps[0].is_dir(), "it exists while setup runs");
    let marker = r.installer.read_marker(&r.installer.marker_path()).unwrap();
    assert_eq!(marker.tmp_dir, tmps[0].to_string_lossy());
    // Something setup left in it goes with it, once setup has ended.
    std::fs::write(tmps[0].join("IS-leftover.tmp"), b"x").unwrap();
    r.setup.exited.store(true, Ordering::SeqCst);
    wait_until("the update to finish", || !r.installer.is_busy());
    assert!(!tmps[0].exists(), "the folder and its contents are deleted after setup ends");
    assert!(r.staged_files().is_empty(), "{:?}", r.staged_files());
}

#[test]
fn a_refused_install_leaves_no_temp_folder() {
    let r = rig("0.1.0");
    r.setup.fail.store(true, Ordering::SeqCst);
    let response = r.install(PAYLOAD, SIG_011);
    assert_refused_clean(&r, &response, error_codes::INTERNAL, "could not start setup");
}

#[test]
fn the_environment_block_sets_tmp_temp_path_and_psmodulepath_and_keeps_the_rest() {
    use crate::update_install::win::build_env_block;
    use std::ffi::OsString;
    let vars = vec![
        (OsString::from("Temp"), OsString::from(r"C:\Windows\Temp")),
        (OsString::from("TMP"), OsString::from(r"C:\Windows\Temp")),
        (OsString::from("SystemRoot"), OsString::from(r"D:\Win")),
        (OsString::from("ProgramFiles"), OsString::from(r"D:\Programs")),
        // A user-writable folder first on the service's own PATH and module path must not leak through.
        (OsString::from("Path"), OsString::from(r"C:\Users\someone\evil;D:\Win\System32")),
        (OsString::from("PSModulePath"), OsString::from(r"C:\Users\someone\Documents\PowerShell\Modules")),
        (OsString::from("windir"), OsString::from(r"D:\Win")),
    ];
    let tmp = Path::new(r"C:\ProgramData\canvas-rgb\updates\tmp-1");
    let block = build_env_block(vars, tmp);
    assert_eq!(block[block.len() - 2..], [0, 0], "double NUL end");
    let text = String::from_utf16(&block[..block.len() - 1]).unwrap();
    let entries: Vec<&str> = text.split('\0').filter(|e| !e.is_empty()).collect();
    assert_eq!(
        entries,
        vec![
            r"Path=D:\Win\System32;D:\Win;D:\Win\System32\Wbem;D:\Win\System32\WindowsPowerShell\v1.0",
            r"ProgramFiles=D:\Programs",
            r"PSModulePath=D:\Programs\WindowsPowerShell\Modules;D:\Win\System32\WindowsPowerShell\v1.0\Modules",
            r"SystemRoot=D:\Win",
            r"TEMP=C:\ProgramData\canvas-rgb\updates\tmp-1",
            r"TMP=C:\ProgramData\canvas-rgb\updates\tmp-1",
            r"windir=D:\Win",
        ]
    );
    assert!(!text.contains("evil"), "nothing user-writable is searched");
}

#[test]
fn the_marker_is_on_disk_with_the_setup_pid_before_setup_is_resumed() {
    let r = rig("0.1.0");
    let marker_path = r.installer.marker_path();
    let seen = Arc::new(Mutex::new(None));
    let seen_in = Arc::clone(&seen);
    *r.setup.on_resume.lock().unwrap() = Some(Box::new(move || {
        let text = std::fs::read_to_string(&marker_path).expect("the marker exists at resume time");
        *seen_in.lock().unwrap() = Some(serde_json::from_str::<PendingUpdate>(&text).unwrap());
    }));
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    let marker = seen.lock().unwrap().clone().expect("resume ran");
    assert_eq!((marker.setup_pid, marker.setup_created), (7000, 123_456), "pid and creation time, never 0");
    assert!(r.setup.resumed.load(Ordering::SeqCst));
}

#[test]
fn a_setup_that_cannot_be_resumed_is_ended_and_everything_is_taken_back() {
    let r = rig("0.1.0");
    r.setup.fail_resume.store(true, Ordering::SeqCst);
    let response = r.install(PAYLOAD, SIG_011);
    assert_eq!(code_of(&response), error_codes::INTERNAL, "{response:?}");
    assert!(r.setup.killed.load(Ordering::SeqCst), "the suspended process was terminated");
    assert!(!r.setup.resumed.load(Ordering::SeqCst));
    wait_until("the watcher to take everything back", || r.staged_files().is_empty() && !r.installer.is_busy());
    assert!(r.commands().is_empty(), "the finish steps did not run for a setup that never ran");
}

#[test]
fn a_marker_that_cannot_be_written_ends_setup_before_it_runs() {
    // A folder where the marker file goes makes the rename fail.
    let r = rig_with("0.1.0", PUB_TEST, |_| {}, |i| {
        i.with_stage_seam(|staged| std::fs::create_dir(staged.parent().unwrap().join("update-pending.json")).unwrap())
    });
    let response = r.install(PAYLOAD, SIG_011);
    assert_eq!(code_of(&response), error_codes::INTERNAL, "{response:?}");
    assert!(message_of(&response).contains("marker"), "{response:?}");
    assert!(r.setup.killed.load(Ordering::SeqCst), "setup was terminated");
    assert!(!r.setup.resumed.load(Ordering::SeqCst), "and never ran");
    assert!(!r.installer.is_busy());
}

#[test]
fn a_watcher_that_cannot_start_ends_setup_and_refuses_instead_of_reporting_started() {
    let r = rig_with("0.1.0", PUB_TEST, |_| {}, |i| i.with_failing_watcher());
    let response = r.install(PAYLOAD, SIG_011);
    assert_eq!(code_of(&response), error_codes::INTERNAL, "{response:?}");
    assert!(message_of(&response).contains("watcher"), "{response:?}");
    assert!(r.setup.killed.load(Ordering::SeqCst));
    assert!(!r.setup.resumed.load(Ordering::SeqCst), "setup never ran");
    assert!(!r.marker_exists());
    assert!(!r.installer.is_busy());
}

// ---- the install slot ----

/// A reader that waits for the test before it returns the bytes.
struct BlockingReader {
    entered: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl ClientFileReader for BlockingReader {
    fn read(&mut self, _path: &Path, _max: u64) -> io::Result<Vec<u8>> {
        self.entered.send(()).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).map_err(|_| io::Error::other("the test never released the reader"))?;
        Ok(PAYLOAD.to_vec())
    }
}

#[test]
fn the_install_slot_is_taken_before_the_file_is_read() {
    // No marker exists yet while the first request is still reading, so only the in-process guard can
    // answer this conflict: removing it makes the second request read too, and this fails.
    let r = rig("0.1.0");
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let installer = Arc::clone(&r.installer);
    let first = std::thread::spawn(move || {
        installer.install(WHO, SOURCE, SIG_011, Box::new(BlockingReader { entered: entered_tx, release: release_rx }))
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).expect("the first request is inside its read");
    assert!(!r.marker_exists(), "nothing but the in-process guard can stop a second request");
    let reads = Arc::new(AtomicU32::new(0));
    struct CountingReader(Arc<AtomicU32>);
    impl ClientFileReader for CountingReader {
        fn read(&mut self, _p: &Path, _m: u64) -> io::Result<Vec<u8>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(PAYLOAD.to_vec())
        }
    }
    let second = r.installer.install(WHO, SOURCE, SIG_011, Box::new(CountingReader(Arc::clone(&reads))));
    assert_eq!(code_of(&second), error_codes::CONFLICT, "{second:?}");
    assert_eq!(reads.load(Ordering::SeqCst), 0, "the second request read nothing");
    release_tx.send(()).unwrap();
    assert!(matches!(first.join().unwrap(), PipeResponse::UpdateStarted { .. }));
}

// ---- real Win32 ----

fn temp_file_with(bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup.exe");
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

/// Runs one read through `ImpersonatingReader` against a real in-process named pipe whose client opened
/// with `qos` (`None`: tokio's default, SECURITY_IDENTIFICATION).
async fn impersonated_read(tag: &str, qos: Option<u32>, path: PathBuf) -> io::Result<Vec<u8>> {
    use std::os::windows::io::AsRawHandle;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let name = format!(r"\\.\pipe\canvas-rgb-svc-test-{tag}-{}", std::process::id());
    let server = crate::pipe_server::create_instance(&name, true).unwrap();
    let mut options = tokio::net::windows::named_pipe::ClientOptions::new();
    if let Some(flags) = qos {
        options.security_qos_flags(flags);
    }
    let mut client = options.open(&name).unwrap();
    let ten = Duration::from_secs(10);
    tokio::time::timeout(ten, server.connect()).await.expect("the pipe client connects within 10 s").unwrap();
    // The server must have read from the pipe before it can impersonate.
    client.write_all(b"x").await.unwrap();
    let mut one = [0u8; 1];
    let mut server = server;
    tokio::time::timeout(ten, server.read_exact(&mut one)).await.expect("the server reads the client's byte within 10 s").unwrap();
    let handle = server.as_raw_handle() as usize;
    let result = tokio::time::timeout(
        ten,
        tokio::task::spawn_blocking(move || {
            let mut reader = crate::update_install::win::ImpersonatingReader::of(handle as *mut std::ffi::c_void);
            reader.read(&path, MAX_INSTALLER_BYTES)
        }),
    )
    .await
    .expect("the impersonated read finishes within 10 s")
    .unwrap();
    drop(client);
    drop(server);
    result
}

#[tokio::test(flavor = "multi_thread")]
async fn the_real_impersonating_reader_reads_a_file_when_the_client_allows_impersonation() {
    // SECURITY_IMPERSONATION (0x20000) | SECURITY_SQOS_PRESENT (0x100000), as the daemon's pipe client opens.
    let (_dir, path) = temp_file_with(PAYLOAD);
    let bytes = impersonated_read("imp-ok", Some(0x0002_0000 | 0x0010_0000), path).await.expect("an impersonating client can be read as");
    assert_eq!(bytes, PAYLOAD);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_real_impersonating_reader_refuses_a_client_that_only_allows_identification() {
    let (_dir, path) = temp_file_with(PAYLOAD);
    let err = impersonated_read("imp-ident", None, path).await.expect_err("tokio's default is identification only");
    assert!(err.to_string().contains("no impersonation"), "{err}");
}

#[test]
fn the_real_probe_tells_a_running_process_from_a_reused_pid() {
    use crate::update_install::win::{creation_time, WinProbe};
    use std::os::windows::io::AsRawHandle;
    let mut child = std::process::Command::new("ping")
        .args(["-n", "20", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let created = creation_time(child.as_raw_handle()).unwrap();
    let mut probe = WinProbe;
    assert!(probe.alive(child.id(), created), "the true creation time: alive");
    assert!(!probe.alive(child.id(), created + 1), "a wrong creation time is a different process");
    child.kill().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < end, "the killed child did not exit within 10 s");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!probe.alive(child.id(), created), "an ended process is not alive");
}

#[test]
fn the_real_setup_launcher_creates_suspended_with_a_private_tmp_and_temp() {
    use crate::update_install::win::StdSetupLauncher;
    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path().join("tmp-private");
    std::fs::create_dir(&tmp).unwrap();
    let out = dir.path().join("env.txt");
    let cmd = Path::new(r"C:\Windows\System32\cmd.exe");
    // cmd expands %TMP% and %TEMP% from the environment block it was given, then exits with code 7.
    let args = format!("/C \"echo %TMP%;%TEMP%> \"{}\" & exit 7\"", out.display());
    let mut launcher = StdSetupLauncher;
    let mut process = launcher.launch(cmd, &args, dir.path(), &tmp).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(process.try_wait().unwrap(), None, "created suspended: it has not run");
    assert!(!out.exists());
    assert_ne!(process.created(), 0);
    process.resume().unwrap();
    wait_until("the process to end", || process.try_wait().unwrap().is_some());
    assert_eq!(process.try_wait().unwrap(), Some(7));
    let text = std::fs::read_to_string(&out).unwrap();
    let want = tmp.to_string_lossy().into_owned();
    assert_eq!(text.trim(), format!("{want};{want}"), "TMP and TEMP point at the private folder");
}

#[test]
fn a_real_setup_process_that_is_dropped_before_resume_is_terminated() {
    use crate::update_install::win::StdSetupLauncher;
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran.txt");
    let args = format!("/C \"echo ran> \"{}\"\"", marker.display());
    let process = StdSetupLauncher.launch(Path::new(r"C:\Windows\System32\cmd.exe"), &args, dir.path(), dir.path()).unwrap();
    let (pid, created) = (process.id(), process.created());
    drop(process);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!marker.exists(), "it never ran");
    let mut probe = crate::update_install::win::WinProbe;
    assert!(!probe.alive(pid, created), "and the process is gone");
}

// ---- update-result.json gates the per-user script ----

#[test]
fn setup_user_runs_only_for_a_fresh_ok_result_for_this_version_and_the_tray_always_starts() {
    let tray = |r: &Rig| format!("\"{}\\canvas-rgb.exe\" --after-update", r.app.display());
    // (label, how the result file is arranged)
    type Arrange = fn(&Rig);
    let cases: [(&str, Arrange, bool); 6] = [
        ("no result file", |_| {}, false),
        ("a failed setup", |r| r.write_result(false, "0.1.1"), false),
        ("another version", |r| r.write_result(true, "0.0.9"), false),
        ("a stale file from an earlier run", |r| {
            r.write_result(true, "0.1.1");
            let old = std::time::SystemTime::now() - Duration::from_secs(3600);
            std::fs::File::options().write(true).open(r.app.join("update-result.json")).unwrap().set_modified(old).unwrap();
        }, false),
        ("an unreadable file", |r| std::fs::write(r.app.join("update-result.json"), b"{ nope").unwrap(), false),
        ("a fresh ok result", |r| r.write_result(true, "0.1.1"), true),
    ];
    for (label, arrange, runs_script) in cases {
        let r = rig("0.1.1");
        r.leave_pending(0, 0, now());
        arrange(&r);
        assert_eq!(r.installer.resume_once(), Some(Finish::Done), "{label}");
        let commands = r.commands();
        if runs_script {
            assert_eq!(commands, vec![ps_line(&r), tray(&r)], "{label}");
        } else {
            assert_eq!(commands, vec![tray(&r)], "{label}: no per-user script, but the tray starts so the failure shows");
        }
        assert!(r.staged_files().is_empty(), "{label}: cleaned up");
    }
}

// ---- a setup that outlives the wait limit ----

#[test]
fn a_hung_or_stranded_setup_older_than_the_wait_limit_is_ended_and_the_new_install_goes_ahead() {
    let r = rig("0.1.0");
    // The rig's wait limit is 5 s; this marker is 100 s old and its setup is still there.
    let old = r.leave_pending(7000, 1, now() - 100);
    r.probe.procs.lock().unwrap().insert(7000, (1, u32::MAX));
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    assert_eq!(*r.probe.terminated.lock().unwrap(), vec![7000], "the verified setup process was ended");
    assert!(!Path::new(&old.staged).exists(), "and its staged file discarded");
}

#[test]
fn a_young_marker_with_a_live_setup_is_still_a_conflict_and_nothing_is_ended() {
    let r = rig("0.1.0");
    r.leave_pending(7000, 1, now());
    r.probe.procs.lock().unwrap().insert(7000, (1, u32::MAX));
    assert_eq!(code_of(&r.install(PAYLOAD, SIG_011)), error_codes::CONFLICT);
    assert!(r.probe.terminated.lock().unwrap().is_empty());
}

#[test]
fn a_stranded_setup_is_ended_by_the_new_service_on_start() {
    let r = rig("0.1.1");
    let old = r.leave_pending(5000, 99, now() - 100);
    r.probe.procs.lock().unwrap().insert(5000, (99, u32::MAX));
    assert_eq!(r.installer.resume_once(), None);
    assert_eq!(*r.probe.terminated.lock().unwrap(), vec![5000]);
    assert!(!r.marker_exists());
    assert!(!Path::new(&old.staged).exists());
    assert!(r.commands().is_empty(), "nothing finishes for a setup that never completed");
}

#[test]
fn a_stranded_pid_that_was_reused_by_another_process_is_not_ended() {
    let r = rig("0.1.0");
    r.leave_pending(7000, 1, now() - 100);
    r.probe.procs.lock().unwrap().insert(7000, (2, u32::MAX));
    assert!(matches!(r.install(PAYLOAD, SIG_011), PipeResponse::UpdateStarted { .. }));
    assert!(r.probe.terminated.lock().unwrap().is_empty());
}

// ---- dev builds order correctly ----

#[test]
fn dev_builds_order_between_the_release_before_and_the_release_after() {
    assert!(check_newer("0.1.2-dev.5", "0.1.1").is_ok(), "a dev build is newer than the release before it");
    assert!(check_newer("0.1.2", "0.1.2-dev.5").is_ok(), "the release is newer than its dev builds");
    assert!(check_newer("0.1.2-dev.6", "0.1.2-dev.5").is_ok(), "a later dev build is newer");
    assert!(check_newer("0.1.2-dev.5", "0.1.2-dev.5").unwrap_err().message.contains("replay"), "not newer than itself");
    assert!(check_newer("0.1.2-dev.5", "0.1.2").unwrap_err().message.contains("downgrade"), "a dev build never replaces its release");
}

// ---- the install folder is trusted before it is used ----

/// Ways {app} can be untrustworthy. Each arranges the rig and names the path the refusal must mention.
type Breaker = fn(&Rig) -> PathBuf;

fn app_breakers() -> Vec<(&'static str, Breaker)> {
    vec![
        ("the folder is writable by non-admins", |r| {
            r.writable.lock().unwrap().push(r.app.clone());
            r.app.clone()
        }),
        ("installer\\ is writable by non-admins", |r| {
            let p = r.app.join("installer");
            r.writable.lock().unwrap().push(p.clone());
            p
        }),
        ("setup-user.ps1 is writable by non-admins", |r| {
            let p = r.app.join("installer").join("setup-user.ps1");
            r.writable.lock().unwrap().push(p.clone());
            p
        }),
        ("lib.ps1 (dot-sourced by setup-user.ps1) is writable by non-admins", |r| {
            let p = r.app.join("installer").join("lib.ps1");
            r.writable.lock().unwrap().push(p.clone());
            p
        }),
        ("canvas-rgb.exe is owned by someone else", |r| {
            let p = r.app.join("canvas-rgb.exe");
            r.untrusted.lock().unwrap().push(p.clone());
            p
        }),
        ("the folder is owned by someone else", |r| {
            r.untrusted.lock().unwrap().push(r.app.clone());
            r.app.clone()
        }),
        ("canvas-rgb-svc.exe (the service, run as SYSTEM) is writable by non-admins", |r| {
            let p = r.app.join("canvas-rgb-svc.exe");
            r.writable.lock().unwrap().push(p.clone());
            p
        }),
        ("canvas-rgb-svc.exe is owned by someone else", |r| {
            let p = r.app.join("canvas-rgb-svc.exe");
            r.untrusted.lock().unwrap().push(p.clone());
            p
        }),
        ("canvas-rgb-svc.exe is missing", |r| {
            let p = r.app.join("canvas-rgb-svc.exe");
            std::fs::remove_file(&p).unwrap();
            p
        }),
        ("a file deep in openrgb\\ is writable by non-admins", |r| {
            let deep = r.app.join("openrgb").join("plugins").join("evil.dll");
            std::fs::create_dir_all(deep.parent().unwrap()).unwrap();
            std::fs::write(&deep, b"MZ").unwrap();
            r.writable.lock().unwrap().push(deep.clone());
            deep
        }),
        ("a file in openrgb\\ is owned by someone else", |r| {
            let f = r.app.join("openrgb").join("OpenRGB.exe");
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(&f, b"MZ").unwrap();
            r.untrusted.lock().unwrap().push(f.clone());
            f
        }),
        ("openrgb\\ holds a junction", |r| {
            let elsewhere = r.dir.path().join("elsewhere-openrgb");
            std::fs::create_dir_all(&elsewhere).unwrap();
            let link = r.app.join("openrgb").join("plugins");
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            mklink_junction(&link, &elsewhere);
            link
        }),
        ("a folder above the install folder lets a non-admin change its permissions or delete in it", |r| {
            let parent = r.app.parent().unwrap().to_path_buf();
            r.controlled.lock().unwrap().push(parent.clone());
            parent
        }),
        ("a folder above the install folder is owned by someone else", |r| {
            let parent = r.app.parent().unwrap().to_path_buf();
            r.untrusted.lock().unwrap().push(parent.clone());
            parent
        }),
        ("canvas-rgb.exe is missing", |r| {
            let p = r.app.join("canvas-rgb.exe");
            std::fs::remove_file(&p).unwrap();
            p
        }),
        ("a leftover update-result.json is writable by non-admins", |r| {
            r.write_result(true, "0.1.1");
            let p = r.app.join("update-result.json");
            r.writable.lock().unwrap().push(p.clone());
            p
        }),
        ("installer\\ is a junction", |r| {
            let p = r.app.join("installer");
            let elsewhere = r.dir.path().join("elsewhere-installer");
            std::fs::create_dir_all(&elsewhere).unwrap();
            std::fs::write(elsewhere.join("setup-user.ps1"), b"# evil").unwrap();
            std::fs::remove_dir_all(&p).unwrap();
            mklink_junction(&p, &elsewhere);
            p
        }),
    ]
}

#[test]
fn an_untrusted_install_folder_refuses_the_install_before_anything_is_staged() {
    for (label, breaker) in app_breakers() {
        let r = rig("0.1.0");
        let path = breaker(&r);
        let before: Vec<_> = walk(&r.app);
        let response = r.install(PAYLOAD, SIG_011);
        assert_eq!(code_of(&response), error_codes::INTERNAL, "{label}: {response:?}");
        assert!(message_of(&response).contains(path.to_string_lossy().as_ref()), "{label}: names {}: {response:?}", path.display());
        assert_eq!(r.launches(), 0, "{label}");
        assert!(!r.updates().exists(), "{label}: nothing was staged (not even the staging folder was needed)");
        assert_eq!(walk(&r.app), before, "{label}: nothing on disk changed in the install folder");
        assert!(!r.installer.is_busy(), "{label}");
    }
}

#[test]
fn an_untrusted_install_folder_skips_the_finish_steps() {
    for (label, breaker) in app_breakers() {
        let r = rig("0.1.1");
        r.leave_pending(0, 0, now());
        r.write_result(true, "0.1.1");
        // The marker and staged file are in place; now the folder goes bad.
        let _ = breaker(&r);
        assert_eq!(r.installer.resume_once(), Some(Finish::Done), "{label}");
        assert!(r.commands().is_empty(), "{label}: neither setup-user.ps1 nor the tray was started");
    }
}

/// Every path under `root` with its length, sorted: a cheap "nothing changed" fingerprint.
fn walk(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let meta = std::fs::symlink_metadata(e.path()).unwrap();
            out.push((e.path(), meta.len()));
            if meta.is_dir() {
                stack.push(e.path());
            }
        }
    }
    out.sort();
    out
}

// ---- setup-user.ps1's outcome is logged, and the tray starts regardless ----

/// Captures what the install code logs on this thread.
fn capture_logs<T>(f: impl FnOnce() -> T) -> (T, String) {
    #[derive(Clone)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let sink = Sink(Arc::new(Mutex::new(Vec::new())));
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt().with_writer(move || writer.clone()).with_ansi(false).with_max_level(tracing::Level::TRACE).finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    (out, text)
}

#[test]
fn a_non_zero_setup_user_exit_is_logged_as_a_warning_with_the_code_and_the_tray_still_starts() {
    let r = rig("0.1.1");
    r.leave_pending(0, 0, now());
    r.write_result(true, "0.1.1");
    // The fake step exits with code 1.
    let (outcome, logs) = capture_logs(|| r.installer.resume_once());
    assert_eq!(outcome, Some(Finish::Done));
    assert_eq!(r.commands().len(), 2, "the tray started after the failing script");
    let line = logs.lines().find(|l| l.contains("non-zero code")).unwrap_or_else(|| panic!("no warning in: {logs}"));
    assert!(line.contains("WARN"), "{line}");
    assert!(line.contains("0x00000001"), "the code is in the line: {line}");
}

#[test]
fn a_setup_user_that_hits_the_step_limit_is_logged_as_a_warning_and_the_tray_still_starts() {
    let r = rig_with("0.1.1", PUB_TEST, |c| c.user_step_limit = Duration::from_millis(30), |i| i);
    r.leave_pending(0, 0, now());
    r.write_result(true, "0.1.1");
    r.user.hang.store(true, Ordering::SeqCst);
    let (outcome, logs) = capture_logs(|| r.installer.resume_once());
    assert_eq!(outcome, Some(Finish::Done));
    assert!(r.user.killed.load(Ordering::SeqCst), "the hung script was ended before the tray started");
    assert!(logs.contains("ended the user step that hit the wait limit"), "{logs}");
    assert_eq!(r.commands().len(), 2, "the tray started after the hung script");
    let line = logs.lines().find(|l| l.contains("did not finish within the wait limit")).unwrap_or_else(|| panic!("no warning in: {logs}"));
    assert!(line.contains("WARN"), "{line}");
}

/// Real Win32, no fake: the folder Inno installed on this PC must pass the install-folder check, or
/// every update through the service is refused. Skips (and says so) where nothing is installed.
#[test]
fn the_real_installed_folder_passes_the_install_folder_check() {
    let app = PathBuf::from(r"C:\Program Files\canvas-rgb");
    if !app.join("canvas-rgb.exe").exists() {
        eprintln!("SKIPPED (UNKNOWN, not a pass): no install at {}; the real-folder check was not run", app.display());
        return;
    }
    let installer = UpdateInstaller::production(std::env::temp_dir(), app);
    installer.check_app_dir().expect("the real install folder must pass check_app_dir");
}

/// The parent walk on the real machine: the folders above the default install path (`C:\`,
/// `C:\Program Files`, owned by TrustedInstaller) must pass, or an update of the real install would start
/// refusing. Read-only; needs no install.
#[test]
fn the_real_folders_above_program_files_pass_the_parent_walk() {
    let app = PathBuf::from(r"C:\Program Files\canvas-rgb");
    crate::secure::check_install_parents(&crate::secure::WinFs, &app).expect("C:\\ and C:\\Program Files must pass the parent walk");
}
