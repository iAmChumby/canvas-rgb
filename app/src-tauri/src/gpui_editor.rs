//! The GPUI editor as the tray's child process (#202).
//!
//! The GPUI editor is its own program, `canvas-rgb-editor.exe`, and it sits beside the tray's
//! exe. The tray starts it by absolute path, never through `PATH` and never from the current
//! directory, and it talks to it over the child's own stdio: one JSON line per message. There is
//! no socket, no port and no named pipe anywhere in this file.
//!
//! The rules the launch is hardened by (G12):
//! - the exe is the one beside the tray, by absolute path;
//! - `--page` only ever carries one of the allowed page names;
//! - a line longer than [`EDITOR_IPC_LINE_MAX`] is dropped and logged, never acted on;
//! - a line that is not a message the tray knows is dropped and logged, never acted on.
//!
//! When the GPUI editor cannot be used, React opens instead (G3, G4): the pref stays as it is,
//! the log says why, and after two failed starts in a minute the tray stops trying for the rest
//! of its session. That giving up is sticky: it is never taken back, so a child that falls over
//! once a minute does not keep the tray busy for ever.
//!
//! The end of the editor is seen by a watcher that polls the child itself, not by the end of
//! its pipe: a grandchild of the editor can hold the pipe open long after the editor has
//! ended, and the tray has to see the end anyway.
//!
//! A chatty editor cannot fill the log: the first few lines are logged on their own, the rest
//! are counted and come as one summary line per window.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use canvas_proto::tauri::{
    EditorChoice, EditorToTray, TrayToEditor, EDITOR_IPC_LINE_MAX, EDITOR_PAGES, GPUI_EDITOR_EXE,
};
use tauri::{AppHandle, Manager, Runtime};

use crate::{editor, log, update};

/// How many failed starts inside [`FAILURE_WINDOW_MS`] mean: React for the rest of the session.
pub const FAILURE_LIMIT: usize = 2;
/// The window the failed starts are counted in (G4).
pub const FAILURE_WINDOW_MS: u64 = 60_000;
/// How many stderr lines are kept for the log when the editor ends badly.
const STDERR_TAIL: usize = 20;
/// How long the tray waits for the editor to end before it ends it itself.
const EXIT_WAIT: Duration = Duration::from_secs(5);
/// How long the tray waits between two looks at whether the editor has ended.
const EXIT_POLL: Duration = Duration::from_millis(20);
/// How often the watcher looks at whether the editor has ended. The end of the editor is seen
/// here, not at the end of its pipe: a grandchild of the editor can hold the pipe open.
const EXIT_WATCH_POLL: Duration = Duration::from_millis(200);
/// How long the exit path waits for the stderr reader. It is a bounded wait: a grandchild of
/// the editor can hold the stderr pipe open too.
const STDERR_WAIT: Duration = Duration::from_millis(500);
/// How long the tray waits for a child that is closing to end before it ends it itself.
const CLOSING_WAIT: Duration = Duration::from_secs(2);
/// The longest a dropped line is shown in the log.
const LOGGED_LINE: usize = 120;
/// How many lines from the editor the log takes on their own. The rest is counted.
pub const LOG_LINE_CAP: u32 = 5;
/// How long the counted lines wait before they come as one summary line.
pub const LOG_SUMMARY_WINDOW: Duration = Duration::from_secs(30);
/// The line the log takes for a line that is over the cap.
fn too_long_line() -> String {
    format!(
        "editor: a line from the GPUI editor was longer than {EDITOR_IPC_LINE_MAX} bytes and was dropped"
    )
}

/// The GPUI editor's exe, beside the tray's: `<the tray's folder>\canvas-rgb-editor.exe`.
pub fn exe_path_in(dir: &Path) -> PathBuf {
    dir.join(GPUI_EDITOR_EXE)
}

/// The GPUI editor's exe beside the running tray. `None` when the tray's own path is unknown.
///
/// The path is absolute by construction: it comes from the running exe's own path, never from
/// `PATH` and never from the current directory.
pub fn exe_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(exe_path_in(dir))
}

/// The command line for a page: `--page <name>` when the name is one of the editor's pages, and
/// nothing at all when it is not. A name that is not allowed is logged and dropped: the editor
/// opens on its own page instead.
pub fn page_args(page: Option<&str>) -> Vec<String> {
    let Some(page) = page else {
        return Vec::new();
    };
    if !EDITOR_PAGES.contains(&page) {
        log::append(&format!(
            "editor: {page:?} is not one of the editor's pages ({:?}); the editor opens on its own",
            EDITOR_PAGES
        ));
        return Vec::new();
    }
    vec!["--page".to_string(), page.to_string()]
}

/// What the GPUI editor told the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorEvent {
    /// The editor is up and said it is ready.
    Ready,
    /// The editor asks for the other editor.
    SwitchEditor(EditorChoice),
    /// The editor cannot be used: it did not start, or it ended with a failure.
    Unusable {
        /// Why, in a sentence for the log.
        reason: String,
    },
}

/// What the tray does with what the editor said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing opens.
    Nothing,
    /// The React editor opens on this page.
    OpenReact(Option<String>),
    /// The other editor opens on this page; the open one closes first.
    Switch(EditorChoice, Option<String>),
}

/// What one event means for the tray: which editor opens, and on which page.
///
/// `current` is the editor that is open now, `page` the page it is on.
pub fn decide(event: &EditorEvent, current: EditorChoice, page: Option<String>) -> Action {
    match event {
        EditorEvent::Ready => Action::Nothing,
        EditorEvent::SwitchEditor(to) => {
            if *to == current {
                Action::Nothing
            } else {
                Action::Switch(*to, page)
            }
        }
        // The editor cannot be used: React opens on the same page (G3, G4).
        EditorEvent::Unusable { .. } => Action::OpenReact(page),
    }
}

/// Why a line was not acted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    /// The line was longer than the cap.
    TooLong,
    /// The line was not a message the tray knows.
    Unknown,
    /// The line was not valid UTF-8.
    NotUtf8,
    /// The stream ended or failed.
    Io,
}

/// Reads one line from the editor. `Err` says why the line was dropped, and a dropped line is
/// never acted on.
pub fn read_line(line: &str) -> Result<EditorEvent, Dropped> {
    if line.len() > EDITOR_IPC_LINE_MAX {
        return Err(Dropped::TooLong);
    }
    // A line that ends with CRLF keeps its carriage return here: it is not part of the message.
    let line = line.trim_end_matches(['\r', '\n']);
    match serde_json::from_str::<EditorToTray>(line) {
        Ok(EditorToTray::Ready) => Ok(EditorEvent::Ready),
        Ok(EditorToTray::SwitchEditor { to }) => Ok(EditorEvent::SwitchEditor(to)),
        Err(_) => Err(Dropped::Unknown),
    }
}

/// Reads one line from `reader`, at most `max` bytes long.
///
/// A longer line is dropped: the rest of it is thrown away and [`Dropped::TooLong`] comes back,
/// so the reader never holds more than the cap in memory. A line that is not valid UTF-8 is
/// dropped too ([`Dropped::NotUtf8`]): it is never repaired into a message that is acted on.
/// `Ok(None)` is the end of the stream.
fn read_capped_line<R: BufRead>(reader: &mut R, max: usize) -> Result<Option<String>, Dropped> {
    let mut buf: Vec<u8> = Vec::new();
    let mut over = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(Dropped::Io),
        };
        if available.is_empty() {
            // The end of the stream. A last line without its newline still counts.
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
        return Err(Dropped::TooLong);
    }
    // A line that ends with CRLF keeps its carriage return: it is not part of the message.
    let buf = match buf.last() {
        Some(b'\r') => &buf[..buf.len() - 1],
        _ => &buf[..],
    };
    match String::from_utf8(buf.to_vec()) {
        Ok(line) => Ok(Some(line)),
        Err(_) => Err(Dropped::NotUtf8),
    }
}

/// The GPUI editor child: started by the tray, talked to over its stdio.
pub struct GpuiEditor {
    inner: Arc<Mutex<Inner>>,
    on_event: Arc<dyn Fn(EditorEvent) + Send + Sync>,
}

/// The log lines the tray writes, collected by the tests.
#[cfg(test)]
type LogLines = Arc<Mutex<Vec<String>>>;
/// A sink that keeps the log lines. The tests use it.
#[cfg(test)]
type LogSink = Box<dyn Fn(&str) + Send + Sync>;

struct Inner {
    /// The exe's absolute path.
    exe: PathBuf,
    /// The running child, when there is one.
    running: Option<Running>,
    /// The ms of the failed starts that are still inside the window.
    failures: Vec<u64>,
    /// Set when the limit of failed starts was reached. It is never taken back: the tray gives
    /// up on the GPUI editor for the rest of its session.
    gave_up: bool,
    /// How many times the missing editor has been reported in this session.
    missing_reports: u32,
    /// The clock the failed starts are counted on. The tests move it.
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    /// Extra environment for the child. The production child inherits the tray's.
    env: Vec<(String, String)>,
    /// Where the tray's log lines go. The production tray writes them to its log file; the
    /// tests collect them, so a test can see what a chatty editor did to the log.
    log_sink: Arc<dyn Fn(&str) + Send + Sync>,
}

/// The running child. Cloned into the reader thread, so it can end the child and say how it
/// ended.
#[derive(Clone)]
struct Running {
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    /// Set when the tray asked the editor to close: its end is not a failure.
    closing: Arc<AtomicBool>,
    /// Set when the child's end has been handled. Only one of the threads that watch the child
    /// does it.
    ended: Arc<AtomicBool>,
    /// Set when this child asked for the other editor: one switch per child life.
    switched: Arc<AtomicBool>,
}

/// What happened when the tray asked for the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenOutcome {
    /// The editor was started, or the running one was told where to go.
    Started,
    /// React is opened instead. `reason` says why.
    React {
        /// Why the GPUI editor cannot be used, in a sentence for the log.
        reason: String,
    },
}

impl GpuiEditor {
    /// The editor the running tray starts: the exe beside it, with the real clock.
    pub fn production(on_event: Box<dyn Fn(EditorEvent) + Send + Sync>) -> Self {
        Self::new(exe_path().unwrap_or_default(), Box::new(now_ms), on_event)
    }

    /// The editor at `exe`, with the clock the failed starts are counted on.
    pub fn new(
        exe: PathBuf,
        clock: Box<dyn Fn() -> u64 + Send + Sync>,
        on_event: Box<dyn Fn(EditorEvent) + Send + Sync>,
    ) -> Self {
        GpuiEditor {
            inner: Arc::new(Mutex::new(Inner {
                exe,
                running: None,
                failures: Vec::new(),
                gave_up: false,
                missing_reports: 0,
                clock,
                env: Vec::new(),
                log_sink: Arc::new(log::append),
            })),
            on_event: Arc::from(on_event),
        }
    }

    /// The extra environment the child gets. The production child inherits the tray's; the tests
    /// give the stand-in editor its own.
    #[cfg(test)]
    pub fn with_env(self, env: Vec<(String, String)>) -> Self {
        {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.env = env;
        }
        self
    }

    /// Where the log lines go. The production tray writes them to its log file; the tests
    /// collect them.
    #[cfg(test)]
    pub fn with_log_sink(self, sink: LogSink) -> Self {
        {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.log_sink = Arc::from(sink);
        }
        self
    }

    /// The exe's path.
    pub fn exe(&self) -> PathBuf {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).exe.clone()
    }

    /// True while the editor is running.
    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).running.is_some()
    }

    /// Ends the running child. The tests use it when the stand-in editor does not go.
    #[cfg(test)]
    pub fn kill_running(&self) {
        if let Some(running) = self.running() {
            let mut child = running.child.lock().unwrap_or_else(|p| p.into_inner());
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// True when the tray has given up on the GPUI editor: React is used for the rest of this
    /// session.
    ///
    /// Two failed starts inside the window set a flag that is never taken back, so an editor
    /// that falls over once a minute does not keep the tray busy for ever.
    pub fn given_up(&self) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if inner.gave_up {
            return true;
        }
        let now = (inner.clock)();
        let inside = inner
            .failures
            .iter()
            .filter(|at| now.saturating_sub(**at) < FAILURE_WINDOW_MS)
            .count();
        if inside >= FAILURE_LIMIT {
            inner.gave_up = true;
            return true;
        }
        false
    }

    /// Opens the editor on `page`, or tells the running one to go there.
    pub fn open(&self, page: Option<&str>) -> OpenOutcome {
        if self.given_up() {
            return OpenOutcome::React {
                reason: format!(
                    "the GPUI editor failed to start {FAILURE_LIMIT} times in the last {} s",
                    FAILURE_WINDOW_MS / 1000
                ),
            };
        }
        if let Some(running) = self.running() {
            if running.closing.load(Ordering::SeqCst) {
                // The child that is closing is not reused: a child on its way out gets no
                // message, and the new one starts under it.
                self.end_closing(&running);
            } else {
                if let Some(page) = page {
                    send(&running, &TrayToEditor::Navigate { page: page.to_string() });
                }
                // The tray has no handle to another process's window, so it cannot focus it.
                log::append("editor: the GPUI editor is already open; focusing its window is UNKNOWN");
                return OpenOutcome::Started;
            }
        }
        let exe = self.exe();
        if !exe.is_file() {
            // A stable build replaced the experimental one, or the editor was never installed.
            // React opens, the pref stays, and the log says so once (G3).
            let first = {
                let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
                let first = inner.missing_reports == 0;
                if first {
                    inner.missing_reports += 1;
                }
                first
            };
            if first {
                log::append(&format!(
                    "editor: the GPUI editor is not there ({}); the React editor opens instead",
                    exe.display()
                ));
            }
            return OpenOutcome::React {
                reason: format!("the GPUI editor is not installed ({})", exe.display()),
            };
        }
        match self.start(page) {
            Ok(()) => OpenOutcome::Started,
            Err(reason) => {
                note_failure(&self.inner);
                log::append(&format!("editor: {reason}; the React editor opens instead"));
                OpenOutcome::React { reason }
            }
        }
    }

    /// Tells the running editor to close.
    pub fn close(&self) {
        if let Some(running) = self.running() {
            running.closing.store(true, Ordering::SeqCst);
            send(&running, &TrayToEditor::Close);
        }
    }

    fn running(&self) -> Option<Running> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).running.clone()
    }

    /// Starts the child and the threads that watch it.
    fn start(&self, page: Option<&str>) -> Result<(), String> {
        let exe = self.exe();
        // token: the tray's own, non-elevated (the person's). The editor runs as the person who
        // opened it, never elevated; it is not the daemon.
        let mut command = Command::new(&exe);
        command.args(page_args(page));
        {
            let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            command.envs(inner.env.iter().map(|(k, v)| (k, v)));
        }
        // The child speaks over its own pipes: it never inherits the tray's console.
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("the GPUI editor did not start ({e})"))?;
        let stdin = child.stdin.take().ok_or("the GPUI editor has no stdin")?;
        let stdout = child.stdout.take().ok_or("the GPUI editor has no stdout")?;
        let stderr = child.stderr.take().ok_or("the GPUI editor has no stderr")?;
        let running = Running {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            closing: Arc::new(AtomicBool::new(false)),
            ended: Arc::new(AtomicBool::new(false)),
            switched: Arc::new(AtomicBool::new(false)),
        };
        {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.running = Some(running.clone());
        }
        log::append(&format!("editor: the GPUI editor was started ({})", exe.display()));
        self.watch(running, stdout, stderr);
        Ok(())
    }

    /// Watches the child: the reader of its messages, the reader of its last words, and the
    /// watcher that sees it end.
    ///
    /// The end of the child is seen by the watcher, not by the end of its pipe: a grandchild of
    /// the editor can hold the pipe open long after the editor itself has ended, and the tray
    /// has to see the end anyway (G4).
    fn watch(
        &self,
        running: Running,
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
    ) {
        let on_event = self.on_event.clone();
        let inner = self.inner.clone();
        let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
        // The stderr reader keeps the last lines: they are the log of a crash. It says when it
        // is done, so the exit path can wait for it without waiting for ever.
        let (stderr_done, stderr_finished) = std::sync::mpsc::channel::<()>();
        let stderr_finished: Arc<Mutex<std::sync::mpsc::Receiver<()>>> =
            Arc::new(Mutex::new(stderr_finished));
        let stderr_tail = tail.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                match read_capped_line(&mut reader, EDITOR_IPC_LINE_MAX) {
                    Ok(Some(line)) => {
                        let mut lines = stderr_tail.lock().unwrap_or_else(|p| p.into_inner());
                        lines.push_back(line);
                        while lines.len() > STDERR_TAIL {
                            lines.pop_front();
                        }
                    }
                    // The end of the stream.
                    Ok(None) => break,
                    // A line that is too long or not valid UTF-8 is dropped, and the reader
                    // goes on with the next one.
                    Err(Dropped::TooLong) | Err(Dropped::NotUtf8) => continue,
                    Err(Dropped::Unknown) | Err(Dropped::Io) => break,
                }
            }
            let _ = stderr_done.send(());
        });

        // The reader of the editor's messages.
        {
            let running = running.clone();
            let inner = inner.clone();
            let tail = tail.clone();
            let on_event = on_event.clone();
            let stderr_finished = stderr_finished.clone();
            std::thread::spawn(move || {
                let mut limiter = LogLimiter::new();
                let mut reader = BufReader::new(stdout);
                loop {
                    if running.ended.load(Ordering::SeqCst) {
                        // The child is gone: nothing more of it is acted on.
                        break;
                    }
                    match read_capped_line(&mut reader, EDITOR_IPC_LINE_MAX) {
                        Ok(Some(line)) => match read_line(&line) {
                            Ok(EditorEvent::Ready) => {
                                log_line_to(
                                    &mut limiter,
                                    &inner,
                                    "editor: the GPUI editor is ready",
                                );
                                on_event(EditorEvent::Ready);
                            }
                            Ok(EditorEvent::SwitchEditor(to)) => {
                                if running.closing.load(Ordering::SeqCst) {
                                    log::append("editor: the editor asked for the other editor while it was closing; the ask was ignored");
                                } else if running.switched.swap(true, Ordering::SeqCst) {
                                    // One switch per child life: the child cannot flip the
                                    // editor over and over.
                                    log::append("editor: the editor asked for the other editor again; the second ask was ignored");
                                } else {
                                    on_event(EditorEvent::SwitchEditor(to));
                                }
                            }
                            Err(Dropped::TooLong) => {
                                log_line_to(&mut limiter, &inner, &too_long_line())
                            }
                            Err(Dropped::Unknown) => {
                                let text = format!(
                                    "editor: a line from the GPUI editor was not a message the tray knows and was dropped: {}",
                                    logged(&line)
                                );
                                log_line_to(&mut limiter, &inner, &text);
                            }
                            Err(Dropped::NotUtf8) => log_line_to(
                                &mut limiter,
                                &inner,
                                "editor: a line from the GPUI editor was not valid UTF-8 and was dropped",
                            ),
                            Err(Dropped::Io) => {}
                            Ok(EditorEvent::Unusable { .. }) => {
                                // The editor never says this: it is the tray's own event.
                            }
                        },
                        Ok(None) => break,
                        Err(Dropped::TooLong) => {
                            log_line_to(&mut limiter, &inner, &too_long_line())
                        }
                        Err(Dropped::Unknown) | Err(Dropped::NotUtf8) | Err(Dropped::Io) => break,
                    }
                }
                // The pipe ended: the code the child ended with, and its last words on stderr.
                let code = wait_for_exit(&running.child);
                end_child(
                    &inner,
                    &on_event,
                    &running,
                    &tail,
                    &stderr_finished,
                    code,
                );
            });
        }

        // The watcher: the child's end is seen here, not at the end of its pipe.
        {
            let running = running.clone();
            let inner = inner.clone();
            let tail = tail.clone();
            let on_event = on_event.clone();
            let stderr_finished = stderr_finished.clone();
            std::thread::spawn(move || loop {
                let ended = {
                    let mut child = running.child.lock().unwrap_or_else(|p| p.into_inner());
                    match child.try_wait() {
                        Ok(Some(status)) => Some(Some(status.code().unwrap_or(-1))),
                        Ok(None) => None,
                        Err(_) => Some(None),
                    }
                };
                match ended {
                    Some(code) => {
                        end_child(
                            &inner,
                            &on_event,
                            &running,
                            &tail,
                            &stderr_finished,
                            code,
                        );
                        return;
                    }
                    None => std::thread::sleep(EXIT_WATCH_POLL),
                }
            });
        }
    }

    /// Waits for a child that is closing to end, and ends it when it does not. A child that is
    /// on its way out holds the editor's place, so the new one cannot start until it is gone.
    fn end_closing(&self, running: &Running) {
        let deadline = Instant::now() + CLOSING_WAIT;
        loop {
            {
                let mut child = running.child.lock().unwrap_or_else(|p| p.into_inner());
                match child.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) => {}
                    Err(_) => return,
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(EXIT_POLL);
        }
        // It did not go: the tray ends it, so the next child starts with free pipes.
        let mut child = running.child.lock().unwrap_or_else(|p| p.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// The child has ended: the tray's state is cleared, the log says how it ended, and the tray is
/// told when the editor cannot be used.
///
/// Only the first caller does it: the reader and the watcher can both see the end.
fn end_child(
    inner: &Arc<Mutex<Inner>>,
    on_event: &Arc<dyn Fn(EditorEvent) + Send + Sync>,
    running: &Running,
    tail: &Arc<Mutex<VecDeque<String>>>,
    stderr_finished: &Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
    code: Option<i32>,
) {
    if running.ended.swap(true, Ordering::SeqCst) {
        return;
    }
    // The stderr reader gets a bounded time to finish: a grandchild of the editor can hold the
    // stderr pipe open too, and the exit path must not wait for it.
    {
        let finished = stderr_finished.lock().unwrap_or_else(|p| p.into_inner());
        let _ = finished.recv_timeout(STDERR_WAIT);
    }
    let stderr_tail = tail
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ");
    // The child is gone: the tray has no editor to talk to.
    {
        let mut guard = inner.lock().unwrap_or_else(|p| p.into_inner());
        let same = guard
            .running
            .as_ref()
            .is_some_and(|r| Arc::ptr_eq(&r.child, &running.child));
        if same {
            guard.running = None;
        }
    }
    if running.closing.load(Ordering::SeqCst) {
        log_to(inner, "editor: the GPUI editor closed");
        return;
    }
    match code {
        Some(0) => log_to(inner, "editor: the GPUI editor ended"),
        Some(code) => {
            let reason = format!(
                "the GPUI editor ended with code {code}{}",
                tail_in_reason(&stderr_tail)
            );
            log_to(inner, &format!("editor: {reason}; the React editor opens instead"));
            note_failure(inner);
            on_event(EditorEvent::Unusable { reason });
        }
        None => {
            let reason = format!("the GPUI editor did not end{}", tail_in_reason(&stderr_tail));
            log_to(inner, &format!("editor: {reason}; the React editor opens instead"));
            note_failure(inner);
            on_event(EditorEvent::Unusable { reason });
        }
    }
}

/// The stderr tail as it goes into the reason for the crash.
fn tail_in_reason(tail: &str) -> String {
    if tail.is_empty() {
        String::new()
    } else {
        format!(" ({tail})")
    }
}

/// Writes one line to the tray's log: the sink the tests collect, or the log file.
fn log_to(inner: &Arc<Mutex<Inner>>, line: &str) {
    let guard = inner.lock().unwrap_or_else(|p| p.into_inner());
    (guard.log_sink)(line);
}

/// What the tray does with one line from the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineLog {
    /// The line is logged on its own.
    Line,
    /// The line is counted; nothing is logged now.
    Counted,
    /// The counted lines are logged as one summary line, with the count.
    Summary(u32),
}

/// The tray's log keeps the first [`LOG_LINE_CAP`] lines from the editor and counts the rest,
/// so a chatty editor cannot fill the log. The counted lines come as one summary line per
/// [`LOG_SUMMARY_WINDOW`].
#[derive(Debug)]
pub struct LogLimiter {
    /// The lines logged on their own so far.
    logged: u32,
    /// The lines counted since the last summary.
    counted: u32,
    /// When the last summary was logged.
    last: Instant,
}

impl LogLimiter {
    /// A limiter that starts empty.
    pub fn new() -> Self {
        LogLimiter {
            logged: 0,
            counted: 0,
            last: Instant::now(),
        }
    }

    /// What to do with one more line from the editor, at `now`.
    pub fn next(&mut self, now: Instant) -> LineLog {
        if self.logged < LOG_LINE_CAP {
            self.logged += 1;
            return LineLog::Line;
        }
        self.counted += 1;
        if now.duration_since(self.last) >= LOG_SUMMARY_WINDOW {
            let count = self.counted;
            self.counted = 0;
            self.last = now;
            return LineLog::Summary(count);
        }
        LineLog::Counted
    }
}

impl Default for LogLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// The line to log about one message from the editor, or `None` when it is only counted.
fn log_line(limiter: &mut LogLimiter, now: Instant, message: &str) -> Option<String> {
    match limiter.next(now) {
        LineLog::Line => Some(message.to_string()),
        LineLog::Counted => None,
        LineLog::Summary(count) => Some(format!(
            "editor: {count} more lines from the GPUI editor were not logged on their own"
        )),
    }
}

/// Logs the line about one message from the editor, when the limiter lets it through.
fn log_line_to(limiter: &mut LogLimiter, inner: &Arc<Mutex<Inner>>, message: &str) {
    if let Some(text) = log_line(limiter, Instant::now(), message) {
        log_to(inner, &text);
    }
}

/// The milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Records a failed start, forgets the ones that are outside the window, and gives up for the
/// rest of the session when the limit is reached.
fn note_failure(inner: &Mutex<Inner>) {
    let mut guard = inner.lock().unwrap_or_else(|p| p.into_inner());
    let now = (guard.clock)();
    guard.failures.retain(|at| now.saturating_sub(*at) < FAILURE_WINDOW_MS);
    guard.failures.push(now);
    if guard.failures.len() >= FAILURE_LIMIT {
        // The giving up is sticky: the window moves, the giving up does not.
        guard.gave_up = true;
    }
}

/// Waits for the child to end, and says how it ended. A child that does not stop is ended by
/// the tray.
fn wait_for_exit(child: &Arc<Mutex<Child>>) -> Option<i32> {
    let deadline = Instant::now() + EXIT_WAIT;
    loop {
        {
            let mut guard = child.lock().unwrap_or_else(|p| p.into_inner());
            match guard.try_wait() {
                Ok(Some(status)) => return Some(status.code().unwrap_or(-1)),
                Ok(None) => {}
                Err(_) => return None,
            }
        }
        if Instant::now() >= deadline {
            let mut guard = child.lock().unwrap_or_else(|p| p.into_inner());
            let _ = guard.kill();
            let _ = guard.wait();
            return None;
        }
        std::thread::sleep(EXIT_POLL);
    }
}

/// Writes one message to the editor, as one line.
fn send(running: &Running, message: &TrayToEditor) -> bool {
    let text = match serde_json::to_string(message) {
        Ok(text) => text,
        Err(e) => {
            log::append(&format!("editor: could not write to the GPUI editor: {e}"));
            return false;
        }
    };
    let mut stdin = running.stdin.lock().unwrap_or_else(|p| p.into_inner());
    let written = stdin
        .write_all(text.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush());
    match written {
        Ok(()) => true,
        Err(e) => {
            log::append(&format!("editor: could not write to the GPUI editor: {e}"));
            false
        }
    }
}

/// The line as it goes into the log, shortened.
fn logged(line: &str) -> String {
    let line = line.trim();
    if line.chars().count() <= LOGGED_LINE {
        return line.to_string();
    }
    let short: String = line.chars().take(LOGGED_LINE).collect();
    format!("{short}…")
}

// ---- the tray's side ----

/// True when the running tray is an experimental build: its version carries a pre-release part
/// (`0.8.5-dev.12`). A stable build has no editor choice, and the pref is ignored.
pub fn is_experimental_build<R: Runtime>(app: &AppHandle<R>) -> bool {
    update::try_controller(app).is_some_and(|c| c.is_experimental())
}

/// The editor the tray opens now: the pref's choice on an experimental build, React otherwise.
pub fn effective_editor<R: Runtime>(app: &AppHandle<R>) -> EditorChoice {
    if !is_experimental_build(app) {
        return EditorChoice::React;
    }
    update::try_controller(app)
        .map(|c| c.prefs().editor)
        .unwrap_or_default()
}

/// Opens the GPUI editor, or React when it cannot be used. The editor that opens comes back.
pub fn open<R: Runtime>(app: &AppHandle<R>, page: Option<&str>) -> EditorChoice {
    let outcome = match app.try_state::<GpuiEditor>() {
        Some(state) => state.open(page),
        None => OpenOutcome::React {
            reason: "the GPUI editor is not there".to_string(),
        },
    };
    match outcome {
        OpenOutcome::Started => EditorChoice::Gpui,
        OpenOutcome::React { reason } => {
            log::append(&format!("editor: {reason}; the React editor opens instead"));
            editor::open_react(app, page);
            EditorChoice::React
        }
    }
}

/// Tells the running GPUI editor to close.
pub fn close<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<GpuiEditor>() {
        state.close();
    }
}

/// What the tray does with what the GPUI editor said.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: EditorEvent) {
    // The editor that is open now, before the pref changes.
    let current = effective_editor(app);
    let page = editor::last_page();
    match &event {
        EditorEvent::Ready => {
            // The reader of the editor's messages logged it, rate-limited: a chatty editor
            // cannot fill the log.
            return;
        }
        EditorEvent::Unusable { reason } => {
            log::append(&format!("editor: {reason}; the React editor opens instead"));
        }
        EditorEvent::SwitchEditor(to) => {
            // The editor asks for the other one: the tray keeps the choice.
            let Some(ctl) = update::try_controller(app) else {
                log::append("editor: the editor choice was not kept: the tray is not ready");
                return;
            };
            if let Err(e) = ctl.set_editor(*to) {
                log::append(&format!("editor: the editor choice was not kept: {e}"));
                return;
            }
        }
    }
    match decide(&event, current, page) {
        Action::Nothing => {}
        Action::OpenReact(page) => editor::open_react(app, page.as_deref()),
        Action::Switch(to, page) => editor::switch_editor(app, to, page.as_deref()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, AtomicU64};

    use canvas_proto::tauri::{TrayPrefs, UpdateChannel};
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

    use super::*;

    /// A folder that cleans up after itself.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join(format!(
                "canvas-rgb-gpui-{name}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A tray-like app: a mock app with a controller whose version and prefs are the given ones.
    fn app_with(version: &str, editor: EditorChoice) -> (tauri::App<MockRuntime>, TempDir) {
        let dir = TempDir::new("app");
        let prefs_path = dir.path().join("prefs.json");
        crate::prefs::save(
            &prefs_path,
            &TrayPrefs { editor, channel: UpdateChannel::Stable },
        )
        .unwrap();
        let mut settings = update::Settings::production();
        settings.state_path = dir.path().join("update.json");
        settings.prefs_path = prefs_path;
        settings.updates_dir = dir.path().join("updates");
        settings.result_path = dir.path().join("update-result.json");
        settings.guard_root = dir.path().to_path_buf();
        let launcher: update::Launcher = Arc::new(|_| Ok(None));
        let controller = Arc::new(update::Controller::new(
            settings,
            version.parse().unwrap(),
            launcher,
            Box::new(|_| {}),
        ));
        let mut ctx = mock_context(noop_assets());
        ctx.package_info_mut().version = version.parse().unwrap();
        let app = mock_builder().build(ctx).unwrap();
        app.manage(controller);
        (app, dir)
    }

    /// The events the editor sends, collected.
    fn sink() -> (std::sync::mpsc::Sender<EditorEvent>, std::sync::mpsc::Receiver<EditorEvent>) {
        std::sync::mpsc::channel()
    }

    /// An editor whose clock the test moves, and the clock itself.
    fn editor_with_clock(name: &str) -> (GpuiEditor, Arc<AtomicU64>) {
        let dir = TempDir::new(name);
        let now = Arc::new(AtomicU64::new(0));
        let clock = {
            let now = now.clone();
            Box::new(move || now.load(Ordering::SeqCst))
        };
        let (tx, _rx) = sink();
        let gpui = GpuiEditor::new(
            dir.path().join(GPUI_EDITOR_EXE),
            clock,
            Box::new(move |event| {
                let _ = tx.send(event);
            }),
        );
        (gpui, now)
    }

    #[test]
    fn only_an_allowed_page_is_passed_on() {
        assert_eq!(page_args(Some("fans")), vec!["--page".to_string(), "fans".to_string()]);
        for page in EDITOR_PAGES {
            assert_eq!(page_args(Some(page)), vec!["--page".to_string(), (*page).to_string()]);
        }
        assert!(page_args(Some("nowhere")).is_empty(), "a page that is not on the list is dropped");
        assert!(page_args(Some("")).is_empty());
        assert!(page_args(None).is_empty());
    }

    #[test]
    fn a_line_over_the_cap_is_dropped() {
        let over = "x".repeat(EDITOR_IPC_LINE_MAX + 1);
        assert_eq!(read_line(&over), Err(Dropped::TooLong));
        // A line of exactly the cap is not over it: it is dropped for what it says, not its length.
        let at_cap = "x".repeat(EDITOR_IPC_LINE_MAX);
        assert_eq!(read_line(&at_cap), Err(Dropped::Unknown));
    }

    #[test]
    fn an_unknown_message_is_dropped() {
        assert_eq!(read_line(r#"{"type":"open"}"#), Err(Dropped::Unknown));
        assert_eq!(read_line("not json at all"), Err(Dropped::Unknown));
        assert_eq!(read_line(""), Err(Dropped::Unknown));
    }

    #[test]
    fn a_switch_editor_line_is_acted_on() {
        assert_eq!(
            read_line(r#"{"type":"switch-editor","to":"react"}"#),
            Ok(EditorEvent::SwitchEditor(EditorChoice::React))
        );
        assert_eq!(
            read_line(r#"{"type":"switch-editor","to":"gpui"}"#),
            Ok(EditorEvent::SwitchEditor(EditorChoice::Gpui))
        );
        assert_eq!(read_line(r#"{"type":"ready"}"#), Ok(EditorEvent::Ready));
    }

    #[test]
    fn the_launch_path_is_the_exe_beside_the_tray() {
        let dir = std::env::temp_dir().join("canvas-rgb install");
        let path = exe_path_in(&dir);
        assert_eq!(path, dir.join(GPUI_EDITOR_EXE));
        assert!(path.is_absolute(), "the editor is started by absolute path");
        assert_eq!(path.parent(), Some(dir.as_path()));
        // The running tray's own path is the only source: never PATH, never the current folder.
        let exe = std::env::current_exe().unwrap();
        assert_eq!(exe_path().unwrap(), exe.parent().unwrap().join(GPUI_EDITOR_EXE));
    }

    #[test]
    fn a_stable_build_refuses_the_choice_and_ignores_the_pref() {
        let (app, _dir) = app_with("0.8.4", EditorChoice::Gpui);
        let handle = app.handle().clone();
        // The pref says gpui, but the build is stable: React is what opens.
        assert_eq!(effective_editor(&handle), EditorChoice::React);
        assert!(!is_experimental_build(&handle));
        // The choice is refused, and the pref stays as it was.
        let ctl = update::try_controller(&handle).unwrap();
        assert!(ctl.set_editor(EditorChoice::Gpui).is_err());
        assert_eq!(ctl.prefs().editor, EditorChoice::Gpui);
        assert_eq!(editor::open(&handle, None), EditorChoice::React);
    }

    #[test]
    fn an_experimental_build_keeps_the_choice() {
        let (app, _dir) = app_with("0.8.5-dev.12", EditorChoice::React);
        let handle = app.handle().clone();
        assert!(is_experimental_build(&handle));
        let ctl = update::try_controller(&handle).unwrap();
        let prefs = ctl.set_editor(EditorChoice::Gpui).unwrap();
        assert_eq!(prefs.editor, EditorChoice::Gpui);
        assert_eq!(effective_editor(&handle), EditorChoice::Gpui);
    }

    #[test]
    fn a_missing_exe_falls_back_to_react_and_keeps_the_pref() {
        let (app, dir) = app_with("0.8.5-dev.12", EditorChoice::Gpui);
        let handle = app.handle().clone();
        let (tx, _rx) = sink();
        let gpui = GpuiEditor::new(
            dir.path().join(GPUI_EDITOR_EXE),
            Box::new(now_ms),
            Box::new(move |event| {
                let _ = tx.send(event);
            }),
        );
        // The exe is not there: React opens and the pref stays.
        assert!(matches!(gpui.open(None), OpenOutcome::React { .. }));
        assert!(!gpui.is_running());
        let ctl = update::try_controller(&handle).unwrap();
        assert_eq!(ctl.prefs().editor, EditorChoice::Gpui);
    }

    #[test]
    fn the_missing_exe_is_reported_once() {
        let dir = TempDir::new("missing");
        let (tx, _rx) = sink();
        let gpui = GpuiEditor::new(
            dir.path().join(GPUI_EDITOR_EXE),
            Box::new(now_ms),
            Box::new(move |event| {
                let _ = tx.send(event);
            }),
        );
        assert!(matches!(gpui.open(None), OpenOutcome::React { .. }));
        assert!(matches!(gpui.open(None), OpenOutcome::React { .. }));
        let reports = gpui.inner.lock().unwrap_or_else(|p| p.into_inner()).missing_reports;
        assert_eq!(reports, 1, "the missing editor is logged once, not on every open");
    }

    #[test]
    fn two_failed_starts_in_a_minute_stop_the_trying() {
        let (gpui, now) = editor_with_clock("guard");
        note_failure(&gpui.inner);
        assert!(!gpui.given_up(), "one failure is not enough");
        now.store(30_000, Ordering::SeqCst);
        note_failure(&gpui.inner);
        assert!(gpui.given_up(), "two failures inside the window stop the trying");
        // The window moves with the clock, and the giving up does not: it is for the rest of
        // the session.
        now.store(30_000 + FAILURE_WINDOW_MS + 1, Ordering::SeqCst);
        assert!(gpui.given_up(), "the giving up is not taken back");
    }

    #[test]
    fn a_failure_exactly_a_window_old_does_not_count() {
        // A failure that is exactly at the edge of the window is outside it.
        let (gpui, now) = editor_with_clock("edge");
        note_failure(&gpui.inner);
        now.store(FAILURE_WINDOW_MS, Ordering::SeqCst);
        note_failure(&gpui.inner);
        assert!(!gpui.given_up(), "a failure exactly a window old does not count");
        // A millisecond inside the window does count.
        let (gpui, now) = editor_with_clock("inside");
        note_failure(&gpui.inner);
        now.store(FAILURE_WINDOW_MS - 1, Ordering::SeqCst);
        note_failure(&gpui.inner);
        assert!(gpui.given_up(), "two failures inside the window stop the trying");
    }

    #[test]
    fn the_giving_up_sticks_after_the_window_has_passed() {
        let (gpui, now) = editor_with_clock("sticky");
        note_failure(&gpui.inner);
        now.store(30_000, Ordering::SeqCst);
        note_failure(&gpui.inner);
        assert!(gpui.given_up());
        // The clock moves on: an editor that falls over once a minute does not keep the tray
        // busy for ever.
        now.store(30_000 + FAILURE_WINDOW_MS + 1, Ordering::SeqCst);
        assert!(gpui.given_up(), "the giving up sticks for the rest of the session");
        assert!(matches!(gpui.open(None), OpenOutcome::React { .. }));
    }

    #[test]
    fn the_giving_up_opens_react() {
        let dir = TempDir::new("gave-up");
        let now = Arc::new(AtomicU64::new(0));
        let clock = {
            let now = now.clone();
            Box::new(move || now.load(Ordering::SeqCst))
        };
        let (tx, _rx) = sink();
        let gpui = GpuiEditor::new(
            dir.path().join(GPUI_EDITOR_EXE),
            clock,
            Box::new(move |event| {
                let _ = tx.send(event);
            }),
        );
        note_failure(&gpui.inner);
        note_failure(&gpui.inner);
        assert!(matches!(gpui.open(None), OpenOutcome::React { .. }));
    }

    #[test]
    fn a_crash_opens_react_on_the_same_page() {
        // The editor cannot be used: React opens on the page that was asked for.
        let event = EditorEvent::Unusable { reason: "it ended with code 3".into() };
        assert_eq!(
            decide(&event, EditorChoice::Gpui, Some("fans".into())),
            Action::OpenReact(Some("fans".into()))
        );
    }

    #[test]
    fn a_switch_editor_from_the_editor_swaps_the_editors() {
        let event = EditorEvent::SwitchEditor(EditorChoice::React);
        assert_eq!(
            decide(&event, EditorChoice::Gpui, Some("fans".into())),
            Action::Switch(EditorChoice::React, Some("fans".into()))
        );
        // The editor that is already open is not swapped for itself.
        assert_eq!(
            decide(&event, EditorChoice::React, Some("fans".into())),
            Action::Nothing
        );
    }

    #[test]
    fn the_prefs_file_is_not_rewritten_when_the_editor_does_not_change() {
        let (app, dir) = app_with("0.8.5-dev.12", EditorChoice::Gpui);
        let handle = app.handle().clone();
        let prefs_path = dir.path().join("prefs.json");
        let ctl = update::try_controller(&handle).unwrap();
        let before = std::fs::metadata(&prefs_path).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        // The same choice again: the file is not written again.
        ctl.set_editor(EditorChoice::Gpui).unwrap();
        let after = std::fs::metadata(&prefs_path).unwrap().modified().unwrap();
        assert_eq!(before, after, "the prefs file was written again for the same choice");
        // A different choice is kept in the file.
        std::thread::sleep(Duration::from_millis(100));
        ctl.set_editor(EditorChoice::React).unwrap();
        let changed = std::fs::metadata(&prefs_path).unwrap().modified().unwrap();
        assert_ne!(before, changed, "the new choice is kept in the file");
    }

    // ---- the live child: the stand-in editor ----

    /// The exe suffix of this platform.
    const EXE_SUFFIX: &str = if cfg!(windows) { ".exe" } else { "" };

    /// The stand-in editor: the example the tests run as the GPUI editor.
    fn fake_editor() -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        // .../target/debug/deps/<test>.exe -> .../target/debug/examples/fake_gpui_editor.exe
        let dir = exe.parent().unwrap().parent().unwrap();
        let path = dir.join("examples").join(format!("fake_gpui_editor{}", EXE_SUFFIX));
        assert!(
            path.is_file(),
            "the stand-in editor was not built: {}",
            path.display()
        );
        path
    }

    /// The stand-in editor, with the environment the test gives it.
    fn stand_in(env: Vec<(&str, String)>) -> (GpuiEditor, std::sync::mpsc::Receiver<EditorEvent>) {
        stand_in_with_log(env, Box::new(|_| {}))
    }

    /// The stand-in editor, with the environment the test gives it and a sink that keeps the
    /// log lines.
    fn stand_in_with_log(
        env: Vec<(&str, String)>,
        log_sink: LogSink,
    ) -> (GpuiEditor, std::sync::mpsc::Receiver<EditorEvent>) {
        let (tx, rx) = sink();
        let gpui = GpuiEditor::new(
            fake_editor(),
            Box::new(now_ms),
            Box::new(move |event| {
                let _ = tx.send(event);
            }),
        )
        .with_env(
            env.into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<Vec<(String, String)>>(),
        )
        .with_log_sink(log_sink);
        (gpui, rx)
    }

    /// The log lines the tray writes, and a sink that keeps them.
    fn log_lines() -> (LogLines, LogSink) {
        let lines: LogLines = Arc::default();
        let kept = lines.clone();
        let sink: LogSink = Box::new(move |line| {
            kept.lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(line.to_string());
        });
        (lines, sink)
    }

    /// How long a test waits for the child to say something.
    const PATIENCE: Duration = Duration::from_secs(20);

    #[test]
    fn the_allowed_page_is_passed_to_the_editor() {
        let dir = TempDir::new("page");
        let page_file = dir.path().join("page.txt");
        let (gpui, rx) = stand_in(vec![(
            "FAKE_GPUI_EDITOR_PAGE_FILE",
            page_file.display().to_string(),
        )]);
        assert!(matches!(gpui.open(Some("fans")), OpenOutcome::Started));
        assert_eq!(rx.recv_timeout(PATIENCE).unwrap(), EditorEvent::Ready);
        assert_eq!(std::fs::read_to_string(&page_file).unwrap(), "fans");
        gpui.close();
    }

    #[test]
    fn a_page_that_is_not_on_the_list_is_not_passed_on() {
        let dir = TempDir::new("bad-page");
        let page_file = dir.path().join("page.txt");
        let (gpui, rx) = stand_in(vec![(
            "FAKE_GPUI_EDITOR_PAGE_FILE",
            page_file.display().to_string(),
        )]);
        assert!(matches!(gpui.open(Some("nowhere")), OpenOutcome::Started));
        assert_eq!(rx.recv_timeout(PATIENCE).unwrap(), EditorEvent::Ready);
        assert_eq!(
            std::fs::read_to_string(&page_file).unwrap(),
            "",
            "a page that is not on the list is not passed on"
        );
        gpui.close();
    }

    #[test]
    fn a_switch_editor_from_the_editor_is_acted_on() {
        let (gpui, rx) = stand_in(vec![(
            "FAKE_GPUI_EDITOR_EMIT",
            r#"{"type":"switch-editor","to":"react"}"#.to_string(),
        )]);
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        assert_eq!(
            rx.recv_timeout(PATIENCE).unwrap(),
            EditorEvent::SwitchEditor(EditorChoice::React)
        );
        gpui.close();
    }

    #[test]
    fn an_over_long_line_is_dropped_and_the_next_line_still_arrives() {
        // The same message under the cap would be acted on; over the cap it is dropped.
        let padded = format!(
            "{{\"type\":\"switch-editor\",\"to\":\"react\"}}{}",
            " ".repeat(EDITOR_IPC_LINE_MAX)
        );
        let (gpui, rx) = stand_in(vec![("FAKE_GPUI_EDITOR_EMIT", padded)]);
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        assert_eq!(
            rx.recv_timeout(PATIENCE).unwrap(),
            EditorEvent::Ready,
            "the line after the long one is still read"
        );
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "the long line acted on nothing"
        );
        gpui.close();
    }

    #[test]
    fn an_unknown_type_is_dropped_and_the_next_line_still_arrives() {
        let (gpui, rx) = stand_in(vec![("FAKE_GPUI_EDITOR_EMIT", r#"{"type":"open"}"#.to_string())]);
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        assert_eq!(
            rx.recv_timeout(PATIENCE).unwrap(),
            EditorEvent::Ready,
            "the line after the unknown one is still read"
        );
        gpui.close();
    }

    #[test]
    fn a_crash_is_reported_with_its_code_and_its_last_words() {
        let (gpui, rx) = stand_in(vec![
            ("FAKE_GPUI_EDITOR_EXIT", "3".to_string()),
            ("FAKE_GPUI_EDITOR_STDERR", "the editor fell over".to_string()),
        ]);
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        let event = rx.recv_timeout(PATIENCE).unwrap();
        let EditorEvent::Unusable { reason } = event else {
            panic!("the crash was not reported: {event:?}");
        };
        assert!(reason.contains('3'), "{reason}");
        assert!(reason.contains("the editor fell over"), "{reason}");
        // The failed start is counted, so the second one in a minute stops the trying.
        let failures = gpui.inner.lock().unwrap_or_else(|p| p.into_inner()).failures.len();
        assert_eq!(failures, 1);
    }

    #[test]
    fn a_grandchild_that_holds_the_pipes_does_not_hide_the_editors_end() {
        let dir = TempDir::new("grandchild");
        let pid_file = dir.path().join("grandchild.pid");
        let (gpui, rx) = stand_in(vec![
            ("FAKE_GPUI_EDITOR_EXIT", "3".to_string()),
            ("FAKE_GPUI_EDITOR_GRANDCHILD", "1".to_string()),
            ("FAKE_GPUI_EDITOR_PID_FILE", pid_file.display().to_string()),
        ]);
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        // The editor ended, but its pipe is still open: the tray has to see the end anyway.
        let event = rx
            .recv_timeout(Duration::from_secs(15))
            .expect("the end of the editor was seen");
        let EditorEvent::Unusable { reason } = event else {
            panic!("the crash was not reported: {event:?}");
        };
        assert!(reason.contains('3'), "{reason}");
        assert!(!gpui.is_running(), "the editor is not running any more");
        // The child that holds the pipes open is ended, so it does not stay behind.
        kill_the_grandchild(&pid_file);
    }

    #[test]
    fn a_child_that_asks_to_switch_twice_switches_once() {
        let ask = r#"{"type":"switch-editor","to":"react"}"#;
        let (gpui, rx) = stand_in(vec![(
            "FAKE_GPUI_EDITOR_EMIT",
            format!("{ask}\n{ask}"),
        )]);
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        // The first ask is acted on.
        assert_eq!(
            rx.recv_timeout(PATIENCE).unwrap(),
            EditorEvent::SwitchEditor(EditorChoice::React)
        );
        // The second one is not: one switch per child life.
        let mut switches = 0;
        while let Ok(event) = rx.recv_timeout(Duration::from_millis(500)) {
            if event == EditorEvent::SwitchEditor(EditorChoice::React) {
                switches += 1;
            }
        }
        assert_eq!(switches, 0, "the child asked to switch twice; it switched once");
        gpui.close();
    }

    #[test]
    fn an_open_while_the_child_is_closing_starts_a_new_child() {
        // The stand-in editor keeps running after a close: the tray has to start a new child
        // under the one that is going away.
        let (gpui, rx) = stand_in(vec![("FAKE_GPUI_EDITOR_IGNORE_CLOSE", "1".to_string())]);
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        assert_eq!(rx.recv_timeout(PATIENCE).unwrap(), EditorEvent::Ready);
        // The child is told to close, and it does not go.
        gpui.close();
        // A click in that moment must not talk to the dying child: a new one starts.
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        assert_eq!(
            rx.recv_timeout(PATIENCE).unwrap(),
            EditorEvent::Ready,
            "the new child is the one that answers"
        );
        assert!(gpui.is_running(), "a live child is running");
        gpui.kill_running();
    }

    #[test]
    fn close_tells_the_editor_to_close() {
        let dir = TempDir::new("close");
        let close_file = dir.path().join("closed.txt");
        let (gpui, _rx) = stand_in(vec![(
            "FAKE_GPUI_EDITOR_CLOSE_FILE",
            close_file.display().to_string(),
        )]);
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        gpui.close();
        let deadline = Instant::now() + PATIENCE;
        while !close_file.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(close_file.exists(), "the editor was told to close");
    }

    #[test]
    fn a_chatty_editor_does_not_fill_the_log() {
        const FLOOD: usize = 200;
        let (lines, log_sink) = log_lines();
        let flood = (0..FLOOD)
            .map(|_| r#"{"type":"ready"}"#.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (gpui, rx) = stand_in_with_log(vec![("FAKE_GPUI_EDITOR_EMIT", flood)], log_sink);
        assert!(matches!(gpui.open(None), OpenOutcome::Started));
        // Every line the editor sent has been read: the flood, and the ready the stand-in
        // editor says at the end.
        let mut seen = 0u32;
        let deadline = Instant::now() + PATIENCE;
        while seen < (FLOOD + 1) as u32 && Instant::now() < deadline {
            seen += rx.try_iter().count() as u32;
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(seen, (FLOOD + 1) as u32, "every line was read");
        // The log took the first few lines and counted the rest.
        let logged = lines
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|line| line.contains("is ready"))
            .count();
        assert_eq!(
            logged,
            LOG_LINE_CAP as usize,
            "the log took only the first lines"
        );
        gpui.close();
    }

    #[test]
    fn the_limiter_counts_the_lines_it_did_not_log() {
        let mut limiter = LogLimiter::new();
        let now = Instant::now();
        for _ in 0..LOG_LINE_CAP {
            assert_eq!(limiter.next(now), LineLog::Line);
        }
        assert_eq!(limiter.next(now), LineLog::Counted);
        assert_eq!(limiter.next(now), LineLog::Counted);
        // The window passes: the counted lines come as one summary line.
        let later = now + LOG_SUMMARY_WINDOW;
        assert_eq!(limiter.next(later), LineLog::Summary(3));
        // The count starts again.
        assert_eq!(limiter.next(later), LineLog::Counted);
    }

    #[test]
    fn a_line_that_ends_with_crlf_parses() {
        let mut reader = std::io::Cursor::new(
            b"{\"type\":\"ready\"}\r\n{\"type\":\"switch-editor\",\"to\":\"react\"}\r\n".to_vec(),
        );
        let first = read_capped_line(&mut reader, EDITOR_IPC_LINE_MAX).unwrap().unwrap();
        assert_eq!(
            first, "{\"type\":\"ready\"}",
            "the carriage return is not part of the message"
        );
        assert_eq!(read_line(&first), Ok(EditorEvent::Ready));
        let second = read_capped_line(&mut reader, EDITOR_IPC_LINE_MAX).unwrap().unwrap();
        assert_eq!(second, "{\"type\":\"switch-editor\",\"to\":\"react\"}");
        assert_eq!(
            read_line(&second),
            Ok(EditorEvent::SwitchEditor(EditorChoice::React))
        );
        // A line that is handed over with its carriage return is read the same way.
        assert_eq!(read_line("{\"type\":\"ready\"}\r"), Ok(EditorEvent::Ready));
    }

    #[test]
    fn a_line_that_is_not_utf8_is_dropped() {
        let mut bytes = b"{\"type\":\"ready\"}".to_vec();
        bytes.push(0xff);
        bytes.push(b'\n');
        let mut reader = std::io::Cursor::new(bytes);
        assert_eq!(
            read_capped_line(&mut reader, EDITOR_IPC_LINE_MAX),
            Err(Dropped::NotUtf8),
            "a line that is not valid UTF-8 is dropped"
        );
        // A message with a broken byte in it is dropped too, and nothing is acted on.
        let mut broken = b"{\"type\":\"switch-editor\",\"to\":\"re".to_vec();
        broken.push(0xff);
        broken.extend_from_slice(b"ct\"}\n");
        let mut reader = std::io::Cursor::new(broken);
        assert_eq!(
            read_capped_line(&mut reader, EDITOR_IPC_LINE_MAX),
            Err(Dropped::NotUtf8)
        );
    }

    /// Ends the child the stand-in editor left behind. It holds the editor's pipes open, and
    /// the test does not leave it running.
    fn kill_the_grandchild(pid_file: &Path) {
        let deadline = Instant::now() + PATIENCE;
        while !pid_file.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let Ok(text) = std::fs::read_to_string(pid_file) else {
            return;
        };
        let Ok(pid) = text.trim().parse::<u32>() else {
            return;
        };
        // token: the person's own session, non-elevated; it ends the test's own stand-in
        // child, not the daemon.
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}
