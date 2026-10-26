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
//! Framing is `canvas_proto::pipe`: a u32 little-endian length, then JSON. One request gets
//! one response, in order. Frames over `MAX_FRAME` get an error and the connection closes.
//! The OS calls for step 1 sit behind [`PeerProbe`], so tests use a fake.

use crate::service::Core;
use canvas_proto::api::error_codes;
use canvas_proto::pipe::{decode_body, decode_len, encode, PipeRequest, PipeResponse, PIPE_NAME};
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
        tokio::spawn(handle_client(connected, Arc::clone(&core), peer));
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

/// Admits one client, then reads requests from it until it disconnects. The first message must
/// be `Hello`.
pub async fn handle_client(mut pipe: NamedPipeServer, core: Arc<Mutex<Core>>, peer: Box<dyn PeerProbe>) {
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
            Ok(request) => core.lock().await.handle(request, Instant::now()),
            Err(e) => error(error_codes::VALIDATION, format!("invalid request: {e}")),
        };
        if write_response(&mut pipe, &response).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
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

    fn test_core(home: PathBuf) -> Core {
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
        let daemon = DaemonSupervisor::new(PathBuf::from("canvas-rgbd.exe"), Box::new(FakeLauncher(FakeProcesses::new())));
        Core::new(fans, openrgb, daemon)
    }

    async fn round_trip(client: &mut NamedPipeClient, request: &PipeRequest) -> PipeResponse {
        let frame = encode(request).unwrap();
        client.write_all(&frame).await.unwrap();
        client.flush().await.unwrap();
        read_response(client).await
    }

    async fn read_response(client: &mut NamedPipeClient) -> PipeResponse {
        let mut prefix = [0u8; 4];
        client.read_exact(&mut prefix).await.unwrap();
        let mut body = vec![0u8; decode_len(prefix).unwrap() as usize];
        client.read_exact(&mut body).await.unwrap();
        decode_body(&body).unwrap()
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
        loop {
            match tokio::net::windows::named_pipe::ClientOptions::new().open(name) {
                Ok(client) => return client,
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
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
        let core = Arc::new(Mutex::new(test_core(dir.path().to_path_buf())));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first, peers(peer(CONSOLE, 77))));

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
