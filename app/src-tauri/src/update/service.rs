//! Asking the service to install (#43). The tray never opens the service pipe: it posts to the
//! daemon's `update/install` route (after the usual check of who holds port 9430) and the daemon
//! forwards it. The service runs setup as SYSTEM, so there is no permission prompt and the tray
//! comes back as the user.

use std::path::Path;
use std::time::Duration;

use canvas_proto::api::{routes, InstallUpdateRequest};

use crate::api::{self, ApiError};

/// The service copies and checks a whole installer before it answers. Longer than the usual
/// budget, because a timeout here must not be mistaken for "no service" (see [`answer_of`]).
const INSTALL_TIMEOUT: Duration = Duration::from_secs(120);

/// What asking the service came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceAnswer {
    /// Setup is running for this version (`202`). The tray's part is over.
    Started,
    /// The request certainly never reached the service (a refused or failed connection before
    /// anything was sent, a `503`, an untrusted listener, no token): start the installer the old
    /// way and let setup raise its own permission prompt.
    Unavailable,
    /// The service looked at the file and said no (`422`, `409`, or any other refusal). No
    /// fallback.
    Refused(String),
    /// The request went out and the outcome is unknown (a timeout, a reset, a reply lost because
    /// setup stopped the daemon). Setup may be running, so the tray starts nothing and keeps
    /// `pending`.
    Unknown(String),
}

/// Asks the service, through the daemon, to install `installer` signed by `signature`.
pub type ServiceAsker = std::sync::Arc<dyn Fn(&Path, &str) -> ServiceAnswer + Send + Sync>;

/// The real asker.
pub fn ask_daemon(installer: &Path, signature: &str) -> ServiceAnswer {
    let body = InstallUpdateRequest {
        installer: installer.to_string_lossy().into_owned(),
        signature: signature.trim().to_string(),
    };
    let answer = api::client().post_slow::<_, serde_json::Value>(routes::UPDATE_INSTALL, &body, INSTALL_TIMEOUT);
    answer_of(answer)
}

/// Maps the daemon's answer to what the updater does next. The fallback is only for a request
/// that certainly never reached the service: a second setup beside a running one is worse than a
/// failed update.
pub fn answer_of(answer: Result<serde_json::Value, ApiError>) -> ServiceAnswer {
    match answer {
        Ok(_) => ServiceAnswer::Started,
        // The daemon could not open the service pipe.
        Err(ApiError::Http { status: 503, .. }) => ServiceAnswer::Unavailable,
        // The daemon's pipe exchange broke after the pipe opened: the service may have started setup.
        Err(ApiError::Http { status: 502, message }) => ServiceAnswer::Unknown(message),
        Err(ApiError::Http { message, .. }) => ServiceAnswer::Refused(message),
        Err(ApiError::Lost(why)) => ServiceAnswer::Unknown(why),
        Err(ApiError::Decode(why)) => ServiceAnswer::Unknown(why),
        Err(ApiError::Unreachable | ApiError::NotTrusted(_) | ApiError::Token(_)) => ServiceAnswer::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16) -> Result<serde_json::Value, ApiError> {
        Err(ApiError::Http { status, message: format!("m{status}") })
    }

    #[test]
    fn the_daemons_answers_map_to_the_updaters_next_step() {
        assert_eq!(answer_of(Ok(serde_json::json!({"version": "0.2.1"}))), ServiceAnswer::Started);
        // Certainly never reached the service: fall back.
        assert_eq!(answer_of(http(503)), ServiceAnswer::Unavailable);
        assert_eq!(answer_of(Err(ApiError::Unreachable)), ServiceAnswer::Unavailable);
        assert_eq!(answer_of(Err(ApiError::NotTrusted("x".into()))), ServiceAnswer::Unavailable);
        assert_eq!(answer_of(Err(ApiError::Token("x".into()))), ServiceAnswer::Unavailable);
        // The service answered no: end the update, never fall back.
        assert_eq!(answer_of(http(422)), ServiceAnswer::Refused("m422".into()));
        assert_eq!(answer_of(http(409)), ServiceAnswer::Refused("m409".into()));
        assert_eq!(answer_of(http(500)), ServiceAnswer::Refused("m500".into()));
        assert_eq!(answer_of(http(403)), ServiceAnswer::Refused("m403".into()));
        // The request went out and the outcome is unknown: never fall back.
        assert_eq!(answer_of(http(502)), ServiceAnswer::Unknown("m502".into()));
        assert_eq!(answer_of(Err(ApiError::Lost("reset".into()))), ServiceAnswer::Unknown("reset".into()));
        assert!(matches!(answer_of(Err(ApiError::Decode("x".into()))), ServiceAnswer::Unknown(_)));
    }
}
