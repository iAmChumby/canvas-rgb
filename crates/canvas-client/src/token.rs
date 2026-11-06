//! Reading the daemon API token.
//!
//! The token is a bearer secret: whoever holds it can drive the lights and the fans. It is
//! only handed out when the daemon really holds the port (see [`crate::peer`]), and it is
//! read from the file at call time, never cached: the daemon may rotate it.

use canvas_proto::consts;

use crate::peer;

/// Why the token could not be read.
#[derive(Debug)]
pub enum TokenError {
    /// The token file is missing or empty.
    Token(String),
    /// The process holding the port is not the daemon (or runs as another user), so the
    /// token is not sent to it.
    NotTrusted(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::Token(e) => write!(f, "API token unavailable: {e}"),
            TokenError::NotTrusted(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for TokenError {}

/// Reads the token file at call time. Never cached: the daemon may rotate it.
///
/// The token is only handed out when the daemon really holds the port: it is a bearer
/// secret and must not reach another process.
pub fn read_token() -> Result<String, TokenError> {
    peer::check(consts::API_PORT).map_err(|refusal| TokenError::NotTrusted(refusal.message()))?;
    let path = consts::token_path();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| TokenError::Token(format!("{}: {e}", path.display())))?;
    let token = text.trim().to_string();
    if token.is_empty() {
        return Err(TokenError::Token(format!("{} is empty", path.display())));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::{caller_sid, trusted, PeerInfo, Refusal};

    /// A client asked to trust a listener held by another process (not `canvas-rgbd.exe`)
    /// is refused, even when that process runs as the same user. Uses the fake
    /// listener-owner seam in `peer` (no socket is opened on the API port) and the same
    /// refusal the token read would pass on.
    #[test]
    fn a_client_refuses_a_listener_held_by_another_process() {
        let me = caller_sid().expect("the caller's own SID");
        let other = PeerInfo {
            pid: 4242,
            port: consts::API_PORT,
            image_name: Some("evil.exe".to_string()),
            sid: Some(me.clone()),
        };
        let refusal = trusted(Some(other), &me).expect_err("held by evil.exe, not the daemon");
        assert_eq!(
            refusal,
            Refusal::HeldBy {
                name: "evil.exe".to_string(),
                port: consts::API_PORT,
            }
        );
        assert_eq!(
            refusal.message(),
            "canvas-rgb daemon not running (port 9430 is held by evil.exe)"
        );
        // The token read surfaces this exact refusal, so the token is never handed out.
        assert_eq!(
            TokenError::NotTrusted(refusal.message()).to_string(),
            refusal.message()
        );
    }
}
