//! A stand-in for the GPUI editor (#202): it speaks the same stdio protocol, so the tray's
//! rules can be tested against a live child without the real editor.
//!
//! It is a test-only program. `cargo test` builds it, and the tray's tests run it as the
//! GPUI editor.
//!
//! What it does:
//! - `--page <name>` is the page it was started with. It writes that page (or nothing) into the
//!   file named by `FAKE_GPUI_EDITOR_PAGE_FILE`, so a test can see what the tray passed on.
//! - the lines in `FAKE_GPUI_EDITOR_EMIT` (separated by newlines) are written to stdout first.
//! - `FAKE_GPUI_EDITOR_EXIT` is the code it ends with right away, without saying anything.
//! - `FAKE_GPUI_EDITOR_CLOSE_FILE` is written when the tray says `close`, so a test can see
//!   that the message arrived.
//! - `FAKE_GPUI_EDITOR_IGNORE_CLOSE` keeps it alive after a `close`: the tray asked it to go,
//!   and it does not go.
//! - `FAKE_GPUI_EDITOR_GRANDCHILD` makes it leave behind a child of its own that holds the
//!   pipes open. The tray has to see the end of the editor even though its pipe stays open.
//!   The pid of that child goes into `FAKE_GPUI_EDITOR_PID_FILE`.
//! - `--sleep <seconds>` is what that child runs: it holds the pipes open and does nothing else.
//! - otherwise it says `{"type":"ready"}` and then reads the tray's messages: a `navigate` is
//!   answered with a `ready`, a `close` ends it with code 0.

use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::Value;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut page: Option<String> = None;
    let mut sleep: Option<u64> = None;
    let mut at = 1;
    while at < args.len() {
        if args[at] == "--page" && at + 1 < args.len() {
            page = Some(args[at + 1].clone());
            at += 2;
            continue;
        }
        if args[at] == "--sleep" && at + 1 < args.len() {
            sleep = args[at + 1].parse().ok();
            at += 2;
            continue;
        }
        at += 1;
    }

    // The mode of the child that holds the pipes open: it does nothing else, and it ignores
    // every environment variable of the editor.
    if let Some(seconds) = sleep {
        std::thread::sleep(Duration::from_secs(seconds));
        return;
    }

    if let Some(file) = std::env::var_os("FAKE_GPUI_EDITOR_PAGE_FILE") {
        let _ = std::fs::write(file, page.clone().unwrap_or_default());
    }

    if let Ok(lines) = std::env::var("FAKE_GPUI_EDITOR_EMIT") {
        for line in lines.split('\n') {
            println!("{line}");
        }
        let _ = std::io::stdout().flush();
    }

    // The child that holds the pipes open: it inherits stdout, so the tray's reader waits for
    // it long after this editor is gone.
    if std::env::var_os("FAKE_GPUI_EDITOR_GRANDCHILD").is_some() {
        let seconds = std::env::var("FAKE_GPUI_EDITOR_GRANDCHILD_SLEEP").unwrap_or_else(|_| "600".to_string());
        // token: the person's own session, non-elevated; this is the test's own stand-in
        // child, not the daemon.
        let grandchild = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--sleep")
            .arg(seconds)
            .stdin(std::process::Stdio::null())
            .spawn()
            .expect("the stand-in child starts");
        if let Some(file) = std::env::var_os("FAKE_GPUI_EDITOR_PID_FILE") {
            let _ = std::fs::write(file, grandchild.id().to_string());
        }
        // The child lives on its own: the editor ends without waiting for it.
        let _ = grandchild;
    }

    if let Ok(code) = std::env::var("FAKE_GPUI_EDITOR_EXIT") {
        let code: i32 = code.parse().unwrap_or(1);
        if let Ok(text) = std::env::var("FAKE_GPUI_EDITOR_STDERR") {
            eprint!("{text}");
            let _ = std::io::stderr().flush();
        }
        let _ = std::io::stdout().flush();
        std::process::exit(code);
    }

    println!("{{\"type\":\"ready\"}}");
    let _ = std::io::stdout().flush();

    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        match stdin.lock().read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let message: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
        match message.get("type").and_then(|t| t.as_str()) {
            Some("navigate") => println!("{{\"type\":\"ready\"}}"),
            Some("close") => {
                // The tray said close: the test can see that the message arrived.
                if let Some(file) = std::env::var_os("FAKE_GPUI_EDITOR_CLOSE_FILE") {
                    let _ = std::fs::write(file, "close");
                }
                if std::env::var_os("FAKE_GPUI_EDITOR_IGNORE_CLOSE").is_some() {
                    // The tray asked it to go, and it does not go.
                    continue;
                }
                break;
            }
            _ => {}
        }
        let _ = std::io::stdout().flush();
    }
}
