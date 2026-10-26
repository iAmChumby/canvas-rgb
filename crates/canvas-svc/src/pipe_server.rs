//! The named-pipe server the daemon talks to (`\\.\pipe\canvas-rgb-svc`).
//!
//! Security (from the security review): the first instance is created with
//! `first_pipe_instance(true)`, so a squatter that created the name first makes the service
//! fail loudly. The DACL grants SYSTEM full control and the interactive user (S-1-5-4)
//! read and write only. Remote clients are rejected.
//!
//! Framing is `canvas_proto::pipe`: a u32 little-endian length, then JSON. One request gets
//! one response, in order. Frames over `MAX_FRAME` get an error and the connection closes.

use crate::service::Core;
use canvas_proto::api::error_codes;
use canvas_proto::pipe::{decode_body, decode_len, encode, PipeRequest, PipeResponse, FrameError, PIPE_NAME};
use std::ffi::c_void;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::Mutex;
use windows_sys::Win32::Foundation::{LocalFree, FALSE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;

/// Overrides the pipe name. Tests only; the service uses the spine name.
pub const PIPE_NAME_ENV: &str = "CANVAS_RGB_PIPE_NAME";
/// SYSTEM: full control. Interactive user (S-1-5-4): read and write only.
pub const PIPE_SDDL: &str = "D:(A;;GA;;;SY)(A;;GRGW;;;S-1-5-4)";
const SDDL_REVISION_1: u32 = 1;

pub fn pipe_name() -> String {
    std::env::var(PIPE_NAME_ENV).unwrap_or_else(|_| PIPE_NAME.to_string())
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
pub async fn serve(core: Arc<Mutex<Core>>, name: String, first: NamedPipeServer) -> io::Result<()> {
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
        tokio::spawn(handle_client(connected, Arc::clone(&core)));
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

/// Reads requests from one client until it disconnects.
pub async fn handle_client(mut pipe: NamedPipeServer, core: Arc<Mutex<Core>>) {
    loop {
        let mut prefix = [0u8; 4];
        if pipe.read_exact(&mut prefix).await.is_err() {
            return;
        }
        let len = match decode_len(prefix) {
            Ok(len) => len as usize,
            Err(FrameError::TooLarge(n)) => {
                let _ = write_response(&mut pipe, &error(error_codes::VALIDATION, format!("frame of {n} bytes is over the limit"))).await;
                return;
            }
            Err(e) => {
                let _ = write_response(&mut pipe, &error(error_codes::VALIDATION, e.to_string())).await;
                return;
            }
        };
        let mut body = vec![0u8; len];
        if len > 0 && pipe.read_exact(&mut body).await.is_err() {
            return;
        }
        let response = match decode_body::<PipeRequest>(&body) {
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
        let mut prefix = [0u8; 4];
        client.read_exact(&mut prefix).await.unwrap();
        let mut body = vec![0u8; decode_len(prefix).unwrap() as usize];
        client.read_exact(&mut body).await.unwrap();
        decode_body(&body).unwrap()
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

    #[tokio::test(flavor = "multi_thread")]
    async fn the_pipe_round_trips_every_operation() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("every-op");
        let first = create_instance(&name, true).unwrap();
        let core = Arc::new(Mutex::new(test_core(dir.path().to_path_buf())));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first));

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
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first));
        let mut client = connect(&name).await;

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
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first));
        let mut client = connect(&name).await;
        let huge = (canvas_proto::pipe::MAX_FRAME + 1).to_le_bytes();
        client.write_all(&huge).await.unwrap();
        let mut prefix = [0u8; 4];
        client.read_exact(&mut prefix).await.unwrap();
        let mut body = vec![0u8; decode_len(prefix).unwrap() as usize];
        client.read_exact(&mut body).await.unwrap();
        let response: PipeResponse = decode_body(&body).unwrap();
        assert!(matches!(response, PipeResponse::Error { ref code, .. } if code == "validation"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_json_gets_an_error_and_the_pipe_keeps_working() {
        let dir = tempfile::tempdir().unwrap();
        let name = unique_name("badjson");
        let first = create_instance(&name, true).unwrap();
        let core = Arc::new(Mutex::new(test_core(dir.path().to_path_buf())));
        tokio::spawn(serve(Arc::clone(&core), name.clone(), first));
        let mut client = connect(&name).await;
        let junk = b"{not json";
        let mut frame = (junk.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(junk);
        client.write_all(&frame).await.unwrap();
        let mut prefix = [0u8; 4];
        client.read_exact(&mut prefix).await.unwrap();
        let mut body = vec![0u8; decode_len(prefix).unwrap() as usize];
        client.read_exact(&mut body).await.unwrap();
        let response: PipeResponse = decode_body(&body).unwrap();
        assert!(matches!(response, PipeResponse::Error { ref code, .. } if code == "validation"));
        // The connection stays usable after a bad body.
        let ok = round_trip(&mut client, &PipeRequest::Heartbeat).await;
        assert_eq!(ok, PipeResponse::Ok);
    }

}
