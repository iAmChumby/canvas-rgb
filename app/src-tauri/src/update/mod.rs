//! Auto-update (#21). The tray checks the release feed, downloads the installer, checks its
//! signature and starts it. The plugin (`tauri-plugin-updater`) is used verify-only: `check()` and
//! `download()` (which returns the bytes only after the signature passed). `install()` and
//! `download_and_install()` are never called; our Inno installer runs instead, elevated, after
//! a second signature check on the file we are about to start.
//!
//! The editor never touches the network or the file system here. It reads [`UpdateState`] and sends
//! the person's choices through the commands at the bottom.

mod launch;
pub mod rules;
mod state;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use canvas_proto::tauri::{
    CheckFailReason, UpdateInfo, UpdatePhase, UpdateState, AFTER_UPDATE_ARG, EVT_UPDATE_STATE,
};
use semver::Version;
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};
use time::OffsetDateTime;
use url::Url;

use crate::log;
use rules::DownloadPolicy;
use state::Stored;

/// The file the installer writes into its install folder when it ran with `/UPDATE`.
pub const RESULT_FILE: &str = "update-result.json";

const PROGRESS_EVERY: Duration = Duration::from_millis(100);
/// One download may take this long in all.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// True when the arguments hold `--after-update` (the installer's `/UPDATE` relaunch).
pub fn has_after_update_arg<S: AsRef<str>>(args: &[S]) -> bool {
    args.iter().any(|a| a.as_ref() == AFTER_UPDATE_ARG)
}

/// Where everything lives. Production values from [`Settings::production`]; the tests point it at
/// a temp folder and a fake release server.
#[derive(Debug, Clone)]
pub struct Settings {
    pub endpoint: Url,
    pub pubkey: String,
    pub policy: DownloadPolicy,
    pub updates_dir: PathBuf,
    pub state_path: PathBuf,
    /// `<install folder>\update-result.json`, written by the installer.
    pub result_path: PathBuf,
    /// `%LOCALAPPDATA%\canvas-rgb`: nothing from here down to a folder the tray writes in may be a
    /// junction or symlink.
    pub guard_root: PathBuf,
    /// How often download progress is sent (the tests send every chunk).
    pub progress_every: Duration,
}

impl Settings {
    pub fn production() -> Self {
        let tray = crate::onboard::tray_data_dir();
        let install_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_default();
        Settings {
            endpoint: rules::ENDPOINT.parse().expect("the endpoint constant is a URL"),
            pubkey: rules::PUBKEY.to_string(),
            policy: DownloadPolicy::production(),
            updates_dir: tray.join("updates"),
            state_path: tray.join("update.json"),
            result_path: install_dir.join(RESULT_FILE),
            guard_root: tray.parent().map(Path::to_path_buf).unwrap_or(tray),
            progress_every: PROGRESS_EVERY,
        }
    }
}

/// What the installer left behind (see `canvas-rgb.iss`, `/UPDATE`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InstallResult {
    pub version: String,
    pub ok: bool,
    #[serde(default)]
    pub message: String,
}

fn read_result(path: &Path) -> Option<InstallResult> {
    // Lenient: the installer writes UTF-8 without a BOM, but a BOM or a stray byte must not hide its report.
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let bom = char::from_u32(0xFEFF).unwrap_or(' ');
    serde_json::from_str(text.trim_start_matches(bom)).ok()
}

/// The result file, only when it belongs to the run that started at `since`: written at or after
/// that moment. Inno deletes an old file only in PrepareToInstall, which never runs when Setup's own
/// permission prompt is declined, so a file from the previous update can still be there (#28).
fn read_fresh_result(path: &Path, since: SystemTime) -> Option<InstallResult> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    if modified < since {
        return None;
    }
    read_result(path)
}

/// How a start with a `pending` version ends. Only the running version and the installer's own
/// report decide this; "pending" alone proves nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum Settled {
    Updated,
    Failed(String),
}

pub fn settle(pending: &str, running: &str, result: Option<&InstallResult>) -> Settled {
    match result.filter(|r| r.version == pending) {
        Some(r) if !r.ok => Settled::Failed(if r.message.is_empty() {
            "The installer reported a failure.".to_string()
        } else {
            r.message.clone()
        }),
        Some(_) if running != pending => {
            Settled::Failed("Installed, but the old canvas-rgb is still running. Quit it and start it again.".into())
        }
        _ if running == pending => Settled::Updated,
        _ => Settled::Failed("The installer ended without finishing.".into()),
    }
}

/// The version the tray menu and tooltip offer: an available release the person has not skipped.
pub fn tray_version(state: &UpdateState) -> Option<String> {
    let available = state.available.as_ref()?;
    (state.skipped_version.as_deref() != Some(available.version.as_str())).then(|| available.version.clone())
}

fn in_flight(phase: &UpdatePhase) -> bool {
    matches!(
        phase,
        UpdatePhase::Checking
            | UpdatePhase::Downloading { .. }
            | UpdatePhase::Verifying
            | UpdatePhase::WaitingForUac
            | UpdatePhase::Installing
    )
}

pub type Launcher = Arc<dyn Fn(&Path) -> Result<launch::Started, launch::RunError> + Send + Sync>;

struct Inner {
    stored: Stored,
    phase: UpdatePhase,
    available: Option<UpdateInfo>,
    update: Option<Update>,
    /// The version whose install did not finish, shown in the tray while the phase says so.
    install_failed_for: Option<String>,
}

pub struct Controller {
    settings: Settings,
    current: Version,
    inner: Mutex<Inner>,
    launcher: Launcher,
    on_change: Box<dyn Fn(&UpdateState) + Send + Sync>,
}

impl Controller {
    pub fn new(
        settings: Settings,
        current: Version,
        launcher: Launcher,
        on_change: Box<dyn Fn(&UpdateState) + Send + Sync>,
    ) -> Self {
        let stored = state::load(&settings.state_path);
        Controller {
            settings,
            current,
            inner: Mutex::new(Inner {
                stored,
                phase: UpdatePhase::Idle,
                available: None,
                update: None,
                install_failed_for: None,
            }),
            launcher,
            on_change,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn build_state(&self, inner: &Inner) -> UpdateState {
        UpdateState {
            current_version: self.current.to_string(),
            auto_check: inner.stored.auto_check,
            last_check: inner.stored.last_check.clone(),
            phase: inner.phase.clone(),
            available: inner.available.clone(),
            skipped_version: inner.stored.skipped_version.clone(),
            snoozed: state::snoozed(&inner.stored, OffsetDateTime::now_utc()),
            updated_to: inner.stored.updated_to.clone(),
        }
    }

    /// The version whose install did not finish, while the phase says so (for the tray tooltip).
    pub fn failed_version(&self) -> Option<String> {
        let inner = self.lock();
        matches!(inner.phase, UpdatePhase::InstallFailed { .. })
            .then(|| inner.install_failed_for.clone())
            .flatten()
    }

    pub fn state(&self) -> UpdateState {
        self.build_state(&self.lock())
    }

    /// Runs `f` on the inner state, saves the stored part, and tells the listeners.
    fn change(&self, f: impl FnOnce(&mut Inner)) -> UpdateState {
        let state = {
            let mut inner = self.lock();
            let before = inner.stored.clone();
            f(&mut inner);
            if inner.stored != before {
                let saved = match state::refuse_reparse(&self.settings.guard_root, &self.settings.state_path) {
                    Ok(()) => state::save(&self.settings.state_path, &inner.stored).map_err(|e| e.to_string()),
                    Err(why) => Err(why),
                };
                if let Err(e) = saved {
                    log::append(&format!("update: could not save {}: {e}", self.settings.state_path.display()));
                }
            }
            self.build_state(&inner)
        };
        (self.on_change)(&state);
        state
    }

    fn set_phase(&self, phase: UpdatePhase) -> UpdateState {
        self.change(|i| i.phase = phase)
    }

    /// Starts an operation: refused while another one is in flight.
    fn begin(&self, phase: UpdatePhase) -> bool {
        let mut began = false;
        self.change(|i| {
            if !in_flight(&i.phase) {
                i.phase = phase;
                began = true;
            }
        });
        began
    }

    /// Settles a `pending` version from the running version and the installer's result file, then
    /// sweeps the downloaded installer away. Runs at start, and after an installer the tray
    /// outlived.
    pub fn reconcile_pending(&self) {
        self.reconcile_since(None);
    }

    /// [`reconcile_pending`] for an installer this tray started at `since`: a result file older
    /// than that belongs to an earlier run and is ignored.
    fn reconcile_since(&self, since: Option<SystemTime>) {
        let result = match since {
            None => read_result(&self.settings.result_path),
            Some(t) => read_fresh_result(&self.settings.result_path, t),
        };
        self.change(|i| {
            let Some(pending) = i.stored.pending.take() else { return };
            match settle(&pending.version, &self.current.to_string(), result.as_ref()) {
                Settled::Updated => {
                    i.stored.updated_to = Some(pending);
                    i.phase = UpdatePhase::Idle;
                }
                Settled::Failed(message) => {
                    log::append(&format!("update: {} did not install: {message}", pending.version));
                    i.install_failed_for = Some(pending.version.clone());
                    // The editor's failed-install banner and the tooltip name this version; the
                    // plugin object is gone after a restart, so Try again checks first.
                    i.available = Some(pending);
                    i.phase = UpdatePhase::InstallFailed { message };
                }
            }
        });
        if !in_flight(&self.lock().phase) {
            match state::refuse_reparse(&self.settings.guard_root, &self.settings.updates_dir) {
                Ok(()) => clear_dir(&self.settings.updates_dir),
                Err(why) => log::append(&format!("update: not clearing the updates folder: {why}")),
            }
        }
    }

    /// The installer process ended while this tray still runs. Setup raises its own permission
    /// prompt (#28), so a Setup that exits with "cancelled before install" and wrote no result file
    /// was declined at that prompt: nothing started, no pending version, the Declined phase. Any
    /// other end is settled from the result file as before.
    /// `since` is when this tray started Setup: only a result written after it counts, and only for
    /// the pending version (a stale `ok` from the previous update must not turn a decline into a
    /// failure).
    pub fn settle_after_installer(&self, exit_code: Option<u32>, since: SystemTime) {
        let pending = self.lock().stored.pending.as_ref().map(|p| p.version.clone());
        let fresh = read_fresh_result(&self.settings.result_path, since)
            .filter(|r| pending.as_deref() == Some(r.version.as_str()));
        if exit_code == Some(launch::SETUP_CANCELLED_CODE) && fresh.is_none() {
            log::append("update: setup ended with code 2 and no result: the permission prompt was declined");
            self.change(|i| {
                i.stored.pending = None;
                i.phase = UpdatePhase::Declined;
            });
            return;
        }
        self.reconcile_since(Some(since));
    }

    // ---- the person's choices ----

    pub fn later(&self) -> UpdateState {
        self.change(|i| i.stored.snoozed_at = Some(state::rfc3339(OffsetDateTime::now_utc())))
    }

    pub fn skip(&self) -> UpdateState {
        self.change(|i| {
            if let Some(a) = &i.available {
                i.stored.skipped_version = Some(a.version.clone());
            }
        })
    }

    pub fn unskip(&self) -> UpdateState {
        self.change(|i| i.stored.skipped_version = None)
    }

    pub fn set_auto(&self, enabled: bool) -> UpdateState {
        self.change(|i| i.stored.auto_check = enabled)
    }

    pub fn dismiss_updated(&self) -> UpdateState {
        self.change(|i| i.stored.updated_to = None)
    }

    // ---- the schedule ----

    /// True when the automatic check should run now.
    pub fn auto_due(&self, now: OffsetDateTime) -> bool {
        let inner = self.lock();
        !in_flight(&inner.phase) && state::auto_check_due(&inner.stored, now)
    }

    /// Called by the tray's poll once the daemon has answered: runs the automatic check when due.
    /// Blocking, so call it off the UI thread.
    pub fn tick<R: Runtime>(&self, app: &AppHandle<R>) {
        if self.auto_due(OffsetDateTime::now_utc()) {
            tauri::async_runtime::block_on(self.check(app, false));
        }
    }

    // ---- the check ----

    fn build_updater<R: Runtime>(&self, app: &AppHandle<R>) -> tauri_plugin_updater::Result<Updater> {
        let policy = self.settings.policy.clone();
        app.updater_builder()
            .pubkey(self.settings.pubkey.clone())
            .target(rules::TARGET)
            .version_comparator(|current, release| rules::is_update(&current, &release.version))
            .timeout(CHECK_TIMEOUT)
            // A redirect may only go to a host the download itself could have come from.
            .configure_client(move |client| client.redirect(redirect_policy(policy.clone())))
            .endpoints(vec![self.settings.endpoint.clone()])?
            .build()
    }

    async fn fetch_manifest<R: Runtime>(&self, app: &AppHandle<R>) -> Result<Option<(Update, UpdateInfo)>, CheckFailReason> {
        let updater = self.build_updater(app).map_err(|e| {
            log::append(&format!("update: could not build the updater: {e}"));
            CheckFailReason::Other
        })?;
        match updater.check().await {
            Ok(None) => Ok(None),
            Ok(Some(mut update)) => {
                if let Err(why) = self.settings.policy.check(&update.download_url) {
                    log::append(&format!("update: {} refused: {why}", update.version));
                    return Err(CheckFailReason::Other);
                }
                update.timeout = Some(DOWNLOAD_TIMEOUT);
                let info = info_of(&update);
                Ok(Some((update, info)))
            }
            Err(e) => {
                log::append(&format!("update: check failed: {e}"));
                Err(classify(&e, &self.settings.endpoint).await)
            }
        }
    }

    /// Checks the feed. `manual` is Check now: it works with the automatic switch off. Refused
    /// (returns the current state) while another operation runs.
    pub async fn check<R: Runtime>(&self, app: &AppHandle<R>, manual: bool) -> UpdateState {
        if !self.begin(UpdatePhase::Checking) {
            return self.state();
        }
        let outcome = self.fetch_manifest(app).await;
        let now = state::rfc3339(OffsetDateTime::now_utc());
        self.change(|i| match outcome {
            Ok(None) => {
                i.stored.last_check = Some(now.clone());
                i.stored.last_result = Some(state::LastResult::UpToDate);
                if !manual {
                    i.stored.snoozed_at = None;
                }
                i.available = None;
                i.update = None;
                i.phase = UpdatePhase::UpToDate;
            }
            Ok(Some((update, info))) => {
                i.stored.last_check = Some(now.clone());
                i.stored.last_result = Some(state::LastResult::Available { version: info.version.clone() });
                // The next automatic check ends Later; so does a Check now that finds a version.
                i.stored.snoozed_at = None;
                i.available = Some(info);
                i.update = Some(update);
                i.phase = UpdatePhase::Available;
            }
            Err(reason) => {
                // A failed check withdraws the offer: nothing stale stays installable.
                i.available = None;
                i.update = None;
                i.stored.last_result = Some(state::LastResult::Failed { reason });
                // An automatic check that failed still counts for the day; Check now is not held back.
                if !manual {
                    i.stored.last_attempt = Some(now.clone());
                }
                i.phase = UpdatePhase::CheckFailed { reason };
            }
        })
    }

    // ---- the install ----

    /// Takes the available release and moves to `Downloading`. `None` while another operation
    /// runs or when nothing is available.
    pub fn begin_install(&self) -> Option<Update> {
        let mut taken = None;
        self.change(|i| {
            if !in_flight(&i.phase) {
                if let (Some(u), Some(_)) = (i.update.clone(), i.available.as_ref()) {
                    i.phase = UpdatePhase::Downloading { downloaded: 0, total: None };
                    taken = Some(u);
                }
            }
        });
        taken
    }

    /// Downloads, checks and starts the installer for an update from [`begin_install`]. Ends when
    /// the installer has been started (and, if this tray outlives it, when it has ended).
    pub async fn run_install(self: &Arc<Self>, update: &Update) -> UpdateState {
        let since = SystemTime::now();
        let (state, started) = self.start_install(update).await;
        if let Some(handle) = started {
            // Normally the installer stops this tray. If it ends first, its result file says why.
            // The wait is unbounded, so it takes a blocking thread, not an async worker.
            let code = tauri::async_runtime::spawn_blocking(move || launch::wait_and_close(handle))
                .await
                .ok()
                .flatten();
            self.settle_after_installer(code, since);
            return self.state();
        }
        state
    }

    /// Everything up to the installer running. Returns the process to wait on, if any.
    pub async fn start_install(self: &Arc<Self>, update: &Update) -> (UpdateState, launch::Started) {
        match self.fetch(update).await {
            Ok(path) => {
                // ShellExecuteExW waits for the permission prompt: a blocking thread, not an async worker.
                let (this, update) = (self.clone(), update.clone());
                match tauri::async_runtime::spawn_blocking(move || this.launch_stage(&update, &path)).await {
                    Ok(out) => out,
                    Err(e) => {
                        log::append(&format!("update: the installer start panicked: {e}"));
                        let message = "canvas-rgb could not start the installer.".to_string();
                        (self.set_phase(UpdatePhase::InstallFailed { message }), None)
                    }
                }
            }
            Err(phase) => {
                // Name the version in the tray tooltip, as `launch_stage` does.
                let state = self.change(|i| {
                    if matches!(phase, UpdatePhase::InstallFailed { .. }) {
                        i.install_failed_for = Some(update.version.clone());
                    }
                    i.phase = phase;
                });
                (state, None)
            }
        }
    }

    /// Try again with no live release object (after a restart): checks the feed first, then installs
    /// only a release at least as new as the one that was on offer. A check that finds nothing or
    /// fails leaves its own honest phase; nothing is silently dropped.
    pub async fn check_and_install<R: Runtime>(self: &Arc<Self>, app: &AppHandle<R>) -> UpdateState {
        let wanted = self.lock().available.as_ref().and_then(|a| a.version.parse::<Version>().ok());
        let state = self.check(app, true).await;
        if state.phase != UpdatePhase::Available {
            return state;
        }
        let found = state.available.as_ref().and_then(|a| a.version.parse::<Version>().ok());
        if matches!((&wanted, &found), (Some(w), Some(f)) if f < w) || found.is_none() {
            return state;
        }
        match self.begin_install() {
            Some(update) => self.run_install(&update).await,
            None => self.state(),
        }
    }

    /// True when Try again has no release object to install and may start a check.
    pub fn needs_check_to_install(&self) -> bool {
        let inner = self.lock();
        !in_flight(&inner.phase) && inner.update.is_none()
    }

    /// `begin_install` then `run_install`.
    #[cfg(test)]
    pub async fn install(self: &Arc<Self>) -> UpdateState {
        match self.begin_install() {
            Some(update) => self.start_install(&update).await.0,
            None => self.state(),
        }
    }

    /// Policy check, download (the plugin checks the signature before it returns the bytes), and
    /// the file on disk. `Err` is the phase to show.
    async fn fetch(&self, update: &Update) -> Result<PathBuf, UpdatePhase> {
        if let Err(why) = self.settings.policy.check(&update.download_url) {
            log::append(&format!("update: {} refused: {why}", update.version));
            return Err(UpdatePhase::InstallFailed {
                message: "The download is not from an address canvas-rgb trusts, so it was not started.".into(),
            });
        }
        let version: Version = update.version.parse().map_err(|_| UpdatePhase::InstallFailed {
            message: "The release has no usable version.".into(),
        })?;

        let mut downloaded = 0u64;
        let mut last_sent = Instant::now();
        let bytes = update
            .download(
                |chunk, total| {
                    downloaded += chunk as u64;
                    if last_sent.elapsed() >= self.settings.progress_every {
                        last_sent = Instant::now();
                        self.set_phase(UpdatePhase::Downloading { downloaded, total });
                    }
                },
                || {
                    self.set_phase(UpdatePhase::Verifying);
                },
            )
            .await
            .map_err(|e| {
                log::append(&format!("update: download of {} failed: {e}", update.version));
                download_failure(&e)
            })?;

        let dir = &self.settings.updates_dir;
        // Never write, delete or follow through a junction or symlink on the way to the folder.
        state::refuse_reparse(&self.settings.guard_root, dir).map_err(|why| {
            log::append(&format!("update: {why}"));
            UpdatePhase::InstallFailed {
                message: format!("The updates folder cannot be used: {why}. Remove it and try again."),
            }
        })?;
        let path = dir.join(rules::installer_file_name(&version));
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(dir)?;
            clear_dir(dir);
            std::fs::write(&path, &bytes)
        };
        write().map_err(|e| UpdatePhase::InstallFailed {
            message: format!("Could not save the installer: {e}"),
        })?;
        Ok(path)
    }

    /// Opens the file denying write/delete, checks it once more, writes `pending` and starts it.
    pub fn launch_stage(&self, update: &Update, path: &Path) -> (UpdateState, launch::Started) {
        self.set_phase(UpdatePhase::Verifying);
        let pending = info_of(update);
        let outcome = launch::run_locked(
            path,
            &|bytes| match update.verify(bytes) {
                Ok(()) => true,
                Err(e) => {
                    log::append(&format!("update: the saved installer failed its check: {e}"));
                    false
                }
            },
            &|p| {
                // Before the start: the tray dies during the install, and the next start reads this.
                self.change(|i| {
                    i.stored.pending = Some(pending.clone());
                    i.phase = UpdatePhase::WaitingForUac;
                });
                (self.launcher)(p)
            },
        );
        match outcome {
            Ok(Some(handle)) => (self.set_phase(UpdatePhase::Installing), Some(handle)),
            Ok(None) => {
                // Started, but nothing to follow. `pending` stays: if the installer does finish, the
                // next start settles it. This run must not sit on Installing forever.
                let message = "Windows started the installer, but canvas-rgb cannot follow it. If canvas-rgb does not restart on its own soon, run the update again.".to_string();
                self.change(|i| i.install_failed_for = Some(pending.version.clone()));
                (self.set_phase(UpdatePhase::InstallFailed { message }), None)
            }
            Err(e) => {
                let phase = match e {
                    launch::LaunchError::VerifyFailed => UpdatePhase::VerifyFailed,
                    launch::LaunchError::Declined => UpdatePhase::Declined,
                    launch::LaunchError::Failed(message) => UpdatePhase::InstallFailed { message },
                };
                // Nothing started: no pending version.
                let state = self.change(|i| {
                    i.stored.pending = None;
                    if matches!(phase, UpdatePhase::InstallFailed { .. }) {
                        i.install_failed_for = Some(pending.version.clone());
                    }
                    i.phase = phase;
                });
                (state, None)
            }
        }
    }
}

fn info_of(update: &Update) -> UpdateInfo {
    UpdateInfo {
        version: update.version.clone(),
        notes: update.body.clone().unwrap_or_default(),
        pub_date: update.date.map(state::rfc3339),
    }
}

fn clear_dir(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn redirect_policy(policy: DownloadPolicy) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("too many redirects");
        }
        match policy.check(attempt.url()) {
            Ok(()) => attempt.follow(),
            Err(why) => attempt.error(why),
        }
    })
}

/// A failed download is a failed check of the file when the signature (or the version it was
/// signed for) is wrong; anything else is a download problem.
fn download_failure(e: &tauri_plugin_updater::Error) -> UpdatePhase {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) | E::SignedVersionMismatch { .. } | E::MissingSignedVersion => {
            UpdatePhase::VerifyFailed
        }
        E::Reqwest(r) if r.is_connect() => UpdatePhase::InstallFailed {
            message: "The download failed: no connection.".into(),
        },
        other => UpdatePhase::InstallFailed {
            message: format!("The download failed: {other}"),
        },
    }
}

pub(crate) async fn classify(e: &tauri_plugin_updater::Error, endpoint: &Url) -> CheckFailReason {
    use tauri_plugin_updater::Error as E;
    match e {
        // Only a failure to connect means offline; a timeout after connecting is something else.
        E::Reqwest(r) if r.is_connect() => CheckFailReason::Offline,
        // The plugin swallows a non-success status, so ask once more to learn which one it was.
        E::ReleaseNotFound => {
            let endpoint = endpoint.clone();
            tauri::async_runtime::spawn_blocking(move || probe(&endpoint, PROBE_TIMEOUT))
                .await
                .unwrap_or(CheckFailReason::Other)
        }
        _ => CheckFailReason::Other,
    }
}

fn unreachable(kind: std::io::ErrorKind) -> bool {
    use std::io::ErrorKind as K;
    matches!(
        kind,
        K::ConnectionRefused
            | K::ConnectionReset
            | K::ConnectionAborted
            | K::NotConnected
            | K::AddrNotAvailable
            | K::HostUnreachable
            | K::NetworkUnreachable
            | K::NetworkDown
    )
}

/// One plain GET of the feed after the plugin found no manifest: 403/429 with
/// `x-ratelimit-remaining: 0` is the rate limit, no connection is offline, the rest is other.
fn probe(endpoint: &Url, timeout: Duration) -> CheckFailReason {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(timeout))
        .build()
        .into();
    match agent.get(endpoint.as_str()).call() {
        // As in `classify`: only a failure to reach the host is offline; a timeout is something else.
        Err(ureq::Error::ConnectionFailed | ureq::Error::HostNotFound) => CheckFailReason::Offline,
        Err(ureq::Error::Io(e)) if unreachable(e.kind()) => CheckFailReason::Offline,
        Err(_) => CheckFailReason::Other,
        Ok(resp) => {
            let status = resp.status().as_u16();
            let exhausted = resp
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.trim() == "0");
            if (status == 403 || status == 429) && exhausted {
                CheckFailReason::RateLimited
            } else {
                CheckFailReason::Other
            }
        }
    }
}

// ---- wiring ----

/// Creates the controller (managed state) and settles a pending update. `args` are the tray's own.
pub fn init<R: Runtime>(app: &AppHandle<R>, args: &[String]) {
    let current = app.package_info().version.clone();
    let emitter = app.clone();
    let last_tray: Mutex<Option<String>> = Mutex::new(None);
    let on_change = Box::new(move |state: &UpdateState| {
        let _ = emitter.emit(EVT_UPDATE_STATE, state.clone());
        let now = tray_version(state);
        let changed = {
            let mut last = last_tray.lock().unwrap_or_else(|p| p.into_inner());
            let changed = *last != now;
            *last = now;
            changed
        };
        if changed {
            // The menu and tooltip rebuild from a refresh; it polls the daemon, so not on this thread.
            let handle = emitter.clone();
            std::thread::spawn(move || {
                crate::tray::refresh(&handle);
            });
        }
    });
    let launcher: Launcher = Arc::new(|path| launch::start_non_elevated(path, launch::INSTALLER_ARGS));
    let controller = Arc::new(Controller::new(Settings::production(), current, launcher, on_change));
    app.manage(controller.clone());
    if has_after_update_arg(args) {
        log::append("update: started by the installer (--after-update)");
    }
    controller.reconcile_pending();
}

/// The controller. Panics before `init`, like any missing managed state.
pub fn controller<R: Runtime>(app: &AppHandle<R>) -> Arc<Controller> {
    app.state::<Arc<Controller>>().inner().clone()
}

/// The tray's "Check for updates": a Check now on a worker thread. No window opens; the result
/// reaches the tooltip and the "Update to" row through the usual refresh.
pub fn check_in_background<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    std::thread::spawn(move || {
        tauri::async_runtime::block_on(controller(&app).check(&app, true));
    });
}

/// The version whose install did not finish, for the tray tooltip.
pub fn failed_version<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    controller(app).failed_version()
}

/// The version the tray offers right now (see [`tray_version`]).
pub fn offered_version<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    tray_version(&controller(app).state())
}

// ---- commands (names in canvas_proto::tauri) ----

#[tauri::command]
pub fn update_state<R: Runtime>(app: AppHandle<R>) -> UpdateState {
    controller(&app).state()
}

#[tauri::command]
pub async fn update_check_now<R: Runtime>(app: AppHandle<R>) -> UpdateState {
    controller(&app).check(&app, true).await
}

/// Starts the install and returns at once; progress arrives as `canvas://update-state`. Refused
/// (returns the current state) while a check or an install runs.
#[tauri::command]
pub fn update_install<R: Runtime>(app: AppHandle<R>) -> UpdateState {
    let ctl = controller(&app);
    match ctl.begin_install() {
        Some(update) => {
            let state = ctl.state();
            tauri::async_runtime::spawn(async move {
                ctl.run_install(&update).await;
            });
            state
        }
        None if ctl.needs_check_to_install() => {
            // No release object (the tray restarted): check first, install what that finds.
            let state = ctl.state();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                ctl.check_and_install(&app).await;
            });
            state
        }
        None => ctl.state(),
    }
}

#[tauri::command]
pub fn update_later<R: Runtime>(app: AppHandle<R>) -> UpdateState {
    controller(&app).later()
}

#[tauri::command]
pub fn update_skip<R: Runtime>(app: AppHandle<R>) -> UpdateState {
    controller(&app).skip()
}

#[tauri::command]
pub fn update_unskip<R: Runtime>(app: AppHandle<R>) -> UpdateState {
    controller(&app).unskip()
}

/// `invoke("update_set_auto", { enabled })`.
#[tauri::command]
pub fn update_set_auto<R: Runtime>(app: AppHandle<R>, enabled: bool) -> UpdateState {
    controller(&app).set_auto(enabled)
}

#[tauri::command]
pub fn update_dismiss_updated<R: Runtime>(app: AppHandle<R>) -> UpdateState {
    controller(&app).dismiss_updated()
}
