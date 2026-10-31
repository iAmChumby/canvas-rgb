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
pub(crate) fn wait_until(what: &str, cond: impl Fn() -> bool) {
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
    fn has_nonadmin_write(&self, _p: &Path) -> io::Result<bool> {
        Ok(false)
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
    pub exited: Arc<AtomicBool>,
    pub fail: AtomicBool,
    /// Runs inside `launch`, i.e. while the service holds the staged handle.
    pub on_launch: Mutex<Option<LaunchHook>>,
}

struct FakeSetup(Arc<SetupState>);

struct FakeSetupProcess {
    exited: Arc<AtomicBool>,
}

impl ChildProcess for FakeSetupProcess {
    fn try_wait(&mut self) -> io::Result<Option<i32>> {
        Ok(self.exited.load(Ordering::SeqCst).then_some(0))
    }
    fn kill(&mut self) -> io::Result<()> {
        self.exited.store(true, Ordering::SeqCst);
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
}

impl SetupLauncher for FakeSetup {
    fn launch(&mut self, exe: &Path, raw_args: &str, dir: &Path) -> io::Result<Box<dyn SetupProcess>> {
        if self.0.fail.load(Ordering::SeqCst) {
            return Err(io::Error::other("fake setup failure"));
        }
        if let Some(hook) = self.0.on_launch.lock().unwrap().as_ref() {
            hook(exe);
        }
        self.0.launches.lock().unwrap().push((exe.to_path_buf(), raw_args.to_string(), dir.to_path_buf()));
        Ok(Box::new(FakeSetupProcess { exited: Arc::clone(&self.0.exited) }))
    }
}

#[derive(Default)]
pub(crate) struct UserState {
    pub commands: Mutex<Vec<String>>,
    pub no_user: AtomicBool,
}

struct FakeUser(Arc<UserState>, FakeProcesses);

impl UserCommands for FakeUser {
    fn start(&mut self, command_line: &str, _dir: &Path) -> io::Result<Box<dyn ChildProcess>> {
        if self.0.no_user.load(Ordering::SeqCst) {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "nobody is logged on (fake)"));
        }
        self.0.commands.lock().unwrap().push(command_line.to_string());
        // A child that has already exited.
        let child = crate::process::ProcessSpawner::spawn(&mut crate::fakes::FakeSpawner(self.1.clone()), Path::new("user-step.exe"), &[])?;
        self.1.exit_last();
        Ok(child)
    }
}

/// pid -> (creation time the process really has, polls it stays alive for; `u32::MAX` = forever).
#[derive(Default)]
pub(crate) struct ProbeState {
    pub procs: Mutex<HashMap<u32, (u64, u32)>>,
    pub asked: AtomicU32,
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
    std::fs::create_dir_all(&app).unwrap();
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
    };
    tweak(&mut cfg);
    let untrusted = Arc::new(Mutex::new(Vec::new()));
    let setup = Arc::new(SetupState::default());
    let user = Arc::new(UserState::default());
    let probe = Arc::new(ProbeState::default());
    let installer = UpdateInstaller::new(
        cfg,
        Box::new(TreeFs { untrusted: Arc::clone(&untrusted) }),
        Box::new(FakeSetup(Arc::clone(&setup))),
        Box::new(FakeUser(Arc::clone(&user), FakeProcesses::new())),
        Box::new(FakeProbe(Arc::clone(&probe))),
    );
    Rig { dir, home, app, installer: Arc::new(wrap(installer)), setup, user, probe, untrusted }
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
        };
        self.installer.write_marker(&updates, &pending).unwrap();
        pending
    }

    pub fn marker_exists(&self) -> bool {
        self.installer.marker_path().exists()
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
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

#[test]
fn a_source_that_changes_after_the_read_does_not_change_what_runs() {
    let r = rig("0.1.0");
    let source = r.dir.path().join("source.exe");
    std::fs::write(&source, PAYLOAD).unwrap();
    let mut reader = FakeReader::of(PAYLOAD);
    let path = source.clone();
    reader.after_read = Some(Box::new(move || std::fs::write(path, b"evil bytes after the read").unwrap()));
    let response = r.installer.install(WHO, SOURCE, SIG_011, reader);
    assert!(matches!(response, PipeResponse::UpdateStarted { .. }), "{response:?}");
    assert_eq!(std::fs::read(&source).unwrap(), b"evil bytes after the read", "the source did change");
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
fn a_fresh_marker_nobody_is_working_on_blocks_a_new_install() {
    let r = rig("0.1.0");
    r.leave_pending(7000, 1, now());
    let response = r.install(PAYLOAD, SIG_011);
    assert_eq!(code_of(&response), error_codes::CONFLICT, "{response:?}");
    assert_eq!(r.launches(), 0);
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
    // Something else now has pid 5000 (another creation time) and runs for a long time.
    r.probe.procs.lock().unwrap().insert(5000, (100, u32::MAX));
    assert_eq!(r.installer.resume_pending(), Some(Finish::Done));
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
fn with_nobody_logged_on_the_update_stays_pending() {
    let r = rig("0.1.1");
    r.leave_pending(0, 0, now());
    r.user.no_user.store(true, Ordering::SeqCst);
    assert_eq!(r.installer.resume_pending(), Some(Finish::NoUser));
    assert!(r.marker_exists());
    assert_eq!(r.staged_files().len(), 2, "marker and staged file both kept");
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
