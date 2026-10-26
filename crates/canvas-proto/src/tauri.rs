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
    /// `"layout" | "effects" | "devices" | "fans" | "settings"`
    pub page: String,
}
