//! Blocking client for the daemon's loopback API. Only the tray's background thread and the
//! menu actions call it; nothing here runs on the UI thread for longer than a menu click.

use std::sync::OnceLock;
use std::time::Duration;

use canvas_proto::api::{routes, ApiError as ApiErrorBody, Status};
use canvas_proto::consts;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::peer;

/// Connect and read budget. The daemon is on localhost: anything slower is down.
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum ApiError {
    /// The token file is missing or empty.
    Token(String),
    /// The connection failed: the daemon is not running.
    Unreachable,
    /// The process holding the port is not the daemon (or runs as another user), so the
    /// token is not sent to it.
    NotTrusted(String),
    /// The daemon answered with an error status, e.g. 503 when the fan service is down.
    Http { status: u16, message: String },
    /// The response did not parse as the expected type.
    Decode(String),
    /// The request may have reached the daemon, but no usable answer came back (a timeout, a reset,
    /// a reply cut off). Whether the daemon acted is UNKNOWN.
    Lost(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Token(e) => write!(f, "API token unavailable: {e}"),
            ApiError::Unreachable => write!(f, "daemon not reachable"),
            ApiError::NotTrusted(message) => write!(f, "{message}"),
            ApiError::Http { status, message } => write!(f, "HTTP {status}: {message}"),
            ApiError::Decode(e) => write!(f, "unexpected response: {e}"),
            ApiError::Lost(why) => write!(f, "no answer from the daemon: {why}"),
        }
    }
}

#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
    Patch,
}

pub struct Client {
    base: String,
    agent: ureq::Agent,
}

/// The one shared client (the agent keeps its connection pool and TLS config).
pub fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| build_client(consts::api_base_url()))
}

fn build_client(base: String) -> Client {
    let config = ureq::Agent::config_builder()
        // Answer 4xx and 5xx with the response, so the API's own message can be shown.
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .build();
    Client { base, agent: config.into() }
}

/// Reads the token file at call time (G9). Never cached: the daemon may rotate it.
///
/// The token is only handed out when the daemon really holds the port: it is a bearer
/// secret and must not reach another process.
pub fn read_token() -> Result<String, ApiError> {
    peer::check(consts::API_PORT).map_err(|refusal| ApiError::NotTrusted(refusal.message()))?;
    let path = consts::token_path();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ApiError::Token(format!("{}: {e}", path.display())))?;
    let token = text.trim().to_string();
    if token.is_empty() {
        return Err(ApiError::Token(format!("{} is empty", path.display())));
    }
    Ok(token)
}

impl Client {
    /// `GET /api/v1/status`: the one route that needs no token.
    pub fn status(&self) -> Result<Status, ApiError> {
        self.request(Method::Get, routes::STATUS, None, false, None)
    }

    /// Authenticated `GET`.
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.request(Method::Get, path, None, true, None)
    }

    /// Authenticated `POST` with a JSON body (`{}` when the route takes none).
    pub fn post<T: Serialize, R: DeserializeOwned>(&self, path: &str, body: &T) -> Result<R, ApiError> {
        self.request(Method::Post, path, Some(encode(body)?), true, None)
    }

    /// Authenticated `POST` that may take longer than the usual budget (the update install: the
    /// service copies and checks a whole installer before it answers).
    pub fn post_slow<T: Serialize, R: DeserializeOwned>(&self, path: &str, body: &T, timeout: Duration) -> Result<R, ApiError> {
        self.request(Method::Post, path, Some(encode(body)?), true, Some(timeout))
    }

    /// Authenticated `PATCH` with a JSON body.
    pub fn patch<T: Serialize, R: DeserializeOwned>(&self, path: &str, body: &T) -> Result<R, ApiError> {
        self.request(Method::Patch, path, Some(encode(body)?), true, None)
    }

    fn request<R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
        authenticated: bool,
        timeout: Option<Duration>,
    ) -> Result<R, ApiError> {
        let url = format!("{}{}", self.base, path);
        let auth = if authenticated {
            Some(format!("Bearer {}", read_token()?))
        } else {
            None
        };
        let body = body.unwrap_or_default();

        let response = match method {
            Method::Get => {
                let mut req = self.agent.get(&url);
                if let Some(auth) = &auth {
                    req = req.header("Authorization", auth);
                }
                req.call()
            }
            Method::Post | Method::Patch => {
                let mut req = match method {
                    Method::Post => self.agent.post(&url),
                    _ => self.agent.patch(&url),
                };
                if let Some(auth) = &auth {
                    req = req.header("Authorization", auth);
                }
                if let Some(timeout) = timeout {
                    req = req.config().timeout_global(Some(timeout)).build();
                }
                req.content_type("application/json").send(body.as_bytes())
            }
        };

        let mut response = response.map_err(|e| match e {
            // Nothing was sent: the connection never opened.
            ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => ApiError::Unreachable,
            ureq::Error::Timeout(ureq::Timeout::Resolve | ureq::Timeout::Connect) => ApiError::Unreachable,
            ureq::Error::Io(ref io) if io.kind() == std::io::ErrorKind::ConnectionRefused => ApiError::Unreachable,
            // Anything else may have happened after the request went out.
            other => ApiError::Lost(other.to_string()),
        })?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap_or_default();
        if status >= 400 {
            let message = serde_json::from_str::<ApiErrorBody>(&text)
                .map(|e| e.message)
                .unwrap_or(text);
            return Err(ApiError::Http { status, message });
        }
        let text = if text.trim().is_empty() { "null".to_string() } else { text };
        serde_json::from_str(&text).map_err(|e| ApiError::Decode(e.to_string()))
    }
}

fn encode<T: Serialize>(body: &T) -> Result<String, ApiError> {
    serde_json::to_string(body).map_err(|e| ApiError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The update fallback rests on this: a daemon that is not listening must read as
    /// `Unreachable` (nothing was sent), never `Lost` (maybe sent), or the tray would not fall back.
    #[test]
    fn a_closed_port_is_unreachable_and_the_updater_falls_back() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        // The listener is dropped: the port is closed.
        let client = build_client(format!("http://127.0.0.1:{port}"));
        let err = client
            .request::<serde_json::Value>(Method::Get, "/api/v1/status", None, false, None)
            .expect_err("nothing listens there");
        assert!(matches!(err, ApiError::Unreachable), "got {err:?}");
        assert_eq!(
            crate::update::service::answer_of(Err(err)),
            crate::update::service::ServiceAnswer::Unavailable
        );
    }
}
