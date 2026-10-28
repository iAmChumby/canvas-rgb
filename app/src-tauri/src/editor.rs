//! The editor window. Created on demand, destroyed on close (not hidden): a destroyed
//! window takes its WebView2 processes with it, so an idle tray costs no WebView.

use std::time::Duration;

use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

use canvas_proto::tauri::{NavigatePayload, EVT_NAVIGATE, EVT_STATE_CHANGED};

use crate::log;

pub const EDITOR_LABEL: &str = "editor";

/// After the first page load, the navigate event waits this long so the editor's listener is
/// registered first. An event sent to a window with no listener is dropped.
const NAVIGATE_DELAY: Duration = Duration::from_millis(800);

/// Opens the editor, or focuses the open one. `page` (e.g. `"fans"`) is sent as
/// `canvas://navigate` once the editor has loaded. Safe to call from any thread.
pub fn open<R: Runtime>(app: &AppHandle<R>, page: Option<&str>) {
    let handle = app.clone();
    let page = page.map(str::to_string);
    if let Err(e) = app.run_on_main_thread(move || open_on_main(&handle, page)) {
        log::append(&format!("could not schedule the editor window: {e}"));
    }
}

fn open_on_main<R: Runtime>(app: &AppHandle<R>, page: Option<String>) {
    if let Some(window) = app.get_webview_window(EDITOR_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        if let Some(page) = page {
            let _ = window.emit(EVT_NAVIGATE, NavigatePayload { page });
        }
        return;
    }

    let mut builder = WebviewWindowBuilder::new(app, EDITOR_LABEL, WebviewUrl::App("index.html".into()))
        .title("canvas-rgb")
        .inner_size(1280.0, 860.0)
        .min_inner_size(900.0, 600.0)
        .center();

    // The taskbar shows the window's own icon. Without one, Windows falls back to its icon cache
    // for the exe's path, which kept showing the old mark after the icon changed.
    match tauri::image::Image::from_bytes(include_bytes!("../icons/128x128@2x.png")) {
        Ok(icon) => match builder.icon(icon) {
            Ok(b) => builder = b,
            Err(e) => {
                log::append(&format!("could not set the editor window icon: {e}"));
                return;
            }
        },
        Err(e) => log::append(&format!("could not load the editor window icon: {e}")),
    }

    if let Some(page) = page {
        let handle = app.clone();
        builder = builder.on_page_load(move |_window, payload| {
            if payload.event() != PageLoadEvent::Finished {
                return;
            }
            let handle = handle.clone();
            let page = page.clone();
            std::thread::spawn(move || {
                std::thread::sleep(NAVIGATE_DELAY);
                if let Some(window) = handle.get_webview_window(EDITOR_LABEL) {
                    let _ = window.emit(EVT_NAVIGATE, NavigatePayload { page });
                }
            });
        });
    }

    if let Err(e) = builder.build() {
        log::append(&format!("could not create the editor window: {e}"));
    }
}

/// Sends `canvas://state-changed` to the editor when it is open (no payload: the editor refetches).
pub fn notify_state_changed<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(EDITOR_LABEL) {
            let _ = window.emit(EVT_STATE_CHANGED, ());
        }
    });
}
