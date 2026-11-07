//! `canvas-rgb-editor.exe` — the GPUI editor beside the React editor.
//!
//! It reads the same daemon API as the React editor (through `client.rs`),
//! opens no listening socket, and knows the tray protocol from
//! `canvas_proto::tauri`: one JSON line per message on stdio, capped at
//! `TRAY_TO_EDITOR_LINE_MAX` in, unknown lines dropped and logged, and
//! `EDITOR_IPC_LINE_MAX` out. When stdin is not a pipe (run by hand) the
//! protocol is skipped.
//!
//! Debug flags (for captures, see `tools/ui-capture/capture-gpui.ps1`):
//! - `--page <name>` — checked against `EDITOR_PAGES` (plus `displays` and
//!   `peripherals`, which the React editor routes but the tray map does not
//!   name); an unknown name prints a usage line and exits without a window.
//! - `--theme dark|light|system` — force a theme for captures.
//! - `--min-size WxH` — drop the 900x600 minimum so narrow captures work.
//! - `--size WxH` — the window size to open at.

use std::io::{BufRead, BufReader, Write};

use canvas_proto::tauri::{
    EDITOR_PAGES, EditorChoice, EditorToTray, TRAY_TO_EDITOR_LINE_MAX, TrayToEditor,
    UpdateChannel,
};
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
    let tray_present = stdin_is_a_pipe();
    let ipc_rx = if tray_present {
        Some(spawn_ipc_reader())
    } else {
        None
    };

    let (w, h) = window_size.unwrap_or((1280, 860));
    let theme = forced_theme.unwrap_or_else(default_theme);
    let min_size = min_size.or(Some((900, 600)));

    Application::new()
        .with_assets(assets::IconAssets)
        .run(move |cx| {
            fonts::register(cx);
            let bounds = Bounds::centered(None, size(px(w as f32), px(h as f32)), cx);
            let min = min_size.map(|(mw, mh)| size(px(mw as f32), px(mh as f32)));
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: min,
                    focus: false,
                    ..Default::default()
                },
                move |_, cx| {
                    cx.new(|cx| {
                        let displays = cx.new(|cx| DisplaysView::new(theme, cx));
                        Shell::new(
                            theme,
                            page.clone(),
                            None,
                            None,
                            displays,
                            cx,
                            tray_present,
                            ipc_rx,
                        )
                    })
                },
            )
            .unwrap();
            cx.activate(false);
            if tray_present {
                send_ready();
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
        run_ipc_reader(BufReader::new(stdin), tx);
    });
    rx
}

/// Why one line from the tray was dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CappedError {
    TooLong,
    NotUtf8,
    Io,
}

/// Reads one line from `reader`, never holding more than `max` bytes.
///
/// A longer line is dropped (the rest is thrown away) and a non-UTF-8 line
/// is dropped too; `Ok(None)` is EOF. Mirrors the tray's `read_capped_line`
/// (`app/src-tauri/src/gpui_editor.rs`): the old `read_line` buffered a whole
/// line before the cap check and `break`ed on invalid UTF-8, after which the
/// editor never heard `close` or state again.
fn read_capped_line<R: BufRead>(reader: &mut R, max: usize) -> Result<Option<String>, CappedError> {
    let mut buf: Vec<u8> = Vec::new();
    let mut over = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(CappedError::Io),
        };
        if available.is_empty() {
            if buf.is_empty() && !over {
                return Ok(None);
            }
            break;
        }
        let newline = available.iter().position(|b| *b == b'\n');
        let take = newline.unwrap_or(available.len());
        if !over {
            if buf.len() + take > max {
                over = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&available[..take]);
            }
        }
        let consumed = match newline {
            Some(at) => at + 1,
            None => available.len(),
        };
        reader.consume(consumed);
        if newline.is_some() {
            break;
        }
    }
    if over {
        return Err(CappedError::TooLong);
    }
    let buf = match buf.last() {
        Some(b'\r') => &buf[..buf.len() - 1],
        _ => &buf[..],
    };
    match String::from_utf8(buf.to_vec()) {
        Ok(line) => Ok(Some(line)),
        Err(_) => Err(CappedError::NotUtf8),
    }
}

/// Reads tray lines until EOF or an I/O error, forwarding each known message.
/// Over-long and non-UTF-8 lines are dropped and logged, and reading goes on.
fn run_ipc_reader<R: BufRead>(mut reader: R, tx: std::sync::mpsc::Sender<TrayToEditor>) {
    loop {
        match read_capped_line(&mut reader, TRAY_TO_EDITOR_LINE_MAX) {
            Ok(None) => break,
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
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
            Err(CappedError::TooLong) => {
                eprintln!("canvas-rgb-editor: stdin line over {TRAY_TO_EDITOR_LINE_MAX} bytes, dropped")
            }
            Err(CappedError::NotUtf8) => {
                eprintln!("canvas-rgb-editor: stdin line was not valid UTF-8, dropped")
            }
            Err(CappedError::Io) => {
                eprintln!("canvas-rgb-editor: stdin read failed");
                break;
            }
        }
    }
}

/// One line to the tray over stdio. Only when the tray launched us with a
/// piped stdin; standalone runs have no tray to talk to.
fn send_line(msg: &EditorToTray) {
    if !stdin_is_a_pipe() {
        return;
    }
    let mut out = std::io::stdout();
    let line = serde_json::to_string(msg).unwrap_or_default();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// `switch-editor` to the tray over stdio (the Editor radio in Settings).
/// Only when the tray launched us with a piped stdin; standalone runs
/// have no tray to swap with.
pub fn send_switch_editor(to: EditorChoice) {
    send_line(&EditorToTray::SwitchEditor { to });
}

/// `set-channel` to the tray over stdio (Settings > Experimental builds).
pub fn send_set_channel(channel: UpdateChannel) {
    send_line(&EditorToTray::SetChannel { channel });
}

/// `update-check-now` to the tray over stdio (Settings > Check now).
pub fn send_update_check_now() {
    send_line(&EditorToTray::UpdateCheckNow);
}

/// `update-install` to the tray over stdio.
pub fn send_update_install() {
    send_line(&EditorToTray::UpdateInstall);
}

/// `update-later` to the tray over stdio.
pub fn send_update_later() {
    send_line(&EditorToTray::UpdateLater);
}

/// `update-skip` to the tray over stdio.
pub fn send_update_skip() {
    send_line(&EditorToTray::UpdateSkip);
}

/// `update-unskip` to the tray over stdio (Settings > Undo).
pub fn send_update_unskip() {
    send_line(&EditorToTray::UpdateUnskip);
}

/// `update-set-auto` to the tray over stdio (Settings > Check automatically).
pub fn send_update_set_auto(enabled: bool) {
    send_line(&EditorToTray::UpdateSetAuto { enabled });
}

/// `update-dismiss-updated` to the tray over stdio.
pub fn send_update_dismiss_updated() {
    send_line(&EditorToTray::UpdateDismissUpdated);
}

/// `open-logs-folder` to the tray over stdio (Settings > Open logs folder).
pub fn send_open_logs_folder() {
    send_line(&EditorToTray::OpenLogsFolder);
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
    fn a_300kib_line_is_dropped_and_the_next_line_arrives() {
        use std::io::Cursor;
        let over = "x".repeat(300 * 1024);
        let data = format!("{over}\n{{\"type\":\"close\"}}\n");
        let (tx, rx) = std::sync::mpsc::channel();
        run_ipc_reader(Cursor::new(data.into_bytes()), tx);
        // The over-long line acted on nothing; the close after it still arrives.
        assert_eq!(rx.recv().unwrap(), TrayToEditor::Close);
        assert!(rx.recv().is_err(), "only the close arrived");
    }

    #[test]
    fn a_non_utf8_line_is_dropped_and_the_close_after_it_arrives() {
        use std::io::Cursor;
        let mut data = vec![0xff, 0xfe, b'\n'];
        data.extend_from_slice(b"{\"type\":\"close\"}\n");
        let (tx, rx) = std::sync::mpsc::channel();
        run_ipc_reader(Cursor::new(data), tx);
        assert_eq!(rx.recv().unwrap(), TrayToEditor::Close);
        assert!(rx.recv().is_err(), "only the close arrived");
    }

    #[test]
    fn reading_past_eof_is_empty() {
        use std::io::Read as _;
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let mut buf = [0u8; 8];
        assert_eq!(cursor.read(&mut buf).unwrap(), 0);
    }
}
