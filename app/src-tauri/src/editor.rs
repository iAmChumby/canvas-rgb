//! The editor window. Created on demand, destroyed on close (not hidden): a destroyed
//! window takes its WebView2 processes with it, so an idle tray costs no WebView.
//!
//! The React editor is the default. On an experimental build the tray may open the GPUI editor
//! instead (#202); this module opens whichever one the tray's pref names, and owns the swap
//! between them.

use std::sync::Mutex;
use std::time::Duration;

use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

use canvas_proto::tauri::{EditorChoice, NavigatePayload, EVT_NAVIGATE, EVT_STATE_CHANGED};

use crate::log;
use crate::gpui_editor;

pub const EDITOR_LABEL: &str = "editor";

/// After the first page load, the navigate event waits this long so the editor's listener is
/// registered first. An event sent to a window with no listener is dropped.
const NAVIGATE_DELAY: Duration = Duration::from_millis(800);

/// The page the editor was last asked to open. The tray keeps it so that a swap between the
/// editors opens the other one on the same page. (The editor does not report the page it is on
/// back to the tray; this is the page the tray last asked for.)
static LAST_PAGE: Mutex<Option<String>> = Mutex::new(None);

/// The page the editor was last asked to open.
pub fn last_page() -> Option<String> {
    LAST_PAGE.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Remembers the page the editor was asked to open.
fn remember_page(page: Option<&str>) {
    let Some(page) = page else {
        return;
    };
    *LAST_PAGE.lock().unwrap_or_else(|p| p.into_inner()) = Some(page.to_string());
}

/// Opens the editor, or focuses the open one. `page` (e.g. `"fans"`) is sent as
/// `canvas://navigate` once the editor has loaded. Safe to call from any thread.
///
/// The editor that opens comes back: the React one, the GPUI one, or React because the GPUI
/// editor could not be used.
pub fn open<R: Runtime>(app: &AppHandle<R>, page: Option<&str>) -> EditorChoice {
    remember_page(page);
    match gpui_editor::effective_editor(app) {
        EditorChoice::React => {
            open_react(app, page);
            EditorChoice::React
        }
        EditorChoice::Gpui => gpui_editor::open(app, page),
    }
}

/// Opens the React editor, or focuses the open one.
pub fn open_react<R: Runtime>(app: &AppHandle<R>, page: Option<&str>) {
    let handle = app.clone();
    let page = page.map(str::to_string);
    if let Err(e) = app.run_on_main_thread(move || open_on_main(&handle, page)) {
        log::append(&format!("could not schedule the editor window: {e}"));
    }
}

/// Closes the React editor window, if it is open.
pub fn close_react<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(EDITOR_LABEL) {
            let _ = window.close();
        }
    }) {
        log::append(&format!("could not close the editor window: {e}"));
    }
}

/// Closes the editor that is open and opens `to` on the same page.
pub fn switch_editor<R: Runtime>(app: &AppHandle<R>, to: EditorChoice, page: Option<&str>) {
    close_react(app);
    gpui_editor::close(app);
    match to {
        EditorChoice::React => open_react(app, page),
        EditorChoice::Gpui => {
            gpui_editor::open(app, page);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_the_tray_asked_for_is_remembered() {
        let before = last_page();
        remember_page(Some("fans"));
        assert_eq!(last_page().as_deref(), Some("fans"));
        // Opening without a page does not forget the page the editor is on.
        remember_page(None);
        assert_eq!(last_page().as_deref(), Some("fans"));
        remember_page(Some("updates"));
        assert_eq!(last_page().as_deref(), Some("updates"));
        *LAST_PAGE.lock().unwrap_or_else(|p| p.into_inner()) = before;
    }
}
