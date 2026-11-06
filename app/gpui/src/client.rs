//! The daemon client: HTTP to the loopback API, with the token only after the owner check.
//!
//! The token is a bearer secret: it is sent only to the process that really holds the
//! API port. [`DaemonClient`] runs the shared owner check from `canvas-client` before
//! reading the token, and re-runs it before every request. Calls block (ureq, the
//! same client and TLS setup as the tray), so callers run them off the UI thread.

use std::time::Duration;

use canvas_client::peer;
use canvas_proto::consts;

/// The API base URL the client talks to: `canvas_proto::consts::api_base_url()`,
/// which follows `CANVAS_RGB_TEST_API_PORT` in debug builds.
pub fn base_url() -> String {
    consts::api_base_url()
}

/// The request deadline, matching the React client's `REQUEST_TIMEOUT_MS`.
pub const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub enum ClientError {
    /// Nothing is listening, or the port is held by another process.
    NotTrusted(String),
    /// The token could not be read after the check passed.
    Token(String),
    /// The daemon answered with an error status and a message.
    Daemon { status: u16, message: String },
    /// The request never got an answer within [`TIMEOUT`] (status 0 in the React client).
    Timeout,
    /// The request could not reach the daemon (status 0, code `unreachable`).
    Unreachable,
    /// The request failed before any answer.
    Other(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::NotTrusted(m) => write!(f, "{m}"),
            ClientError::Token(m) => write!(f, "API token unavailable: {m}"),
            ClientError::Daemon { message, .. } => write!(f, "{message}"),
            ClientError::Timeout => write!(
                f,
                "canvas-rgb did not answer within 10 seconds. It may be stopped, or busy."
            ),
            ClientError::Unreachable => write!(
                f,
                "canvas-rgb did not answer. It may be stopped, or the browser was not allowed to read its reply."
            ),
            ClientError::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ClientError {}

fn daemon_message(body: &str, status: u16) -> String {
    #[derive(serde::Deserialize)]
    struct ErrorBody {
        #[serde(default)]
        message: Option<String>,
    }
    if let Ok(parsed) = serde_json::from_str::<ErrorBody>(body) {
        if let Some(message) = parsed.message {
            return message;
        }
    }
    format!("HTTP {status}")
}

fn token_for(base: &str) -> Result<String, ClientError> {
    #[cfg(debug_assertions)]
    if test_home_on_real_port(std::env::var_os("CANVAS_RGB_HOME").is_some(), base) {
        return Err(ClientError::NotTrusted(format!(
            "CANVAS_RGB_HOME points at a test home but the API base is {base}, the real daemon:              set CANVAS_RGB_TEST_API_PORT to the fake daemon's port"
        )));
    }
    peer::check_base(base).map_err(|refusal| ClientError::NotTrusted(refusal.message()))?;
    // `canvas_client::token::read_token` checks the fixed port 9430 and reads the real
    // path, so it cannot serve a debug run on the test port. The owner check above is
    // the same one it runs; the token is read only after it passes.
    let path = token_path();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| ClientError::Token(format!("{}: {e}", path.display())))?;
    let jwt = text.trim().to_string();
    if jwt.is_empty() {
        return Err(ClientError::Token(format!("{} is empty", path.display())));
    }
    Ok(jwt)
}

/// A test home (`CANVAS_RGB_HOME`) with the real API port: a debug run meant for the fakes
/// that would talk to the owner's real daemon. Refused before anything is sent (#239).
#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn test_home_on_real_port(home_set: bool, base: &str) -> bool {
    let port = base
        .trim_end_matches('/')
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<u16>().ok());
    home_set && port.is_none_or(|p| p == consts::API_PORT)
}

/// The token file. In debug builds a fakes daemon's home (`CANVAS_RGB_HOME`, see
/// `tools/run-fake-daemon.ps1`) overrides it, matching the daemon's fakes mode, which
/// exists only in debug builds. Release builds always read the real path.
fn token_path() -> std::path::PathBuf {
    #[cfg(debug_assertions)]
    if let Some(home) = std::env::var_os("CANVAS_RGB_HOME") {
        return std::path::PathBuf::from(home).join(consts::TOKEN_FILE);
    }
    consts::token_path()
}

pub enum Method {
    Get,
    Put,
    Patch,
    Delete,
}

/// One small client for the daemon's loopback API.
pub struct DaemonClient {
    base: String,
    agent: ureq::Agent,
}

impl DaemonClient {
    pub fn new() -> Self {
        Self::with_base(base_url())
    }

    pub fn with_base(base: String) -> Self {
        let config = ureq::Agent::config_builder()
            // Answer 4xx and 5xx with the response, so the API's own message can be shown.
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build();
        DaemonClient {
            base,
            agent: config.into(),
        }
    }

    pub fn get(&self, path: &str) -> Result<String, ClientError> {
        self.request(Method::Get, path, None)
    }

    pub fn put_json(&self, path: &str, body: &str) -> Result<String, ClientError> {
        self.request(Method::Put, path, Some(body))
    }

    /// `PATCH` with a JSON body: the fan header's mode, the fan profile switch.
    pub fn patch_json(&self, path: &str, body: &str) -> Result<String, ClientError> {
        self.request(Method::Patch, path, Some(body))
    }

    pub fn delete(&self, path: &str) -> Result<String, ClientError> {
        self.request(Method::Delete, path, None)
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> Result<String, ClientError> {
        // The owner check runs before the token is read, and therefore before
        // anything is sent: a port held by another process never sees it.
        let jwt = self.token()?;
        let url = format!("{}{}", self.base, path);
        let auth = format!("Bearer {jwt}");
        let result = match method {
            Method::Get => self.agent.get(&url).header("Authorization", auth).call(),
            Method::Put => self
                .agent
                .put(&url)
                .header("Authorization", auth)
                .content_type("application/json")
                .send(body.unwrap_or_default().as_bytes()),
            Method::Patch => self
                .agent
                .patch(&url)
                .header("Authorization", auth)
                .content_type("application/json")
                .send(body.unwrap_or_default().as_bytes()),
            Method::Delete => self.agent.delete(&url).header("Authorization", auth).call(),
        };
        let mut response = result.map_err(|e| match e {
            ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => ClientError::Unreachable,
            ureq::Error::Timeout(_) => ClientError::Timeout,
            ureq::Error::StatusCode(status) => ClientError::Daemon {
                status,
                message: format!("HTTP {status}"),
            },
            other => ClientError::Other(other.to_string()),
        })?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap_or_default();
        if (200..300).contains(&status) {
            Ok(text)
        } else {
            Err(ClientError::Daemon {
                status,
                message: daemon_message(&text, status),
            })
        }
    }

    fn token(&self) -> Result<String, ClientError> {
        token_for(&self.base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #239: a capture run set the fakes' home but not the test port, and the editor
    /// talked to the real daemon on 9430. A test home with the real port is refused.
    #[test]
    fn a_test_home_never_talks_to_the_real_port() {
        assert!(test_home_on_real_port(true, "http://127.0.0.1:9430"));
        assert!(test_home_on_real_port(true, "http://127.0.0.1:9430/"));
        assert!(test_home_on_real_port(true, "http://localhost"));
        assert!(!test_home_on_real_port(true, "http://127.0.0.1:19430"));
        assert!(!test_home_on_real_port(false, "http://127.0.0.1:9430"));
    }

    #[test]
    fn daemon_message_prefers_the_daemon_text() {
        assert_eq!(
            daemon_message(
                r#"{"error":"bad","message":"decimals must be 0 or 1, got 2"}"#,
                422
            ),
            "decimals must be 0 or 1, got 2"
        );
        assert_eq!(daemon_message("not json", 500), "HTTP 500");
        assert_eq!(daemon_message("", 500), "HTTP 500");
    }

    #[test]
    fn base_url_follows_the_debug_test_port() {
        // `api_base_url()` reads `CANVAS_RGB_TEST_API_PORT` in debug builds;
        // in this test the variable is unset, so it is the real port.
        let base = base_url();
        assert!(base.starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn a_refused_owner_check_sends_nothing() {
        // Nothing listens on an unused loopback port: the check refuses, so the
        // token is never even read, let alone sent.
        let client = DaemonClient::with_base("http://127.0.0.1:9".into());
        let result = client.get("/api/v1/displays");
        match result {
            Err(ClientError::NotTrusted(_)) => {}
            Ok(_) => panic!("expected NotTrusted, got an answer"),
            Err(other) => panic!("expected NotTrusted, got: {other:?}"),
        }
    }
}
