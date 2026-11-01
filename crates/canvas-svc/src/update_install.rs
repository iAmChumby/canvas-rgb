//! Installs a signed update as SYSTEM, with no permission prompt (#43).
//!
//! The pipe delivers `InstallUpdate { installer, signature }` from the daemon. Per request, in order:
//! 1. The request is shaped right (an absolute drive path, a non-empty signature of at most 4 KiB) and
//!    no other install is running (`conflict`).
//! 2. The installer is read **as the pipe client**, never as SYSTEM ([`ClientFileReader`]): the service
//!    impersonates the client for the read, so it cannot be used to read a file the user cannot. A
//!    reparse point at the file, or a file over 256 MiB, is refused.
//! 3. The bytes are staged in `<home>\updates\` ([`UpdateInstaller::secure_updates_dir`]): a folder only
//!    SYSTEM and Administrators may touch, checked every time and failing closed (a junction, a symlink
//!    or an untrusted owner is refused and named, never deleted, renamed or adopted). The copy is
//!    written `CREATE_NEW` under a fresh name.
//! 4. The staged file is reopened with share-read only. While that handle lives nobody can write,
//!    replace, rename or delete the file. Its bytes are read back **from that handle** and checked:
//!    a minisign signature against the release key, and a signed `version:` that is valid semver and
//!    strictly newer than this service ([`verify_update`]). Older is a downgrade, equal is a replay.
//! 5. A marker (`update-pending.json`) is written, then setup runs as SYSTEM with the handle still held.
//! 6. After setup, whoever is alive finishes the update: this service if setup failed early and left it
//!    running, otherwise the new service on its next start ([`UpdateInstaller::resume_pending`]). That
//!    means `setup-user.ps1` and then `canvas-rgb.exe --after-update`, both as the console user (the
//!    same `WTSQueryUserToken` path the daemon launch uses), then deleting the staged file and the marker.
//!
//! Every OS touch sits behind a trait ([`ClientFileReader`], [`FsAcl`], [`SetupLauncher`],
//! [`UserCommands`], [`ProcessProbe`]), so the suite runs the whole flow on fakes and never starts setup.

use crate::pipe_server::Admitted;
use crate::process::ChildProcess;
use crate::secure::{check_tree, offending, refused, FsAcl};
use crate::paths;
use base64::Engine;
use canvas_proto::api::error_codes;
use canvas_proto::consts::{SERVICE_SETUP_ARGS, UPDATE_PUBKEY};
use canvas_proto::pipe::PipeResponse;
use minisign_verify::{PublicKey, Signature};
use semver::{BuildMetadata, Version};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The largest installer the service stages.
pub const MAX_INSTALLER_BYTES: u64 = 256 * 1024 * 1024;
/// The longest signature text accepted (a minisign signature file is under 400 bytes once encoded).
pub const MAX_SIGNATURE_BYTES: usize = 4096;
/// The marker in the staging folder that says an update is in flight.
pub const MARKER_FILE: &str = "update-pending.json";
/// A marker older than this is stale: it is dropped, with the staged file it names.
pub const MARKER_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const STAGED_PREFIX: &str = "canvas-rgb-setup-";
const STAGED_SUFFIX: &str = ".exe";
const TMP_PREFIX: &str = "tmp-";
/// `FILE_SHARE_READ`: readers only. No writer, no delete, no rename while the handle lives.
const SHARE_READ: u32 = 1;

// ---- refusals ----

/// Why a request was refused: an `Error` response with this `code` (`validation`, `conflict`, `internal`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    fn validation(message: impl Into<String>) -> Self {
        Self { code: error_codes::VALIDATION, message: message.into() }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self { code: error_codes::CONFLICT, message: message.into() }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self { code: error_codes::INTERNAL, message: message.into() }
    }
}

impl From<Refusal> for PipeResponse {
    fn from(r: Refusal) -> Self {
        PipeResponse::Error { code: r.code.into(), message: r.message }
    }
}

// ---- seams ----

/// Reads the installer as the pipe client. Real: [`win::ImpersonatingReader`]. Tests: a fake.
pub trait ClientFileReader: Send {
    /// The whole file, or an error. Refuses a reparse point, a non-file and a file over `max_bytes`.
    fn read(&mut self, path: &Path, max_bytes: u64) -> io::Result<Vec<u8>>;
}

/// The reader for a client that cannot be impersonated: it refuses every read.
pub struct RefusingReader;

impl ClientFileReader for RefusingReader {
    fn read(&mut self, _path: &Path, _max_bytes: u64) -> io::Result<Vec<u8>> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "this client cannot be impersonated"))
    }
}

/// A running setup. `created` is its creation time (a `FILETIME`), recorded with the PID in the marker
/// so a reused PID is not mistaken for it.
pub trait SetupProcess: ChildProcess {
    fn created(&self) -> u64;
    /// Lets a process that was created suspended run. [`SetupLauncher::launch`] returns it suspended, so
    /// the marker (with its PID and creation time) is on disk before a single instruction of setup runs.
    /// A process dropped without being resumed is terminated.
    fn resume(&mut self) -> io::Result<()>;
}

/// Starts setup as the service's own token (SYSTEM), with no window, **suspended**.
pub trait SetupLauncher: Send {
    /// `raw_args` is the exact argument text (`/VERYSILENT ... /LOG="..."`), `dir` the working directory.
    /// `tmp` is a private folder (SYSTEM and Administrators only): setup gets `TMP` and `TEMP` pointing
    /// at it, so the files it unpacks and then runs as SYSTEM never sit in the system temp folder, where
    /// ordinary users may create files.
    fn launch(&mut self, exe: &Path, raw_args: &str, dir: &Path, tmp: &Path) -> io::Result<Box<dyn SetupProcess>>;
}

/// Starts a command as the console user, non-elevated.
pub trait UserCommands: Send {
    /// `command_line` is the quoted exe and its arguments. An error of kind
    /// [`io::ErrorKind::NotConnected`] means nobody is logged on at the console.
    fn start(&mut self, command_line: &str, dir: &Path) -> io::Result<Box<dyn ChildProcess>>;
}

/// Whether the process that was started as `pid` at `created` still runs. A different creation time
/// means the PID was reused, so it is not the process.
pub trait ProcessProbe: Send {
    fn alive(&mut self, pid: u32, created: u64) -> bool;
    /// Ends the process started as `pid` at `created`, only if it is that process (a reused PID is left
    /// alone). `true` when it is gone afterwards.
    fn terminate(&mut self, pid: u32, created: u64) -> bool;
}

// ---- configuration ----

pub struct UpdateConfig {
    /// The machine home (`%ProgramData%\canvas-rgb`).
    pub home: PathBuf,
    /// The install folder: the service exe's own folder.
    pub app_dir: PathBuf,
    /// This service's version. A signed version must be strictly newer.
    pub own_version: String,
    /// The release key: base64 of the minisign public key file.
    pub pubkey: String,
    pub powershell: PathBuf,
    /// How often the waits look.
    pub poll: Duration,
    /// The longest wait for setup to end.
    pub setup_wait_limit: Duration,
    /// The longest wait for `setup-user.ps1`.
    pub user_step_limit: Duration,
    /// The longest wait for a console user to appear within one finish attempt.
    pub user_session_wait: Duration,
    /// How often a finish that found nobody logged on is tried again (until the marker is stale).
    pub resume_retry: Duration,
    /// The clock the age of the update marker is measured with, in milliseconds since the Unix
    /// epoch. A test replaces it, so the age of a marker is the test's to decide and not the
    /// wall clock's (class `cause:timing-test`, #66).
    pub now_unix_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl UpdateConfig {
    /// The production settings.
    pub fn production(home: PathBuf, app_dir: PathBuf) -> Self {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        Self {
            home,
            app_dir,
            own_version: env!("CARGO_PKG_VERSION").to_string(),
            pubkey: UPDATE_PUBKEY.to_string(),
            powershell: root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
            poll: Duration::from_millis(250),
            setup_wait_limit: Duration::from_secs(30 * 60),
            user_step_limit: Duration::from_secs(5 * 60),
            user_session_wait: Duration::from_secs(2 * 60),
            resume_retry: Duration::from_secs(10),
            now_unix_ms: Arc::new(now_unix_ms),
        }
    }
}

// ---- the marker ----

/// `updates\update-pending.json`. Written before setup starts, read by whoever finishes the update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingUpdate {
    /// The signed version being installed.
    pub version: String,
    /// The staged installer (a file this service made, inside the staging folder).
    pub staged: String,
    /// The setup process. Known before setup runs: it is created suspended.
    pub setup_pid: u32,
    /// The setup process's creation time (a `FILETIME`), against PID reuse.
    pub setup_created: u64,
    pub requester_pid: u32,
    pub requester_session: u32,
    /// Seconds since the Unix epoch.
    pub started_unix: u64,
    /// Milliseconds since the Unix epoch: the same moment, to the millisecond. The age of the
    /// marker is measured with it, so a wait limit shorter than a second is not rounded down to
    /// nothing. Zero in a marker written before this field existed; then `started_unix` is used.
    #[serde(default)]
    pub started_unix_ms: u64,
    /// The private `TMP`/`TEMP` folder setup ran with (a folder this service made inside the staging
    /// folder). Empty in a marker from before it existed.
    #[serde(default)]
    pub tmp_dir: String,
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The wall clock, in milliseconds since the Unix epoch. The default clock of the marker age.
pub(crate) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---- verification ----

fn b64_to_string(text: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(text).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| "it is not text".to_string())
}

/// The `version:` field of a minisign trusted comment, which the Tauri CLI writes as tab-separated
/// `key:value` pairs (`timestamp:..\tfile:..\tversion:0.1.1`).
pub fn signed_version(trusted_comment: &str) -> Option<&str> {
    trusted_comment.split('\t').find_map(|field| field.strip_prefix("version:"))
}

fn parse_version(text: &str) -> Result<Version, semver::Error> {
    let mut v = Version::parse(text.trim().trim_start_matches('v'))?;
    // Build metadata does not take part in precedence (the semver crate's `Ord` adds it as a tiebreak).
    v.build = BuildMetadata::EMPTY;
    Ok(v)
}

/// The signed version must be valid semver and strictly newer than `own`.
pub fn check_newer(signed: &str, own: &str) -> Result<Version, Refusal> {
    let signed_v = parse_version(signed)
        .map_err(|e| Refusal::validation(format!("the signed version '{signed}' is not valid semver: {e}")))?;
    let own_v = parse_version(own).map_err(|e| Refusal::internal(format!("this service's version '{own}' is not valid semver: {e}")))?;
    match signed_v.cmp(&own_v) {
        std::cmp::Ordering::Greater => Ok(signed_v),
        std::cmp::Ordering::Equal => Err(Refusal::validation(format!(
            "the signed version {signed_v} is the version already installed; a replay is refused"
        ))),
        std::cmp::Ordering::Less => Err(Refusal::validation(format!(
            "the signed version {signed_v} is older than the installed {own_v}; a downgrade is refused"
        ))),
    }
}

/// Checks `bytes` against `signature_text` (the `.sig` file's text: base64 of a minisign signature
/// file) under `pubkey` (base64 of a minisign public key file), then the signed version against
/// `own_version`. Returns the signed version. The trusted comment is read only after the signature,
/// which covers it, has verified.
pub fn verify_update(pubkey: &str, own_version: &str, bytes: &[u8], signature_text: &str) -> Result<Version, Refusal> {
    let key_text = b64_to_string(pubkey.trim()).map_err(|e| Refusal::internal(format!("the release key is unreadable: {e}")))?;
    let key = PublicKey::decode(&key_text).map_err(|e| Refusal::internal(format!("the release key is unreadable: {e}")))?;
    let sig_text = b64_to_string(signature_text.trim()).map_err(|e| Refusal::validation(format!("the signature is not base64 text: {e}")))?;
    let signature = Signature::decode(&sig_text).map_err(|e| Refusal::validation(format!("the signature is unreadable: {e}")))?;
    key.verify(bytes, &signature, true)
        .map_err(|e| Refusal::validation(format!("the signature does not match the installer under the release key: {e}")))?;
    let Some(signed) = signed_version(signature.trusted_comment()) else {
        return Err(Refusal::validation("the signature carries no version, so it cannot be checked against the installed one"));
    };
    check_newer(signed, own_version)
}

// ---- request shape ----

/// A drive-letter absolute path only: no relative path, no UNC share, no device path (`\\.\pipe\x`).
fn validate_request(installer: &str, signature: &str) -> Result<PathBuf, Refusal> {
    if signature.trim().is_empty() {
        return Err(Refusal::validation("the signature is empty"));
    }
    if signature.len() > MAX_SIGNATURE_BYTES {
        return Err(Refusal::validation(format!("the signature is over {MAX_SIGNATURE_BYTES} bytes")));
    }
    if installer.contains('\0') {
        return Err(Refusal::validation("the installer path has a NUL in it"));
    }
    let path = PathBuf::from(installer);
    let mut parts = path.components();
    let drive = matches!(parts.next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)));
    if !drive || !matches!(parts.next(), Some(Component::RootDir)) || path.file_name().is_none() {
        return Err(Refusal::validation(format!(
            "the installer path must be absolute, on a drive letter ({installer})"
        )));
    }
    Ok(path)
}

// ---- state ----

/// One install at a time. Cleared when dropped.
struct BusyGuard(Arc<AtomicBool>);

impl BusyGuard {
    fn claim(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(Arc::clone(flag)))
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// The staged file this service made. Holding it holds the share-read handle. Dropping it closes the
/// handle first, then deletes the file unless `keep` was set. It only ever deletes the file it made.
struct Staged {
    path: PathBuf,
    file: Option<File>,
    delete_on_drop: bool,
}

impl Staged {
    fn keep(&mut self) {
        self.delete_on_drop = false;
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        self.file.take();
        if self.delete_on_drop {
            remove_file_logged(&self.path, "the staged installer");
        }
    }
}

/// The private `TMP`/`TEMP` folder this service made for setup. Dropping it deletes the folder and what
/// is in it unless `keep` was set; it only ever deletes the folder it made.
struct SetupTmp {
    path: PathBuf,
    delete_on_drop: bool,
}

impl SetupTmp {
    fn keep(&mut self) {
        self.delete_on_drop = false;
    }
}

impl Drop for SetupTmp {
    fn drop(&mut self) {
        if self.delete_on_drop {
            remove_dir_logged(&self.path);
        }
    }
}

fn remove_dir_logged(path: &Path) {
    match std::fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(path = %path.display(), error = %e, "could not delete the setup temp folder"),
    }
}

fn remove_file_logged(path: &Path, what: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(path = %path.display(), error = %e, "could not delete {what}"),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What the finish step ended with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// The user steps ran (or failed for good); the staged file and the marker are gone.
    Done,
    /// Nobody is logged on at the console; the marker and the staged file stay for the next start.
    NoUser,
}

type StageSeam = Box<dyn Fn(&Path) + Send + Sync>;

pub struct UpdateInstaller {
    cfg: UpdateConfig,
    fs: Mutex<Box<dyn FsAcl>>,
    setup: Mutex<Box<dyn SetupLauncher>>,
    user: Mutex<Box<dyn UserCommands>>,
    probe: Mutex<Box<dyn ProcessProbe>>,
    busy: Arc<AtomicBool>,
    counter: AtomicU64,
    /// Test seam: runs between the staged write and the share-read reopen. `None` in production.
    seam: Option<StageSeam>,
    /// Test seam: the watcher thread "fails to start". `false` in production.
    fail_watcher: bool,
}

impl UpdateInstaller {
    pub fn new(
        cfg: UpdateConfig,
        fs: Box<dyn FsAcl>,
        setup: Box<dyn SetupLauncher>,
        user: Box<dyn UserCommands>,
        probe: Box<dyn ProcessProbe>,
    ) -> Self {
        Self {
            cfg,
            fs: Mutex::new(fs),
            setup: Mutex::new(setup),
            user: Mutex::new(user),
            probe: Mutex::new(probe),
            busy: Arc::new(AtomicBool::new(false)),
            counter: AtomicU64::new(0),
            seam: None,
            fail_watcher: false,
        }
    }

    /// The real thing: Win32 filesystem checks, setup as SYSTEM, user commands in the console session.
    pub fn production(home: PathBuf, app_dir: PathBuf) -> Self {
        Self::new(
            UpdateConfig::production(home, app_dir),
            Box::new(crate::secure::WinFs),
            Box::new(win::StdSetupLauncher),
            Box::new(win::ConsoleUserCommands),
            Box::new(win::WinProbe),
        )
    }

    /// Runs `seam` with the staged path between the write and the reopen. Tests only.
    pub fn with_stage_seam(mut self, seam: impl Fn(&Path) + Send + Sync + 'static) -> Self {
        self.seam = Some(Box::new(seam));
        self
    }

    /// Makes the watcher thread fail to start. Tests only.
    pub fn with_failing_watcher(mut self) -> Self {
        self.fail_watcher = true;
        self
    }

    /// Whether an install (or a resumed finish) is running.
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    fn updates_dir(&self) -> PathBuf {
        paths::updates_dir(&self.cfg.home)
    }

    pub(crate) fn marker_path(&self) -> PathBuf {
        self.updates_dir().join(MARKER_FILE)
    }

    // ---- the staging folder ----

    /// Checks the staging folder and returns it, creating it (private DACL) when `create` and it is
    /// missing. `Ok(None)` when it is missing and `create` is false. Fails closed and names the path:
    /// the home or the folder is a junction or symlink, owned by someone other than SYSTEM or
    /// Administrators, or writable by someone else, or anything under it is. Nothing is deleted, renamed
    /// or adopted.
    pub fn secure_updates_dir(&self, create: bool) -> io::Result<Option<PathBuf>> {
        let mut fs = lock(&self.fs);
        let home = self.cfg.home.as_path();
        if !fs.exists(home)? || fs.is_reparse(home)? || !fs.is_dir(home)? {
            return Err(offending(home, "the home is missing, not a directory, or a junction or symlink"));
        }
        if !fs.owner_trusted(home)? {
            return Err(offending(home, "the home is owned by someone other than SYSTEM or Administrators"));
        }
        let dir = self.updates_dir();
        if fs.exists(&dir)? {
            if fs.is_reparse(&dir)? {
                return Err(offending(&dir, "the update folder is a junction or symlink"));
            }
            if !fs.is_dir(&dir)? {
                return Err(refused(&dir, "exists and is not a directory"));
            }
            if !fs.owner_trusted(&dir)? {
                return Err(offending(&dir, "the update folder is owned by someone other than SYSTEM or Administrators"));
            }
            if fs.has_nonadmin_write(&dir)? {
                return Err(offending(&dir, "the update folder has write access granted to a non-admin principal"));
            }
        } else if create {
            fs.create_dir_private(&dir)?;
        } else {
            return Ok(None);
        }
        let first = fs.file_id(&dir)?;
        check_tree(&**fs, &dir)?;
        if fs.file_id(&dir)? != first {
            return Err(refused(&dir, "file ID changed while checking; refusing"));
        }
        Ok(Some(dir))
    }

    /// The install folder is trusted with the same rules as the staging folder, because the update
    /// reads `update-result.json` there and starts `setup-user.ps1` and `canvas-rgb.exe` from it: no
    /// reparse point on the path to it, the folder, `installer\` and the files the finish step runs owned
    /// by SYSTEM or Administrators, and none of them writable by anyone else. Changes nothing; names the
    /// first path that fails.
    pub fn check_app_dir(&self) -> io::Result<()> {
        let fs = lock(&self.fs);
        let app = self.cfg.app_dir.as_path();
        for ancestor in app.ancestors() {
            // The drive root has no attributes worth reading; a missing ancestor is the folder's own failure below.
            if ancestor.parent().is_some() && fs.exists(ancestor)? && fs.is_reparse(ancestor)? {
                return Err(offending(ancestor, "the install path goes through a junction or symlink"));
            }
        }
        let installer = app.join("installer");
        let folders = [app.to_path_buf(), installer.clone()];
        // setup-user.ps1 dot-sources lib.ps1, so it is run as part of the script.
        let files = [installer.join("setup-user.ps1"), installer.join("lib.ps1"), app.join("canvas-rgb.exe")];
        let result = app.join("update-result.json");
        let mut all: Vec<(PathBuf, bool)> = folders.into_iter().map(|p| (p, true)).collect();
        all.extend(files.into_iter().map(|p| (p, false)));
        // A result file left by an earlier run is read too, so it is checked when it is there.
        if fs.exists(&result)? {
            all.push((result, false));
        }
        for (path, is_folder) in all {
            if !fs.exists(&path)? {
                return Err(offending(&path, "an install path the update needs is missing"));
            }
            if fs.is_reparse(&path)? {
                return Err(offending(&path, "an install path is a junction or symlink"));
            }
            if fs.is_dir(&path)? != is_folder {
                return Err(refused(&path, if is_folder { "is not a folder" } else { "is not a file" }));
            }
            if !fs.owner_trusted(&path)? {
                return Err(offending(&path, "an install path is owned by someone other than SYSTEM or Administrators"));
            }
            if fs.has_nonadmin_write(&path)? {
                return Err(offending(&path, "an install path has write access granted to a non-admin principal"));
            }
        }
        Ok(())
    }

    // ---- install ----

    /// Handles one `InstallUpdate`. `who` is the admitted pipe client (for the log), `reader` reads the
    /// installer as that client. Blocking: call it from a blocking thread.
    pub fn install(self: &Arc<Self>, who: Admitted, installer: &str, signature: &str, mut reader: Box<dyn ClientFileReader>) -> PipeResponse {
        tracing::info!(pid = who.pid, session = who.session, installer, "update install requested");
        match self.try_install(who, installer, signature, reader.as_mut()) {
            Ok(version) => {
                tracing::info!(pid = who.pid, session = who.session, %version, "update install started: setup is running");
                PipeResponse::UpdateStarted { version }
            }
            Err(refusal) => {
                tracing::warn!(
                    pid = who.pid,
                    session = who.session,
                    code = refusal.code,
                    reason = %refusal.message,
                    "update install refused"
                );
                refusal.into()
            }
        }
    }

    fn try_install(self: &Arc<Self>, who: Admitted, installer: &str, signature: &str, reader: &mut dyn ClientFileReader) -> Result<String, Refusal> {
        let source = validate_request(installer, signature)?;
        // The slot is taken before anything is read, so concurrent requests cannot each hold a buffer
        // of up to 256 MiB.
        let guard = BusyGuard::claim(&self.busy).ok_or_else(|| Refusal::conflict("an update is already being installed"))?;

        self.check_app_dir()
            .map_err(|e| Refusal::internal(format!("the install folder cannot be trusted, so no update is staged: {e}")))?;

        let updates = self
            .secure_updates_dir(true)
            .map_err(|e| Refusal::internal(format!("the update folder cannot be trusted: {e}")))?
            .ok_or_else(|| Refusal::internal("the update folder could not be made"))?;
        self.supersede_or_refuse(&updates)?;

        // 1. Read it as the client.
        let bytes = reader
            .read(&source, MAX_INSTALLER_BYTES)
            .map_err(|e| Refusal::validation(format!("cannot read {} as the requesting user: {e}", source.display())))?;
        if bytes.is_empty() {
            return Err(Refusal::validation("the installer file is empty"));
        }

        // 2. Stage it (the first copy is dropped before the read-back), and 3. verify what is on disk,
        // through the handle that stays open.
        let (staged, on_disk) = self.stage(&updates, bytes)?;
        let version = verify_update(&self.cfg.pubkey, &self.cfg.own_version, &on_disk, signature)?;
        drop(on_disk);
        let version_text = version.to_string();

        // 4. A private TMP/TEMP for setup, then setup, created suspended.
        let tmp = self.make_setup_tmp(&updates)?;
        let log = paths::logs_dir(&self.cfg.home).join(format!("setup-{version_text}.log"));
        let args = format!("{SERVICE_SETUP_ARGS} /LOG=\"{}\"", log.display());
        let launched = lock(&self.setup).launch(&staged.path, &args, &updates, &tmp.path);
        let process = launched.map_err(|e| Refusal::internal(format!("could not start setup: {e}")))?;
        let process = Arc::new(Mutex::new(process));
        // From here every failure ends the suspended process: nothing of setup has run yet.
        let end_setup = |why: &str| {
            let _ = lock(&process).kill();
            tracing::error!(why, "ended the suspended setup process");
        };
        let pending = {
            let p = lock(&process);
            PendingUpdate {
                version: version_text.clone(),
                staged: staged.path.to_string_lossy().into_owned(),
                setup_pid: p.id(),
                setup_created: p.created(),
                requester_pid: who.pid,
                requester_session: who.session,
                started_unix: now_unix(),
                started_unix_ms: (self.cfg.now_unix_ms)(),
                tmp_dir: tmp.path.to_string_lossy().into_owned(),
            }
        };
        if let Err(e) = self.write_marker(&updates, &pending) {
            end_setup("the update marker could not be written");
            self.remove_marker();
            return Err(Refusal::internal(format!("cannot write the update marker (setup was ended before it ran): {e}")));
        }
        tracing::info!(pid = who.pid, session = who.session, setup_pid = pending.setup_pid, args = %args, "setup created suspended as SYSTEM; the marker is written");

        // 5. The watcher exists before setup runs. It waits for the go-ahead below.
        let (go, go_wait) = std::sync::mpsc::channel::<bool>();
        let me = Arc::clone(self);
        let watched = Arc::clone(&process);
        let watcher_pending = pending.clone();
        let work = move || {
            let mut staged = staged;
            let mut tmp = tmp;
            if go_wait.recv() == Ok(true) {
                let outcome = me.watch_setup(&watched, &watcher_pending, &mut staged, &mut tmp);
                // The slot frees before a retry, so a new install is not locked out while nobody is logged on.
                drop(guard);
                if outcome == Some(Finish::NoUser) {
                    me.resume_pending();
                }
            } else {
                // Setup never ran: take back what was made for it.
                staged.file.take();
                me.discard(&me.updates_dir(), &watcher_pending);
                drop(guard);
            }
        };
        let spawned = if self.fail_watcher {
            drop(work);
            Err(io::Error::other("the watcher thread could not be started (injected)"))
        } else {
            std::thread::Builder::new().name("update-finish".into()).spawn(work).map(|_| ())
        };
        if let Err(e) = spawned {
            end_setup("the update watcher thread could not start");
            self.remove_marker();
            return Err(Refusal::internal(format!("cannot start the update watcher (setup was ended before it ran): {e}")));
        }
        let resumed = lock(&process).resume();
        if let Err(e) = resumed {
            end_setup("setup could not be resumed");
            let _ = go.send(false);
            return Err(Refusal::internal(format!("could not let setup run: {e}")));
        }
        let _ = go.send(true);
        Ok(version_text)
    }

    /// A marker still there when no install runs in this process is an update nobody finished. While its
    /// setup process (PID and creation time) is alive it blocks a new install (`conflict`); once that
    /// process has ended, the new install supersedes it: the staged file, the setup temp folder and the
    /// marker are discarded.
    fn supersede_or_refuse(&self, updates: &Path) -> Result<(), Refusal> {
        let marker = self.marker_path();
        if !matches!(lock(&self.fs).exists(&marker), Ok(true)) {
            return Ok(());
        }
        match self.read_marker(&marker) {
            Ok(pending) => {
                if pending.setup_pid != 0 && lock(&self.probe).alive(pending.setup_pid, pending.setup_created) {
                    if !self.marker_is_overdue(&pending) {
                        return Err(Refusal::conflict(format!(
                            "an earlier update to {} is still running (setup pid {})",
                            pending.version, pending.setup_pid
                        )));
                    }
                    self.end_stranded_setup(&pending);
                }
                tracing::warn!(version = %pending.version, "a leftover update marker whose setup has ended is superseded by this install");
                self.discard(updates, &pending);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                tracing::warn!(error = %e, "an unreadable update marker is removed");
                self.remove_marker();
                Ok(())
            }
            Err(e) => Err(Refusal::internal(format!("an update marker is in the way and cannot be trusted: {e}"))),
        }
    }

    /// The marker is older than the longest wait for setup: that setup is hung, or was stranded suspended
    /// by a service crash between the marker write and the resume.
    ///
    /// The age is measured in milliseconds against the limit in milliseconds, so a limit shorter
    /// than a second is a limit and not nothing (class `cause:timing-test`, #66).
    fn marker_is_overdue(&self, pending: &PendingUpdate) -> bool {
        self.marker_age_ms(pending) > self.cfg.setup_wait_limit.as_millis()
    }

    /// How long ago the marker was written, in milliseconds. The clock is the config's, so a test
    /// decides the age of a marker.
    fn marker_age_ms(&self, pending: &PendingUpdate) -> u128 {
        // A marker from before the millisecond field existed falls back to its seconds.
        let started = if pending.started_unix_ms != 0 {
            pending.started_unix_ms
        } else {
            pending.started_unix.saturating_mul(1000)
        };
        u128::from((self.cfg.now_unix_ms)().saturating_sub(started))
    }

    /// Ends an overdue setup (PID and creation time verified by the probe). No job object ties setup to
    /// the service: a setup that is running must survive the service being stopped.
    fn end_stranded_setup(&self, pending: &PendingUpdate) {
        let gone = lock(&self.probe).terminate(pending.setup_pid, pending.setup_created);
        tracing::warn!(
            setup_pid = pending.setup_pid,
            version = %pending.version,
            gone,
            "ended a setup process that outlived the wait limit (hung, or stranded suspended)"
        );
    }

    /// A fresh private folder (SYSTEM and Administrators only) for setup's `TMP` and `TEMP`.
    fn make_setup_tmp(&self, updates: &Path) -> Result<SetupTmp, Refusal> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let path = updates.join(format!("{TMP_PREFIX}{}-{nanos}-{n}", std::process::id()));
        lock(&self.fs)
            .create_dir_private(&path)
            .map_err(|e| Refusal::internal(format!("cannot make the setup temp folder {}: {e}", path.display())))?;
        Ok(SetupTmp { path, delete_on_drop: true })
    }

    /// Writes `bytes` to a fresh file, `CREATE_NEW`, drops them, then reopens the file share-read only
    /// and reads it back. The returned bytes are what is on disk; the returned [`Staged`] holds the
    /// handle. At most one copy is in memory at a time.
    fn stage(&self, updates: &Path, bytes: Vec<u8>) -> Result<(Staged, Vec<u8>), Refusal> {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let name = format!("{STAGED_PREFIX}{}-{nanos}-{n}{STAGED_SUFFIX}", std::process::id());
        let path = updates.join(name);
        let len = bytes.len();

        // share_mode(0): nobody else has it open while it is written.
        let mut writer = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&path)
            .map_err(|e| Refusal::internal(format!("cannot create the staged file {}: {e}", path.display())))?;
        // From here the file is ours: dropping `staged` deletes it on any refusal.
        let mut staged = Staged { path: path.clone(), file: None, delete_on_drop: true };
        writer
            .write_all(&bytes)
            .and_then(|()| writer.sync_all())
            .map_err(|e| Refusal::internal(format!("cannot write the staged file {}: {e}", path.display())))?;
        drop(writer);
        drop(bytes);

        if let Some(seam) = &self.seam {
            seam(&path);
        }

        let mut held = OpenOptions::new()
            .read(true)
            .share_mode(SHARE_READ)
            .open(&path)
            .map_err(|e| Refusal::internal(format!("cannot reopen the staged file {}: {e}", path.display())))?;
        let mut on_disk = Vec::with_capacity(len);
        Read::by_ref(&mut held)
            .take(MAX_INSTALLER_BYTES + 1)
            .read_to_end(&mut on_disk)
            .map_err(|e| Refusal::internal(format!("cannot read back the staged file {}: {e}", path.display())))?;
        staged.file = Some(held);
        Ok((staged, on_disk))
    }

    // ---- the marker ----

    pub(crate) fn write_marker(&self, updates: &Path, pending: &PendingUpdate) -> io::Result<()> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let tmp = updates.join(format!("{MARKER_FILE}.{}-{n}.tmp", std::process::id()));
        let text = serde_json::to_vec_pretty(pending).map_err(io::Error::other)?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let written = file.write_all(&text).and_then(|()| file.sync_all());
        drop(file);
        if let Err(e) = written {
            remove_file_logged(&tmp, "a marker temp file");
            return Err(e);
        }
        std::fs::rename(&tmp, updates.join(MARKER_FILE)).inspect_err(|_| remove_file_logged(&tmp, "a marker temp file"))
    }

    pub(crate) fn read_marker(&self, marker: &Path) -> io::Result<PendingUpdate> {
        {
            let fs = lock(&self.fs);
            if fs.is_reparse(marker)? {
                return Err(offending(marker, "the update marker is a junction or symlink"));
            }
            if !fs.owner_trusted(marker)? {
                return Err(offending(marker, "the update marker is owned by someone other than SYSTEM or Administrators"));
            }
        }
        let text = std::fs::read(marker)?;
        serde_json::from_slice(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", marker.display())))
    }

    fn remove_marker(&self) {
        remove_file_logged(&self.marker_path(), "the update marker");
    }

    /// The staged path the marker names, only when it is a file this service could have made: directly
    /// inside the staging folder, with the staged-file name, and not a reparse point.
    fn staged_path_of(&self, updates: &Path, pending: &PendingUpdate) -> Option<PathBuf> {
        let staged = PathBuf::from(&pending.staged);
        let name = staged.file_name()?.to_string_lossy().into_owned();
        let same_dir = staged.parent().is_some_and(|p| p.to_string_lossy().eq_ignore_ascii_case(&updates.to_string_lossy()));
        let shaped = name.starts_with(STAGED_PREFIX) && name.ends_with(STAGED_SUFFIX);
        if !same_dir || !shaped {
            tracing::warn!(staged = %staged.display(), "the update marker names a file that is not a staged installer; leaving it alone");
            return None;
        }
        if lock(&self.fs).is_reparse(&staged).unwrap_or(false) {
            tracing::warn!(staged = %staged.display(), "the staged installer is a reparse point; leaving it alone");
            return None;
        }
        Some(staged)
    }

    /// The setup temp folder the marker names, only when it is a folder this service could have made:
    /// directly inside the staging folder, with the temp-folder name, and not a reparse point.
    fn tmp_path_of(&self, updates: &Path, pending: &PendingUpdate) -> Option<PathBuf> {
        if pending.tmp_dir.is_empty() {
            return None;
        }
        let tmp = PathBuf::from(&pending.tmp_dir);
        let name = tmp.file_name()?.to_string_lossy().into_owned();
        let same_dir = tmp.parent().is_some_and(|p| p.to_string_lossy().eq_ignore_ascii_case(&updates.to_string_lossy()));
        if !same_dir || !name.starts_with(TMP_PREFIX) {
            tracing::warn!(tmp = %tmp.display(), "the update marker names a folder that is not a setup temp folder; leaving it alone");
            return None;
        }
        if lock(&self.fs).is_reparse(&tmp).unwrap_or(false) {
            tracing::warn!(tmp = %tmp.display(), "the setup temp folder is a reparse point; leaving it alone");
            return None;
        }
        Some(tmp)
    }

    /// Deletes the staged file and the setup temp folder the marker names (when valid), then the marker.
    fn discard(&self, updates: &Path, pending: &PendingUpdate) {
        if let Some(staged) = self.staged_path_of(updates, pending) {
            remove_file_logged(&staged, "the staged installer");
        }
        if let Some(tmp) = self.tmp_path_of(updates, pending) {
            remove_dir_logged(&tmp);
        }
        self.remove_marker();
    }

    // ---- after setup ----

    /// The old service, still alive when setup ends: waits for the setup process, then finishes.
    /// `None` when setup outlived the wait limit.
    fn watch_setup(
        &self,
        process: &Arc<Mutex<Box<dyn SetupProcess>>>,
        pending: &PendingUpdate,
        staged: &mut Staged,
        tmp: &mut SetupTmp,
    ) -> Option<Finish> {
        let deadline = Instant::now() + self.cfg.setup_wait_limit;
        loop {
            let polled = lock(process).try_wait();
            match polled {
                Ok(Some(code)) => {
                    tracing::info!(
                        setup_pid = pending.setup_pid,
                        code = %crate::process::describe_exit_code(code),
                        "setup ended while this service is still running; finishing the update here"
                    );
                    break;
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "could not query the setup process"),
            }
            if Instant::now() >= deadline {
                tracing::error!(setup_pid = pending.setup_pid, "setup is still running after the wait limit; the marker stays for the next start");
                staged.keep();
                tmp.keep();
                return None;
            }
            std::thread::sleep(self.cfg.poll);
        }
        let updates = self.updates_dir();
        let outcome = self.finish_user_steps(pending);
        match outcome {
            Finish::Done => {
                // The handle closes before the delete.
                staged.file.take();
                self.discard(&updates, pending);
            }
            Finish::NoUser => {
                staged.file.take();
                staged.keep();
                tmp.keep();
            }
        }
        Some(outcome)
    }

    /// The new service, on start: finishes an update whose setup the old service did not outlive.
    /// When nobody is logged on it tries again every `resume_retry` until the marker is stale
    /// ([`MARKER_MAX_AGE`]) or another install supersedes it. Blocking: run it on its own thread.
    pub fn resume_pending(self: &Arc<Self>) -> Option<Finish> {
        loop {
            match self.resume_once() {
                Some(Finish::NoUser) => std::thread::sleep(self.cfg.resume_retry),
                other => return other,
            }
        }
    }

    /// One attempt of [`Self::resume_pending`]. The install slot is held only during the attempt.
    pub(crate) fn resume_once(self: &Arc<Self>) -> Option<Finish> {
        let _guard = BusyGuard::claim(&self.busy)?;
        let updates = match self.secure_updates_dir(false) {
            Ok(Some(dir)) => dir,
            Ok(None) => return None,
            Err(e) => {
                tracing::error!(error = %e, "the update folder cannot be trusted; no pending update is resumed");
                return None;
            }
        };
        let marker = self.marker_path();
        if !matches!(lock(&self.fs).exists(&marker), Ok(true)) {
            return None;
        }
        let pending = match self.read_marker(&marker) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, "an update marker is there but cannot be used; leaving it");
                return None;
            }
        };
        if self.marker_age_ms(&pending) > MARKER_MAX_AGE.as_millis() {
            tracing::warn!(version = %pending.version, "dropping a stale update marker");
            self.discard(&updates, &pending);
            return None;
        }
        tracing::info!(version = %pending.version, setup_pid = pending.setup_pid, "a pending update was found; waiting for setup to end");
        if pending.setup_pid != 0 && self.marker_is_overdue(&pending) && lock(&self.probe).alive(pending.setup_pid, pending.setup_created) {
            self.end_stranded_setup(&pending);
            self.discard(&updates, &pending);
            return None;
        }
        if pending.setup_pid != 0 {
            let deadline = Instant::now() + self.cfg.setup_wait_limit;
            while lock(&self.probe).alive(pending.setup_pid, pending.setup_created) {
                if Instant::now() >= deadline {
                    tracing::error!(setup_pid = pending.setup_pid, "setup is still running after the wait limit; the update stays pending");
                    return None;
                }
                std::thread::sleep(self.cfg.poll);
            }
        }
        let outcome = self.finish_user_steps(&pending);
        if outcome == Finish::Done {
            self.discard(&updates, &pending);
        }
        Some(outcome)
    }

    /// `setup-user.ps1` as the console user (waited for), then `canvas-rgb.exe --after-update` as the
    /// console user. Both non-elevated: this is the root fix for #28.
    fn finish_user_steps(&self, pending: &PendingUpdate) -> Finish {
        let app = &self.cfg.app_dir;
        if let Err(e) = self.check_app_dir() {
            tracing::error!(
                version = %pending.version,
                error = %e,
                "the install folder cannot be trusted: setup-user.ps1 and the tray are not started"
            );
            return Finish::Done;
        }
        let run_script = self.setup_reported_success(pending);
        if run_script {
            let script = app.join("installer").join("setup-user.ps1");
            let script_line = format!(
                "\"{}\" -NoProfile -ExecutionPolicy Bypass -File \"{}\" -InstallDir \"{}\" -SignalRgb N",
                self.cfg.powershell.display(),
                script.display(),
                app.display()
            );
            match self.run_as_user(&script_line, app, true) {
                Ok(Some(0)) => tracing::info!(version = %pending.version, "setup-user.ps1 ran as the console user and exited 0"),
                Ok(Some(code)) => tracing::warn!(
                    version = %pending.version,
                    code = %crate::process::describe_exit_code(code),
                    "setup-user.ps1 exited with a non-zero code; the tray still starts"
                ),
                Ok(None) => tracing::warn!(
                    version = %pending.version,
                    limit_s = self.cfg.user_step_limit.as_secs_f32(),
                    "setup-user.ps1 did not finish within the wait limit; the tray still starts"
                ),
                Err(e) if e.kind() == io::ErrorKind::NotConnected => {
                    tracing::warn!(version = %pending.version, error = %e, "no user is logged on at the console; the update stays pending");
                    return Finish::NoUser;
                }
                Err(e) => tracing::error!(version = %pending.version, error = %e, "setup-user.ps1 could not be run as the console user"),
            }
        }
        // The tray starts either way: after a failed setup it is what shows update-result.json.
        let tray_line = format!("\"{}\" --after-update", app.join("canvas-rgb.exe").display());
        match self.run_as_user(&tray_line, app, false) {
            Ok(_) => tracing::info!(version = %pending.version, "canvas-rgb.exe --after-update started as the console user"),
            Err(e) if e.kind() == io::ErrorKind::NotConnected && !run_script => {
                tracing::warn!(version = %pending.version, error = %e, "no user is logged on at the console; the update stays pending");
                return Finish::NoUser;
            }
            Err(e) if e.kind() == io::ErrorKind::NotConnected => {
                tracing::warn!(version = %pending.version, error = %e, "no user is logged on at the console; the tray was not started");
            }
            Err(e) => tracing::error!(version = %pending.version, error = %e, "the tray could not be started as the console user"),
        }
        Finish::Done
    }

    /// Whether `{app}\update-result.json` is fresh (written at or after the marker's start time) and says
    /// `ok:true` for the marker's version. Only then is the per-user script run: after a failed or
    /// missing setup the files it would configure may be half-installed.
    fn setup_reported_success(&self, pending: &PendingUpdate) -> bool {
        let path = self.cfg.app_dir.join("update-result.json");
        let written = match std::fs::metadata(&path).and_then(|m| m.modified()) {
            Ok(t) => t.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "no update-result.json: setup-user.ps1 is skipped");
                return false;
            }
        };
        if written < pending.started_unix {
            tracing::warn!(path = %path.display(), written, started = pending.started_unix, "update-result.json is older than this update: setup-user.ps1 is skipped");
            return false;
        }
        let parsed = std::fs::read(&path).ok().and_then(|text| serde_json::from_slice::<serde_json::Value>(&text).ok());
        let Some(result) = parsed else {
            tracing::warn!(path = %path.display(), "update-result.json is unreadable: setup-user.ps1 is skipped");
            return false;
        };
        let ok = result.get("ok").and_then(serde_json::Value::as_bool) == Some(true);
        let same_version = result
            .get("version")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|v| v.trim_start_matches('v') == pending.version.trim_start_matches('v'));
        if !ok || !same_version {
            tracing::warn!(path = %path.display(), ok, same_version, "setup did not report success for this version: setup-user.ps1 is skipped");
        }
        ok && same_version
    }

    /// Starts `command_line` as the console user, waiting up to `user_session_wait` for one to appear.
    /// With `wait`, also waits for it to exit and returns its code.
    fn run_as_user(&self, command_line: &str, dir: &Path, wait: bool) -> io::Result<Option<i32>> {
        let session_deadline = Instant::now() + self.cfg.user_session_wait;
        let mut child = loop {
            let started = lock(&self.user).start(command_line, dir);
            match started {
                Ok(child) => break child,
                Err(e) if e.kind() == io::ErrorKind::NotConnected && Instant::now() < session_deadline => {
                    std::thread::sleep(self.cfg.poll);
                }
                Err(e) => return Err(e),
            }
        };
        if !wait {
            return Ok(None);
        }
        let deadline = Instant::now() + self.cfg.user_step_limit;
        loop {
            match child.try_wait() {
                Ok(Some(code)) => return Ok(Some(code)),
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "could not query the user step"),
            }
            if Instant::now() >= deadline {
                // The tray must not start while the script may still be writing the user's HKCU.
                match child.kill() {
                    Ok(()) => tracing::warn!(command = command_line, "ended the user step that hit the wait limit"),
                    Err(e) => tracing::warn!(command = command_line, error = %e, "could not end the user step that hit the wait limit"),
                }
                return Ok(None);
            }
            std::thread::sleep(self.cfg.poll);
        }
    }
}

/// Windows implementations of the seams.
pub mod win {
    use super::{ClientFileReader, ProcessProbe, SetupLauncher, SetupProcess, UserCommands};
    use crate::process::ChildProcess;
    use std::ffi::c_void;
    use std::io::{self, Read};
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::ffi::OsString;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{
        GetTokenInformation, RevertToSelf, TokenImpersonationLevel, TokenUser, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
    use windows_sys::Win32::System::Pipes::ImpersonateNamedPipeClient;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetCurrentThread, GetExitCodeProcess, GetProcessTimes, OpenProcess, OpenThreadToken,
        ResumeThread, TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
        CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        STARTUPINFOW,
    };

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const SECURITY_IMPERSONATION: u32 = 2;
    const ERROR_INVALID_PARAMETER: i32 = 87;

    /// The whole-file read, with the checks. Runs on the impersonating thread; also what the tests
    /// exercise on real files. A symlink or junction at the file itself is refused: the handle is
    /// opened with `FILE_FLAG_OPEN_REPARSE_POINT`, so the file reports its own attributes.
    pub fn read_regular_file(path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
        let file = std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
        let meta = file.metadata()?;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "it is a symlink or junction (a reparse point)"));
        }
        if !meta.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "it is not a regular file"));
        }
        if meta.len() > max_bytes {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("it is {} bytes, over the {max_bytes} byte limit", meta.len())));
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        file.take(max_bytes + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max_bytes {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("it grew past the {max_bytes} byte limit while being read")));
        }
        Ok(bytes)
    }

    /// The pipe client as a Win32 handle number (so the reader is `Send`). Valid while the pipe
    /// instance lives; `handle_client` owns that instance for the whole request.
    pub struct ImpersonatingReader {
        pipe: usize,
    }

    impl ImpersonatingReader {
        pub fn of(pipe: HANDLE) -> Self {
            Self { pipe: pipe as usize }
        }
    }

    impl ClientFileReader for ImpersonatingReader {
        fn read(&mut self, path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
            let pipe = self.pipe;
            let path = path.to_path_buf();
            // A thread of its own that ends with the read: whatever token it had goes with it, and the
            // impersonation never leaks into a pooled thread. Nothing awaits between the two calls.
            std::thread::Builder::new()
                .name("update-read-as-client".into())
                .spawn(move || {
                    let _impersonating = Impersonation::begin(pipe as HANDLE)?;
                    ensure_client_token()?;
                    read_regular_file(&path, max_bytes)
                })?
                .join()
                .map_err(|_| io::Error::other("the read-as-client thread panicked"))?
        }
    }

    /// Impersonation of the pipe client on this thread. Reverts on drop.
    struct Impersonation;

    impl Impersonation {
        fn begin(pipe: HANDLE) -> io::Result<Self> {
            // SAFETY: `pipe` is the live pipe instance of the request being served, already read from.
            if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self)
        }
    }

    impl Drop for Impersonation {
        fn drop(&mut self) {
            // SAFETY: takes no pointers; ends the impersonation on the calling thread.
            if unsafe { RevertToSelf() } == 0 {
                // The thread ends right after, which drops its token with it.
                tracing::error!(error = %io::Error::last_os_error(), "RevertToSelf failed on the read-as-client thread");
            }
        }
    }

    /// Refuses unless this thread really runs as the client: it has a thread token, at impersonation
    /// level or above, and its user is not SYSTEM. Belt and braces for "never read as SYSTEM".
    fn ensure_client_token() -> io::Result<()> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the current-thread pseudo handle; `token` is a valid out-pointer. TRUE = open as self.
        if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) } == 0 {
            return Err(io::Error::other(format!("the thread is not impersonating the client: {}", io::Error::last_os_error())));
        }
        let result = (|| {
            let mut level = 0u32;
            let mut returned = 0u32;
            // SAFETY: `token` is open with TOKEN_QUERY; `level` is a 4-byte out-buffer.
            let ok = unsafe {
                GetTokenInformation(token, TokenImpersonationLevel, &mut level as *mut u32 as *mut c_void, 4, &mut returned)
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            if level < SECURITY_IMPERSONATION {
                return Err(io::Error::other("the client allows no impersonation (anonymous or identification level)"));
            }
            // 8-byte aligned, far larger than a TOKEN_USER and a SID.
            let mut buffer = [0u64; 64];
            // SAFETY: as above; the buffer length passed is its size in bytes.
            let ok = unsafe {
                GetTokenInformation(token, TokenUser, buffer.as_mut_ptr() as *mut c_void, std::mem::size_of_val(&buffer) as u32, &mut returned)
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the buffer now holds a TOKEN_USER whose SID points into the same buffer.
            let sid = unsafe { (*(buffer.as_ptr() as *const TOKEN_USER)).User.Sid };
            let mut text: *mut u16 = std::ptr::null_mut();
            // SAFETY: `sid` is valid; `text` receives a LocalAlloc'd string freed below.
            if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut len = 0usize;
            // SAFETY: the converted string is NUL-terminated.
            let user = unsafe {
                while *text.add(len) != 0 {
                    len += 1;
                }
                String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
            };
            // SAFETY: `text` came from ConvertSidToStringSidW and is freed once.
            unsafe { LocalFree(text as *mut c_void) };
            if user == "S-1-5-18" {
                return Err(io::Error::other("the thread token is SYSTEM, not the client; refusing to read as SYSTEM"));
            }
            Ok(())
        })();
        // SAFETY: `token` was opened above and is closed once.
        unsafe { CloseHandle(token) };
        result
    }

    /// Creation time of a process handle as one 64-bit `FILETIME` value.
    pub(crate) fn creation_time(process: HANDLE) -> io::Result<u64> {
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: `process` is a live process handle with query access; the out-pointers are valid.
        if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }

    /// The environment block setup gets: this process's environment with `TMP` and `TEMP` at `tmp`, and
    /// `PATH` and `PSModulePath` fixed to system-only values built from the real `SystemRoot`, so
    /// nothing a user could write to is searched by the scripts setup runs as SYSTEM. UTF-16
    /// `NAME=value\0` entries sorted by name and ended by an extra NUL.
    pub fn build_env_block(vars: Vec<(OsString, OsString)>, tmp: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        let find = |name: &str| {
            vars.iter()
                .find(|(k, _)| k.to_string_lossy().eq_ignore_ascii_case(name))
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        let root = find("SystemRoot").unwrap_or_else(|| r"C:\Windows".to_string());
        let program_files = find("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".to_string());
        let path = format!(r"{root}\System32;{root};{root}\System32\Wbem;{root}\System32\WindowsPowerShell\v1.0");
        let ps_modules = format!(r"{program_files}\WindowsPowerShell\Modules;{root}\System32\WindowsPowerShell\v1.0\Modules");
        let replaced = ["TMP", "TEMP", "PATH", "PSMODULEPATH"];
        let mut entries: Vec<(String, Vec<u16>)> = Vec::new();
        for (key, value) in &vars {
            let name = key.to_string_lossy().into_owned();
            if name.is_empty() || replaced.iter().any(|r| name.eq_ignore_ascii_case(r)) {
                continue;
            }
            let mut entry: Vec<u16> = key.encode_wide().collect();
            entry.push(u16::from(b'='));
            entry.extend(value.encode_wide());
            entries.push((name.to_uppercase(), entry));
        }
        let fixed: [(&str, Vec<u16>); 4] = [
            ("Path", path.encode_utf16().collect()),
            ("PSModulePath", ps_modules.encode_utf16().collect()),
            ("TEMP", tmp.as_os_str().encode_wide().collect()),
            ("TMP", tmp.as_os_str().encode_wide().collect()),
        ];
        for (name, value) in fixed {
            let mut entry: Vec<u16> = name.encode_utf16().collect();
            entry.push(u16::from(b'='));
            entry.extend(value);
            entries.push((name.to_uppercase(), entry));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut block = Vec::new();
        for (_, entry) in entries {
            block.extend(entry);
            block.push(0);
        }
        block.push(0);
        block
    }

    /// Starts setup with `CreateProcessW`: the service's own token, no window, **suspended**, with its
    /// own environment block (`TMP` and `TEMP` at the private folder).
    pub struct StdSetupLauncher;

    impl SetupLauncher for StdSetupLauncher {
        fn launch(&mut self, exe: &Path, raw_args: &str, dir: &Path, tmp: &Path) -> io::Result<Box<dyn SetupProcess>> {
            use std::os::windows::ffi::OsStrExt;
            let mut command: Vec<u16> = format!("\"{}\" {raw_args}", exe.display()).encode_utf16().chain(Some(0)).collect();
            let cwd: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
            let env = build_env_block(std::env::vars_os().collect(), tmp);
            // SAFETY: an all-zero STARTUPINFOW with `cb` set is the documented "no options" value.
            let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
            startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
            // SAFETY: an all-zero PROCESS_INFORMATION is a valid out-buffer.
            let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
            // token: the service's own (LocalSystem), on purpose (#43): the installer has just been
            // signature-checked against the release key and is held open share-read; setup needs SYSTEM
            // to replace the service. The per-user steps never run from here: they go through
            // `launch_as_console_user`.
            // SAFETY: `command`, `env`, `cwd`, `startup` and `info` live past the call; the environment is
            // UTF-16 (CREATE_UNICODE_ENVIRONMENT) and double-NUL ended. No handle is inherited.
            let created = unsafe {
                CreateProcessW(
                    std::ptr::null(),
                    command.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    CREATE_NO_WINDOW | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                    env.as_ptr() as *const c_void,
                    cwd.as_ptr(),
                    &startup,
                    &mut info,
                )
            };
            if created == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut child = SetupChild { process: info.hProcess, thread: info.hThread, pid: info.dwProcessId, created: 0, resumed: false };
            // On an error the child drops here, which ends the suspended process.
            child.created = creation_time(child.process)?;
            Ok(Box::new(child))
        }
    }

    struct SetupChild {
        process: HANDLE,
        thread: HANDLE,
        pid: u32,
        created: u64,
        resumed: bool,
    }

    // SAFETY: the handles are only used through `&mut self` or `&self` reads of plain values by the
    // one thread that owns the box at a time (the install thread, then the watcher).
    unsafe impl Send for SetupChild {}

    impl ChildProcess for SetupChild {
        fn try_wait(&mut self) -> io::Result<Option<i32>> {
            // SAFETY: `process` is a live process handle owned by this value.
            unsafe {
                match WaitForSingleObject(self.process, 0) {
                    WAIT_OBJECT_0 => {
                        let mut code: u32 = 0;
                        if GetExitCodeProcess(self.process, &mut code) == 0 {
                            return Err(io::Error::last_os_error());
                        }
                        Ok(Some(code as i32))
                    }
                    WAIT_TIMEOUT => Ok(None),
                    _ => Err(io::Error::last_os_error()),
                }
            }
        }

        fn kill(&mut self) -> io::Result<()> {
            // SAFETY: as above; exit code 1.
            if unsafe { TerminateProcess(self.process, 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        fn id(&self) -> u32 {
            self.pid
        }
    }

    impl SetupProcess for SetupChild {
        fn created(&self) -> u64 {
            self.created
        }

        fn resume(&mut self) -> io::Result<()> {
            // SAFETY: `thread` is the suspended primary thread handle from CreateProcessW.
            if unsafe { ResumeThread(self.thread) } == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            self.resumed = true;
            Ok(())
        }
    }

    impl Drop for SetupChild {
        fn drop(&mut self) {
            // SAFETY: both handles are owned here and closed once. A process that never ran is ended.
            unsafe {
                if !self.resumed {
                    TerminateProcess(self.process, 1);
                }
                CloseHandle(self.thread);
                CloseHandle(self.process);
            }
        }
    }

    /// Runs commands in the console session with the user's own token (`WTSQueryUserToken`).
    pub struct ConsoleUserCommands;

    impl UserCommands for ConsoleUserCommands {
        fn start(&mut self, command_line: &str, dir: &Path) -> io::Result<Box<dyn ChildProcess>> {
            // The user steps must run with the user's own environment ($env:APPDATA is theirs, not SYSTEM's):
            // no environment block, no process.
            crate::daemon::launch_as_console_user(command_line, Some(dir), true)
        }
    }

    /// Looks a process up by PID and checks its creation time.
    pub struct WinProbe;

    impl ProcessProbe for WinProbe {
        fn alive(&mut self, pid: u32, created: u64) -> bool {
            // SAFETY: a plain open; the handle is closed on every path.
            unsafe {
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid);
                if process.is_null() {
                    // No such PID: gone. Any other failure is unknown, which counts as still running.
                    return io::Error::last_os_error().raw_os_error() != Some(ERROR_INVALID_PARAMETER);
                }
                let same = creation_time(process).map(|t| t == created).unwrap_or(true);
                let running = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
                CloseHandle(process);
                same && running
            }
        }

        fn terminate(&mut self, pid: u32, created: u64) -> bool {
            // SAFETY: a plain open; the handle is closed on every path. The creation time is checked
            // on the handle that is then terminated, so a reused PID is never ended.
            unsafe {
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE | PROCESS_TERMINATE, 0, pid);
                if process.is_null() {
                    return io::Error::last_os_error().raw_os_error() == Some(ERROR_INVALID_PARAMETER);
                }
                let ours = creation_time(process).map(|t| t == created).unwrap_or(false);
                let gone = if ours {
                    TerminateProcess(process, 1) != 0 && WaitForSingleObject(process, 5000) != WAIT_TIMEOUT
                } else {
                    true
                };
                CloseHandle(process);
                gone
            }
        }
    }
}
