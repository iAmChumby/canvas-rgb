//! Blocking client for the daemon's loopback API. Only the tray's background thread and the
//! menu actions call it; nothing here runs on the UI thread for longer than a menu click.

use std::sync::OnceLock;
use std::time::Duration;

use canvas_proto::api::{routes, ApiError as ApiErrorBody, Status};
use canvas_proto::consts;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Connect and read budget. The daemon is on localhost: anything slower is down.
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum ApiError {
    /// The token file is missing or empty.
    Token(String),
    /// The connection failed: the daemon is not running.
    Unreachable,
    /// The daemon answered with an error status, e.g. 503 when the fan service is down.
    Http { status: u16, message: String },
    /// The response did not parse as the expected type.
    Decode(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Token(e) => write!(f, "API token unavailable: {e}"),
            ApiError::Unreachable => write!(f, "daemon not reachable"),
            ApiError::Http { status, message } => write!(f, "HTTP {status}: {message}"),
            ApiError::Decode(e) => write!(f, "unexpected response: {e}"),
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
    CLIENT.get_or_init(|| {
        let config = ureq::Agent::config_builder()
            // Answer 4xx and 5xx with the response, so the API's own message can be shown.
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build();
        Client {
            base: consts::api_base_url(),
            agent: config.into(),
        }
    })
}

/// Reads the token file at call time (G9). Never cached: the daemon may rotate it.
pub fn read_token() -> Result<String, ApiError> {
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
        self.request(Method::Get, routes::STATUS, None, false)
    }

    /// Authenticated `GET`.
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.request(Method::Get, path, None, true)
    }

    /// Authenticated `POST` with a JSON body (`{}` when the route takes none).
    pub fn post<T: Serialize, R: DeserializeOwned>(&self, path: &str, body: &T) -> Result<R, ApiError> {
        self.request(Method::Post, path, Some(encode(body)?), true)
    }

    /// Authenticated `PATCH` with a JSON body.
    pub fn patch<T: Serialize, R: DeserializeOwned>(&self, path: &str, body: &T) -> Result<R, ApiError> {
        self.request(Method::Patch, path, Some(encode(body)?), true)
    }

    fn request<R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<String>,
        authenticated: bool,
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
                req.content_type("application/json").send(body.as_bytes())
            }
        };

        let mut response = response.map_err(|_| ApiError::Unreachable)?;
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
