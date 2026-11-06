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
//! of its session.

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
/// The longest a dropped line is shown in the log.
const LOGGED_LINE: usize = 120;

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
    /// The stream ended or failed.
    Io,
}

/// Reads one line from the editor. `Err` says why the line was dropped, and a dropped line is
/// never acted on.
pub fn read_line(line: &str) -> Result<EditorEvent, Dropped> {
    if line.len() > EDITOR_IPC_LINE_MAX {
        return Err(Dropped::TooLong);
    }
    match serde_json::from_str::<EditorToTray>(line) {
        Ok(EditorToTray::Ready) => Ok(EditorEvent::Ready),
        Ok(EditorToTray::SwitchEditor { to }) => Ok(EditorEvent::SwitchEditor(to)),
        Err(_) => Err(Dropped::Unknown),
    }
}

/// Reads one line from `reader`, at most `max` bytes long.
///
/// A longer line is dropped: the rest of it is thrown away and [`Dropped::TooLong`] comes back,
/// so the reader never holds more than the cap in memory. `Ok(None)` is the end of the stream.
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
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// The GPUI editor child: started by the tray, talked to over its stdio.
pub struct GpuiEditor {
    inner: Arc<Mutex<Inner>>,
    on_event: Arc<dyn Fn(EditorEvent) + Send + Sync>,
}

struct Inner {
    /// The exe's absolute path.
    exe: PathBuf,
    /// The running child, when there is one.
    running: Option<Running>,
    /// The ms of the failed starts that are still inside the window.
    failures: Vec<u64>,
    /// How many times the missing editor has been reported in this session.
    missing_reports: u32,
    /// The clock the failed starts are counted on. The tests move it.
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    /// Extra environment for the child. The production child inherits the tray's.
    env: Vec<(String, String)>,
}

/// The running child. Cloned into the reader thread, so it can end the child and say how it
/// ended.
#[derive(Clone)]
struct Running {
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    /// Set when the tray asked the editor to close: its end is not a failure.
    closing: Arc<AtomicBool>,
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
                missing_reports: 0,
                clock,
                env: Vec::new(),
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

    /// The exe's path.
    pub fn exe(&self) -> PathBuf {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).exe.clone()
    }

    /// True while the editor is running.
    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).running.is_some()
    }

    /// True when two starts failed inside the window: React is used for the rest of this session.
    pub fn given_up(&self) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let now = (inner.clock)();
        inner
            .failures
            .iter()
            .filter(|at| now.saturating_sub(**at) < FAILURE_WINDOW_MS)
            .count()
            >= FAILURE_LIMIT
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
            if let Some(page) = page {
                send(&running, &TrayToEditor::Navigate { page: page.to_string() });
            }
            // The tray has no handle to another process's window, so it cannot focus it.
            log::append("editor: the GPUI editor is already open; focusing its window is UNKNOWN");
            return OpenOutcome::Started;
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

    /// Starts the child and the thread that reads it.
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
        let child = Arc::new(Mutex::new(child));
        let closing = Arc::new(AtomicBool::new(false));
        {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.running = Some(Running {
                child: child.clone(),
                stdin: Arc::new(Mutex::new(stdin)),
                closing: closing.clone(),
            });
        }
        log::append(&format!("editor: the GPUI editor was started ({})", exe.display()));
        self.read_until_the_end(child, stdout, stderr, closing);
        Ok(())
    }

    /// Reads the child until it ends, and says how it ended.
    fn read_until_the_end(
        &self,
        child: Arc<Mutex<Child>>,
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
        closing: Arc<AtomicBool>,
    ) {
        let on_event = self.on_event.clone();
        let inner = self.inner.clone();
        let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
        // The stderr reader keeps the last lines: they are the log of a crash.
        let stderr_thread = {
            let tail = tail.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                while let Ok(Some(line)) = read_capped_line(&mut reader, EDITOR_IPC_LINE_MAX) {
                    let mut lines = tail.lock().unwrap_or_else(|p| p.into_inner());
                    lines.push_back(line);
                    while lines.len() > STDERR_TAIL {
                        lines.pop_front();
                    }
                }
            })
        };
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_capped_line(&mut reader, EDITOR_IPC_LINE_MAX) {
                    Ok(Some(line)) => match read_line(&line) {
                        Ok(event) => on_event(event),
                        Err(Dropped::TooLong) => log::append(&format!(
                            "editor: a line from the GPUI editor was longer than {EDITOR_IPC_LINE_MAX} bytes and was dropped"
                        )),
                        Err(Dropped::Unknown) => log::append(&format!(
                            "editor: a line from the GPUI editor was not a message the tray knows and was dropped: {}",
                            logged(&line)
                        )),
                        Err(Dropped::Io) => {}
                    },
                    Ok(None) => break,
                    Err(Dropped::TooLong) => log::append(&format!(
                        "editor: a line from the GPUI editor was longer than {EDITOR_IPC_LINE_MAX} bytes and was dropped"
                    )),
                    Err(Dropped::Unknown) | Err(Dropped::Io) => break,
                }
            }
            // The editor ended: the code it ended with, and its last words on stderr.
            let code = wait_for_exit(&child);
            // The child is gone, so its stderr is closed: the reader has what is left of it.
            let _ = stderr_thread.join();
            let stderr_tail = tail
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ");
            let asked_to_close = closing.load(Ordering::SeqCst);
            {
                let mut guard = inner.lock().unwrap_or_else(|p| p.into_inner());
                let same = guard
                    .running
                    .as_ref()
                    .is_some_and(|r| Arc::ptr_eq(&r.child, &child));
                if same {
                    guard.running = None;
                }
            }
            if asked_to_close {
                log::append("editor: the GPUI editor closed");
                return;
            }
            match code {
                Some(0) => log::append("editor: the GPUI editor ended"),
                Some(code) => {
                    let reason = format!(
                        "the GPUI editor ended with code {code}{}",
                        if stderr_tail.is_empty() {
                            String::new()
                        } else {
                            format!(" ({stderr_tail})")
                        }
                    );
                    log::append(&format!("editor: {reason}; the React editor opens instead"));
                    note_failure(&inner);
                    on_event(EditorEvent::Unusable { reason });
                }
                None => {
                    let reason = format!(
                        "the GPUI editor did not end{}",
                        if stderr_tail.is_empty() {
                            String::new()
                        } else {
                            format!(" ({stderr_tail})")
                        }
                    );
                    log::append(&format!("editor: {reason}; the React editor opens instead"));
                    note_failure(&inner);
                    on_event(EditorEvent::Unusable { reason });
                }
            }
        });
    }
}

/// The milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Records a failed start, and forgets the ones that are outside the window.
fn note_failure(inner: &Mutex<Inner>) {
    let mut guard = inner.lock().unwrap_or_else(|p| p.into_inner());
    let now = (guard.clock)();
    guard.failures.retain(|at| now.saturating_sub(*at) < FAILURE_WINDOW_MS);
    guard.failures.push(now);
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
            log::append("editor: the GPUI editor is ready");
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
        let dir = TempDir::new("guard");
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
        assert!(!gpui.given_up(), "one failure is not enough");
        now.store(30_000, Ordering::SeqCst);
        note_failure(&gpui.inner);
        assert!(gpui.given_up(), "two failures inside the window stop the trying");
        // The window moves with the clock: an old failure does not count for ever.
        now.store(30_000 + FAILURE_WINDOW_MS + 1, Ordering::SeqCst);
        assert!(!gpui.given_up());
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
        );
        (gpui, rx)
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
}
