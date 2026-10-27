//! Commands the editor invokes (G9) and the tray's "Open logs folder" item.

use canvas_proto::consts;
use canvas_proto::tauri::ApiConfig;
use tauri::{AppHandle, Runtime};
use tauri_plugin_opener::OpenerExt;

use crate::api;

/// `get_api_config`: the token is read from the file at call time, so the editor never
/// bundles it. The token is returned to the WebView only; it is not logged.
///
/// The token is only handed out when the daemon really holds 127.0.0.1:9430: it is a
/// bearer secret, and the editor sends it to whatever answers on that port.
#[tauri::command]
pub fn get_api_config() -> Result<ApiConfig, String> {
    let token = api::read_token().map_err(|e| e.to_string())?;
    Ok(ApiConfig {
        base_url: consts::api_base_url(),
        token,
    })
}

/// `open_logs_folder`: opens `%LOCALAPPDATA%\canvas-rgb\logs` in Explorer (G8).
#[tauri::command]
pub fn open_logs_folder<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    open_logs(&app)
}

/// Shared by the command and the tray item.
pub fn open_logs<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let dir = consts::user_log_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    app.opener()
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| e.to_string())
}
