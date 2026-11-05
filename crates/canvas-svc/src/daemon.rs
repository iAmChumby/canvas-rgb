//! Daemon supervision (grill G3). The daemon runs in the user's session, so the service starts it
//! there through [`SessionLauncher`]. A missed heartbeat for 5 s or an exit counts as a crash.
//! The service restarts a crashed daemon at most `DAEMON_RESTART_LIMIT` times in
//! `DAEMON_RESTART_WINDOW_S`, then writes black through OpenRGB and stops restarting it.
//! A daemon that sent `DaemonQuitting` is not restarted.
//!
//! A daemon that ended with its session (its user logged off, Windows tore the session down) is
//! not a crash (#6). Those exits count against a separate budget of [`SESSION_END_LIMIT`] per
//! console session, and the relaunch is held until the session is gone, the console id changes
//! or [`SESSION_END_HOLD`] passes. Past that budget they count as crashes again. A new console
//! session (a logon) clears the crash stop and both budgets, so a crash stop does not strand the
//! user until the service restarts.

use crate::process::ChildProcess;
use canvas_proto::consts::{DAEMON_RESTART_LIMIT, DAEMON_RESTART_WINDOW_S};
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// A missed-heartbeat run this long counts as a crash.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

/// How many exits that mean "the daemon ended with its session" one console session gets before
/// they count as crashes again (#6).
pub const SESSION_END_LIMIT: u32 = 3;

/// How long the relaunch is held after such an exit (#6). The hold ends earlier when the session
/// is gone or the console id changes; this is the bound for a session that stays alive.
pub const SESSION_END_HOLD: Duration = Duration::from_secs(60);

/// The exit codes that mean the daemon ended with its session, not with a crash (#6):
/// `0x40010004` (`DBG_TERMINATE_PROCESS`: the session was torn down) and `0xC000026B`
/// (`STATUS_DLL_INIT_FAILED_LOGOFF`: the window station is shutting down).
fn is_session_end_code(code: i32) -> bool {
    matches!(code as u32, 0x4001_0004 | 0xC000_026B)
}

/// The relaunch held after an exit that ended with the session (#6).
#[derive(Debug, Clone, Copy)]
struct SessionHold {
    /// When the hold started.
    since: Instant,
    /// The session the daemon ended in.
    session: Option<u32>,
    /// The console session id when the hold started.
    console: Option<u32>,
}

/// Starts the daemon in the active user session.
pub trait SessionLauncher: Send {
    /// Starts the daemon. An error of kind [`io::ErrorKind::NotConnected`] means no user is logged
    /// on yet (no console session, or `WTSQueryUserToken` found no token: os error 1008); the
    /// supervisor waits for a session on that and does not count it as a crash.
    fn launch(&mut self, exe: &Path) -> io::Result<Box<dyn ChildProcess>>;

    /// Every `canvas-rgbd.exe` process running right now, with its image path and session.
    /// The default finds none (the fakes override it).
    fn running_daemons(&mut self) -> Vec<RunningDaemon> {
        Vec::new()
    }

    /// The active console session, or `None` when there is none. `None` is also what a switch in
    /// progress reports for a moment, so the supervisor treats it as "unknown", never as a move.
    fn console_session(&mut self) -> Option<u32> {
        None
    }

    /// Whether a user is still logged on in `session`. A logoff leaves the console session id as it
    /// was, so this (not the id) tells the supervisor the daemon's session has ended. Unknown counts
    /// as logged on.
    fn session_alive(&mut self, _session: u32) -> bool {
        true
    }
}

/// A `canvas-rgbd.exe` process that is already running, found before the service launches its own.
pub struct RunningDaemon {
    /// The full path of the process's image.
    pub image: PathBuf,
    /// The session the process runs in.
    pub session: u32,
    pub process: Box<dyn ChildProcess>,
}

/// How long the service waits for a stale daemon to exit after ending it.
const STALE_EXIT_WAIT: Duration = Duration::from_secs(5);

/// A different console session must stay the active one this long before the daemon is moved to it.
/// A console that flips back and forth every second (or reports no session for a moment during a
/// switch) must not cost a kill and a launch per flip: moves bypass the crash budget, so this settle
/// time is their bound, one move per `SESSION_SETTLE` at most.
pub const SESSION_SETTLE: Duration = Duration::from_secs(3);

/// Ends a daemon process, and says why first. Every kill of a daemon goes through here, so a
/// line naming the pid and the reason is logged before the process is ended, on every path
/// (class `cause:timing-test`, #45 and #55: a test asserts the line on each path).
///
/// `reason` completes "terminating the daemon (pid N): ...". A stop that worked is routine, so
/// the line is an INFO one; only the failure is a warning. A failed kill is logged too, and
/// returned for a caller that has to stop on it.
fn terminate_daemon(process: &mut dyn ChildProcess, reason: &str) -> io::Result<()> {
    let pid = process.id();
    tracing::info!(pid, reason, "terminating the daemon (pid {pid}): {reason}");
    let ended = process.kill();
    if let Err(e) = &ended {
        tracing::warn!(pid, reason, error = %e, "could not end the daemon");
    }
    ended
}

/// Ends `child` (already told to die) and waits up to `STALE_EXIT_WAIT` for it to go, so the next
/// launch does not start while the old daemon still holds the API port (it would lose the bind and
/// count as a crash).
fn wait_for_exit(child: &mut dyn ChildProcess, pid: u32) {
    let deadline = Instant::now() + STALE_EXIT_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                tracing::warn!(pid, "the old daemon did not exit within 5 s");
                break;
            }
            Err(e) => {
                tracing::warn!(pid, error = %e, "could not wait for the old daemon");
                break;
            }
        }
    }
}

/// What the service knew about the daemon at the moment it ended OpenRGB, for the log line of that
/// stop (#81): a stick that went dark after an update needs the order and the gap between the two
/// kills to be on record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonBeforeStop {
    /// The service ended the daemon `at`; it did not wait for the daemon to exit.
    Stopped { pid: u32, at: Instant },
    /// The daemon runs and was not told: it keeps sending frames.
    Running { pid: u32 },
    /// The daemon had already exited when the question was asked (#86).
    Exited { pid: u32, code: i32 },
    /// The service tried to end the daemon and the kill failed: it may still be running.
    NotEnded { pid: u32 },
    /// No daemon process.
    NotRunning,
}

impl DaemonBeforeStop {
    /// The sentence part of the OpenRGB stop line: what became of the daemon, and how long before.
    pub fn describe(&self, now: Instant) -> String {
        match *self {
            Self::Stopped { pid, at } => {
                let gap_ms = now.saturating_duration_since(at).as_secs_f64() * 1000.0;
                // The kill asks the daemon to die; it is not waited on, so the daemon exits
                // later than this moment and the gap is an upper bound (#86).
                format!(
                    "the daemon (pid {pid}) was told to stop {gap_ms:.3} ms earlier (its exit was not waited on)"
                )
            }
            Self::Running { pid } => format!("the daemon (pid {pid}) is still running and was not told; it keeps sending frames"),
            Self::Exited { pid, code } => format!(
                "the daemon (pid {pid}) had already exited ({})",
                crate::process::describe_exit_code(code)
            ),
            Self::NotEnded { pid } => {
                format!("the daemon (pid {pid}) could not be ended; it may still be running")
            }
            Self::NotRunning => "no daemon is running".to_string(),
        }
    }
}

/// Compares two Windows paths without regard to case or a verbatim (`\\?\`) prefix.
fn same_image(a: &Path, b: &Path) -> bool {
    fn norm(p: &Path) -> String {
        let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
        s.strip_prefix("\\\\?\\").map(str::to_owned).unwrap_or(s)
    }
    norm(a) == norm(b)
}

/// The log reason for a daemon that exited: the code in hex with its NTSTATUS name (#19).
fn exited_reason(code: i32) -> String {
    format!("exited with code {}", crate::process::describe_exit_code(code))
}

/// What the caller must do after a supervision pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonAction {
    None,
    /// The restart limit was hit: write black through OpenRGB, once.
    WriteBlack,
}

/// Shared with the pipe server: the heartbeat clock the supervisor watches, and the pid it
/// belongs to. The pipe stamps this on arrival so a fan write that holds the core lock cannot
/// age a heartbeat (#129), and only for beats from the launched daemon's pid, so a stale
/// daemon's queued beats after a restart cannot stamp the new daemon's clock.
#[derive(Default)]
pub struct HeartbeatClock {
    /// When the launched daemon's last heartbeat arrived; cleared when a daemon is killed or stopped.
    at: Option<Instant>,
    /// The pid of the daemon the supervisor launched; `None` while no daemon runs (or after a stop).
    pid: Option<u32>,
    /// True once the supervisor is stopped; beats are ignored from then on.
    stopped: bool,
}

/// What the pipe server does with one heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    /// The launched daemon's beat, freshly stamped.
    Stamped,
    /// A beat from a pid that is not the launched daemon; ignored.
    Foreign,
    /// The supervisor is stopped; the beat is ignored.
    Stopped,
}

impl HeartbeatClock {
    /// Stamps a beat from `pid`; reports what was done with it.
    pub(crate) fn stamp(&mut self, pid: u32, now: Instant) -> Stamp {
        if self.stopped {
            return Stamp::Stopped;
        }
        if self.pid == Some(pid) {
            self.at = Some(now);
            return Stamp::Stamped;
        }
        Stamp::Foreign
    }

    /// The clock's current beat time, for tests.
    #[cfg(test)]
    pub(crate) fn at(&self) -> Option<Instant> {
        self.at
    }

    /// No daemon runs (or it was killed/expected to exit): beats from any pid are ignored.
    fn clear(&mut self) {
        self.at = None;
        self.pid = None;
    }

    /// The supervisor is stopped: beats are ignored from then on, and the clock is cleared.
    fn mark_stopped(&mut self) {
        self.stopped = true;
        self.at = None;
        self.pid = None;
    }

    /// The supervisor starts again (a new console session cleared the crash stop): beats count
    /// again (#6).
    fn unmark_stopped(&mut self) {
        self.stopped = false;
    }
}

pub struct DaemonSupervisor {
    exe: PathBuf,
    launcher: Box<dyn SessionLauncher>,
    child: Option<Box<dyn ChildProcess>>,
    started: bool,
    /// Shared with the pipe server (see [`HeartbeatClock`]).
    last_heartbeat: Arc<Mutex<HeartbeatClock>>,
    restarts: VecDeque<Instant>,
    quitting: bool,
    /// The service is stopping: the daemon is not started again, and a new console session does
    /// not change that.
    service_stopped: bool,
    /// The crash limit tripped: the daemon is not started again until a new console session
    /// clears it (#6).
    crash_stopped: bool,
    /// The console session the supervisor last saw.
    console: Option<u32>,
    /// The last seen console session has no user in it: a logoff was seen.
    logged_off: bool,
    /// The exits that ended with the session, in the console session of `budget_session` (#6).
    session_ends: u32,
    /// The console session the session-end budget belongs to.
    budget_session: Option<u32>,
    /// The relaunch is held after an exit that ended with the session (#6).
    hold: Option<SessionHold>,
    /// The last launch found no user session. Not a crash: the next ticks launch again until a user
    /// is logged on (#6).
    waiting_for_session: bool,
    /// The console session the running daemon was launched into (`None` when the launcher cannot say).
    child_session: Option<u32>,
    /// A different console session seen while the daemon runs, and since when (see `SESSION_SETTLE`).
    pending_move: Option<(u32, Instant)>,
    /// The daemon `stop()` ended, and when (#81): the OpenRGB stop line reports the gap.
    stopped_daemon: Option<(u32, Instant)>,
    /// The daemon `stop()` could not end (#81): the OpenRGB stop line says that instead of
    /// claiming the daemon was terminated.
    unended_daemon: Option<u32>,
}

impl DaemonSupervisor {
    pub fn new(exe: PathBuf, launcher: Box<dyn SessionLauncher>) -> Self {
        Self {
            exe,
            launcher,
            child: None,
            started: false,
            last_heartbeat: Arc::new(Mutex::new(HeartbeatClock::default())),
            restarts: VecDeque::new(),
            quitting: false,
            service_stopped: false,
            crash_stopped: false,
            console: None,
            logged_off: false,
            session_ends: 0,
            budget_session: None,
            hold: None,
            waiting_for_session: false,
            child_session: None,
            pending_move: None,
            stopped_daemon: None,
            unended_daemon: None,
        }
    }

    /// Starts the daemon for the first time (service start).
    pub fn start(&mut self, now: Instant) {
        self.started = true;
        self.end_stale_daemons();
        self.launch(now);
    }

    /// Ends a `canvas-rgbd` left over from a previous service instance. It holds the API port, so
    /// every daemon launched beside it would exit with code 1 and the restart limit would write
    /// black. Only a daemon whose image is this service's own `exe` and that runs in the active
    /// console session is ended; a `canvas-rgbd` from any other path is never touched.
    fn end_stale_daemons(&mut self) {
        let Some(console) = self.launcher.console_session() else {
            return;
        };
        for mut stale in self.launcher.running_daemons() {
            let pid = stale.process.id();
            if !same_image(&stale.image, &self.exe) || stale.session != console {
                tracing::info!(
                    pid,
                    image = %stale.image.display(),
                    session = stale.session,
                    "left a canvas-rgbd alone: not this install's daemon in the console session"
                );
                continue;
            }
            if terminate_daemon(stale.process.as_mut(), "a stale daemon from a previous service instance").is_err() {
                continue;
            }
            let deadline = Instant::now() + STALE_EXIT_WAIT;
            loop {
                match stale.process.try_wait() {
                    Ok(Some(_)) => {
                        tracing::info!(pid, "the stale daemon has exited");
                        break;
                    }
                    Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                    Ok(None) => {
                        tracing::warn!(pid, "the stale daemon did not exit within 5 s");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(pid, error = %e, "could not wait for the stale daemon");
                        break;
                    }
                }
            }
        }
    }

    /// The daemon that was launched into `was` has lost its session: its user logged off (the
    /// console id can stay the same) or another session definitely holds the console. A console that
    /// reports no session is unknown, not a move. Used for a daemon that already exited.
    fn session_ended(&mut self, was: Option<u32>) -> bool {
        let Some(was) = was else { return false };
        !self.launcher.session_alive(was) || self.launcher.console_session().is_some_and(|c| c != was)
    }

    /// The same question for a daemon that is still running: a logoff is immediate, a move to
    /// another console session only after it has held for `SESSION_SETTLE`, and only to a session
    /// with a user in it. "Switch user" puts the logon screen on the console (no user token there):
    /// the daemon stays with the user it runs for until someone logs on in the new session.
    fn session_changed(&mut self, now: Instant) -> bool {
        let Some(was) = self.child_session else {
            self.pending_move = None;
            return false;
        };
        if !self.launcher.session_alive(was) {
            return true;
        }
        match self.launcher.console_session() {
            Some(c) if c != was && self.launcher.session_alive(c) => match self.pending_move {
                Some((target, since)) if target == c => now.saturating_duration_since(since) >= SESSION_SETTLE,
                _ => {
                    self.pending_move = Some((c, now));
                    false
                }
            },
            _ => {
                self.pending_move = None;
                false
            }
        }
    }

    fn launch(&mut self, now: Instant) {
        let session = self.launcher.console_session();
        match self.launcher.launch(&self.exe) {
            Ok(child) => {
                let pid = child.id();
                self.child = Some(child);
                self.child_session = session;
                self.waiting_for_session = false;
                // The records of the daemon before this one belong to that daemon: a new one
                // replaces them, or the OpenRGB line names a pid that is long gone (#87).
                self.stopped_daemon = None;
                self.unended_daemon = None;
                {
                    let mut hb = self.heartbeat_clock();
                    hb.at = Some(now);
                    hb.pid = Some(pid);
                }
                tracing::info!(exe = %self.exe.display(), "daemon started in the user session");
            }
            Err(e) if e.kind() == io::ErrorKind::NotConnected => {
                self.child = None;
                self.clear_heartbeat_clock();
                // Logged once; the ticks that follow retry without a line each.
                if !self.waiting_for_session {
                    tracing::info!(error = %e, "no user is logged on yet; waiting for a session to start the daemon");
                }
                self.waiting_for_session = true;
            }
            Err(e) => {
                self.child = None;
                self.waiting_for_session = false;
                self.clear_heartbeat_clock();
                tracing::error!(error = %e, "could not start the daemon");
            }
        }
    }

    /// The shared heartbeat clock the pipe server stamps on every heartbeat message (#129).
    pub fn shared_heartbeats(&self) -> Arc<Mutex<HeartbeatClock>> {
        Arc::clone(&self.last_heartbeat)
    }

    fn heartbeat_clock(&self) -> MutexGuard<'_, HeartbeatClock> {
        self.last_heartbeat.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// No daemon is running: beats are ignored, and the clock is cleared.
    fn clear_heartbeat_clock(&self) {
        self.heartbeat_clock().clear();
    }

    fn last_heartbeat(&self) -> Option<Instant> {
        self.heartbeat_clock().at
    }

    pub fn on_heartbeat(&mut self, now: Instant) {
        let mut hb = self.heartbeat_clock();
        if !hb.stopped {
            hb.at = Some(now);
        }
    }

    /// The service is stopping: the daemon it launched ends too and is not restarted.
    /// Without this a service restart leaves the old daemon holding the API port, so the new
    /// service's daemon cannot bind and crash-loops (lighting session 2).
    pub fn stop(&mut self) {
        self.service_stopped = true;
        self.heartbeat_clock().mark_stopped();
        if let Some(mut child) = self.child.take() {
            let pid = child.id();
            // The stop is recorded only when the daemon really ended. A kill that failed leaves
            // the daemon running, and the OpenRGB line must not say it was terminated (#81).
            if terminate_daemon(child.as_mut(), "the service is stopping").is_ok() {
                self.stopped_daemon = Some((pid, Instant::now()));
            } else {
                self.unended_daemon = Some(pid);
            }
        }
    }

    /// What the daemon's state is right now, for the line that logs an OpenRGB stop (#81).
    ///
    /// A daemon that has exited but has not been reaped is not running, so the process is asked
    /// (#86).
    pub fn before_openrgb_stop(&mut self) -> DaemonBeforeStop {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(code)) => {
                    let pid = child.id();
                    return DaemonBeforeStop::Exited { pid, code };
                }
                Ok(None) => return DaemonBeforeStop::Running { pid: child.id() },
                Err(e) => {
                    // The process could not be asked. The daemon was not told to stop, so the
                    // line says it runs, and the failure is logged.
                    tracing::warn!(pid = child.id(), error = %e, "could not query the daemon process");
                    return DaemonBeforeStop::Running { pid: child.id() };
                }
            }
        }
        if let Some(pid) = self.unended_daemon {
            return DaemonBeforeStop::NotEnded { pid };
        }
        match self.stopped_daemon {
            Some((pid, at)) => DaemonBeforeStop::Stopped { pid, at },
            None => DaemonBeforeStop::NotRunning,
        }
    }

    /// The daemon is quitting on purpose; its exit is not a crash.
    pub fn on_quitting(&mut self) {
        tracing::info!("daemon announced a quit; it will not be restarted");
        self.quitting = true;
    }

    /// The PID of the daemon this service launched, while that process is running. `None` when
    /// no daemon is running, so the pipe server then admits any client in the console session.
    pub fn daemon_pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    /// True while a daemon process is running and its heartbeat is fresh.
    pub fn is_connected(&self, now: Instant) -> bool {
        self.child.is_some()
            && self
                .last_heartbeat()
                .is_some_and(|hb| now.saturating_duration_since(hb) < HEARTBEAT_TIMEOUT)
    }

    /// Whether the restart limit has been reached and restarts have stopped.
    pub fn stopped(&self) -> bool {
        self.service_stopped || self.crash_stopped
    }

    /// A new console session (a logon) clears the crash stop and both budgets, so a crash stop
    /// does not strand the user until the service restarts (#6).
    fn on_new_session(&mut self, session: u32) {
        if self.crash_stopped {
            tracing::info!(
                session,
                "a new console session started; the daemon is started again"
            );
        }
        self.crash_stopped = false;
        self.heartbeat_clock().unmark_stopped();
        self.restarts.clear();
        self.session_ends = 0;
        self.budget_session = Some(session);
        self.hold = None;
    }

    /// Watches the console session for a new one. A logoff shows up as a session with no user in
    /// it; the logon after it as one with a user again. A different console id is a different
    /// session right away. Says whether a new console session was found.
    ///
    /// The first console session this supervisor sees is not a new one: it is the one the service
    /// started in.
    fn watch_for_new_session(&mut self) -> bool {
        let console = self.launcher.console_session();
        match (self.console, console) {
            (Some(seen), Some(now)) if seen != now => {
                // The console moved to another session: that is a new console session.
                self.console = Some(now);
                self.logged_off = false;
                self.on_new_session(now);
                true
            }
            (Some(_), Some(now)) => {
                // The same session: its user comes and goes in it.
                if self.launcher.session_alive(now) {
                    if self.logged_off {
                        self.logged_off = false;
                        self.on_new_session(now);
                        true
                    } else {
                        false
                    }
                } else {
                    self.logged_off = true;
                    false
                }
            }
            (Some(_), None) => {
                // The console reports no session: unknown, not a move and not a logoff.
                false
            }
            (None, Some(now)) => {
                self.console = Some(now);
                self.logged_off = !self.launcher.session_alive(now);
                false
            }
            (None, None) => false,
        }
    }

    /// One supervision pass per second.
    pub fn tick(&mut self, now: Instant) -> DaemonAction {
        if !self.started || self.service_stopped || self.quitting {
            return DaemonAction::None;
        }
        // A new console session (a logon) clears the crash stop and both budgets (#6). It is
        // watched even while the supervisor is crash-stopped: that stop must not strand the
        // user until the service restarts.
        if self.watch_for_new_session() && self.child.is_none() {
            // The daemon is started again in the new session.
            self.launch(now);
            return DaemonAction::None;
        }
        if self.crash_stopped {
            return DaemonAction::None;
        }
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(code)) => {
                    self.child = None;
                    self.clear_heartbeat_clock();
                    let was = self.child_session;
                    if is_session_end_code(code) {
                        return self.session_end(now, code, was);
                    }
                    if self.session_ended(was) {
                        // Its session logged off or lost the console: the exit is not a crash.
                        tracing::info!(code, "the daemon ended with its session; starting one in the active session");
                        self.launch(now);
                        return DaemonAction::None;
                    }
                    return self.crashed(now, &exited_reason(code));
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "could not query the daemon process"),
            }
            if self.session_changed(now) {
                self.pending_move = None;
                if let Some(mut child) = self.child.take() {
                    let _ = terminate_daemon(child.as_mut(), "the active console session changed");
                    let pid = child.id();
                    wait_for_exit(child.as_mut(), pid);
                    self.clear_heartbeat_clock();
                }
                self.launch(now);
                return DaemonAction::None;
            }
            let stale = self
                .last_heartbeat()
                .is_some_and(|hb| now.saturating_duration_since(hb) >= HEARTBEAT_TIMEOUT);
            if stale {
                if let Some(mut child) = self.child.take() {
                    let _ = terminate_daemon(child.as_mut(), "it missed heartbeats for 5 s");
                    let pid = child.id();
                    wait_for_exit(child.as_mut(), pid);
                    self.clear_heartbeat_clock();
                }
                return self.crashed(now, "missed heartbeats for 5 s");
            }
            return DaemonAction::None;
        }
        // No process: the relaunch is held after an exit that ended with the session (#6).
        if let Some(hold) = self.hold {
            let gone = hold
                .session
                .is_some_and(|s| !self.launcher.session_alive(s));
            let moved = self
                .launcher
                .console_session()
                .is_some_and(|c| Some(c) != hold.console);
            let timed_out = now.saturating_duration_since(hold.since) >= SESSION_END_HOLD;
            if !(gone || moved || timed_out) {
                // The session is on its way out: no daemon runs, and nothing is written black.
                return DaemonAction::None;
            }
            self.hold = None;
            tracing::info!("the session-end hold is over; starting the daemon again");
            self.launch(now);
            return DaemonAction::None;
        }
        // No process because no user was logged on: launch again, no crash counted (#6).
        if self.waiting_for_session {
            self.launch(now);
            return DaemonAction::None;
        }
        // No process: the last launch failed, so count it as a crash and try again.
        self.crashed(now, "not running after a failed launch")
    }

    /// The daemon exited with a code that says its session ended. That is not a crash (#6): it
    /// counts against the session-end budget, and the relaunch is held until the session is
    /// gone, the console id changes, or the hold times out.
    fn session_end(&mut self, now: Instant, code: i32, was: Option<u32>) -> DaemonAction {
        let described = crate::process::describe_exit_code(code);
        let console = self.launcher.console_session();
        // The budget belongs to one console session: a new one starts a new budget.
        let session = console.or(was);
        if session != self.budget_session {
            self.budget_session = session;
            self.session_ends = 0;
        }
        if self.session_ends >= SESSION_END_LIMIT {
            // The budget is used up: this exit counts as a crash again, so a daemon that keeps
            // ending with its session still trips the crash limit.
            tracing::warn!(
                code = %described,
                limit = SESSION_END_LIMIT,
                "the daemon ended with its session too often; counting it as a crash"
            );
            return self.crashed(now, &exited_reason(code));
        }
        self.session_ends += 1;
        tracing::info!(code = %described, "the daemon ended with its session ({described}); not a crash");
        // The relaunch waits: the session is on its way out, and a daemon started into a dying
        // session ends with it again. While the hold is on, `daemon_pid()` is `None`, so the
        // pipe admits any client in the console session: the admission rule is unchanged, and
        // the pipe still wants a `Hello` first.
        self.hold = Some(SessionHold {
            since: now,
            session: was,
            console,
        });
        DaemonAction::None
    }

    fn crashed(&mut self, now: Instant, reason: &str) -> DaemonAction {
        let window = Duration::from_secs(DAEMON_RESTART_WINDOW_S);
        while let Some(front) = self.restarts.front() {
            if now.saturating_duration_since(*front) >= window {
                self.restarts.pop_front();
            } else {
                break;
            }
        }
        if self.restarts.len() as u32 >= DAEMON_RESTART_LIMIT {
            self.crash_stopped = true;
            self.heartbeat_clock().mark_stopped();
            tracing::error!(
                reason,
                limit = DAEMON_RESTART_LIMIT,
                window_s = DAEMON_RESTART_WINDOW_S,
                "daemon crashed too often; writing black through OpenRGB and stopping restarts"
            );
            return DaemonAction::WriteBlack;
        }
        self.restarts.push_back(now);
        tracing::warn!(reason, restart = self.restarts.len(), "daemon crashed; restarting it");
        self.launch(now);
        DaemonAction::None
    }
}

#[cfg(windows)]
pub use windows_launcher::{launch_as_console_user, WtsLauncher};

#[cfg(windows)]
mod windows_launcher {
    use super::{RunningDaemon, SessionLauncher};
    use crate::process::ChildProcess;
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
    use windows_sys::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
    use windows_sys::Win32::System::Threading::{
        CreateProcessAsUserW, GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
        WaitForSingleObject, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, STARTUPINFOW,
    };

    /// Starts the daemon in the active console session with the user's token
    /// (`WTSQueryUserToken` then `CreateProcessAsUserW`). Runs only as the service.
    pub struct WtsLauncher;

    impl SessionLauncher for WtsLauncher {
        fn console_session(&mut self) -> Option<u32> {
            // SAFETY: a plain query with no arguments.
            let session = unsafe { WTSGetActiveConsoleSessionId() };
            (session != u32::MAX).then_some(session)
        }

        fn session_alive(&mut self, session: u32) -> bool {
            // SAFETY: a plain query; the token handle, when there is one, is closed here.
            unsafe {
                let mut token: HANDLE = std::ptr::null_mut();
                if WTSQueryUserToken(session, &mut token) != 0 {
                    CloseHandle(token);
                    return true;
                }
                // ERROR_NO_TOKEN (1008): nobody is logged on in that session any more. Any other
                // error is unknown, which counts as alive.
                io::Error::last_os_error().raw_os_error() != Some(1008)
            }
        }

        fn running_daemons(&mut self) -> Vec<RunningDaemon> {
            let mut found = Vec::new();
            // SAFETY: the snapshot handle is closed on every path; each process handle is either
            // handed to a `WinChild` (which closes it) or closed here.
            unsafe {
                let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
                if snapshot == INVALID_HANDLE_VALUE {
                    return found;
                }
                let mut entry: PROCESSENTRY32W = std::mem::zeroed();
                entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                let mut more = Process32FirstW(snapshot, &mut entry) != 0;
                while more {
                    let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                    if name.eq_ignore_ascii_case("canvas-rgbd.exe") {
                        let pid = entry.th32ProcessID;
                        let process = OpenProcess(
                            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | SYNCHRONIZE,
                            FALSE,
                            pid,
                        );
                        if !process.is_null() {
                            let mut buf = [0u16; 1024];
                            let mut size = buf.len() as u32;
                            let mut session = 0u32;
                            if QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut size) != 0
                                && ProcessIdToSessionId(pid, &mut session) != 0
                            {
                                found.push(RunningDaemon {
                                    image: PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])),
                                    session,
                                    process: Box::new(WinChild { process, pid }),
                                });
                            } else {
                                CloseHandle(process);
                            }
                        }
                    }
                    more = Process32NextW(snapshot, &mut entry) != 0;
                }
                CloseHandle(snapshot);
            }
            found
        }

        fn launch(&mut self, exe: &Path) -> io::Result<Box<dyn ChildProcess>> {
            launch_as_console_user(&format!("\"{}\"", exe.display()), exe.parent(), false)
        }
    }

    /// Starts `command_line` (the exe, quoted, then its arguments) in the active console session with
    /// the logged-on user's own token: `WTSQueryUserToken` then `CreateProcessAsUserW`, non-elevated,
    /// never the service's. The daemon launch uses it, and so do the update steps that must run as
    /// the user (#43). `dir` is the working directory. An error of kind [`io::ErrorKind::NotConnected`]
    /// means no user is logged on in the console session. With `require_env`, a failed
    /// `CreateEnvironmentBlock` is an error and nothing starts (the child would otherwise inherit the
    /// service's own environment, SYSTEM's profile paths); the daemon launch passes `false`.
    pub fn launch_as_console_user(command_line: &str, dir: Option<&Path>, require_env: bool) -> io::Result<Box<dyn ChildProcess>> {
        // SAFETY: each Win32 call gets pointers into buffers that live until the call returns,
        // and every handle is closed on every path.
        unsafe {
            let session = WTSGetActiveConsoleSessionId();
            if session == u32::MAX {
                return Err(io::Error::new(io::ErrorKind::NotConnected, "no active console session"));
            }
            let mut token: HANDLE = std::ptr::null_mut();
            if WTSQueryUserToken(session, &mut token) == 0 {
                let error = io::Error::last_os_error();
                // ERROR_NO_TOKEN (1008): the console session has no logged-on user yet.
                if error.raw_os_error() == Some(1008) {
                    return Err(io::Error::new(io::ErrorKind::NotConnected, error));
                }
                return Err(error);
            }
            let mut env: *mut c_void = std::ptr::null_mut();
            let have_env = CreateEnvironmentBlock(&mut env, token, FALSE) != 0;
            if !have_env && require_env {
                let error = io::Error::last_os_error();
                CloseHandle(token);
                return Err(io::Error::new(error.kind(), format!("cannot build the user's environment block: {error}")));
            }
            let mut command: Vec<u16> = command_line.encode_utf16().chain(Some(0)).collect();
            let mut desktop: Vec<u16> = "winsta0\\default".encode_utf16().chain(Some(0)).collect();
            let dir: Vec<u16> = dir
                .map(|p| p.as_os_str().encode_wide().chain(Some(0)).collect())
                .unwrap_or_else(|| vec![0]);
            let mut startup: STARTUPINFOW = std::mem::zeroed();
            startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
            startup.lpDesktop = desktop.as_mut_ptr();
            let mut info: PROCESS_INFORMATION = std::mem::zeroed();
            // token: the logged-on user's own (WTSQueryUserToken above), non-elevated; never the service's.
            let created = CreateProcessAsUserW(
                token,
                std::ptr::null(),
                command.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                FALSE,
                // The daemon is a console program: without CREATE_NO_WINDOW the owner's desktop
                // gets a terminal window for it, and closing that window ends the daemon.
                CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
                if have_env { env } else { std::ptr::null() },
                dir.as_ptr(),
                &startup,
                &mut info,
            );
            let error = io::Error::last_os_error();
            if have_env {
                DestroyEnvironmentBlock(env);
            }
            CloseHandle(token);
            if created == 0 {
                return Err(error);
            }
            CloseHandle(info.hThread);
            Ok(Box::new(WinChild { process: info.hProcess, pid: info.dwProcessId }))
        }
    }

    struct WinChild {
        process: HANDLE,
        pid: u32,
    }

    // SAFETY: the process handle is only used by the supervising thread that owns this child.
    unsafe impl Send for WinChild {}

    impl ChildProcess for WinChild {
        fn try_wait(&mut self) -> io::Result<Option<i32>> {
            // SAFETY: `process` is a process handle opened by CreateProcessAsUserW and not yet closed.
            unsafe {
                match WaitForSingleObject(self.process, 0) {
                    WAIT_OBJECT_0 => {
                        let mut code: u32 = 0;
                        if GetExitCodeProcess(self.process, &mut code) == 0 {
                            return Err(io::Error::last_os_error());
                        }
                        Ok(Some(code as i32))
                    }
                    0x102 => Ok(None),
                    _ => Err(io::Error::last_os_error()),
                }
            }
        }

        fn kill(&mut self) -> io::Result<()> {
            // SAFETY: as above; exit code 1 is the supervisor's kill code.
            if unsafe { TerminateProcess(self.process, 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        fn id(&self) -> u32 {
            self.pid
        }
    }

    impl Drop for WinChild {
        fn drop(&mut self) {
            // SAFETY: closes the process handle exactly once.
            unsafe { CloseHandle(self.process) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeLauncher, FakeProcesses};

    fn supervisor() -> (DaemonSupervisor, FakeProcesses) {
        let processes = FakeProcesses::new();
        let sup = DaemonSupervisor::new(
            PathBuf::from(r"C:\svc\canvas-rgbd.exe"),
            Box::new(FakeLauncher(processes.clone())),
        );
        (sup, processes)
    }

    #[test]
    fn stopping_the_service_ends_its_daemon_and_never_restarts_it() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        assert!(sup.daemon_pid().is_some());
        sup.stop();
        assert_eq!(sup.daemon_pid(), None);
        assert!(sup.stopped());
        assert_eq!(sup.tick(t0 + Duration::from_secs(10)), DaemonAction::None);
        assert_eq!(processes.spawns().len(), 1);
    }

    #[test]
    fn a_crash_restarts_the_daemon_three_times_then_stops() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        let mut now = t0;
        for expected_stop in [false, false, false, true] {
            processes.exit_last();
            now += Duration::from_secs(10);
            let action = sup.tick(now);
            assert_eq!(sup.stopped(), expected_stop);
            if expected_stop {
                assert_eq!(action, DaemonAction::WriteBlack);
            }
        }
        // First launch plus three restarts; after the limit no fourth restart.
        assert_eq!(processes.spawns().len(), 4);
        // Once stopped, further ticks do nothing.
        assert_eq!(sup.tick(now + Duration::from_secs(10)), DaemonAction::None);
        assert_eq!(processes.spawns().len(), 4);
    }

    #[test]
    fn the_window_forgets_old_crashes() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        let mut now = t0;
        for _ in 0..3 {
            processes.exit_last();
            now += Duration::from_secs(10);
            sup.tick(now);
        }
        // Six minutes later the three old crashes have aged out of the 5 min window.
        now += Duration::from_secs(360);
        processes.exit_last();
        sup.tick(now);
        assert!(!sup.stopped());
        assert_eq!(processes.spawns().len(), 5);
    }

    #[test]
    fn missed_heartbeats_for_five_seconds_are_a_crash() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        sup.on_heartbeat(t0 + Duration::from_secs(1));
        assert_eq!(sup.tick(t0 + Duration::from_secs(5)), DaemonAction::None);
        // 5 s after the last heartbeat, with the process still alive, is a crash.
        sup.tick(t0 + Duration::from_secs(6));
        assert_eq!(processes.spawns().len(), 2);
    }

    #[test]
    fn a_daemon_that_announced_quitting_is_not_restarted() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        sup.on_quitting();
        processes.exit_last();
        assert_eq!(sup.tick(t0 + Duration::from_secs(1)), DaemonAction::None);
        assert_eq!(processes.spawns().len(), 1);
    }

    #[test]
    fn a_failed_launch_counts_against_the_limit() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.fail_spawns(100);
        sup.start(t0);
        assert!(!sup.is_connected(t0));
        let mut now = t0;
        let mut action = DaemonAction::None;
        for _ in 0..4 {
            now += Duration::from_secs(1);
            action = sup.tick(now);
        }
        assert_eq!(action, DaemonAction::WriteBlack);
        assert!(sup.stopped());
    }

    #[test]
    fn no_user_session_yet_is_waiting_not_a_crash_and_the_daemon_starts_at_logon() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_no_session(true);
        let text = logged(|| sup.start(t0));
        assert!(text.contains("waiting for a session"), "{text}");
        // Far more ticks than DAEMON_RESTART_LIMIT: none counts, none stops the supervisor.
        let mut now = t0;
        let mut ticks_text = String::new();
        for _ in 0..(DAEMON_RESTART_LIMIT + 10) {
            now += Duration::from_secs(1);
            ticks_text.push_str(&logged(|| assert_eq!(sup.tick(now), DaemonAction::None)));
        }
        assert!(!sup.stopped(), "waiting for a session never stops restarts");
        assert!(!ticks_text.contains("waiting for a session"), "the wait is logged once: {ticks_text}");
        assert!(processes.spawns().is_empty());
        // A user logs on: the next tick starts the daemon.
        processes.set_no_session(false);
        now += Duration::from_secs(1);
        sup.tick(now);
        assert_eq!(processes.spawns().len(), 1);
        assert!(sup.daemon_pid().is_some());
        // And a real crash afterwards still counts.
        processes.exit_last();
        now += Duration::from_secs(1);
        sup.tick(now);
        assert_eq!(processes.spawns().len(), 2);
    }

    /// Ticks once a second for `secs` seconds from `from`; returns the time reached.
    fn run_ticks(sup: &mut DaemonSupervisor, from: Instant, secs: u64) -> Instant {
        let mut now = from;
        for _ in 0..secs {
            now += Duration::from_secs(1);
            sup.on_heartbeat(now);
            assert_eq!(sup.tick(now), DaemonAction::None);
            assert!(!sup.stopped());
        }
        now
    }

    #[test]
    fn a_console_session_change_moves_the_daemon_once_it_has_settled() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        processes.set_console(2);
        // Inside the settle time nothing moves.
        let now = run_ticks(&mut sup, t0, 3);
        assert_eq!(processes.spawns().len(), 1);
        let text = logged(|| {
            let _ = run_ticks(&mut sup, now, 2);
        });
        assert!(text.contains("the active console session changed"), "{text}");
        assert_eq!(processes.spawns().len(), 2);
        // Moves are not crashes: six more, far past the restart limit.
        let mut now = now + Duration::from_secs(2);
        for session in [1, 2, 1, 2, 1, 2] {
            processes.set_console(session);
            now = run_ticks(&mut sup, now, 5);
        }
        assert_eq!(processes.spawns().len(), 8);
        assert!(!sup.stopped());
    }

    /// "Switch user": the logon screen takes the console (a session with no user). The daemon of
    /// the user who is still logged on keeps running; it moves once someone logs on there.
    #[test]
    fn a_console_on_the_logon_screen_does_not_take_the_daemon_away() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        let pid = sup.daemon_pid().expect("the daemon runs in session 1");
        processes.log_off(2);
        processes.set_console(2);
        let now = run_ticks(&mut sup, t0, 10);
        assert_eq!(sup.daemon_pid(), Some(pid), "the healthy daemon of session 1 is not killed");
        assert_eq!(processes.spawns().len(), 1);
        processes.log_on(2);
        let _ = run_ticks(&mut sup, now, 5);
        assert_eq!(processes.spawns().len(), 2, "a user logged on in session 2: the daemon moves");
        assert!(!sup.stopped());
    }

    /// A console that flips every second, or two other sessions alternating, never settles, so the
    /// daemon is never killed (moves bypass the crash budget; the settle time is their bound).
    #[test]
    fn a_console_that_flips_every_second_never_moves_the_daemon() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        let mut now = t0;
        for i in 0..30 {
            processes.set_console(if i % 2 == 0 { 2 } else { 1 });
            now = run_ticks(&mut sup, now, 1);
        }
        for i in 0..30 {
            processes.set_console(if i % 2 == 0 { 2 } else { 3 });
            now = run_ticks(&mut sup, now, 1);
        }
        assert_eq!(processes.spawns().len(), 1, "no kill and no launch");
        assert!(sup.daemon_pid().is_some());
        // Held on one other session for the settle time, it moves exactly once.
        processes.set_console(3);
        run_ticks(&mut sup, now, 5);
        assert_eq!(processes.spawns().len(), 2);
    }

    /// The console id reads "none" for a moment during a switch: the running daemon stays.
    #[test]
    fn no_console_session_for_a_moment_does_not_kill_a_healthy_daemon() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        processes.clear_console();
        let now = run_ticks(&mut sup, t0, 20);
        assert_eq!(processes.spawns().len(), 1);
        assert!(sup.daemon_pid().is_some());
        // The same session comes back: still the same daemon.
        processes.set_console(1);
        run_ticks(&mut sup, now, 5);
        assert_eq!(processes.spawns().len(), 1);
    }

    /// A logoff leaves the console session id as it was; only the session's user going away shows it.
    #[test]
    fn a_logoff_with_the_same_console_id_is_not_a_crash() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        let mut now = t0;
        // Four logoffs inside the 300 s window would stop restarts if each counted.
        for round in 1..=4 {
            processes.exit_last();
            processes.log_off(1);
            now = run_ticks(&mut sup, now, 2);
            assert_eq!(processes.spawns().len(), round, "waiting for a user, nothing launched");
            processes.log_on(1);
            now = run_ticks(&mut sup, now, 2);
            assert_eq!(processes.spawns().len(), round + 1);
        }
        // A daemon still running when its user logs off is replaced too (it is told to end first).
        processes.log_off(1);
        now = run_ticks(&mut sup, now, 2);
        assert!(sup.daemon_pid().is_none());
        processes.log_on(1);
        run_ticks(&mut sup, now, 2);
        assert!(sup.daemon_pid().is_some());
        assert!(!sup.stopped());
    }

    /// The old daemon must be gone before the new one starts, or the new one loses port 9430 and
    /// counts as a crash.
    #[test]
    fn the_old_daemon_has_exited_before_the_new_one_launches() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        processes.set_slow_kill(3);
        sup.start(t0);
        processes.set_console(2);
        run_ticks(&mut sup, t0, 6);
        assert_eq!(processes.spawns().len(), 2);
        assert_eq!(processes.overlaps(), 0, "a launch started while the old daemon still ran");
    }

    #[test]
    fn a_fresh_heartbeat_means_connected() {
        let (mut sup, _processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        assert!(sup.is_connected(t0 + Duration::from_secs(2)));
        assert!(!sup.is_connected(t0 + Duration::from_secs(6)));
    }

    #[test]
    fn the_service_ends_a_stale_daemon_of_its_own_install_before_launching() {
        let (mut sup, processes) = supervisor();
        processes.set_console(1);
        // Same install (different case), console session: ended. Another path: left alone.
        // Same path in another session: left alone.
        processes.add_running(9001, r"c:\SVC\canvas-rgbd.exe", 1);
        processes.add_running(9002, r"C:\other\canvas-rgbd.exe", 1);
        processes.add_running(9003, r"C:\svc\canvas-rgbd.exe", 2);
        sup.start(Instant::now());
        assert!(processes.running_ended(9001));
        assert!(!processes.running_ended(9002));
        assert!(!processes.running_ended(9003));
        // The service's own daemon still launched.
        assert_eq!(processes.spawns().len(), 1);
        assert!(sup.daemon_pid().is_some());
    }

    #[test]
    fn with_no_console_session_no_stale_daemon_is_touched() {
        let (mut sup, processes) = supervisor();
        processes.clear_console();
        processes.add_running(9001, r"C:\svc\canvas-rgbd.exe", 1);
        sup.start(Instant::now());
        assert!(!processes.running_ended(9001));
    }

    /// Runs `run` under a subscriber that collects its lines (the shared test helper).
    fn logged(run: impl FnOnce()) -> String {
        crate::test_logs::captured(run).1
    }

    /// Whether `text` has a line that announces the end of daemon `pid` for `reason`.
    fn announces_kill(text: &str, pid: u32, reason: &str) -> bool {
        let announcement = format!("terminating the daemon (pid {pid}): {reason}");
        text.lines().any(|line| line.contains(&announcement))
    }

    /// Every place the service ends the daemon says why, with the pid, in the line before the kill.
    ///
    /// Each path asserts the log line itself: a fake's kill record alone would pass with the line
    /// removed. The reason is matched as the full announcement for that pid, so the crash line
    /// that follows a missed heartbeat ("daemon crashed", reason "missed heartbeats for 5 s")
    /// cannot satisfy it by accident (class `cause:timing-test`, #45 and #55).
    #[test]
    fn every_daemon_kill_is_preceded_by_a_line_naming_the_reason() {
        // 1. Missed heartbeats.
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        let pid = sup.daemon_pid().expect("a daemon is running");
        let text = logged(|| {
            sup.tick(t0 + Duration::from_secs(6));
        });
        assert!(announces_kill(&text, pid, "it missed heartbeats for 5 s"), "{text}");
        let announced = text.find("terminating the daemon").expect("announced");
        let crashed = text.find("daemon crashed").expect("the crash is logged after the kill");
        assert!(announced < crashed, "the reason comes before the crash line: {text}");
        assert_eq!(processes.spawns().len(), 2);

        // 2. The service stops.
        let (mut sup, _processes) = supervisor();
        sup.start(Instant::now());
        let pid = sup.daemon_pid().expect("a daemon is running");
        let text = logged(|| sup.stop());
        assert!(announces_kill(&text, pid, "the service is stopping"), "{text}");

        // 3. A stale daemon from a previous service instance.
        let (mut sup, processes) = supervisor();
        processes.set_console(1);
        processes.add_running(9001, r"C:\svc\canvas-rgbd.exe", 1);
        let text = logged(|| sup.start(Instant::now()));
        assert!(announces_kill(&text, 9001, "a stale daemon from a previous service instance"), "{text}");
        assert!(processes.running_ended(9001));

        // 4. The active console session changed.
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        let pid = sup.daemon_pid().expect("a daemon is running");
        processes.set_console(2);
        let text = logged(|| {
            let _ = run_ticks(&mut sup, t0, 6);
        });
        assert!(announces_kill(&text, pid, "the active console session changed"), "{text}");
        assert_eq!(processes.spawns().len(), 2);
    }

    #[test]
    fn a_daemon_exit_reason_is_hex_with_the_status_name() {
        assert_eq!(exited_reason(-1_073_741_502), "exited with code 0xC0000142 STATUS_DLL_INIT_FAILED");
        assert_eq!(exited_reason(1), "exited with code 0x00000001");
    }

    /// A kill that failed did not stop the daemon, so the service must not record it as stopped:
    /// the OpenRGB line would then say the daemon was terminated when it was not (#81).
    #[test]
    fn a_daemon_the_service_could_not_end_is_not_recorded_as_stopped() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        let pid = sup.daemon_pid().expect("a daemon is running");
        processes.fail_kills(1);

        sup.stop();

        assert_eq!(sup.before_openrgb_stop(), DaemonBeforeStop::NotEnded { pid });
        let text = sup.before_openrgb_stop().describe(t0);
        assert!(!text.contains("terminated"), "the daemon was not terminated: {text}");
        assert!(!text.contains("ms earlier"), "there is no gap to report: {text}");
    }

    /// A kill that worked is recorded, and the OpenRGB line says how long before. The kill is a
    /// request, not the exit: the line says the daemon was told to stop (#86).
    #[test]
    fn a_daemon_the_service_ended_is_recorded_as_stopped() {
        let (mut sup, _processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        let pid = sup.daemon_pid().expect("a daemon is running");

        sup.stop();

        assert!(matches!(sup.before_openrgb_stop(), DaemonBeforeStop::Stopped { pid: p, .. } if p == pid));
        let text = sup.before_openrgb_stop().describe(t0);
        assert!(text.contains("was told to stop"), "{text}");
        assert!(!text.contains("was terminated"), "the exit was not waited on: {text}");
    }

    /// A daemon that exited but has not been reaped is not running (#86): the line must not say it
    /// is still running and was not told.
    #[test]
    fn a_daemon_that_had_already_exited_is_not_logged_as_running() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        let pid = sup.daemon_pid().expect("a daemon is running");
        // The daemon is gone, but the service has not asked about it yet.
        processes.exit_last();

        match sup.before_openrgb_stop() {
            DaemonBeforeStop::Exited { pid: exited, .. } => assert_eq!(exited, pid),
            other => panic!("the daemon had exited: {other:?}"),
        }
        let text = sup.before_openrgb_stop().describe(t0);
        let running = text.contains("is still running");
        assert!(!running, "the daemon had exited: {text}");
        assert!(text.contains("had already exited"), "{text}");
    }

    /// The record of a daemon the service could not end belongs to that daemon (#87): a new one
    /// replaces it, or the OpenRGB line names a pid that is long gone.
    #[test]
    fn a_new_daemon_clears_the_record_of_the_one_that_could_not_be_ended() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        let pid = sup.daemon_pid().expect("a daemon is running");
        processes.fail_kills(1);

        sup.stop();

        assert_eq!(sup.unended_daemon, Some(pid), "the record is there");

        // A new daemon replaces the one that could not be ended.
        sup.launch(t0);

        assert_eq!(sup.unended_daemon, None, "the record is gone");
        let before = sup.before_openrgb_stop();
        assert_ne!(before, DaemonBeforeStop::NotEnded { pid });
    }

    /// The exit codes of the shutdown of 2026-11-01 (#6), as the service saw them.
    const DBG_TERMINATE_PROCESS: i32 = 0x4001_0004u32 as i32;
    const STATUS_DLL_INIT_FAILED_LOGOFF: i32 = 0xC000_026Bu32 as i32;
    const STATUS_ACCESS_VIOLATION: i32 = 0xC000_0005u32 as i32;

    /// The shutdown of 2026-11-01: the daemon exited with `0x40010004`, the service restarted
    /// it, and the next two exits were `0xC000026B`. The third hit the crash limit, wrote black
    /// and stopped the daemon (#6). The session was alive for all of them (the token query kept
    /// working), so none of them is a crash.
    #[test]
    fn the_shutdown_sequence_of_2026_11_01_never_reaches_the_crash_limit() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        let mut now = t0;
        let mut text = String::new();
        for code in [
            DBG_TERMINATE_PROCESS,
            STATUS_DLL_INIT_FAILED_LOGOFF,
            STATUS_DLL_INIT_FAILED_LOGOFF,
        ] {
            processes.exit_last_with(code);
            now += Duration::from_secs(1);
            text.push_str(&logged(|| assert_eq!(sup.tick(now), DaemonAction::None)));
            // The relaunch is held: no daemon is started while the session is on its way out.
            for _ in 0..5 {
                now += Duration::from_secs(1);
                text.push_str(&logged(|| assert_eq!(sup.tick(now), DaemonAction::None)));
            }
            assert!(!sup.stopped(), "the crash limit was reached: {text}");
            // The hold ends when its 60 s are up: the daemon starts again.
            now += Duration::from_secs(SESSION_END_HOLD.as_secs());
            text.push_str(&logged(|| assert_eq!(sup.tick(now), DaemonAction::None)));
            assert!(!sup.stopped(), "the crash limit was reached: {text}");
        }
        assert_eq!(
            processes.spawns().len(),
            4,
            "one start and one after each hold"
        );
        assert!(
            !text.contains("writing black"),
            "nothing was written black: {text}"
        );
        assert!(
            !text.contains("daemon crashed"),
            "no exit was counted as a crash: {text}"
        );
    }

    /// The session-end budget is three per console session. The fourth exit counts as a crash
    /// again, so a daemon that keeps ending with its session still trips the crash limit.
    #[test]
    fn a_fourth_session_end_exit_in_one_session_counts_as_a_crash() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        let mut now = t0;
        // Three session ends fit in the budget.
        for round in 1..=SESSION_END_LIMIT {
            processes.exit_last_with(STATUS_DLL_INIT_FAILED_LOGOFF);
            now += Duration::from_secs(1);
            assert_eq!(sup.tick(now), DaemonAction::None);
            now += Duration::from_secs(SESSION_END_HOLD.as_secs());
            assert_eq!(sup.tick(now), DaemonAction::None);
            assert_eq!(
                processes.spawns().len(),
                round as usize + 1,
                "the daemon started again after the hold"
            );
            assert!(!sup.stopped());
        }
        // The fourth is a crash: it takes the crash budget, and the daemon starts again at once.
        processes.exit_last_with(STATUS_DLL_INIT_FAILED_LOGOFF);
        now += Duration::from_secs(1);
        let text = logged(|| assert_eq!(sup.tick(now), DaemonAction::None));
        assert!(text.contains("counting it as a crash"), "{text}");
        // Three more, and the crash limit trips: a session end that keeps happening is a crash.
        let mut action = DaemonAction::None;
        for _ in 0..3 {
            processes.exit_last_with(STATUS_DLL_INIT_FAILED_LOGOFF);
            now += Duration::from_secs(1);
            action = sup.tick(now);
        }
        assert_eq!(action, DaemonAction::WriteBlack);
        assert!(sup.stopped());
    }

    /// A real crash still trips the limit: the session-end codes do not make the supervisor
    /// lenient with a daemon that crashes (#6).
    #[test]
    fn four_access_violations_still_trip_the_crash_limit() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        let mut now = t0;
        let mut action = DaemonAction::None;
        for _ in 0..4 {
            processes.exit_last_with(STATUS_ACCESS_VIOLATION);
            now += Duration::from_secs(1);
            action = sup.tick(now);
        }
        assert_eq!(action, DaemonAction::WriteBlack);
        assert!(sup.stopped());
        assert_eq!(processes.spawns().len(), 4);
    }

    /// A crash stop does not strand the user until the service restarts: the next console
    /// session (a logon) clears it (#6).
    #[test]
    fn a_logon_after_a_crash_stop_starts_the_daemon_again() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        let mut now = t0;
        // Four crashes: the limit trips, and the daemon stays off.
        for _ in 0..4 {
            processes.exit_last_with(STATUS_ACCESS_VIOLATION);
            now += Duration::from_secs(1);
            sup.tick(now);
        }
        assert!(sup.stopped());
        assert_eq!(processes.spawns().len(), 4);
        // The user logs off: no daemon starts while there is no session.
        processes.log_off(1);
        now += Duration::from_secs(1);
        assert_eq!(sup.tick(now), DaemonAction::None);
        assert_eq!(processes.spawns().len(), 4);
        // The user logs on again: the daemon starts.
        processes.log_on(1);
        now += Duration::from_secs(1);
        assert_eq!(sup.tick(now), DaemonAction::None);
        assert!(!sup.stopped(), "the crash stop did not end at the logon");
        assert_eq!(processes.spawns().len(), 5);
        assert!(sup.daemon_pid().is_some());
    }

    /// The hold ends when its 60 s are up, even with the session still alive (#6).
    #[test]
    fn the_hold_after_a_session_end_ends_after_sixty_seconds() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        processes.exit_last_with(STATUS_DLL_INIT_FAILED_LOGOFF);
        let exited = t0 + Duration::from_secs(1);
        assert_eq!(sup.tick(exited), DaemonAction::None);
        // While the hold is on, no daemon runs and nothing is written black.
        let hold_end = exited + SESSION_END_HOLD;
        let mut now = exited;
        while now + Duration::from_secs(1) < hold_end {
            now += Duration::from_secs(1);
            assert_eq!(sup.tick(now), DaemonAction::None);
            assert_eq!(processes.spawns().len(), 1, "the relaunch is held");
            assert_eq!(sup.daemon_pid(), None);
        }
        // 60 s after the exit the hold is over: the daemon starts again.
        assert_eq!(sup.tick(hold_end), DaemonAction::None);
        assert_eq!(
            processes.spawns().len(),
            2,
            "the hold ended and the daemon started"
        );
        assert!(sup.daemon_pid().is_some());
    }

    /// The line that says the daemon ended with its session names the code (#6).
    #[test]
    fn the_session_end_line_names_the_code() {
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        processes.set_console(1);
        sup.start(t0);
        processes.exit_last_with(STATUS_DLL_INIT_FAILED_LOGOFF);
        let text = logged(|| {
            sup.tick(t0 + Duration::from_secs(1));
        });
        assert!(
            text.contains("the daemon ended with its session (0xC000026B STATUS_DLL_INIT_FAILED_LOGOFF); not a crash"),
            "{text}"
        );

        let (mut sup, processes) = supervisor();
        processes.set_console(1);
        sup.start(t0);
        processes.exit_last_with(DBG_TERMINATE_PROCESS);
        let text = logged(|| {
            sup.tick(t0 + Duration::from_secs(1));
        });
        assert!(
            text.contains(
                "the daemon ended with its session (0x40010004 DBG_TERMINATE_PROCESS); not a crash"
            ),
            "{text}"
        );
    }
}
