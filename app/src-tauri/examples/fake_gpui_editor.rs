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
//! - otherwise it says `{"type":"ready"}` and then reads the tray's messages: a `navigate` is
//!   answered with a `ready`, a `close` ends it with code 0.

use std::io::{BufRead, Write};

use serde_json::Value;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut page: Option<String> = None;
    let mut at = 1;
    while at < args.len() {
        if args[at] == "--page" && at + 1 < args.len() {
            page = Some(args[at + 1].clone());
            at += 2;
            continue;
        }
        at += 1;
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
            Some("close") => break,
            _ => {}
        }
        let _ = std::io::stdout().flush();
    }
}
