//! Daemon supervision (grill G3). The daemon runs in Luke's session, so the service starts it
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
    fn launch(&mut self, exe: &Path) -> io::Result<Box<dyn ChildProcess>>;
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
        }
    }

    /// Starts the daemon for the first time (service start).
    pub fn start(&mut self, now: Instant) {
        self.started = true;
        self.launch(now);
    }

    fn launch(&mut self, now: Instant) {
        match self.launcher.launch(&self.exe) {
            Ok(child) => {
                self.child = Some(child);
                // The first heartbeat gets the same 5 s the rest do.
                self.last_heartbeat = Some(now);
                tracing::info!(exe = %self.exe.display(), "daemon started in the user session");
            }
            Err(e) => {
                self.child = None;
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
            tracing::info!(pid = child.id(), "the service is stopping; ending the daemon it launched");
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
                    return self.crashed(now, &format!("exited with code {code}"));
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "could not query the daemon process"),
            }
            let stale = self
                .last_heartbeat
                .is_some_and(|hb| now.saturating_duration_since(hb) >= HEARTBEAT_TIMEOUT);
            if stale {
                if let Some(mut child) = self.child.take() {
                    let _ = child.kill();
                }
                return self.crashed(now, "missed heartbeats for 5 s");
            }
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
    use super::SessionLauncher;
    use crate::process::ChildProcess;
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
    use windows_sys::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
    use windows_sys::Win32::System::Threading::{
        CreateProcessAsUserW, GetExitCodeProcess, TerminateProcess, WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT,
        PROCESS_INFORMATION, STARTUPINFOW,
    };

    /// Starts the daemon in the active console session with the user's token
    /// (`WTSQueryUserToken` then `CreateProcessAsUserW`). Runs only as the service.
    pub struct WtsLauncher;

    impl SessionLauncher for WtsLauncher {
        fn launch(&mut self, exe: &Path) -> io::Result<Box<dyn ChildProcess>> {
            // SAFETY: each Win32 call gets pointers into buffers that live until the call returns,
            // and every handle is closed on every path.
            unsafe {
                let session = WTSGetActiveConsoleSessionId();
                if session == u32::MAX {
                    return Err(io::Error::other("no active console session"));
                }
                let mut token: HANDLE = std::ptr::null_mut();
                if WTSQueryUserToken(session, &mut token) == 0 {
                    return Err(io::Error::last_os_error());
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
                let created = CreateProcessAsUserW(
                    token,
                    std::ptr::null(),
                    command.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    FALSE,
                    CREATE_UNICODE_ENVIRONMENT,
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
    fn a_fresh_heartbeat_means_connected() {
        let (mut sup, _processes) = supervisor();
        let t0 = Instant::now();
        sup.start(t0);
        assert!(sup.is_connected(t0 + Duration::from_secs(2)));
        assert!(!sup.is_connected(t0 + Duration::from_secs(6)));
    }
}
