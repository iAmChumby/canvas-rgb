//! Daemon supervision (grill G3). The daemon runs in the user's session, so the service starts it
//! there through [`SessionLauncher`]. A missed heartbeat for 5 s or an exit counts as a crash.
//! The service restarts a crashed daemon at most `DAEMON_RESTART_LIMIT` times in
//! `DAEMON_RESTART_WINDOW_S`, then writes black through OpenRGB and stops restarting it.
//! A daemon that sent `DaemonQuitting` is not restarted.

use crate::process::ChildProcess;
use canvas_proto::consts::{DAEMON_RESTART_LIMIT, DAEMON_RESTART_WINDOW_S};
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A missed-heartbeat run this long counts as a crash.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

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

pub struct DaemonSupervisor {
    exe: PathBuf,
    launcher: Box<dyn SessionLauncher>,
    child: Option<Box<dyn ChildProcess>>,
    started: bool,
    last_heartbeat: Option<Instant>,
    restarts: VecDeque<Instant>,
    quitting: bool,
    stopped: bool,
    /// The last launch found no user session. Not a crash: the next ticks launch again until a user
    /// is logged on (#6).
    waiting_for_session: bool,
    /// The console session the running daemon was launched into (`None` when the launcher cannot say).
    child_session: Option<u32>,
    /// A different console session seen while the daemon runs, and since when (see `SESSION_SETTLE`).
    pending_move: Option<(u32, Instant)>,
}

impl DaemonSupervisor {
    pub fn new(exe: PathBuf, launcher: Box<dyn SessionLauncher>) -> Self {
        Self {
            exe,
            launcher,
            child: None,
            started: false,
            last_heartbeat: None,
            restarts: VecDeque::new(),
            quitting: false,
            stopped: false,
            waiting_for_session: false,
            child_session: None,
            pending_move: None,
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
            tracing::warn!(pid, image = %stale.image.display(), "terminating the daemon (pid {pid}): a stale daemon from a previous service instance");
            if let Err(e) = stale.process.kill() {
                tracing::warn!(pid, error = %e, "could not end the stale daemon");
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
    /// another console session only after it has held for `SESSION_SETTLE`.
    fn session_changed(&mut self, now: Instant) -> bool {
        let Some(was) = self.child_session else {
            self.pending_move = None;
            return false;
        };
        if !self.launcher.session_alive(was) {
            return true;
        }
        match self.launcher.console_session() {
            Some(c) if c != was => match self.pending_move {
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
                self.child = Some(child);
                self.child_session = session;
                self.waiting_for_session = false;
                // The first heartbeat gets the same 5 s the rest do.
                self.last_heartbeat = Some(now);
                tracing::info!(exe = %self.exe.display(), "daemon started in the user session");
            }
            Err(e) if e.kind() == io::ErrorKind::NotConnected => {
                self.child = None;
                // Logged once; the ticks that follow retry without a line each.
                if !self.waiting_for_session {
                    tracing::info!(error = %e, "no user is logged on yet; waiting for a session to start the daemon");
                }
                self.waiting_for_session = true;
            }
            Err(e) => {
                self.child = None;
                self.waiting_for_session = false;
                tracing::error!(error = %e, "could not start the daemon");
            }
        }
    }

    pub fn on_heartbeat(&mut self, now: Instant) {
        if !self.stopped {
            self.last_heartbeat = Some(now);
        }
    }

    /// The service is stopping: the daemon it launched ends too and is not restarted.
    /// Without this a service restart leaves the old daemon holding the API port, so the new
    /// service's daemon cannot bind and crash-loops (lighting session 2).
    pub fn stop(&mut self) {
        self.stopped = true;
        if let Some(mut child) = self.child.take() {
            tracing::info!(pid = child.id(), "terminating the daemon (pid {}): the service is stopping", child.id());
            if let Err(e) = child.kill() {
                tracing::warn!(error = %e, "could not end the daemon");
            }
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
                .last_heartbeat
                .is_some_and(|hb| now.saturating_duration_since(hb) < HEARTBEAT_TIMEOUT)
    }

    /// Whether the restart limit has been reached and restarts have stopped.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// One supervision pass per second.
    pub fn tick(&mut self, now: Instant) -> DaemonAction {
        if !self.started || self.stopped || self.quitting {
            return DaemonAction::None;
        }
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(code)) => {
                    self.child = None;
                    let was = self.child_session;
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
                    tracing::info!(pid = child.id(), "terminating the daemon (pid {}): the active console session changed", child.id());
                    if let Err(e) = child.kill() {
                        tracing::warn!(error = %e, "could not end the daemon of the old session");
                    }
                    let pid = child.id();
                    wait_for_exit(child.as_mut(), pid);
                }
                self.launch(now);
                return DaemonAction::None;
            }
            let stale = self
                .last_heartbeat
                .is_some_and(|hb| now.saturating_duration_since(hb) >= HEARTBEAT_TIMEOUT);
            if stale {
                if let Some(mut child) = self.child.take() {
                    tracing::warn!(pid = child.id(), "terminating the daemon (pid {}): it missed heartbeats for 5 s", child.id());
                    if let Err(e) = child.kill() {
                        tracing::warn!(error = %e, "could not terminate the daemon that missed heartbeats");
                    }
                    let pid = child.id();
                    wait_for_exit(child.as_mut(), pid);
                }
                return self.crashed(now, "missed heartbeats for 5 s");
            }
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
            self.stopped = true;
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
pub use windows_launcher::WtsLauncher;

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
                let mut command: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain(Some(0)).collect();
                let mut desktop: Vec<u16> = "winsta0\\default".encode_utf16().chain(Some(0)).collect();
                let dir: Vec<u16> = exe
                    .parent()
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

    /// Runs `run` under a subscriber that collects its lines.
    fn logged(run: impl FnOnce()) -> String {
        #[derive(Clone, Default)]
        struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let buf = Buf::default();
        let sink = buf.clone();
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || sink.clone()).finish();
        tracing::subscriber::with_default(subscriber, run);
        let text = String::from_utf8_lossy(&buf.0.lock().unwrap()).into_owned();
        text
    }

    /// Every place the service ends the daemon says why, with the pid, in the line before the kill.
    #[test]
    fn every_daemon_kill_is_preceded_by_a_line_naming_the_reason() {
        // 1. Missed heartbeats.
        let (mut sup, processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        let text = logged(|| {
            sup.tick(t0 + Duration::from_secs(6));
        });
        assert!(text.contains("terminating the daemon (pid") && text.contains("missed heartbeats"), "{text}");
        assert_eq!(processes.spawns().len(), 2);

        // 2. The service stops.
        let (mut sup, _processes) = supervisor();
        sup.start(Instant::now());
        let text = logged(|| sup.stop());
        assert!(text.contains("terminating the daemon (pid") && text.contains("the service is stopping"), "{text}");

        // 3. A stale daemon from a previous service instance.
        let (mut sup, processes) = supervisor();
        processes.set_console(1);
        processes.add_running(9001, r"C:\svc\canvas-rgbd.exe", 1);
        let text = logged(|| sup.start(Instant::now()));
        assert!(text.contains("terminating the daemon (pid 9001)") && text.contains("stale daemon"), "{text}");
        assert!(processes.running_ended(9001));
    }

    #[test]
    fn a_daemon_exit_reason_is_hex_with_the_status_name() {
        assert_eq!(exited_reason(-1_073_741_502), "exited with code 0xC0000142 STATUS_DLL_INIT_FAILED");
        assert_eq!(exited_reason(1), "exited with code 0x00000001");
    }
}
