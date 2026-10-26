//! Daemon ↔ service named pipe `\\.\pipe\canvas-rgb-svc`.
//!
//! ACL: the interactive user and SYSTEM only. Framing: each message is a `u32`
//! little-endian byte length followed by that many bytes of JSON. One request, one
//! response, in order. Messages over [`MAX_FRAME`] are a protocol error.

use crate::color::Rgb8;
use crate::fans::{FanMode, FanProfile, FansStatus, HeaderId, TempSource};
use crate::config::FansFile;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub const PIPE_NAME: &str = r"\\.\pipe\canvas-rgb-svc";
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME: u32 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum PipeRequest {
    Hello { protocol: u32, daemon_pid: u32 },
    /// Sent every second; the service supervises the daemon by it (G3).
    Heartbeat,
    FanStatus,
    SetHeaderMode { header: HeaderId, mode: FanMode },
    SetProfile { name: String },
    PutProfile { profile: FanProfile },
    /// Refused with `conflict` for the active profile or the last one.
    DeleteProfile { name: String },
    /// Replaces `%ProgramData%\canvas-rgb\fans.toml` (daemon is the writer).
    PutFanConfig { config: FansFile },
    OpenRgbState,
    RestartOpenRgb,
    /// G5: static color the service writes per device until the daemon connects.
    SetBootColor { color: Rgb8 },
    /// Diagnostics: treat this source as failed (safety test, `diag fail-sensor`).
    FailSensor { source: TempSource, failed: bool },
    /// Daemon is quitting on purpose; do not restart it (G3).
    DaemonQuitting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRgbServerState {
    NotStarted,
    Starting,
    Running,
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result")]
pub enum PipeResponse {
    Hello { protocol: u32, service_version: String },
    Ok,
    FanStatus { status: FansStatus },
    OpenRgb { state: OpenRgbServerState, port: u16 },
    /// `code` uses [`crate::api::error_codes`].
    Error { code: String, message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame of {0} bytes exceeds the limit")]
    TooLarge(u32),
    #[error("truncated frame")]
    Truncated,
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Encodes one message as length-prefixed JSON.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(msg)?;
    let len = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge(u32::MAX))?;
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Reads the length prefix. Callers then read exactly that many bytes and call [`decode_body`].
pub fn decode_len(prefix: [u8; 4]) -> Result<u32, FrameError> {
    let len = u32::from_le_bytes(prefix);
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    Ok(len)
}

pub fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, FrameError> {
    Ok(serde_json::from_slice(body)?)
}

/// Decodes one whole frame (prefix + body) from a buffer.
pub fn decode<T: DeserializeOwned>(buf: &[u8]) -> Result<T, FrameError> {
    let prefix: [u8; 4] = buf.get(..4).ok_or(FrameError::Truncated)?.try_into().unwrap();
    let len = decode_len(prefix)? as usize;
    let body = buf.get(4..4 + len).ok_or(FrameError::Truncated)?;
    decode_body(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let req = PipeRequest::SetProfile { name: "Quiet".into() };
        let bytes = encode(&req).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize, bytes.len() - 4);
        assert_eq!(decode::<PipeRequest>(&bytes).unwrap(), req);
        assert!(matches!(decode::<PipeRequest>(&bytes[..6]), Err(FrameError::Truncated)));
    }
}
