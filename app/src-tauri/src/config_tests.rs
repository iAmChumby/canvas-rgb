//! Regression tests for the tray's Tauri configuration.
//!
//! The editor's host bridge (`app/web/src/lib/host.ts`) talks to the tray through
//! `window.__TAURI__`. Tauri 2 only injects that global when `app.withGlobalTauri`
//! is true in `tauri.conf.json`. Without it `isDemo()` returns true and every page
//! renders the built-in demo fixtures instead of the real daemon.

use serde_json::Value;

/// The tray's own config, embedded at compile time so the test cannot drift from it.
const TAURI_CONF: &str = include_str!("../tauri.conf.json");

fn tauri_conf() -> Value {
    serde_json::from_str(TAURI_CONF).expect("app/src-tauri/tauri.conf.json is valid JSON")
}

#[test]
fn config_app_with_global_tauri_is_enabled() {
    let conf = tauri_conf();

    let enabled = conf
        .get("app")
        .and_then(|app| app.get("withGlobalTauri"))
        .and_then(|flag| flag.as_bool());

    assert_eq!(
        enabled,
        Some(true),
        "app.withGlobalTauri must be true in app/src-tauri/tauri.conf.json: \
         Tauri 2 does not inject window.__TAURI__ without it, so app/web/src/lib/host.ts \
         treats the editor as a plain browser, isDemo() returns true, and the editor shows \
         demo fixtures instead of the real daemon"
    );
}

#[test]
fn config_app_object_survives_edits() {
    // A botched edit to the "app" object (the one that carries withGlobalTauri) must not
    // take the rest of the tray's settings with it.
    let app = tauri_conf()
        .get("app")
        .cloned()
        .expect("tauri.conf.json has an \"app\" object");
    assert!(app.is_object(), "tauri.conf.json \"app\" must be a JSON object");
    assert!(
        app.get("security").is_some(),
        "tauri.conf.json app.security (the CSP) must survive edits"
    );
}
