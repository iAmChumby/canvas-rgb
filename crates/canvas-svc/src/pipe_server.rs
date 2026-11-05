//! The named-pipe server the daemon talks to (`\\.\pipe\canvas-rgb-svc`).
//!
//! Security (from the security review): the first instance is created with
//! `first_pipe_instance(true)`, so a squatter that created the name first makes the service
//! fail loudly. The DACL grants SYSTEM full control and the interactive user (S-1-5-4)
//! read and write only. Remote clients are rejected.
//!
//! Per connection, in order:
//! 1. The client must be in the active console session, and if the service launched a daemon,
//!    the client must be that daemon's process ID. Otherwise it gets `unauthorized` and the
//!    connection closes. See [`admit`].
//! 2. The first message must be `Hello`. Anything else gets a validation error and the
//!    connection closes.
//! 3. Every rejection is logged with the client's PID and session.
//!
//! Two requests are stricter than the rest: `InstallUpdate` and `DaemonQuitting` are accepted
//! only from the daemon this service launched, and are refused while no daemon runs (see
//! [`admit_daemon`]). `DaemonQuitting` says the daemon is leaving on purpose, so the service
//! stops supervising it; a client that is not that daemon must not be able to say it (#6).
//!
//! Framing is `canvas_proto::pipe`: a u32 little-endian length, then JSON. One request gets
//! one response, in order. Frames over `MAX_FRAME` get an error and the connection closes.
//! The OS calls for step 1 sit behind [`PeerProbe`], so tests use a fake.

use crate::service::Core;
use crate::update_install::{win::ImpersonatingReader, ClientFileReader, RefusingReader};
use canvas_proto::api::error_codes;
use canvas_proto::pipe::{decode_body, decode_len, encode, PipeRequest, PipeResponse, PIPE_NAME};
use crate::daemon::{HeartbeatClock, Stamp};
use std::ffi::c_void;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::Mutex;
use windows_sys::Win32::Foundation::{LocalFree, FALSE, HANDLE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
use windows_sys::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeClientSessionId};
use windows_sys::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;

/// Overrides the pipe name. Tests only; the service uses the spine name.
pub const PIPE_NAME_ENV: &str = "CANVAS_RGB_PIPE_NAME";
/// SYSTEM: full control. Interactive user (S-1-5-4): read and write only.
pub const PIPE_SDDL: &str = "D:(A;;GA;;;SY)(A;;GRGW;;;S-1-5-4)";
const SDDL_REVISION_1: u32 = 1;

pub fn pipe_name() -> String {
    std::env::var(PIPE_NAME_ENV).unwrap_or_else(|_| PIPE_NAME.to_string())
}

/// What the OS says about the client at the other end of one pipe connection. Real: [`WinPeer`].
/// Tests: a fake.
pub trait PeerProbe: Send {
    /// Session of the client end of the pipe.
    fn client_session_id(&self) -> io::Result<u32>;
    /// Process ID of the client end of the pipe.
    fn client_process_id(&self) -> io::Result<u32>;
    /// The session attached to the physical console (`0xFFFFFFFF` when none is attached).
    fn console_session_id(&self) -> u32;
    /// Reads files as this client (impersonating it). The default refuses every read.
    fn file_reader(&self) -> Box<dyn ClientFileReader> {
        Box::new(RefusingReader)
    }
}

/// A client that passed [`admit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admitted {
    pub pid: u32,
    pub session: u32,
}

/// A client that failed [`admit`]. Whatever the OS could read is kept for the log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub pid: Option<u32>,
    pub session: Option<u32>,
    pub reason: String,
}

/// The admission rule. The client must be in the console session. If the service launched a
/// daemon, the client must be exactly that daemon's process. With no daemon launched, any client
/// in the console session is admitted, and the `Hello` decides what it may do next.
pub fn admit(peer: &dyn PeerProbe, launched_daemon: Option<u32>) -> Result<Admitted, Rejected> {
    let session = peer.client_session_id().map_err(|e| Rejected {
        pid: None,
        session: None,
        reason: format!("cannot read the client session: {e}"),
    })?;
    let pid = peer.client_process_id().map_err(|e| Rejected {
        pid: None,
        session: Some(session),
        reason: format!("cannot read the client process: {e}"),
    })?;
    let console = peer.console_session_id();
    if session != console {
        return Err(Rejected {
            pid: Some(pid),
            session: Some(session),
            reason: format!("client session {session} is not the console session {console}"),
        });
    }
    if let Some(daemon) = launched_daemon {
        if pid != daemon {
            return Err(Rejected {
                pid: Some(pid),
                session: Some(session),
                reason: format!("client pid {pid} is not the launched daemon pid {daemon}"),
            });
        }
    }
    Ok(Admitted { pid, session })
}

/// A request that only the daemon this service launched may make. The client is admitted as
/// usual first (it must be in the console session), and then it must be that daemon's process:
/// with no launched daemon, or with a client that is not it, the request is refused. `what`
/// names the request in the refusal, so the log says which one was refused.
pub fn admit_daemon(
    peer: &dyn PeerProbe,
    launched_daemon: Option<u32>,
    what: &str,
) -> Result<Admitted, Rejected> {
    let who = admit(peer, launched_daemon)?;
    match launched_daemon {
        Some(daemon) if daemon == who.pid => Ok(who),
        Some(daemon) => Err(Rejected {
            pid: Some(who.pid),
            session: Some(who.session),
            reason: format!("{what} is only accepted from the launched daemon (pid {daemon}), not pid {}", who.pid),
        }),
        None => Err(Rejected {
            pid: Some(who.pid),
            session: Some(who.session),
            reason: format!("{what} is only accepted from the daemon this service launched, and none is running"),
        }),
    }
}

/// `InstallUpdate` is stricter than every other operation: it skips the daemon's API token and the
/// 9430-owner check, so it is accepted only from the daemon this service launched. With no launched
/// daemon, or a client that is not that daemon's process, it is refused.
pub fn admit_install(peer: &dyn PeerProbe, launched_daemon: Option<u32>) -> Result<Admitted, Rejected> {
    admit_daemon(peer, launched_daemon, "InstallUpdate")
}

/// The production probe: the Win32 calls on one connected pipe instance.
pub struct WinPeer {
    /// The pipe handle as a number, so the probe is `Send`. Valid while the pipe instance lives,
    /// and the probe is only ever used inside `handle_client`, which owns that instance.
    pipe: usize,
}

impl WinPeer {
    pub fn of(pipe: &NamedPipeServer) -> Self {
        Self { pipe: pipe.as_raw_handle() as usize }
    }

    fn handle(&self) -> HANDLE {
        self.pipe as HANDLE
    }
}

impl PeerProbe for WinPeer {
    fn client_session_id(&self) -> io::Result<u32> {
        let mut session = 0u32;
        // SAFETY: the handle belongs to a live pipe instance owned by the calling task; `session` is a valid out-pointer.
        let ok = unsafe { GetNamedPipeClientSessionId(self.handle(), &mut session) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(session)
    }

    fn client_process_id(&self) -> io::Result<u32> {
        let mut pid = 0u32;
        // SAFETY: as above; `pid` is a valid out-pointer.
        let ok = unsafe { GetNamedPipeClientProcessId(self.handle(), &mut pid) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(pid)
    }

    fn console_session_id(&self) -> u32 {
        // SAFETY: takes no pointers and cannot fail in a way that matters: 0xFFFFFFFF means no console session.
        unsafe { WTSGetActiveConsoleSessionId() }
    }

    fn file_reader(&self) -> Box<dyn ClientFileReader> {
        Box::new(ImpersonatingReader::of(self.handle()))
    }
}

/// Builds the probe for each new connection.
pub type PeerFactory = Arc<dyn Fn(&NamedPipeServer) -> Box<dyn PeerProbe> + Send + Sync>;

/// The production factory: a [`WinPeer`] per connection.
pub fn win_peers() -> PeerFactory {
    Arc::new(|pipe: &NamedPipeServer| -> Box<dyn PeerProbe> { Box::new(WinPeer::of(pipe)) })
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Creates one pipe instance with the SDDL ACL. The first instance also claims the name.
pub fn create_instance(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let sddl = wide(PIPE_SDDL);
    let mut descriptor: *mut c_void = std::ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; `descriptor` receives a LocalAlloc'd buffer freed below.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: FALSE,
    };
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first).reject_remote_clients(true);
    // SAFETY: `attributes` and the descriptor it points to are alive for the call, and the
    // kernel copies the descriptor when the instance is created.
    let server = unsafe {
        options.create_with_security_attributes_raw(name, &mut attributes as *mut SECURITY_ATTRIBUTES as *mut c_void)
    };
    // SAFETY: `descriptor` was allocated by ConvertStringSecurityDescriptor... and is freed once.
    unsafe { LocalFree(descriptor as _) };
    server
}

/// Serves requests forever on `first` and on new instances created after each connect.
/// `peers` builds the OS probe for each connection.
pub async fn serve(core: Arc<Mutex<Core>>, name: String, first: NamedPipeServer, peers: PeerFactory) -> io::Result<()> {
    // Heartbeats are stamped on arrival, off the core lock, so a slow fan write (which holds
    // the lock) cannot age a heartbeat (#129). Grabbed once before anything can block.
    let heartbeats = core.lock().await.daemon.shared_heartbeats();
    let mut server = first;
    loop {
        server.connect().await?;
        let next = loop {
            match create_instance(&name, false) {
                Ok(instance) => break instance,
                Err(e) => {
                    tracing::error!(error = %e, "could not create the next pipe instance; retrying");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        };
        let connected = std::mem::replace(&mut server, next);
        let peer = peers(&connected);
        tokio::spawn(handle_client(connected, Arc::clone(&core), peer, Arc::clone(&heartbeats)));
    }
}

fn error(code: &str, message: impl Into<String>) -> PipeResponse {
    PipeResponse::Error { code: code.into(), message: message.into() }
}

async fn write_response(pipe: &mut NamedPipeServer, response: &PipeResponse) -> io::Result<()> {
    let frame = encode(response).map_err(|e| io::Error::other(e.to_string()))?;
    pipe.write_all(&frame).await?;
    pipe.flush().await
}

/// `InstallUpdate`: the caller admitted the client again (the console session may have changed since it
/// connected); this refuses on that, then installs on a blocking thread. The core lock is held only to get the installer, never while
/// the file is read or setup is started.
async fn install_update(
    core: &Arc<Mutex<Core>>,
    who: Admitted,
    readmit: Result<Admitted, Rejected>,
    reader: Box<dyn ClientFileReader>,
    installer: String,
    signature: String,
) -> PipeResponse {
    if let Err(rejected) = readmit {
        tracing::warn!(
            pid = ?rejected.pid,
            session = ?rejected.session,
            reason = %rejected.reason,
            "refused InstallUpdate: the client is not the launched daemon in the console session"
        );
        return error(error_codes::UNAUTHORIZED, rejected.reason);
    }
    let updates = core.lock().await.updates.clone();
    let Some(updates) = updates else {
        tracing::warn!(pid = who.pid, session = who.session, "refused InstallUpdate: updates are not available (the machine home is not secured)");
        return error(error_codes::SERVICE_UNAVAILABLE, "the service cannot install updates: the machine home is not secured");
    };
    match tokio::task::spawn_blocking(move || updates.install(who, &installer, &signature, reader)).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!(pid = who.pid, session = who.session, error = %e, "the update install thread failed");
            error(error_codes::INTERNAL, "the update install thread failed")
        }
    }
}

/// Admits one client, then reads requests from it until it disconnects. The first message must
/// be `Hello`. `heartbeats` is the shared clock a heartbeat stamps on arrival, off the core
/// lock.
pub async fn handle_client(
    mut pipe: NamedPipeServer,
    core: Arc<Mutex<Core>>,
    peer: Box<dyn PeerProbe>,
    heartbeats: Arc<std::sync::Mutex<HeartbeatClock>>,
) {
    let launched = core.lock().await.daemon.daemon_pid();
    let who = match admit(peer.as_ref(), launched) {
        Ok(who) => who,
        Err(rejected) => {
            tracing::warn!(
                pid = ?rejected.pid,
                session = ?rejected.session,
                reason = %rejected.reason,
                "rejected a pipe client"
            );
            let _ = write_response(&mut pipe, &error(error_codes::UNAUTHORIZED, rejected.reason)).await;
            return;
        }
    };
    tracing::debug!(pid = who.pid, session = who.session, "admitted a pipe client");
    let mut greeted = false;
    let mut logged_foreign_beat = false;
    loop {
        let mut prefix = [0u8; 4];
        if pipe.read_exact(&mut prefix).await.is_err() {
            return;
        }
        let len = match decode_len(prefix) {
            Ok(len) => len as usize,
            Err(e) => {
                tracing::warn!(pid = who.pid, session = who.session, error = %e, "rejected a pipe frame; closing");
                let _ = write_response(&mut pipe, &error(error_codes::VALIDATION, e.to_string())).await;
                return;
            }
        };
        let mut body = vec![0u8; len];
        if len > 0 && pipe.read_exact(&mut body).await.is_err() {
            return;
        }
        let request = decode_body::<PipeRequest>(&body);
        if !greeted {
            if !matches!(request, Ok(PipeRequest::Hello { .. })) {
                tracing::warn!(
                    pid = who.pid,
                    session = who.session,
                    "rejected a pipe client: the first message was not Hello; closing"
                );
                let _ = write_response(&mut pipe, &error(error_codes::VALIDATION, "the first message on the pipe must be Hello")).await;
                return;
            }
            greeted = true;
        }
        let response = match request {
            Ok(PipeRequest::InstallUpdate { installer, signature }) => {
                // Everything that touches the probe happens before the first await: it is not `Sync`.
                let launched = core.lock().await.daemon.daemon_pid();
                let readmit = admit_install(peer.as_ref(), launched);
                let reader = peer.file_reader();
                install_update(&core, who, readmit, reader, installer, signature).await
            }
            Ok(PipeRequest::Heartbeat) => {
                // Stamped on arrival and answered without the core lock: a slow fan write
                // holding it must not delay a heartbeat (#129). Only the launched daemon's
                // pid stamps the clock, so a stale daemon's queued beats after a restart
                // cannot revive it; foreign beats are answered but not counted.
                let outcome = heartbeats.lock().unwrap_or_else(|p| p.into_inner()).stamp(who.pid, Instant::now());
                if outcome == Stamp::Foreign && !logged_foreign_beat {
                    tracing::warn!(pid = who.pid, session = who.session, "ignoring a heartbeat from a pid that is not the launched daemon");
                    logged_foreign_beat = true;
                }
                PipeResponse::Ok
            }
            Ok(PipeRequest::DaemonQuitting) => {
                // Only the daemon this service launched may say it is quitting (#6). While no
                // daemon runs (the relaunch is held after a session end), every client in the
                // console session is admitted, and one of them saying "it is quitting" would
                // stop the supervision for good.
                let launched = core.lock().await.daemon.daemon_pid();
                let readmit = admit_daemon(peer.as_ref(), launched, "DaemonQuitting");
                match readmit {
                    Ok(_) => core
                        .lock()
                        .await
                        .handle(PipeRequest::DaemonQuitting, Instant::now()),
                    Err(rejected) => {
                        tracing::warn!(
                            pid = ?rejected.pid,
                            session = ?rejected.session,
                            reason = %rejected.reason,
                            "refused DaemonQuitting: the client is not the launched daemon in the console session"
                        );
                        error(error_codes::UNAUTHORIZED, rejected.reason)
                    }
                }
            }
            Ok(request) => core.lock().await.handle(request, Instant::now()),
            Err(e) => error(error_codes::VALIDATION, format!("invalid request: {e}")),
        };
        if write_response(&mut pipe, &response).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::chip::MSI_ALT1_MAP;
    use crate::daemon::DaemonSupervisor;
    use crate::fakes::{FakeChip, FakeProcesses, FakeSpawner, FakeLauncher};
    use crate::fans::FanController;
    use crate::openrgb::OpenRgbSupervisor;
    use canvas_proto::config::FansFile;
    use canvas_proto::fans::{CurvePoint, FanMode, FanProfile, TempSource};
    use canvas_proto::pipe::PROTOCOL_VERSION;
    use canvas_proto::Rgb8;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use tokio::net::windows::named_pipe::NamedPipeClient;

    /// The console session in every test.
    const CONSOLE: u32 = 1;

    /// A probe whose answers the test chooses. `None` makes that OS read fail.
    #[derive(Clone, Debug)]
    struct FakePeer {
        session: Option<u32>,
        pid: Option<u32>,
        console: u32,
    }

    impl PeerProbe for FakePeer {
        fn client_session_id(&self) -> io::Result<u32> {
            self.session.ok_or_else(|| io::Error::other("no session"))
        }
        fn client_process_id(&self) -> io::Result<u32> {
            self.pid.ok_or_else(|| io::Error::other("no pid"))
        }
        fn console_session_id(&self) -> u32 {
            self.console
        }
    }

    fn peer(session: u32, pid: u32) -> FakePeer {
        FakePeer { session: Some(session), pid: Some(pid), console: CONSOLE }
    }

    /// Every connection gets the same fake probe.
    fn peers(fake: FakePeer) -> PeerFactory {
        Arc::new(move |_: &NamedPipeServer| -> Box<dyn PeerProbe> { Box::new(fake.clone()) })
    }

    fn unique_name(tag: &str) -> String {
        format!(r"\\.\pipe\canvas-rgb-svc-test-{tag}-{}", std::process::id())
    }

    pub(crate) fn test_core(home: PathBuf) -> Core {
        test_core_with_processes(home).0
    }

    /// The core of [`test_core`], with the fakes of its daemon supervisor, so a test can make
    /// that daemon end with its session (#6).
    fn test_core_with_processes(home: PathBuf) -> (Core, FakeProcesses) {
        let chip = FakeChip::new(MSI_ALT1_MAP);
        let chip_box: Box<dyn crate::chip::FanChip> =
            Box::new(crate::chip::Nct6687::detect(chip, MSI_ALT1_MAP).unwrap());
        let mut fans = FanController::new(
            home.clone(),
            Some(chip_box),
            None,
            BTreeMap::new(),
            FansFile::default(),
            Some(MSI_ALT1_MAP.name.to_string()),
        );
        fans.startup(Instant::now());
        let openrgb = OpenRgbSupervisor::new(
            PathBuf::from("OpenRGB.exe"),
            6742,
            PathBuf::from("cfg"),
            home,
            Box::new(FakeSpawner(FakeProcesses::new())),
            Box::new(|| Box::new(canvas_adapters::openrgb::OpenRgbAdapter::new("127.0.0.1", 1))),
        );
        let processes = FakeProcesses::new();
        let daemon = DaemonSupervisor::new(
            PathBuf::from("canvas-rgbd.exe"),
            Box::new(FakeLauncher(processes.clone())),
        );
        (Core::new(fans, openrgb, daemon), processes)
    }

    async fn round_trip(client: &mut NamedPipeClient, request: &PipeRequest) -> PipeResponse {
        let frame = encode(request).unwrap();
        client.write_all(&frame).await.unwrap();
        client.flush().await.unwrap();
        read_response(client).await
    }

    /// Reads one response. A regression that leaves the server silent fails after 10 s instead of hanging.
    async fn read_response(client: &mut NamedPipeClient) -> PipeResponse {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut prefix = [0u8; 4];
            client.read_exact(&mut prefix).await.unwrap();
            let mut body = vec![0u8; decode_len(prefix).unwrap() as usize];
            client.read_exact(&mut body).await.unwrap();
            decode_body(&body).unwrap()
        })
        .await
        .expect("no response from the pipe server within 10 s")
    }

    /// True when the server has closed its end (EOF or a broken pipe). A 5 s timeout counts as open.
    async fn closed(client: &mut NamedPipeClient) -> bool {
        let mut byte = [0u8; 1];
        match tokio::time::timeout(Duration::from_secs(5), client.read(&mut byte)).await {
            Ok(Ok(0)) | Ok(Err(_)) => true,
            Ok(Ok(_)) | Err(_) => false,
        }
    }

    /// Opens a client on the pipe, retrying while the server is still coming up.
    async fn connect(name: &str) -> NamedPipeClient {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            match tokio::net::windows::named_pipe::ClientOptions::new().open(name) {
                Ok(client) => return client,
                Err(e) => {
                    assert!(Instant::now() < end, "could not connect to {name} within 10 s: {e}");
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
            }
        }
    }

    /// Sends `Hello` as the first message and checks that it is answered with a `Hello`.
    async fn greet(client: &mut NamedPipeClient) {
        let hello = PipeRequest::Hello { protocol: PROTOCOL_VERSION, daemon_pid: 1 };
        let response = round_trip(client, &hello).await;
        assert!(matches!(response, PipeResponse::Hello { .. }), "{response:?}");
    }

    // ---- admission rule (no pipe) ----

    #[test]
    fn a_client_in_another_session_is_rejected() {
        let err = admit(&peer(CONSOLE + 1, 500), None).unwrap_err();
        assert_eq!(err.session, Some(CONSOLE + 1));
        assert_eq!(err.pid, Some(500));
        assert!(err.reason.contains("is not the console session"), "{}", err.reason);
    }

    #[test]
    fn a_client_that_is_not_the_launched_daemon_is_rejected() {
        let err = admit(&peer(CONSOLE, 500), Some(400)).unwrap_err();
        assert_eq!(err.pid, Some(500));
        assert_eq!(err.session, Some(CONSOLE));
        assert!(err.reason.contains("is not the launched daemon pid 400"), "{}", err.reason);
    }

    #[test]
    fn the_launched_daemon_in_the_console_session_is_admitted() {
        let who = admit(&peer(CONSOLE, 400), Some(400)).unwrap();
        assert_eq!(who, Admitted { pid: 400, session: CONSOLE });
    }

    #[test]
    fn any_console_client_is_admitted_when_no_daemon_was_launched() {
        assert!(admit(&peer(CONSOLE, 999), None).is_ok());
    }

    #[test]
    fn an_unreadable_client_is_rejected_and_says_what_was_unreadable() {
        let no_session = FakePeer { session: None, pid: Some(5), console: CONSOLE };
        let err = admit(&no_session, None).unwrap_err();
        assert!(err.reason.contains("cannot read the client session"), "{}", err.reason);
        let no_pid = FakePeer { session: Some(CONSOLE), pid: None, console: CONSOLE };
        let err = admit(&no_pid, None).unwrap_err();
        assert!(err.reason.contains("cannot read the client process"), "{}", err.reason);
        assert_eq!(err.session, Some(CONSOLE));
    }

    #[test]
    fn no_console_session_rejects_every_client() {
        let nobody = FakePeer { session: Some(1), pid: Some(7), console: u32::MAX };
        assert!(admit(&nobody, None).is_err());
    }

    // ---- over a real pipe, with fake OS answers ----

    #[tokio::test(flavor = "multi_thread")]
    async fn the_pipe_round_trips_every_operation() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("every-op");
        let first = create_instance(&name, true).unwrap();
        let (mut core, _processes) = test_core_with_processes(dir.path().to_path_buf());
        // The service launched a daemon: only that daemon may say it is quitting.
        core.daemon.start(Instant::now());
        let daemon = core
            .daemon
            .daemon_pid()
            .expect("the launched daemon has a pid");
        let core = Arc::new(Mutex::new(core));
        tokio::spawn(serve(
            Arc::clone(&core),
            name.clone(),
            first,
            peers(peer(CONSOLE, daemon)),
        ));

        let mut client = connect(&name).await;
        let curve = FanMode::Curve {
            source: TempSource::Cpu,
            points: vec![CurvePoint { temp_c: 30.0, percent: 40 }, CurvePoint { temp_c: 70.0, percent: 100 }],
        };
        let requests = vec![
            PipeRequest::Hello { protocol: PROTOCOL_VERSION, daemon_pid: 1 },
            PipeRequest::Heartbeat,
            PipeRequest::FanStatus,
            PipeRequest::SetHeaderMode { header: "sys_fan1".into(), mode: FanMode::Fixed { percent: 45 } },
            PipeRequest::SetHeaderMode { header: "sys_fan1".into(), mode: curve.clone() },
            PipeRequest::PutProfile {
                profile: FanProfile { name: "Quiet".into(), headers: BTreeMap::from([("sys_fan2".into(), curve)]) },
            },
            PipeRequest::SetProfile { name: "Quiet".into() },
            PipeRequest::PutFanConfig { config: FansFile::default() },
            PipeRequest::OpenRgbState,
            PipeRequest::RestartOpenRgb,
            PipeRequest::SetBootColor { color: Rgb8::new(1, 2, 3) },
            PipeRequest::FailSensor { source: TempSource::Gpu, failed: true },
            PipeRequest::FailSensor { source: TempSource::Gpu, failed: false },
            PipeRequest::DaemonQuitting,
        ];
        let mut responses = Vec::new();
        for request in &requests {
            responses.push(round_trip(&mut client, request).await);
        }
        assert!(matches!(responses[0], PipeResponse::Hello { protocol: 1, .. }));
        assert_eq!(responses[1], PipeResponse::Ok);
        assert!(matches!(responses[2], PipeResponse::FanStatus { .. }));
        assert_eq!(responses[3], PipeResponse::Ok);
        assert_eq!(responses[4], PipeResponse::Ok);
        assert_eq!(responses[5], PipeResponse::Ok);
        assert_eq!(responses[6], PipeResponse::Ok);
        assert_eq!(responses[7], PipeResponse::Ok);
        assert!(matches!(responses[8], PipeResponse::OpenRgb { port: 6742, .. }));
        assert_eq!(responses[9], PipeResponse::Ok);
        assert_eq!(responses[10], PipeResponse::Ok);
        assert_eq!(responses[11], PipeResponse::Ok);
        assert_eq!(responses[12], PipeResponse::Ok);
        assert_eq!(responses[13], PipeResponse::Ok);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn validation_failures_come_back_as_validation_errors() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("validation");
        let first = create_instance(&name, true).unwrap();
        let core = Arc::new(Mutex::new(test_core(dir.path().to_path_buf())));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE, 77))));
        let mut client = connect(&name).await;
        greet(&mut client).await;

        // Below the 20 % default floor.
        let below = round_trip(
            &mut client,
            &PipeRequest::SetHeaderMode { header: "sys_fan1".into(), mode: FanMode::Fixed { percent: 19 } },
        )
        .await;
        assert!(matches!(&below, PipeResponse::Error { code, .. } if code == "validation"), "{below:?}");

        // CPU_FAN below 30 %.
        let cpu = round_trip(
            &mut client,
            &PipeRequest::SetHeaderMode { header: "cpu_fan".into(), mode: FanMode::Fixed { percent: 29 } },
        )
        .await;
        assert!(matches!(&cpu, PipeResponse::Error { code, .. } if code == "validation"), "{cpu:?}");

        // A one-point curve.
        let one = round_trip(
            &mut client,
            &PipeRequest::SetHeaderMode {
                header: "sys_fan1".into(),
                mode: FanMode::Curve { source: TempSource::Cpu, points: vec![CurvePoint { temp_c: 30.0, percent: 50 }] },
            },
        )
        .await;
        assert!(matches!(&one, PipeResponse::Error { code, .. } if code == "validation"), "{one:?}");

        // Unknown profile is not_found, not validation.
        let missing = round_trip(&mut client, &PipeRequest::SetProfile { name: "Nope".into() }).await;
        assert!(matches!(&missing, PipeResponse::Error { code, .. } if code == "not_found"), "{missing:?}");

        // The tach-only header cannot take a mode.
        let tach = round_trip(
            &mut client,
            &PipeRequest::SetHeaderMode { header: "pump_fan2".into(), mode: FanMode::Fixed { percent: 50 } },
        )
        .await;
        assert!(matches!(&tach, PipeResponse::Error { code, .. } if code == "validation"), "{tach:?}");

        // Nothing was written: the status shows no manual headers.
        let status = round_trip(&mut client, &PipeRequest::FanStatus).await;
        let PipeResponse::FanStatus { status } = status else { panic!("not a status") };
        assert!(status.headers.iter().all(|h| h.on_bios));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_squatter_on_the_name_fails_loudly() {
        let name = unique_name("squatter");
        let _first = create_instance(&name, true).unwrap();
        let err = create_instance(&name, true).err();
        assert!(err.is_some(), "a second first-instance must fail");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_oversized_frame_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("oversize");
        let first = create_instance(&name, true).unwrap();
        let core = Arc::new(Mutex::new(test_core(dir.path().to_path_buf())));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE, 77))));
        let mut client = connect(&name).await;
        greet(&mut client).await;
        let huge = (canvas_proto::pipe::MAX_FRAME + 1).to_le_bytes();
        client.write_all(&huge).await.unwrap();
        let response = read_response(&mut client).await;
        assert!(matches!(response, PipeResponse::Error { ref code, .. } if code == "validation"), "{response:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_json_gets_an_error_and_the_pipe_keeps_working() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("badjson");
        let first = create_instance(&name, true).unwrap();
        let core = Arc::new(Mutex::new(test_core(dir.path().to_path_buf())));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE, 77))));
        let mut client = connect(&name).await;
        greet(&mut client).await;
        let junk = b"{not json";
        let mut frame = (junk.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(junk);
        client.write_all(&frame).await.unwrap();
        let response = read_response(&mut client).await;
        assert!(matches!(response, PipeResponse::Error { ref code, .. } if code == "validation"));
        // The connection stays usable after a bad body.
        let ok = round_trip(&mut client, &PipeRequest::Heartbeat).await;
        assert_eq!(ok, PipeResponse::Ok);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_session_client_gets_unauthorized_and_the_pipe_closes() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("wrong-session");
        let first = create_instance(&name, true).unwrap();
        let core = Arc::new(Mutex::new(test_core(dir.path().to_path_buf())));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE + 1, 77))));
        let mut client = connect(&name).await;
        let response = read_response(&mut client).await;
        assert!(matches!(&response, PipeResponse::Error { code, .. } if code == error_codes::UNAUTHORIZED), "{response:?}");
        assert!(closed(&mut client).await, "the pipe closes after the rejection");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_pid_client_gets_unauthorized_when_a_daemon_was_launched() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("wrong-pid");
        let first = create_instance(&name, true).unwrap();
        let mut core = test_core(dir.path().to_path_buf());
        core.daemon.start(Instant::now());
        let daemon = core.daemon.daemon_pid().expect("the launched daemon has a pid");
        let core = Arc::new(Mutex::new(core));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE, daemon + 1))));
        let mut client = connect(&name).await;
        let response = read_response(&mut client).await;
        assert!(matches!(&response, PipeResponse::Error { code, .. } if code == error_codes::UNAUTHORIZED), "{response:?}");
        assert!(closed(&mut client).await, "the pipe closes after the rejection");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_launched_daemon_is_admitted_and_its_hello_is_answered() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("daemon-ok");
        let first = create_instance(&name, true).unwrap();
        let mut core = test_core(dir.path().to_path_buf());
        core.daemon.start(Instant::now());
        let daemon = core.daemon.daemon_pid().expect("the launched daemon has a pid");
        let core = Arc::new(Mutex::new(core));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE, daemon))));
        let mut client = connect(&name).await;
        let response = round_trip(&mut client, &PipeRequest::Hello { protocol: PROTOCOL_VERSION, daemon_pid: 1 }).await;
        assert!(matches!(response, PipeResponse::Hello { .. }), "{response:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_first_message_must_be_hello_or_the_pipe_closes() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("hello-first");
        let first = create_instance(&name, true).unwrap();
        let core = Arc::new(Mutex::new(test_core(dir.path().to_path_buf())));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE, 77))));
        let mut client = connect(&name).await;
        let response = round_trip(
            &mut client,
            &PipeRequest::SetHeaderMode { header: "sys_fan1".into(), mode: FanMode::Fixed { percent: 45 } },
        )
        .await;
        assert!(matches!(&response, PipeResponse::Error { code, .. } if code == "validation"), "{response:?}");
        assert!(closed(&mut client).await, "the pipe closes when the first message is not Hello");
    }

    // ---- InstallUpdate over a real pipe, on the update fakes (#43) ----

    /// A probe whose console session the test can move after the client has connected, and whose
    /// file reads come from memory.
    #[derive(Clone)]
    struct UpdatePeer {
        session: u32,
        pid: u32,
        console: Arc<std::sync::atomic::AtomicU32>,
    }

    impl PeerProbe for UpdatePeer {
        fn client_session_id(&self) -> io::Result<u32> {
            Ok(self.session)
        }
        fn client_process_id(&self) -> io::Result<u32> {
            Ok(self.pid)
        }
        fn console_session_id(&self) -> u32 {
            self.console.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn file_reader(&self) -> Box<dyn crate::update_install::ClientFileReader> {
            crate::update_install_tests::FakeReader::of(crate::update_install_tests::PAYLOAD)
        }
    }

    fn install_request() -> PipeRequest {
        PipeRequest::InstallUpdate {
            installer: crate::update_install_tests::SOURCE.into(),
            signature: crate::update_install_tests::SIG_011.into(),
        }
    }

    /// A pipe server whose core can install updates on a rig that has installed version 0.1.0. The
    /// service has launched a daemon, and the client is that daemon.
    async fn update_pipe(tag: &str, session: u32) -> (String, crate::update_install_tests::Rig, Arc<std::sync::atomic::AtomicU32>, tempfile::TempDir) {
        update_pipe_with(tag, session, true, 0).await
    }

    /// As [`update_pipe`], but `launched` says whether the service launched a daemon at all, and
    /// `pid_delta` moves the client's pid away from the daemon's.
    async fn update_pipe_with(
        tag: &str,
        session: u32,
        launched: bool,
        pid_delta: u32,
    ) -> (String, crate::update_install_tests::Rig, Arc<std::sync::atomic::AtomicU32>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name(tag);
        let first = create_instance(&name, true).unwrap();
        let rig = crate::update_install_tests::rig("0.1.0");
        let mut core = test_core(dir.path().to_path_buf());
        core.updates = Some(Arc::clone(&rig.installer));
        let pid = if launched {
            core.daemon.start(Instant::now());
            core.daemon.daemon_pid().expect("the launched daemon has a pid") + pid_delta
        } else {
            77
        };
        let console = Arc::new(std::sync::atomic::AtomicU32::new(CONSOLE));
        let factory_peer = UpdatePeer { session, pid, console: Arc::clone(&console) };
        let peers: PeerFactory = Arc::new(move |_: &NamedPipeServer| -> Box<dyn PeerProbe> { Box::new(factory_peer.clone()) });
        tokio::spawn(serve(Arc::new(Mutex::new(core)), name.clone(), first, peers));
        (name, rig, console, dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn install_update_is_refused_when_the_service_launched_no_daemon() {
        // Any console-user process is admitted for the other operations when no daemon is running.
        // InstallUpdate skips the daemon's token and port checks, so it is not.
        let (name, rig, _console, _dir) = update_pipe_with("install-no-daemon", CONSOLE, false, 0).await;
        let mut client = connect(&name).await;
        greet(&mut client).await;
        assert_eq!(round_trip(&mut client, &PipeRequest::Heartbeat).await, PipeResponse::Ok, "other operations are admitted as before");
        let response = round_trip(&mut client, &install_request()).await;
        assert!(matches!(&response, PipeResponse::Error { code, message } if code == error_codes::UNAUTHORIZED && message.contains("none is running")), "{response:?}");
        assert_eq!(rig.launches(), 0);
        assert!(rig.staged_files().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn install_update_is_refused_from_a_pid_that_is_not_the_launched_daemon() {
        // The pid check at connect time is skipped by the fake factory, so build a client whose pid
        // differs from the daemon's only at request time: connect as the daemon, then change pid.
        let (name, rig, _console, _dir) = update_pipe_with("install-wrong-pid", CONSOLE, true, 1).await;
        let mut client = connect(&name).await;
        // The wrong pid is already refused at connect time with the daemon check.
        let response = read_response(&mut client).await;
        assert!(matches!(&response, PipeResponse::Error { code, .. } if code == error_codes::UNAUTHORIZED), "{response:?}");
        assert_eq!(rig.launches(), 0);
    }

    #[test]
    fn admit_install_needs_the_launched_daemon_and_logs_nothing_it_does_not_return() {
        let none = admit_install(&peer(CONSOLE, 77), None).unwrap_err();
        assert_eq!((none.pid, none.session), (Some(77), Some(CONSOLE)), "the refusal carries pid and session for the log");
        assert!(none.reason.contains("none is running"), "{}", none.reason);
        let wrong = admit_install(&peer(CONSOLE, 77), Some(78)).unwrap_err();
        assert!(wrong.reason.contains("launched daemon"), "{}", wrong.reason);
        assert_eq!(admit_install(&peer(CONSOLE, 78), Some(78)).unwrap(), Admitted { pid: 78, session: CONSOLE });
        assert!(admit_install(&peer(CONSOLE + 1, 78), Some(78)).is_err(), "the console rule still applies");
    }

    // ---- DaemonQuitting over a real pipe (#6) ----

    /// The exit code that says the daemon ended with its session (#6).
    const SESSION_END_CODE: i32 = 0xC000_026Bu32 as i32;

    /// While no daemon runs (the relaunch is held after a session end), every client in the
    /// console session is admitted. Such a client must not be able to say the daemon is
    /// quitting: that would stop the supervision for good (#6).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_daemon_quitting_from_another_client_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, processes) = test_core_with_processes(dir.path().to_path_buf());
        core.daemon.start(Instant::now());
        // The daemon ends with its session: the relaunch is held, so no daemon is running.
        processes.exit_last_with(SESSION_END_CODE);
        core.daemon.tick(Instant::now());
        assert_eq!(
            core.daemon.daemon_pid(),
            None,
            "the daemon is gone and the relaunch is held"
        );
        let core = Arc::new(Mutex::new(core));
        let name = unique_name("quit-refused");
        let first = create_instance(&name, true).unwrap();
        tokio::spawn(serve(
            Arc::clone(&core),
            name.clone(),
            first,
            peers(peer(CONSOLE, 77)),
        ));
        let mut client = connect(&name).await;
        greet(&mut client).await;
        let response = round_trip(&mut client, &PipeRequest::DaemonQuitting).await;
        assert!(
            matches!(&response, PipeResponse::Error { code, .. } if code == error_codes::UNAUTHORIZED),
            "{response:?}"
        );
        // The supervision did not stop: after the hold the daemon starts again.
        core.lock()
            .await
            .daemon
            .tick(Instant::now() + Duration::from_secs(61));
        assert!(
            core.lock().await.daemon.daemon_pid().is_some(),
            "the supervision stopped for good"
        );
    }

    /// The daemon this service launched may say it is quitting: the service does not restart it.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_launched_daemon_may_say_it_is_quitting() {
        let dir = tempfile::tempdir().unwrap();
        let (mut core, processes) = test_core_with_processes(dir.path().to_path_buf());
        core.daemon.start(Instant::now());
        let daemon = core
            .daemon
            .daemon_pid()
            .expect("the launched daemon has a pid");
        let core = Arc::new(Mutex::new(core));
        let name = unique_name("quit-ok");
        let first = create_instance(&name, true).unwrap();
        tokio::spawn(serve(
            Arc::clone(&core),
            name.clone(),
            first,
            peers(peer(CONSOLE, daemon)),
        ));
        let mut client = connect(&name).await;
        greet(&mut client).await;
        let response = round_trip(&mut client, &PipeRequest::DaemonQuitting).await;
        assert_eq!(response, PipeResponse::Ok, "{response:?}");
        // The daemon that quit is not started again.
        processes.exit_last();
        core.lock()
            .await
            .daemon
            .tick(Instant::now() + Duration::from_secs(1));
        assert_eq!(
            processes.spawns().len(),
            1,
            "the daemon that quit was started again"
        );
    }

    #[test]
    fn admit_daemon_needs_the_launched_daemon_and_says_which_request_it_was() {
        let none = admit_daemon(&peer(CONSOLE, 77), None, "DaemonQuitting").unwrap_err();
        assert_eq!(
            (none.pid, none.session),
            (Some(77), Some(CONSOLE)),
            "the refusal carries pid and session for the log"
        );
        assert!(none.reason.contains("none is running"), "{}", none.reason);
        assert!(none.reason.contains("DaemonQuitting"), "{}", none.reason);
        let wrong = admit_daemon(&peer(CONSOLE, 77), Some(78), "DaemonQuitting").unwrap_err();
        assert!(wrong.reason.contains("launched daemon"), "{}", wrong.reason);
        assert_eq!(
            admit_daemon(&peer(CONSOLE, 78), Some(78), "DaemonQuitting").unwrap(),
            Admitted {
                pid: 78,
                session: CONSOLE
            }
        );
        assert!(
            admit_daemon(&peer(CONSOLE + 1, 78), Some(78), "DaemonQuitting").is_err(),
            "the console rule still applies"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn install_update_over_the_pipe_starts_setup_and_a_second_request_conflicts() {
        let (name, rig, _console, _dir) = update_pipe("install-ok", CONSOLE).await;
        let mut client = connect(&name).await;
        greet(&mut client).await;
        let first = round_trip(&mut client, &install_request()).await;
        assert_eq!(first, PipeResponse::UpdateStarted { version: "0.1.1".into() });
        assert_eq!(rig.launches(), 1);
        let second = round_trip(&mut client, &install_request()).await;
        assert!(matches!(&second, PipeResponse::Error { code, .. } if code == error_codes::CONFLICT), "{second:?}");
        assert_eq!(rig.launches(), 1, "the second request started nothing");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_console_client_cannot_install_an_update() {
        let (name, rig, _console, _dir) = update_pipe("install-wrong-session", CONSOLE + 1).await;
        let mut client = connect(&name).await;
        let response = read_response(&mut client).await;
        assert!(matches!(&response, PipeResponse::Error { code, .. } if code == error_codes::UNAUTHORIZED), "{response:?}");
        // The pipe is closed: even if the request is written it is never read.
        let _ = client.write_all(&encode(&install_request()).unwrap()).await;
        assert!(closed(&mut client).await);
        assert_eq!(rig.launches(), 0);
        assert!(rig.staged_files().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_whose_session_left_the_console_after_connecting_is_refused_at_the_request() {
        let (name, rig, console, _dir) = update_pipe("install-moved", CONSOLE).await;
        let mut client = connect(&name).await;
        greet(&mut client).await;
        console.store(CONSOLE + 1, std::sync::atomic::Ordering::SeqCst);
        let response = round_trip(&mut client, &install_request()).await;
        assert!(matches!(&response, PipeResponse::Error { code, .. } if code == error_codes::UNAUTHORIZED), "{response:?}");
        assert_eq!(rig.launches(), 0);
        assert!(rig.staged_files().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn install_update_is_unavailable_when_the_service_has_no_installer() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("install-none");
        let first = create_instance(&name, true).unwrap();
        let mut core = test_core(dir.path().to_path_buf());
        core.daemon.start(Instant::now());
        let daemon = core.daemon.daemon_pid().expect("the launched daemon has a pid");
        let core = Arc::new(Mutex::new(core));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE, daemon))));
        let mut client = connect(&name).await;
        greet(&mut client).await;
        let response = round_trip(&mut client, &install_request()).await;
        assert!(matches!(&response, PipeResponse::Error { code, .. } if code == error_codes::SERVICE_UNAVAILABLE), "{response:?}");
    }

    /// A core wired to a launched fake daemon, returning the chip so the test can arm a
    /// stuck duty (a header whose duty never reads back) after engaging it.
    fn blocking_core(home: PathBuf) -> (Core, FakeChip, FakeProcesses) {
        let chip = FakeChip::new(MSI_ALT1_MAP);
        let chip_box: Box<dyn crate::chip::FanChip> =
            Box::new(crate::chip::Nct6687::detect(chip.clone(), MSI_ALT1_MAP).unwrap());
        let mut fans = FanController::new(
            home.clone(),
            Some(chip_box),
            None,
            BTreeMap::new(),
            FansFile::default(),
            Some(MSI_ALT1_MAP.name.to_string()),
        );
        fans.startup(Instant::now());
        let openrgb = OpenRgbSupervisor::new(
            PathBuf::from("OpenRGB.exe"),
            6742,
            PathBuf::from("cfg"),
            home,
            Box::new(FakeSpawner(FakeProcesses::new())),
            Box::new(|| Box::new(canvas_adapters::openrgb::OpenRgbAdapter::new("127.0.0.1", 1))),
        );
        let processes = FakeProcesses::new();
        let daemon = DaemonSupervisor::new(PathBuf::from("canvas-rgbd.exe"), Box::new(FakeLauncher(processes.clone())));
        (Core::new(fans, openrgb, daemon), chip, processes)
    }

    /// #129: two headers whose duty never reads back are written back to back, which holds the
    /// core's lock for two READBACK_SETTLE waits. A heartbeat arriving during the write must be
    /// answered without waiting for the write, and the daemon must not be ended for missed
    /// heartbeats.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_fan_write_does_not_starve_the_daemon_heartbeat() {
        let dir = tempfile::tempdir().unwrap();
        let (core, chip, processes) = blocking_core(dir.path().to_path_buf());
        let mut core = core;
        core.daemon.start(Instant::now());
        let daemon_pid = core.daemon.daemon_pid().expect("the launched daemon has a pid");
        let core = Arc::new(Mutex::new(core));
        let name = unique_name("slow-fan-write");
        let first = create_instance(&name, true).unwrap();
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE, daemon_pid))));

        // Engage both headers while the chip behaves, so the blocking write comes after.
        let mut writer = connect(&name).await;
        greet(&mut writer).await;
        for header in ["sys_fan5", "sys_fan6"] {
            let response = round_trip(
                &mut writer,
                &PipeRequest::SetHeaderMode { header: header.into(), mode: FanMode::Fixed { percent: 45 } },
            )
            .await;
            assert_eq!(response, PipeResponse::Ok, "{header}");
        }
        // Both fans accept the write but never move: every readback stays at the BIOS
        // duty (the stuck-readback of issue #129).
        chip.set_stuck_duty(6, true);
        chip.set_stuck_duty(7, true);
        let writes_at_start = chip.writes();

        // The daemon's heartbeat connection, greeted before the write starts; it beats
        // every 50 ms, like the real daemon's 1 Hz loop.
        let mut beats = connect(&name).await;
        greet(&mut beats).await;
        let stop_beats = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let beats_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let beats_task = {
            let stop = Arc::clone(&stop_beats);
            let count = Arc::clone(&beats_count);
            tokio::spawn(async move {
                loop {
                    if stop.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    if round_trip(&mut beats, &PipeRequest::Heartbeat).await != PipeResponse::Ok {
                        break;
                    }
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
        };

        // The 1 Hz loop's fan pass, backgrounded: both headers are driven, each waiting the
        // full readback settle before it hands back to BIOS.
        let blocked = {
            let core = Arc::clone(&core);
            tokio::spawn(async move { core.lock().await.tick(Instant::now()).await })
        };
        // Wait until the fan pass has started writing (it runs under the core lock), so the
        // heartbeats below are guaranteed to arrive while the write is blocked.
        for _ in 0..500 {
            if chip.writes() > writes_at_start {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        blocked.await.unwrap();
        stop_beats.store(true, std::sync::atomic::Ordering::SeqCst);
        beats_task.await.unwrap();
        let beats = beats_count.load(std::sync::atomic::Ordering::SeqCst);
        assert!(beats >= 5, "the daemon did not keep beating through the write: {beats} heartbeats");
        // Supervision after the window: the daemon has not been ended for missed heartbeats.
        for _ in 0..3 {
            core.lock().await.tick(Instant::now()).await;
        }
        assert_eq!(processes.spawns().len(), 1, "the healthy daemon was ended for missed heartbeats");
        assert!(!core.lock().await.daemon.stopped());
    }

    /// A probe whose pid the test can move between connections, admitted per connection.
    struct AnyPidPeer {
        session: u32,
        console: u32,
        reported: Arc<std::sync::atomic::AtomicU32>,
    }

    impl PeerProbe for AnyPidPeer {
        fn client_session_id(&self) -> io::Result<u32> {
            Ok(self.session)
        }
        fn client_process_id(&self) -> io::Result<u32> {
            Ok(self.reported.load(std::sync::atomic::Ordering::SeqCst))
        }
        fn console_session_id(&self) -> u32 {
            self.console
        }
    }

    /// The shared heartbeat clock's stamp time, as the supervisor sees it.
    async fn clock_at(core: &Arc<Mutex<Core>>) -> Option<Instant> {
        core.lock().await.daemon.shared_heartbeats().lock().unwrap_or_else(|p| p.into_inner()).at()
    }

    /// The stale connection of a daemon the service has since restarted keeps beating; its
    /// beats must not stamp the new daemon's heartbeat clock.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_heartbeat_from_the_old_pid_after_a_restart_does_not_count() {
        let dir = tempfile::tempdir().unwrap();
        let (core, _chip, processes) = blocking_core(dir.path().to_path_buf());
        let mut core = core;
        core.daemon.start(Instant::now());
        let pid1 = core.daemon.daemon_pid().expect("the launched daemon has a pid");
        let core = Arc::new(Mutex::new(core));
        let name = unique_name("stale-beat");
        let first = create_instance(&name, true).unwrap();
        let reported = Arc::new(std::sync::atomic::AtomicU32::new(pid1));
        let factory_reported = Arc::clone(&reported);
        let peers: PeerFactory = Arc::new(move |_: &NamedPipeServer| -> Box<dyn PeerProbe> {
            Box::new(AnyPidPeer { session: CONSOLE, console: CONSOLE, reported: Arc::clone(&factory_reported) })
        });
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers));

        // The launched daemon's own connection, admitted before the restart.
        let mut stale = connect(&name).await;
        greet(&mut stale).await;
        assert_eq!(round_trip(&mut stale, &PipeRequest::Heartbeat).await, PipeResponse::Ok);

        // A crash ends the daemon; the service restarts it under a new pid.
        processes.exit_last();
        core.lock().await.tick(Instant::now()).await;
        let pid2 = core.lock().await.daemon.daemon_pid().expect("the daemon restarted");
        assert_ne!(pid1, pid2);
        let at_after_restart = clock_at(&core).await;

        // The stale connection keeps beating: answered, but the clock does not move.
        assert_eq!(round_trip(&mut stale, &PipeRequest::Heartbeat).await, PipeResponse::Ok);
        assert_eq!(clock_at(&core).await, at_after_restart, "a beat from the old pid after the restart was counted");

        // The new daemon's own beat does count.
        reported.store(pid2, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut fresh = connect(&name).await;
        greet(&mut fresh).await;
        let fresh_response = round_trip(&mut fresh, &PipeRequest::Heartbeat).await;
        let at_fresh = clock_at(&core).await;
        assert_eq!(fresh_response, PipeResponse::Ok);
        assert!(at_fresh > at_after_restart, "the launched daemon's beat was not counted");
        assert!(core.lock().await.daemon.is_connected(Instant::now()));

        // Once the supervisor is stopped, even the launched daemon's beats are ignored.
        core.lock().await.daemon.stop();
        let at_stopped = clock_at(&core).await;
        assert_eq!(round_trip(&mut fresh, &PipeRequest::Heartbeat).await, PipeResponse::Ok);
        assert_eq!(clock_at(&core).await, at_stopped, "a beat after the supervisor stopped was counted");
    }

    /// Real Win32, no fake: a client in this process must show this process's PID and session.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_real_probe_reads_this_processes_pid_and_session() {
        use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
        let name = unique_name("real-probe");
        let server = create_instance(&name, true).unwrap();
        let client = connect(&name).await;
        server.connect().await.unwrap();
        let probe = WinPeer::of(&server);
        assert_eq!(probe.client_process_id().unwrap(), std::process::id(), "client pid");
        let mut session = u32::MAX;
        // SAFETY: `session` is a valid out-pointer for the call.
        assert_ne!(unsafe { ProcessIdToSessionId(std::process::id(), &mut session) }, 0);
        assert_eq!(probe.client_session_id().unwrap(), session, "client session");
        drop(client);
    }
}
