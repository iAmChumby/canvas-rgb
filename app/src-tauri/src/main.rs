//! canvas-rgb.exe: the tray app. It hosts the editor (a Tauri window created on demand and
//! destroyed on close) and fronts the daemon's loopback API. It does not own the daemon:
//! the fan service supervises the daemon, and the tray only talks to it.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod commands;
#[cfg(test)]
mod config_tests;
mod editor;
mod elevation;
mod log;
mod peer;
mod tray;

use tauri::RunEvent;

fn main() {
    // Done-line: the daemon and the tray never run elevated. Checked before anything else.
    if elevation::is_elevated() {
        elevation::refuse();
        return;
    }

    let app = tauri::Builder::default()
        // Registered first, as the plugin's docs require: a second launch opens the editor
        // in the running tray instead of starting another tray icon.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            editor::open(app, None);
        }))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_api_config,
            commands::open_logs_folder,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            tray::build(&handle)?;
            tray::start(handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building canvas-rgb");

    app.run(|_app, event| {
        // Closing the last editor window must not exit: the tray keeps running with no
        // WebView alive. An explicit exit (Quit) passes a code and goes through.
        if let RunEvent::ExitRequested { code, api, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
