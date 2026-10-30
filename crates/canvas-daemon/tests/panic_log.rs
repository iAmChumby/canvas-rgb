//! #19 review: a panic that ends the process leaves its line in the log FILE.
//!
//! The log guard is a `static`, which Rust never drops at exit, so the panic hook has to drain the
//! log itself. Each case re-runs this test binary as a child with `CANVAS_RGB_HOME` pointing at a
//! temporary home, then reads the log file the child left.

use std::path::{Path, PathBuf};
use std::process::Command;

const MODE_ENV: &str = "CANVAS_RGB_PANIC_CHILD";

struct TempHome(PathBuf);

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn log_text(home: &Path) -> String {
    let mut out = String::new();
    if let Ok(entries) = std::fs::read_dir(home.join("logs")) {
        for entry in entries.flatten() {
            out.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
        }
    }
    out
}

/// Runs the child test `case` in `mode`; returns its log text and exit code.
fn run_child(case: &str, mode: &str) -> (String, Option<i32>) {
    let home = std::env::temp_dir().join(format!("canvas-rgb-panic-{}-{}", std::process::id(), mode));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let home = TempHome(home);
    let output = Command::new(std::env::current_exe().unwrap())
        .args([case, "--exact", "--nocapture", "--test-threads=1"])
        .env(MODE_ENV, mode)
        .env("CANVAS_RGB_HOME", &home.0)
        .output()
        .expect("the child test binary runs");
    (log_text(&home.0), output.status.code())
}

/// The child half. Without `MODE_ENV` this test does nothing, so a normal run passes it at once.
#[test]
fn child_panics_like_the_daemon() {
    let Ok(mode) = std::env::var(MODE_ENV) else { return };
    canvas_daemon::logging::init("canvas-rgbd").unwrap();
    // Everything runs on a thread that stands in for `main`: it installs the hook.
    let main_like = std::thread::spawn(move || {
        canvas_daemon::exit_log::install_panic_hook();
        match mode.as_str() {
            "main-panic" => {
                // Fill the queue so the writer has work when the panic hits.
                for i in 0..2000 {
                    tracing::warn!("filler line {i}");
                }
                panic!("the main thread lost its sink");
            }
            "task-panic" => {
                let task = std::thread::Builder::new().name("render-task".into()).spawn(|| {
                    panic!("a task lost its sink");
                });
                let _ = task.unwrap().join();
                // The process survived: this line must still be written, so the log is still open.
                tracing::warn!("after the task panic the daemon carries on");
                canvas_daemon::logging::close();
            }
            other => panic!("unknown mode {other}"),
        }
    });
    let _ = main_like.join();
    // Like a panic out of `main`: the process ends with no static dropped and no `close`.
    std::process::exit(101);
}

#[test]
fn a_main_thread_panic_reaches_the_log_file() {
    let (text, code) = run_child("child_panics_like_the_daemon", "main-panic");
    assert_eq!(code, Some(101));
    assert!(text.contains("the daemon panicked"), "the panic line is in the file ({} bytes):\n{text}", text.len());
    assert!(text.contains("the main thread lost its sink"), "{text}");
    assert!(text.contains("panic_log.rs"), "the location names this file: {text}");
}

#[test]
fn a_task_panic_leaves_the_log_open_for_later_lines() {
    let (text, code) = run_child("child_panics_like_the_daemon", "task-panic");
    assert_eq!(code, Some(101));
    assert!(text.contains("a task lost its sink") && text.contains("render-task"), "{text}");
    assert!(text.contains("after the task panic the daemon carries on"), "the log stayed open:\n{text}");
}
