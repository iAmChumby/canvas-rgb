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
    read_token_with(|| peer::check(consts::API_PORT), &consts::token_path())
}

/// [`read_token`] with the owner check and the token path passed in, so tests can prove the
/// check runs first and a refusal never reads the file.
fn read_token_with(
    check: impl FnOnce() -> Result<(), peer::Refusal>,
    path: &std::path::Path,
) -> Result<String, TokenError> {
    check().map_err(|refusal| TokenError::NotTrusted(refusal.message()))?;
    let text = std::fs::read_to_string(path)
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
    }

    /// A token file in the temp folder, removed when dropped.
    struct TempToken(std::path::PathBuf);
    impl TempToken {
        fn new(name: &str, text: &str) -> Self {
            let path = std::env::temp_dir().join(format!("canvas-client-{}-{name}", std::process::id()));
            std::fs::write(&path, text).expect("write the temp token file");
            TempToken(path)
        }
    }
    impl Drop for TempToken {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// A refused owner check stops the token read before the file is touched: the token
    /// file holds a real-looking token, and it is never returned.
    #[test]
    fn a_refused_check_never_reads_the_token() {
        let file = TempToken::new("refused", "secret-token");
        let refusal = Refusal::HeldBy {
            name: "evil.exe".to_string(),
            port: consts::API_PORT,
        };
        let expected = refusal.message();
        let err = read_token_with(|| Err(refusal), &file.0).expect_err("refused listener");
        match err {
            TokenError::NotTrusted(message) => assert_eq!(message, expected),
            other => panic!("expected NotTrusted, got {other:?}"),
        }
    }

    /// With the check passed, the token is read and trimmed; an empty file is refused.
    #[test]
    fn a_trusted_check_reads_and_trims_the_token() {
        let file = TempToken::new("ok", "  abc123\r\n");
        assert_eq!(read_token_with(|| Ok(()), &file.0).expect("token"), "abc123");
        let empty = TempToken::new("empty", " \n");
        assert!(matches!(read_token_with(|| Ok(()), &empty.0), Err(TokenError::Token(_))));
    }
}
