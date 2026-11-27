//! Service-owned safe start (#577).
//!
//! Only the service sees every way a daemon run can end, so it owns the verdict, the streak and
//! the mode. The verdict table is a pure function ([`verdict`]) so every row of the spec can be
//! tested without a process. Both files live in the fail-closed machine home: `svc-run.json` is
//! written at service start and removed when the SCM stops it; `safe-start.json` is the streak
//! and the mode and is never deleted at stop. Nothing here deletes, renames or adopts an
//! untrusted file - it reads as no state and the caller logs the path.

use crate::store::{self, StoreError};
use canvas_proto::api::{LastExit, SafeStartMode, SafeStartReason};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const SVC_RUN_FILE: &str = "svc-run.json";
pub const SAFE_START_FILE: &str = "safe-start.json";

pub fn svc_run_path(home: &Path) -> PathBuf {
    home.join(SVC_RUN_FILE)
}

pub fn safe_start_path(home: &Path) -> PathBuf {
    home.join(SAFE_START_FILE)
}

/// What ended the daemon run, as the service saw it. The exit code alone cannot say which:
/// `TerminateProcess` reports 1, the same code a bind failure reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndCause {
    /// The daemon told the service it was quitting on purpose.
    QuitNotice,
    /// The service is stopping: a planned end.
    ServiceStop,
    /// A kill the service chose: stale daemon, quit replacement, user switch.
    ServiceKill,
    /// The daemon ended with its Windows session.
    SessionEnd,
    /// A `canvas-rgbd` left by another service instance, ended at start.
    StaleAtStart,
    /// A heartbeat budget ran out.
    MissedHeartbeat,
    /// The process exited and the service reaped it.
    Exited,
}

/// Which heartbeat budget the run was on when it ended (#163).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beat {
    BeforeFirst,
    AfterFirst,
}

/// `0x40010004` and `0xC000026B` say the window station / session went away (#6).
fn is_session_end_code(code: i32) -> bool {
    matches!(code as u32, 0x4001_0004 | 0xC000_026B)
}

/// The exit codes that only a crash produces (#577 table).
fn is_crash_code(code: i32) -> bool {
    matches!(code as u32, 101 | 0xC000_0409 | 0xC000_0005 | 0xC000_00FD | 0x8000_0003 | 0xC000_0374)
}

/// The verdict table. Only [`LastExit::Unclean`] counts toward safe start. A service-caused
/// kill and a session end are planned even when the fake child reports code 1; a missed steady
/// heartbeat and a crash code are unclean even though the daemon had no chance to say why.
pub fn verdict(cause: EndCause, code: Option<i32>, beat: Beat) -> LastExit {
    if cause == EndCause::QuitNotice {
        return LastExit::Clean;
    }
    if matches!(cause, EndCause::ServiceStop | EndCause::ServiceKill | EndCause::SessionEnd) {
        return LastExit::Planned;
    }
    if cause == EndCause::StaleAtStart {
        return LastExit::Unknown;
    }
    if code.is_some_and(is_session_end_code) {
        return LastExit::Planned;
    }
    if code == Some(0) {
        // A lost quit notice still ends cleanly (main.rs:108).
        return LastExit::Clean;
    }
    if code.is_some_and(is_crash_code) {
        return LastExit::Unclean;
    }
    if cause == EndCause::MissedHeartbeat {
        return match beat {
            Beat::AfterFirst => LastExit::Unclean,
            Beat::BeforeFirst => LastExit::StartupFailure,
        };
    }
    if code == Some(1) && beat == Beat::BeforeFirst {
        // A bind or token failure before the first beat (#577 table).
        return LastExit::StartupFailure;
    }
    LastExit::Unknown
}

/// Only these two verdicts count against `DAEMON_RESTART_LIMIT`. A clean exit 0 with no notice
/// is relaunched like today, but it does not spend a restart (#577 section 1).
pub fn counts_toward_restart(v: LastExit) -> bool {
    matches!(v, LastExit::Unclean | LastExit::StartupFailure)
}

/// The persisted part of the state: `safe-start.json`, written atomically like the store's
/// other files. `mode` and `reason` are kept so a service restart re-reads the same decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persisted {
    pub unclean_streak: u32,
    pub mode: SafeStartMode,
    #[serde(default)]
    pub reason: Option<SafeStartReason>,
    #[serde(default)]
    pub since: Option<String>,
}

impl Default for Persisted {
    fn default() -> Self {
        Self { unclean_streak: 0, mode: SafeStartMode::Normal, reason: None, since: None }
    }
}

/// An RFC 3339 UTC stamp, for `since` when a safe run begins.
pub fn stamp_now() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // A civil-from-days conversion, the same one the service's log uses. No chrono dependency.
    let days = (secs / 86_400) as i64;
    let within = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", within / 3600, (within / 60) % 60, within % 60)
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + i64::from(m <= 2), m, d)
}

/// Writes `svc-run.json`. A successful write is the marker: a later start that still finds it
/// knows this service did not stop cleanly.
pub fn write_svc_run(home: &Path) -> Result<(), StoreError> {
    store::write_atomic(&svc_run_path(home), b"{\"marker\":1}\n")
}

/// Removes the marker on the SCM's Stop, Shutdown and PreShutdown path.
pub fn remove_svc_run(home: &Path) -> Result<(), StoreError> {
    match std::fs::remove_file(svc_run_path(home)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StoreError::Io { path: svc_run_path(home), source }),
    }
}

pub fn save(home: &Path, state: &Persisted) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| StoreError::Encode("safe-start.json", e.to_string()))?;
    store::write_atomic(&safe_start_path(home), &bytes)
}

/// The state read at service start. A file the caller does not trust, cannot parse or cannot
/// read is ignored and reported through the returned flags; it is never changed. The caller
/// logs the path before it starts fans or the daemon.
pub fn load(home: &Path, trusted: impl Fn(&Path) -> bool) -> Loaded {
    let run = svc_run_path(home);
    let leftover = trusted(&run) && run.exists();
    let state = safe_start_path(home);
    let persisted = if trusted(&state) {
        match std::fs::read(&state) {
            Ok(bytes) => match serde_json::from_slice::<Persisted>(&bytes) {
                Ok(s) if s.unclean_streak < u32::MAX => s,
                Ok(_) => Persisted::default(),
                Err(_) => Persisted::default(),
            },
            Err(_) => Persisted::default(),
        }
    } else {
        Persisted::default()
    };
    Loaded { persisted, leftover, untrusted_run: run.exists() && !trusted(&run), untrusted_state: state.exists() && !trusted(&state) }
}

/// What the fail-closed read found. Both untrusted flags name a file the service leaves alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub persisted: Persisted,
    /// `svc-run.json` survived, so the previous service did not stop cleanly.
    pub leftover: bool,
    pub untrusted_run: bool,
    pub untrusted_state: bool,
}

/// The in-service book. It keeps the verdict of the run that just ended, updates the streak
/// and the mode, and persists the result before the next daemon is launched.
pub struct Book {
    pub persisted: Persisted,
    pub last_exit: LastExit,
    home: PathBuf,
}

impl Book {
    /// Opens the book from the fail-closed read. A leftover marker makes the last verdict
    /// `Unknown`, never `Unclean`; the persisted mode is otherwise honoured as written.
    pub fn open(home: &Path, loaded: Loaded) -> Self {
        // There is no daemon run in this service instance yet. A leftover marker says the
        // previous service did not stop cleanly, but that still reads Unknown, never Unclean.
        let last_exit = LastExit::Unknown;
        Self { persisted: loaded.persisted, last_exit, home: home.to_path_buf() }
    }

    pub fn mode(&self) -> SafeStartMode {
        self.persisted.mode
    }

    pub fn last_exit(&self) -> LastExit {
        self.last_exit
    }

    /// The reason a surface should show, or `None` in normal mode.
    pub fn reason(&self) -> Option<SafeStartReason> {
        (self.persisted.mode == SafeStartMode::Safe).then_some(self.persisted.reason).flatten()
    }

    /// The safe run's `since`, or `None` when it is not safe.
    pub fn since(&self) -> Option<String> {
        (self.persisted.mode == SafeStartMode::Safe).then(|| self.persisted.since.clone()).flatten()
    }

    /// Records how a daemon run ended and decides the mode for the next one. A normal-mode
    /// clean or planned end clears the streak; an unclean end adds one. Logons never call this.
    pub fn record_end(&mut self, v: LastExit, now: &str) {
        self.last_exit = v;
        if v == LastExit::Unclean {
            self.persisted.unclean_streak = self.persisted.unclean_streak.saturating_add(1);
        } else if matches!(v, LastExit::Clean | LastExit::Planned) && self.persisted.mode == SafeStartMode::Normal {
            self.persisted.unclean_streak = 0;
        }
        self.decide(now);
        let _ = self.flush();
    }

    /// "Start normally" or "Start in safe start" from Help & About, the banner or the tray.
    /// A leave clears the streak and sets Normal; an enter sets Safe and `UserChose`.
    pub fn set_user_choice(&mut self, mode: SafeStartMode, now: &str) {
        match mode {
            SafeStartMode::Normal => {
                self.persisted.unclean_streak = 0;
                self.persisted.mode = SafeStartMode::Normal;
                self.persisted.reason = None;
                self.persisted.since = None;
            }
            SafeStartMode::Safe => {
                self.persisted.mode = SafeStartMode::Safe;
                self.persisted.reason = Some(SafeStartReason::UserChose);
                if self.persisted.since.is_none() {
                    self.persisted.since = Some(now.to_string());
                }
            }
        }
        self.flush_ok_or_log();
    }

    /// The mode from the streak, the last verdict and the persisted choice (#577 section 3).
    fn decide(&mut self, now: &str) {
        let streak = self.persisted.unclean_streak;
        let (mode, reason) = if self.last_exit == LastExit::Unclean && streak < 2 {
            (SafeStartMode::Safe, Some(SafeStartReason::UncleanOnce))
        } else if streak >= 2 {
            (SafeStartMode::Safe, Some(SafeStartReason::CrashLoop))
        } else if self.persisted.mode == SafeStartMode::Safe
            && self.persisted.reason == Some(SafeStartReason::UserChose)
        {
            (SafeStartMode::Safe, Some(SafeStartReason::UserChose))
        } else {
            (SafeStartMode::Normal, None)
        };
        if mode == SafeStartMode::Safe && self.persisted.mode != SafeStartMode::Safe {
            self.persisted.since = Some(now.to_string());
        }
        if mode == SafeStartMode::Normal {
            self.persisted.since = None;
        }
        self.persisted.mode = mode;
        self.persisted.reason = reason;
    }

    fn flush(&self) -> Result<(), StoreError> {
        save(&self.home, &self.persisted)
    }

    fn flush_ok_or_log(&self) {
        if let Err(e) = self.flush() {
            tracing::error!(path = %safe_start_path(&self.home).display(), error = %e, "could not save safe-start.json");
        }
    }
}
