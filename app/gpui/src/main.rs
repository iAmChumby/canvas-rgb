//! `canvas-rgb-editor.exe` — the GPUI editor beside the React editor.
//!
//! It reads the same daemon API as the React editor (through `client.rs`),
//! opens no listening socket, and knows the tray protocol from
//! `canvas_proto::tauri`: one JSON line per message on stdio, capped at
//! `EDITOR_IPC_LINE_MAX`, unknown lines dropped and logged. When stdin is
//! not a pipe (run by hand) the protocol is skipped.
//!
//! Debug flags (for captures, see `tools/ui-capture/capture-gpui.ps1`):
//! - `--page <name>` — checked against `EDITOR_PAGES` (plus `displays` and
//!   `peripherals`, which the React editor routes but the tray map does not
//!   name); an unknown name prints a usage line and exits without a window.
//! - `--theme dark|light|system` — force a theme for captures.
//! - `--min-size WxH` — drop the 900x600 minimum so narrow captures work.
//! - `--size WxH` — the window size to open at.

use std::io::{BufRead, Write};

use canvas_proto::tauri::{EDITOR_IPC_LINE_MAX, EDITOR_PAGES, EditorToTray, TrayToEditor};
use gpui::{Application, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};

mod assets;
mod client;
mod components;
mod fonts;
mod shell;
mod theme;
mod views;

use shell::Shell;
use theme::Theme;
use views::displays::DisplaysView;

struct ParsedArgs {
    page: String,
    theme: Option<Theme>,
    min_size: Option<(u32, u32)>,
    window_size: Option<(u32, u32)>,
}

fn parse_args() -> ParsedArgs {
    let mut page = "layout".to_string();
    let mut theme = None;
    let mut min_size = None;
    let mut window_size = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--page" => {
                if let Some(v) = args.next() {
                    page = v;
                }
            }
            "--theme" => {
                theme = match args.next().as_deref() {
                    Some("light") => Some(Theme::light()),
                    Some("dark") => Some(Theme::dark()),
                    _ => None,
                };
            }
            "--min-size" => {
                min_size = args.next().and_then(|v| {
                    let (w, h) = v.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                });
            }
            "--size" => {
                window_size = args.next().and_then(|v| {
                    let (w, h) = v.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                });
            }
            _ => {}
        }
    }
    ParsedArgs {
        page,
        theme,
        min_size,
        window_size,
    }
}

fn default_theme() -> Theme {
    // The React editor follows Windows by default: dark, light when the
    // system prefers it. GPUI has no OS preference query in 0.2.2; the
    // `--theme` flag forces the answer for captures.
    Theme::dark()
}

fn main() {
    let ParsedArgs {
        page,
        theme: forced_theme,
        min_size,
        window_size,
    } = parse_args();
    if !shell::is_valid_page(&page) {
        eprintln!(
            "usage: canvas-rgb-editor.exe --page <{}|displays|peripherals> [--theme dark|light] [--min-size WxH] [--size WxH]",
            EDITOR_PAGES.join("|")
        );
        std::process::exit(2);
    }

    // The tray protocol: stdin is read on a thread, one line at a time.
    // When stdin is not a pipe (a person runs the exe by hand) skip it.
    let ipc_rx = if stdin_is_a_pipe() {
        Some(spawn_ipc_reader())
    } else {
        None
    };

    let (w, h) = window_size.unwrap_or((1280, 860));
    let theme = forced_theme.unwrap_or_else(default_theme);

    Application::new()
        .with_assets(assets::IconAssets)
        .run(move |cx| {
            fonts::register(cx);
            let bounds = Bounds::centered(None, size(px(w as f32), px(h as f32)), cx);
            let min = min_size.map(|(mw, mh)| size(px(mw as f32), px(mh as f32)));
            if let Some(min) = min {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(min),
                        focus: false,
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|cx| {
                            let displays = cx.new(|cx| DisplaysView::new(theme, cx));
                            Shell::new(theme, page.clone(), None, None, displays, cx)
                        })
                    },
                )
                .unwrap();
            } else {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(size(px(900.0), px(600.0))),
                        focus: false,
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|cx| {
                            let displays = cx.new(|cx| DisplaysView::new(theme, cx));
                            Shell::new(theme, page.clone(), None, None, displays, cx)
                        })
                    },
                )
                .unwrap();
            }
            cx.activate(false);
            if let Some(rx) = ipc_rx {
                // `ready` to the tray, then handle stdin lines.
                send_ready();
                if let Ok(msg) = rx.recv() {
                    match msg {
                        TrayToEditor::Navigate { page } => {
                            eprintln!("canvas-rgb-editor: navigate to {page} (via stdin)");
                        }
                        TrayToEditor::Close => {
                            eprintln!("canvas-rgb-editor: tray asked to close");
                            std::process::exit(0);
                        }
                    }
                }
            }
        });
}

fn stdin_is_a_pipe() -> bool {
    // Run by hand, stdin is the console (a terminal); the tray hands us a pipe
    // (or a redirect), which is not a terminal.
    !std::io::IsTerminal::is_terminal(&std::io::stdin())
}

fn spawn_ipc_reader() -> std::sync::mpsc::Receiver<TrayToEditor> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = std::io::BufReader::new(stdin);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.len() > EDITOR_IPC_LINE_MAX {
                        eprintln!("canvas-rgb-editor: stdin line over {EDITOR_IPC_LINE_MAX} bytes, dropped");
                        continue;
                    }
                    match serde_json::from_str::<TrayToEditor>(trimmed) {
                        Ok(msg) => {
                            if tx.send(msg).is_err() {
                                break;
                            }
                        }
                        Err(e) => eprintln!("canvas-rgb-editor: stdin line dropped: {e}"),
                    }
                }
                Err(e) => {
                    eprintln!("canvas-rgb-editor: stdin read failed: {e}");
                    break;
                }
            }
        }
    });
    rx
}

/// `switch-editor` to the tray over stdio (the Editor radio in Settings).
/// Only when the tray launched us with a piped stdin; standalone runs
/// have no tray to swap with.
pub fn send_switch_editor(to: canvas_proto::tauri::EditorChoice) {
    if !stdin_is_a_pipe() {
        return;
    }
    let mut out = std::io::stdout();
    let line = serde_json::to_string(&EditorToTray::SwitchEditor { to }).unwrap_or_default();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn send_ready() {
    let mut out = std::io::stdout();
    let line = serde_json::to_string(&EditorToTray::Ready).unwrap_or_default();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

#[cfg(test)]
mod ipc_tests {
    use super::*;

    #[test]
    fn ipc_lines_round_trip_through_the_spine_types() {
        let nav = TrayToEditor::Navigate { page: "fans".into() };
        let json = serde_json::to_string(&nav).unwrap();
        assert_eq!(json, r#"{"type":"navigate","page":"fans"}"#);
        let parsed: TrayToEditor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, nav);
        assert_eq!(
            serde_json::to_string(&EditorToTray::SwitchEditor {
                to: canvas_proto::tauri::EditorChoice::React
            })
            .unwrap(),
            r#"{"type":"switch-editor","to":"react"}"#
        );
    }

    #[test]
    fn only_the_navigate_pages_plus_displays_open() {
        for page in EDITOR_PAGES {
            assert!(shell::is_valid_page(page), "{page}");
        }
        assert!(shell::is_valid_page("displays"));
        assert!(shell::is_valid_page("peripherals"));
        assert!(!shell::is_valid_page("nope"));
    }

    #[test]
    fn parse_size_pairs() {
        let (w, h) = "390x844".split_once('x').unwrap();
        assert_eq!((w.parse::<u32>().unwrap(), h.parse::<u32>().unwrap()), (390, 844));
    }

    #[test]
    fn the_spine_names_the_gpui_exe() {
        assert_eq!(canvas_proto::tauri::GPUI_EDITOR_EXE, "canvas-rgb-editor.exe");
        // SharedString is what GPUI elements carry; keep the import live.
        let _s: gpui::SharedString = "x".into();
    }

    #[test]
    fn discarded_stdout_is_harmless() {
        let mut dev_null = std::io::sink();
        let _ = writeln!(dev_null, "{}", serde_json::to_string(&EditorToTray::Ready).unwrap());
        let _ = dev_null.flush();
    }

    #[test]
    fn reading_past_eof_is_empty() {
        use std::io::Read as _;
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let mut buf = [0u8; 8];
        assert_eq!(cursor.read(&mut buf).unwrap(), 0);
    }
}
