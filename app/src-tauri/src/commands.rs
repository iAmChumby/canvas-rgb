//! Commands the editor invokes (G9) and the tray's "Open logs folder" item.

use canvas_proto::api::DiagBundle;
use canvas_proto::consts;
use canvas_proto::tauri::{ApiConfig, EditorChoice, TrayPrefs};
use std::path::PathBuf;
use tauri::{AppHandle, Runtime};
use tauri_plugin_opener::OpenerExt;

use crate::api;
use crate::{gpui_editor, update};

/// Every error string that reaches the editor passes through the shared scrubber (R6). The
/// tray builds no error text of its own that carries a folder or a user name.
fn scrub_error(value: impl std::fmt::Display) -> String {
    canvas_diag::scrub::Scrubber::from_env().scrub(&value.to_string())
}

/// `get_api_config`: the token is read from the file at call time, so the editor never
/// bundles it. The token is returned to the WebView only; it is not logged.
///
/// The token is only handed out when the daemon really holds 127.0.0.1:9430: it is a
/// bearer secret, and the editor sends it to whatever answers on that port.
#[tauri::command]
pub fn get_api_config() -> Result<ApiConfig, String> {
    let token = api::read_token().map_err(scrub_error)?;
    Ok(ApiConfig {
        base_url: consts::api_base_url(),
        token,
    })
}

/// `open_logs_folder`: opens `%LOCALAPPDATA%\canvas-rgb\logs` in Explorer (G8).
#[tauri::command]
pub fn open_logs_folder<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    open_logs(&app).map_err(scrub_error)
}

/// Lists what a diagnostics bundle would hold, with the daemon down (#575). The tray does not
/// need the daemon: it reads the same files through `canvas-diag` as the API route does.
#[tauri::command]
pub fn diag_bundle_list() -> Result<DiagBundle, String> {
    Ok(canvas_diag::bundle::Bundle::new().list())
}

/// Writes a diagnostics bundle and answers with where it went. No daemon, no network.
#[tauri::command]
pub fn diag_bundle_save(path: Option<String>) -> Result<DiagBundle, String> {
    let target = path
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| canvas_diag::paths::home().join("diagnostics"));
    let scrubber = canvas_diag::scrub::Scrubber::from_env();
    canvas_diag::bundle::Bundle::new().write(&target, &scrubber).map_err(scrub_error)
}

/// Reveals the saved zip in Explorer, with the file selected.
#[tauri::command]
pub fn reveal_diag_bundle<R: Runtime>(app: AppHandle<R>, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(PathBuf::from(path))
        .map_err(scrub_error)
}

/// Opens the service's own log folder by its fixed path. Never creates or writes to it; a
/// refused folder comes back with the plain reason from the fail-closed check (R5).
#[tauri::command]
pub fn open_service_logs_folder<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let dir = canvas_diag::paths::machine_log_dir();
    if let Some(reason) = canvas_diag::service_logs::refusal(
        &canvas_diag::service_logs::RealLogFs,
        &dir,
    ) {
        return Err(scrub_error(reason));
    }
    app.opener().open_path(dir.to_string_lossy().into_owned(), None::<&str>).map_err(scrub_error)
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

/// What the tray can put in a report when the daemon is down (item 3, R9): its own version and
/// the OS the process runs on. Hardware and other RGB apps stay "not collected" — no guesses.
#[derive(serde::Serialize)]
pub struct ReportPrefill {
    pub version: String,
    pub windows: String,
}

#[tauri::command]
pub fn report_prefill<R: Runtime>(app: AppHandle<R>) -> Result<ReportPrefill, String> {
    Ok(ReportPrefill {
        version: app.package_info().version.to_string(),
        windows: format!(
            "{} ({})",
            std::env::var("OS").unwrap_or_else(|_| std::env::consts::OS.to_string()),
            std::env::consts::ARCH
        ),
    })
}

/// Shared by the command and the tray item.
pub fn open_logs<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let dir = consts::user_log_dir();
    // No folder in the error: a refused create must not name a person's profile path (#576).
    std::fs::create_dir_all(&dir).map_err(|_| "Could not create the log folder.".to_string())?;
    app.opener()
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(scrub_error)
}
