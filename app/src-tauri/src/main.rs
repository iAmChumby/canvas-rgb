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
mod onboard;
mod peer;
mod tray;
mod update;

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
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if update::has_after_update_arg(&args) {
                // The installer's relaunch: stay quiet, never open the editor. Setup may have ended
                // before it stopped this tray; read what it reported.
                update::forwarded_after_update(app);
            } else if onboard::has_onboard_arg(&args) {
                tray::open_onboarding(app);
            } else {
                editor::open(app, None);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        // Verify-only (see update/mod.rs). Its own commands stay out of the editor's capability file.
        .plugin(tauri_plugin_updater::Builder::new().pubkey(update::rules::PUBKEY).build())
        .invoke_handler(tauri::generate_handler![
            commands::get_api_config,
            commands::open_logs_folder,
            update::update_state,
            update::update_check_now,
            update::update_install,
            update::update_later,
            update::update_skip,
            update::update_unskip,
            update::update_set_auto,
            update::update_dismiss_updated,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let args: Vec<String> = std::env::args().collect();
            update::init(&handle, &args);
            tray::build(&handle)?;
            if update::has_after_update_arg(&args) {
                // `--after-update` (the installer's /UPDATE relaunch) opens no window, not even the onboarding.
                tray::stay_quiet(&handle);
            } else if onboard::has_onboard_arg(&args) {
                // `--onboard` (the installer launch) opens the window now, not on a poll.
                tray::open_onboarding(&handle);
            }
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
