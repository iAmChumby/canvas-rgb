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
    /// `"layout" | "effects" | "devices" | "fans" | "settings" | "updates"` ([`EDITOR_PAGES`])
    pub page: String,
}

/// The pages a navigate request may name: exactly the ones the React editor routes
/// (`app/web/src/shell/Shell.tsx` `PAGE_PATHS`). The GPUI editor accepts the same set.
pub const EDITOR_PAGES: &[&str] = &["layout", "effects", "devices", "fans", "settings", "updates"];

// Editor choice and update channel (#202, #203). The tray keeps them in [`TRAY_PREFS_FILE`].

/// Which editor the tray opens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum EditorChoice {
    #[default]
    React,
    Gpui,
}

/// Which update feed the tray checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum UpdateChannel {
    #[default]
    Stable,
    Experimental,
}

/// The tray's preferences. A file without a field reads it as its default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TrayPrefs {
    #[serde(default)]
    pub editor: EditorChoice,
    #[serde(default)]
    pub channel: UpdateChannel,
}

/// In `%LOCALAPPDATA%\canvas-rgb\tray\`, beside `update.json`.
pub const TRAY_PREFS_FILE: &str = "prefs.json";
/// Command: returns [`TrayPrefs`].
pub const CMD_TRAY_PREFS: &str = "tray_prefs";
/// Command: `{ editor: EditorChoice }`.
pub const CMD_SET_EDITOR: &str = "set_editor";
/// Command: `{ channel: UpdateChannel }`.
pub const CMD_SET_CHANNEL: &str = "set_channel";

// Tray <-> GPUI editor, one JSON object per line on the editor's stdio (#202).

/// The tray to the GPUI editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TrayToEditor {
    /// `page` is one of [`EDITOR_PAGES`]; the editor ignores any other.
    Navigate { page: String },
    Close,
}

/// The GPUI editor to the tray.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum EditorToTray {
    Ready,
    SwitchEditor { to: EditorChoice },
}

/// The longest stdio line either side accepts; a longer one is dropped and logged.
pub const EDITOR_IPC_LINE_MAX: usize = 4096;
/// The GPUI editor's executable, beside the tray's.
pub const GPUI_EDITOR_EXE: &str = "canvas-rgb-editor.exe";

#[cfg(test)]
mod gpui_ipc_tests {
    use super::*;

    #[test]
    fn prefs_default_and_old_files() {
        let p: TrayPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p, TrayPrefs { editor: EditorChoice::React, channel: UpdateChannel::Stable });
        let s = serde_json::to_string(&TrayPrefs { editor: EditorChoice::Gpui, channel: UpdateChannel::Experimental }).unwrap();
        assert_eq!(s, r#"{"editor":"gpui","channel":"experimental"}"#);
    }

    #[test]
    fn ipc_lines_are_kebab_tagged_and_strict() {
        assert_eq!(serde_json::to_string(&TrayToEditor::Navigate { page: "fans".into() }).unwrap(), r#"{"type":"navigate","page":"fans"}"#);
        assert_eq!(serde_json::to_string(&EditorToTray::SwitchEditor { to: EditorChoice::React }).unwrap(), r#"{"type":"switch-editor","to":"react"}"#);
        // A field a variant does not have is refused. (serde does not enforce this for unit
        // variants of an internally tagged enum: `{"type":"close","extra":1}` parses as Close.)
        assert!(serde_json::from_str::<TrayToEditor>(r#"{"type":"navigate","page":"fans","extra":1}"#).is_err());
        assert!(serde_json::from_str::<EditorToTray>(r#"{"type":"switch-editor","to":"gpui","x":0}"#).is_err());
        assert!(serde_json::from_str::<TrayToEditor>(r#"{"type":"open"}"#).is_err());
        assert!(serde_json::from_str::<EditorToTray>(r#"{"type":"ready"}"#).is_ok());
    }

    #[test]
    fn editor_pages_are_the_navigate_pages() {
        assert_eq!(EDITOR_PAGES, &["layout", "effects", "devices", "fans", "settings", "updates"]);
    }
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
    /// Set after an update until the person dismisses "Updated to X.Y.Z". Carries the installed
    /// release's notes so What's new has something to show once `available` is gone.
    pub updated_to: Option<UpdateInfo>,
}
