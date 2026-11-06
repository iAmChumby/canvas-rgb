//! Commands the editor invokes (G9) and the tray's "Open logs folder" item.

use canvas_proto::consts;
use canvas_proto::tauri::{ApiConfig, EditorChoice, TrayPrefs};
use tauri::{AppHandle, Runtime};
use tauri_plugin_opener::OpenerExt;

use crate::api;
use crate::{gpui_editor, update};

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

/// `set_editor`: chooses the editor the tray opens (#202).
///
/// Only an experimental build has the choice: a stable build refuses it, and the pref it has is
/// ignored (React always opens there). The tray keeps the choice, and the open editor closes
/// and the other one opens on the same page.
#[tauri::command]
pub fn set_editor<R: Runtime>(app: AppHandle<R>, editor: EditorChoice) -> Result<TrayPrefs, String> {
    let Some(ctl) = update::try_controller(&app) else {
        return Err("the tray is not ready yet".to_string());
    };
    // The editor that is open now, before the pref changes.
    let previous = gpui_editor::effective_editor(&app);
    let prefs = ctl.set_editor(editor)?;
    if previous != editor {
        crate::editor::switch_editor(&app, editor, crate::editor::last_page().as_deref());
    }
    Ok(prefs)
}

/// Shared by the command and the tray item.
pub fn open_logs<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let dir = consts::user_log_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    app.opener()
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| e.to_string())
}
