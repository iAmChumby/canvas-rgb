//! #19: every way the daemon ends leaves a line in its log.
//!
//! - [`install_panic_hook`] logs a panic's message and location through `tracing`.
//! - [`StopReason`] names why a quit signal fired (Ctrl+C, console close, logoff, shutdown, API).
//! - [`log_exit`] writes the one closing line, with the reason and the exit code, for every path
//!   out of `main`.
//!
//! What the daemon cannot log is a kill from outside (`TerminateProcess`, a crash in the loader).
//! The service names those by exit code (`canvas_svc::process::describe_exit_code`).

use std::panic::PanicHookInfo;

/// Why the daemon was asked to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    CtrlC,
    CtrlBreak,
    /// `CTRL_CLOSE_EVENT`: the console window was closed.
    ConsoleClose,
    /// `CTRL_LOGOFF_EVENT`: the user is logging off.
    ConsoleLogoff,
    /// `CTRL_SHUTDOWN_EVENT`: the machine is shutting down.
    ConsoleShutdown,
    /// The API's quit route (the tray's Quit, or the CLI).
    Api,
}

impl StopReason {
    /// The short name logged in the `reason` field.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CtrlC => "ctrl-c",
            Self::CtrlBreak => "ctrl-break",
            Self::ConsoleClose => "console-close",
            Self::ConsoleLogoff => "console-logoff",
            Self::ConsoleShutdown => "console-shutdown",
            Self::Api => "api",
        }
    }

    /// The sentence logged when this reason stops the daemon.
    pub fn describe(self) -> &'static str {
        match self {
            Self::CtrlC => "Ctrl+C was pressed (CTRL_C_EVENT)",
            Self::CtrlBreak => "Ctrl+Break was pressed (CTRL_BREAK_EVENT)",
            Self::ConsoleClose => "the console was closed (CTRL_CLOSE_EVENT)",
            Self::ConsoleLogoff => "the user is logging off (CTRL_LOGOFF_EVENT)",
            Self::ConsoleShutdown => "the PC is shutting down (CTRL_SHUTDOWN_EVENT)",
            Self::Api => "the API's quit route was called",
        }
    }
}

/// Logs that the daemon was asked to stop, and why.
pub fn log_stop_requested(reason: StopReason) {
    tracing::info!(reason = reason.as_str(), "the daemon was asked to stop: {}", reason.describe());
}

/// The closing line: why `main` is about to end the process, and with which code. Code 0 is
/// INFO; anything else is ERROR so a failed exit stands out.
pub fn log_exit(reason: &str, code: i32) {
    if code == 0 {
        tracing::info!(reason, code, "canvas-rgbd is exiting");
    } else {
        tracing::error!(reason, code, "canvas-rgbd is exiting");
    }
}

/// A panic's payload as text. `panic!("x")` carries a `&str`, `panic!("{x}")` a `String`.
pub fn panic_message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic with a payload that is not text".to_string()
    }
}

/// Makes every panic leave a line (message, `file:line:col`, thread) before the default hook
/// runs. A panic on the main thread then ends the process, and `main` drops the log guard on
/// the way out, which flushes the line; a panic in a task is logged and the daemon carries on.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "an unknown location".to_string(), |at| format!("{}:{}:{}", at.file(), at.line(), at.column()));
        let thread = std::thread::current();
        tracing::error!(
            message = %panic_message(info),
            location = %location,
            thread = thread.name().unwrap_or("<unnamed>"),
            "the daemon panicked"
        );
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct LogBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn capture<R>(run: impl FnOnce() -> R) -> (R, String) {
        let buf = LogBuf::default();
        let sink = buf.clone();
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || sink.clone()).finish();
        let result = tracing::subscriber::with_default(subscriber, run);
        let text = String::from_utf8_lossy(&buf.0.lock().unwrap()).into_owned();
        (result, text)
    }

    #[test]
    fn a_panic_leaves_its_message_and_location_in_the_log() {
        let (caught, text) = capture(|| {
            install_panic_hook();
            std::panic::catch_unwind(|| {
                panic!("the render loop lost its sink {}", 7);
            })
        });
        assert!(caught.is_err());
        assert!(text.contains("ERROR"), "{text}");
        assert!(text.contains("the daemon panicked"), "{text}");
        assert!(text.contains("the render loop lost its sink 7"), "{text}");
        assert!(text.contains("exit_log.rs"), "the location names this file: {text}");
    }

    #[test]
    fn a_static_str_panic_is_logged_too() {
        let (_, text) = capture(|| {
            install_panic_hook();
            let _ = std::panic::catch_unwind(|| panic!("plain text"));
        });
        assert!(text.contains("plain text"), "{text}");
    }

    #[test]
    fn every_stop_reason_has_a_distinct_name_and_a_sentence() {
        let all = [
            StopReason::CtrlC,
            StopReason::CtrlBreak,
            StopReason::ConsoleClose,
            StopReason::ConsoleLogoff,
            StopReason::ConsoleShutdown,
            StopReason::Api,
        ];
        let mut names: Vec<_> = all.iter().map(|r| r.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
        assert!(StopReason::ConsoleLogoff.describe().contains("CTRL_LOGOFF_EVENT"));
        assert!(StopReason::ConsoleShutdown.describe().contains("CTRL_SHUTDOWN_EVENT"));
        assert!(StopReason::ConsoleClose.describe().contains("CTRL_CLOSE_EVENT"));
    }

    #[test]
    fn the_closing_line_has_the_reason_and_code_and_a_failure_is_an_error() {
        let (_, text) = capture(|| {
            log_exit("the API port 9430 is taken by pid 4242", 1);
            log_exit("api", 0);
        });
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].contains("ERROR") && lines[0].contains("taken by pid 4242") && lines[0].contains("code=1"), "{}", lines[0]);
        assert!(lines[1].contains("INFO") && lines[1].contains("reason=\"api\"") && lines[1].contains("code=0"), "{}", lines[1]);
    }

    #[test]
    fn a_stop_request_logs_the_reason() {
        let (_, text) = capture(|| log_stop_requested(StopReason::ConsoleShutdown));
        assert!(text.contains("console-shutdown") && text.contains("CTRL_SHUTDOWN_EVENT"), "{text}");
    }
}
