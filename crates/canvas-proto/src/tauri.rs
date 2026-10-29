//! Tauri command and event names shared by the shell (`app/src-tauri`) and the editor
//! (`app/web`). Mirrored to TypeScript as `app/web/src/proto/tauri.ts` by the spine.
//!
//! G9: the shell reads the API token file and returns it from `GET_API_CONFIG`. The token
//! never enters the Next bundle; the editor asks for it at runtime.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Command: returns [`ApiConfig`].
pub const CMD_GET_API_CONFIG: &str = "get_api_config";
/// Command: opens `%LOCALAPPDATA%\canvas-rgb\logs` in Explorer (G8).
pub const CMD_OPEN_LOGS_FOLDER: &str = "open_logs_folder";

/// Event to the editor: navigate to a page, payload [`NavigatePayload`].
pub const EVT_NAVIGATE: &str = "canvas://navigate";
/// Event to the editor: the tray changed state the editor shows (power, profile, effect);
/// the editor refetches. No payload.
pub const EVT_STATE_CHANGED: &str = "canvas://state-changed";

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiConfig {
    /// `http://127.0.0.1:9430`
    pub base_url: String,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NavigatePayload {
    /// `"layout" | "effects" | "devices" | "fans" | "settings" | "updates"`
    pub page: String,
}

// Auto-update (#21). The tray owns the check, the download, the signature check and the
// installer launch; the editor only shows [`UpdateState`] and sends the person's choices.

/// Command: returns [`UpdateState`].
pub const CMD_UPDATE_STATE: &str = "update_state";
/// Command: checks now, even when the automatic check is off. Returns [`UpdateState`].
pub const CMD_UPDATE_CHECK_NOW: &str = "update_check_now";
/// Command: downloads, checks and runs the installer for the available version. Progress
/// arrives as [`EVT_UPDATE_STATE`].
pub const CMD_UPDATE_INSTALL: &str = "update_install";
/// Command: Later. Hides the editor banner until the next automatic check.
pub const CMD_UPDATE_LATER: &str = "update_later";
/// Command: Skip this version.
pub const CMD_UPDATE_SKIP: &str = "update_skip";
/// Command: undoes Skip this version.
pub const CMD_UPDATE_UNSKIP: &str = "update_unskip";
/// Command: turns the automatic check on or off. Payload [`UpdateSetAuto`].
pub const CMD_UPDATE_SET_AUTO: &str = "update_set_auto";
/// Command: dismisses the "Updated to X.Y.Z" banner.
pub const CMD_UPDATE_DISMISS_UPDATED: &str = "update_dismiss_updated";
/// Event to the editor: the update state changed, payload [`UpdateState`].
pub const EVT_UPDATE_STATE: &str = "canvas://update-state";
/// Tray argument passed by the installer's `/UPDATE` relaunch: start quietly, no window.
pub const AFTER_UPDATE_ARG: &str = "--after-update";

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateSetAuto {
    pub enabled: bool,
}

/// A newer stable release than the running one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateInfo {
    /// `"0.1.1"`, no `v`.
    pub version: String,
    /// The release's CHANGELOG section, Markdown (`### Added`, `- item`).
    pub notes: String,
    /// RFC 3339, when the manifest gives one.
    pub pub_date: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CheckFailReason {
    /// No connection, DNS or TLS failure, timeout.
    Offline,
    /// GitHub answered 403/429 with `x-ratelimit-remaining: 0`.
    RateLimited,
    /// The manifest could not be read, or anything else.
    Other,
}

/// Where the update is right now. One phase at a time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[ts(export)]
pub enum UpdatePhase {
    /// No check has finished in this run.
    Idle,
    Checking,
    UpToDate,
    /// `available` holds the version.
    Available,
    Downloading {
        #[ts(type = "number")]
        downloaded: u64,
        /// Bytes, when the server sent a length.
        #[ts(type = "number | null")]
        total: Option<u64>,
    },
    /// Checking the download against the release key.
    Verifying,
    /// The download failed its check and was deleted. Nothing ran.
    VerifyFailed,
    /// The installer was started; Windows is asking for permission.
    WaitingForUac,
    /// The person said No to the permission prompt. Nothing changed.
    Declined,
    /// The installer is running (the tray is about to be stopped by it).
    Installing,
    CheckFailed { reason: CheckFailReason },
    /// Download or launch failed for a reason other than the signature.
    InstallFailed { message: String },
}

/// Everything the editor shows about updates. Sent by [`CMD_UPDATE_STATE`] and
/// [`EVT_UPDATE_STATE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateState {
    /// The running version, `"0.1.0"`.
    pub current_version: String,
    /// The automatic check (at start, then at most once a day). Check now works either way.
    pub auto_check: bool,
    /// RFC 3339 time of the last finished check.
    pub last_check: Option<String>,
    pub phase: UpdatePhase,
    /// The newer release, when a check found one (kept while skipped).
    pub available: Option<UpdateInfo>,
    /// The version the person skipped.
    pub skipped_version: Option<String>,
    /// True after Later, until the next automatic check: the editor banner stays hidden.
    pub snoozed: bool,
    /// Set after an update until the person dismisses "Updated to X.Y.Z".
    pub updated_to: Option<String>,
}
