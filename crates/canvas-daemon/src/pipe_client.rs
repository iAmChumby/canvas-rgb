//! The daemon's client for the service's named pipe (`\\.\pipe\canvas-rgb-svc`).
//!
//! The service does not exist yet, so "pipe absent" is a normal state: the daemon keeps
//! running, `/status` reports `service: down` with `service_down_ms`, and the fans
//! endpoints answer 503 `service_unavailable`.
//!
//! Protocol: length-prefixed JSON, one request and one response in order
//! (`canvas_proto::pipe`).

use canvas_proto::pipe::{decode_len, encode, PipeRequest, PipeResponse, MAX_FRAME};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

/// How long to wait between reconnect attempts while the service is down.
pub const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);

/// The pipe name, from the spine.
pub const PIPE_NAME: &str = canvas_proto::pipe::PIPE_NAME;

/// The service reads an update installer as this client (`ImpersonateNamedPipeClient`, #43), so it
/// may impersonate, not only identify, the daemon. Tokio's default is `SECURITY_IDENTIFICATION`,
/// at which the service's impersonated open of the installer is refused.
pub const PIPE_QOS: u32 = windows::Win32::Storage::FileSystem::SECURITY_IMPERSONATION.0
    | windows::Win32::Storage::FileSystem::SECURITY_SQOS_PRESENT.0;

/// A connected pipe client.
pub struct PipeClient {
    pipe: NamedPipeClient,
}

impl PipeClient {
    /// Opens the pipe. Fails when the service is not listening.
    pub async fn connect() -> std::io::Result<Self> {
        let pipe = ClientOptions::new().security_qos_flags(PIPE_QOS).open(PIPE_NAME)?;
        Ok(Self { pipe })
    }

    /// Sends one request and reads one response.
    pub async fn request(&mut self, request: &PipeRequest) -> Result<PipeResponse, PipeError> {
        let frame = encode(request).map_err(PipeError::Frame)?;
        if frame.len() as u32 > MAX_FRAME {
            return Err(PipeError::Frame(canvas_proto::pipe::FrameError::TooLarge(frame.len() as u32)));
        }
        self.pipe.write_all(&frame).await.map_err(PipeError::Io)?;
        self.pipe.flush().await.map_err(PipeError::Io)?;

        let mut prefix = [0u8; 4];
        self.pipe.read_exact(&mut prefix).await.map_err(PipeError::Io)?;
        let len = decode_len(prefix).map_err(PipeError::Frame)? as usize;
        let mut body = vec![0u8; len];
        if len > 0 {
            self.pipe.read_exact(&mut body).await.map_err(PipeError::Io)?;
        }
        serde_json::from_slice(&body).map_err(PipeError::Json)
    }
}

/// Why a pipe exchange failed.
#[derive(Debug, thiserror::Error)]
pub enum PipeError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame: {0}")]
    Frame(#[from] canvas_proto::pipe::FrameError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// One request to the service and its answer. The daemon's [`App`] holds one of these, so
/// the tests can put a fake service behind the API.
///
/// [`App`]: crate::state::App
#[async_trait::async_trait]
pub trait PipeLink: Send + Sync {
    async fn exchange(&self, request: &PipeRequest) -> Result<PipeResponse, PipeError>;
}

/// The real service pipe: a fresh connection per request, greeted with `Hello`.
pub struct ServicePipe;

#[async_trait::async_trait]
impl PipeLink for ServicePipe {
    async fn exchange(&self, request: &PipeRequest) -> Result<PipeResponse, PipeError> {
        let mut client = PipeClient::connect().await?;
        let hello = PipeRequest::Hello {
            protocol: canvas_proto::pipe::PROTOCOL_VERSION,
            daemon_pid: std::process::id(),
        };
        client.request(&hello).await?;
        client.request(request).await
    }
}

/// Sends one request to the service through the app's pipe.
///
/// A missing pipe is reported as an error, which the API turns into 503
/// `service_unavailable`.
pub async fn send(
    app: &std::sync::Arc<crate::state::App>,
    request: &PipeRequest,
) -> Result<PipeResponse, PipeError> {
    let response = app.pipe.exchange(request).await?;
    // The pipe answered, so the service is up.
    app.service_down_ms.store(0, std::sync::atomic::Ordering::SeqCst);
    Ok(response)
}

/// Tells the service the daemon is quitting on purpose, so it does not restart it (G3).
///
/// The pipe being absent is normal: the service may not exist yet.
pub async fn notify_quitting(app: &std::sync::Arc<crate::state::App>) {
    let _ = app;
    match PipeClient::connect().await {
        Ok(mut client) => {
            let hello = PipeRequest::Hello {
                protocol: canvas_proto::pipe::PROTOCOL_VERSION,
                daemon_pid: std::process::id(),
            };
            if client.request(&hello).await.is_ok() {
                if let Err(e) = client.request(&PipeRequest::DaemonQuitting).await {
                    tracing::debug!(error = %e, "the service did not acknowledge the quit");
                }
            }
        }
        Err(e) => {
            tracing::debug!(error = %e, "the service pipe is not up; nothing to tell");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipe_name_matches_the_spine() {
        assert_eq!(PIPE_NAME, r"\\.\pipe\canvas-rgb-svc");
    }

    #[test]
    fn a_frame_round_trips() {
        let request = PipeRequest::Heartbeat;
        let frame = encode(&request).unwrap();
        let decoded: PipeRequest = canvas_proto::pipe::decode(&frame).unwrap();
        assert_eq!(decoded, request);
    }
}

#[cfg(test)]
mod qos_tests {
    /// The service impersonates the daemon to read an installer (#43); identification level fails that.
    #[test]
    fn the_pipe_client_allows_impersonation() {
        assert_eq!(super::PIPE_QOS, 0x0002_0000 | 0x0010_0000);
    }
}
