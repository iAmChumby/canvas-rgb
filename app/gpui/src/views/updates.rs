//! `UpdatesView`, after `app/web/src/views/UpdatesView.tsx` (and
//! `releaseNotes.ts` / `updateWords.ts` logic).
//!
//! The port keeps the React view's rules (the checklist is
//! `docs/gpui-parity/updates.md`):
//! - the Update now key (`KEY_LABELS` + `downloadingLabel` for the
//!   downloading step) and the 4 px slot under it, filled from real bytes,
//!   with the unknown-length slot lit end to end;
//! - the state line (`stateLine` / `stateTone`), reserved to the longest
//!   line so nothing under it moves;
//! - the UAC sentence under the key, Skip this version (or the skipped
//!   note) and, after an install failure, Open logs folder;
//! - the "Up to date" panel with Check now and the Last checked line when
//!   there is no available release;
//! - the notes column: one panel per `###` heading, the item bars in the
//!   heading's tone, the blank-notes fallback ("No notes were published"),
//!   and nothing when there is no release at all;
//! - the silent tray error after 5 s without an answer, with Try again
//!   re-asking, and the dismiss of the "Updated to" marker once per
//!   release on arrival.
//!
//! The tray link is the stdio protocol Settings already uses: the state
//! arrives as `TrayToEditor::UpdateState` (the shell copies it here from
//! the settings view, which drains the receiver — there is only one
//! reader) and commands leave as `EditorToTray::Update*` lines. Without a
//! tray (stdin not a pipe) there is no state, so the page follows React's
//! browser-with-no-fixture treatment: loading, then the silent tray
//! error, and Try again repeats it. The React demo fixtures (`?update=`)
//! are demo-backend behavior and have no GPUI counterpart.
//!
//! The tray never replies to a command, so a failed command is logged by
//! the tray and shows nothing here. `err` is therefore always `None`
//! today; the field stays so a future reply can surface.

use canvas_proto::tauri::{CheckFailReason, UpdateInfo, UpdatePhase, UpdateState};

use crate::views::settings::{fmt_checked, today_day_utc, tray_acts};

// ---------------------------------------------------------------------------
// Logic, ported from `updateWords.ts` / `releaseNotes.ts` and covered by the
// tests below.

/// How long the tray has to answer before the page says that it did not
/// (`TRAY_TIMEOUT_MS`).
pub const TRAY_TIMEOUT_MS: u64 = 5000;
/// How long the loading placeholder stays invisible (`LOAD_DELAY_MS`).
pub const LOAD_DELAY_MS: u64 = 150;
/// The words a screen reader hears while the state is reading.
#[allow(dead_code)]
pub const LOADING_LABEL: &str = "Reading the update state";
/// The silent tray error's title (steps.md: "No answer from the tray").
pub const SILENT_TITLE: &str = "No answer from the tray";
/// The silent tray error's summary.
pub const SILENT_SUMMARY: &str = "The tray app did not answer. Start it, then try again.";

/// The Update now key's labels (`KEY_LABELS`).
pub const KEY_LABELS: [&str; 6] = [
    "Update now",
    "Downloading 100%",
    "Checking download",
    "Waiting for Windows",
    "Installing",
    "Try again",
];

/// The longest state line; the state line reserves room for it.
pub const LONGEST_STATE: &str =
    "The download failed its check and was deleted. Nothing was installed.";

fn mb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / (1024.0 * 1024.0))
}

/// True while the tray is downloading, checking the download, waiting for
/// Windows or installing (`isInstalling`).
pub fn is_installing(phase: &UpdatePhase) -> bool {
    matches!(
        phase,
        UpdatePhase::Downloading { .. }
            | UpdatePhase::Verifying
            | UpdatePhase::WaitingForUac
            | UpdatePhase::Installing
    )
}

/// Which Swap label of the Update now key is showing: 0 Update now … 4
/// Installing, 5 Try again.
pub fn key_step(phase: &UpdatePhase) -> usize {
    match phase {
        UpdatePhase::Downloading { .. } => 1,
        UpdatePhase::Verifying => 2,
        UpdatePhase::WaitingForUac => 3,
        UpdatePhase::Installing => 4,
        UpdatePhase::InstallFailed { .. } => 5,
        _ => 0,
    }
}

/// The Update now key's visible label: the downloading label at step 1,
/// otherwise `KEY_LABELS[step]`.
pub fn key_label(phase: &UpdatePhase) -> String {
    let step = key_step(phase);
    if step == 1 {
        downloading_label(phase)
    } else {
        KEY_LABELS[step].to_string()
    }
}

/// The text of the Downloading label: a percent when the length is known,
/// megabytes when not.
pub fn downloading_label(phase: &UpdatePhase) -> String {
    let UpdatePhase::Downloading {
        downloaded,
        total,
    } = phase
    else {
        return KEY_LABELS[1].to_string();
    };
    match total {
        Some(total) if *total > 0 => {
            format!("Downloading {}%", (*downloaded as f64 / *total as f64 * 100.0).floor())
        }
        _ => format!("Downloading {} MB", mb(*downloaded)),
    }
}

/// How full the slot under the key is, 0 to 1, or `None` when the length
/// is not known (`slotFill`).
pub fn slot_fill(phase: &UpdatePhase) -> Option<f32> {
    match phase {
        UpdatePhase::Downloading {
            downloaded,
            total,
        } => match total {
            Some(total) if *total > 0 => Some((*downloaded as f32 / *total as f32).min(1.0)),
            _ => None,
        },
        UpdatePhase::Verifying | UpdatePhase::WaitingForUac | UpdatePhase::Installing => {
            Some(1.0)
        }
        _ => Some(0.0),
    }
}

/// The Downloading percent `aria-valuenow` carries (`Math.round`); the key
/// label uses `Math.floor` above. They disagree by design.
#[allow(dead_code)]
pub fn aria_percent(phase: &UpdatePhase) -> Option<i32> {
    slot_fill(phase).map(|fill| (fill * 100.0).round() as i32)
}

/// The one sentence the person reads before Windows asks for permission
/// (`uacLine`).
pub fn uac_line(version: &str) -> String {
    format!(
        "Windows will ask for permission to install {version}. While it installs, canvas-rgb stops for a moment: the lights pause and the fans run on your motherboard's settings. Choosing No leaves everything as it is."
    )
}

/// Whether the available version is the one the person skipped
/// (`isSkipped`).
pub fn is_skipped(u: &UpdateState) -> bool {
    u.available
        .as_ref()
        .is_some_and(|a| u.skipped_version.as_deref() == Some(a.version.as_str()))
}

/// Whether the Check now key is enabled for this state.
pub fn check_now_enabled(u: &UpdateState, acts: bool) -> bool {
    acts && !is_installing(&u.phase) && u.phase != UpdatePhase::Checking
}

/// Whether the Check now key is busy (its label reads "Checking").
pub fn check_now_busy(u: &UpdateState) -> bool {
    u.phase == UpdatePhase::Checking
}

/// What the installer said, when it said a sentence a person can read. An
/// exit code, a hex code or a bare word is not one, and gets the plain
/// line instead (`installerSentence`: the regex
/// `/exit(ed)?\s+(with\s+)?code|0x[0-9a-f]+|error\s*\d+|^\W*\d+\W*$/i`).
pub fn installer_sentence(message: &str) -> String {
    let m = message.trim();
    let lower = m.to_lowercase();
    let raw = lower.contains("exit") || is_hex_code(&lower) || has_error_number(&lower) || is_bare_number(m);
    if m.is_empty() || raw || !m.contains(char::is_whitespace) {
        return "The installer stopped before it finished.".to_string();
    }
    let mut chars = m.chars();
    let cap = match chars.next() {
        Some(first) => first.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    };
    if cap.ends_with(['.', '!', '?']) {
        cap
    } else {
        format!("{cap}.")
    }
}

fn is_bare_number(m: &str) -> bool {
    // ^\W*\d+\W*$ : only digits and punctuation/space.
    m.chars().any(|c| c.is_ascii_digit())
        && m.chars().all(|c| !c.is_alphabetic())
}

/// `0x[0-9a-f]+`: a `0x` followed by at least one hex digit.
fn is_hex_code(lower: &str) -> bool {
    let Some(at) = lower.find("0x") else {
        return false;
    };
    lower[at + 2..]
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_hexdigit())
}

fn has_error_number(m: &str) -> bool {
    // error\s*\d+ : the word "error" followed by a number.
    let lower = m.to_lowercase();
    let Some(at) = lower.find("error") else {
        return false;
    };
    let rest = &lower[at + 5..];
    rest.trim_start().chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// The sentence after an install that did not finish. The version is
/// named only when it is not the version the failed update was for
/// (`installFailedSentence`).
pub fn install_failed_sentence(message: &str, current: &str, failed_for: Option<&str>) -> String {
    let first = installer_sentence(message);
    if current.is_empty() {
        return first;
    }
    if let Some(failed) = failed_for {
        if same_version(failed, current) {
            return first;
        }
    }
    format!("{first} canvas-rgb is still on {current}.")
}

/// "0.3.0" and "v0.3.0" are the same version.
pub fn same_version(a: &str, b: &str) -> bool {
    fn norm(v: &str) -> &str {
        let mut v = v.trim();
        if v.len() >= 2 && (v.starts_with('v') || v.starts_with('V')) {
            v = &v[1..];
        }
        v
    }
    norm(a) == norm(b)
}

/// The state line of the updates page (`stateLine`).
pub fn state_line(u: &UpdateState) -> String {
    let v = u.available.as_ref().map(|a| a.version.as_str()).unwrap_or("");
    match &u.phase {
        UpdatePhase::Checking => "Checking for updates…".to_string(),
        UpdatePhase::Downloading {
            downloaded,
            total,
        } => match total {
            Some(total) if *total > 0 => {
                format!("Downloading {v}: {} of {} MB.", mb(*downloaded), mb(*total))
            }
            _ => format!("Downloading {v}: {} MB.", mb(*downloaded)),
        },
        UpdatePhase::Verifying => {
            "Checking the download against canvas-rgb's release key.".to_string()
        }
        UpdatePhase::VerifyFailed => {
            "The download failed its check and was deleted. Nothing was installed.".to_string()
        }
        UpdatePhase::WaitingForUac => {
            format!("Windows will ask for permission to install {v}.")
        }
        UpdatePhase::Declined => "Permission declined. Nothing was changed.".to_string(),
        UpdatePhase::Installing => format!("Installing {v}."),
        UpdatePhase::CheckFailed { reason } => match reason {
            CheckFailReason::Offline => "Could not check for updates: no connection.".to_string(),
            CheckFailReason::RateLimited => {
                "Could not check for updates: GitHub rate limit.".to_string()
            }
            CheckFailReason::Other => "Could not check for updates.".to_string(),
        },
        UpdatePhase::InstallFailed { message } => install_failed_sentence(
            message,
            &u.current_version,
            u.available.as_ref().map(|a| a.version.as_str()),
        ),
        UpdatePhase::UpToDate | UpdatePhase::Idle | UpdatePhase::Available => {
            if let Some(available) = &u.available {
                return format!("{} available.", available.version);
            }
            if let Some(updated) = &u.updated_to {
                return format!("Updated to {}.", updated.version);
            }
            "Up to date.".to_string()
        }
    }
}

/// The tone of the dot beside the state line (`stateTone`).
pub fn state_tone(u: &UpdateState) -> &'static str {
    match &u.phase {
        UpdatePhase::CheckFailed { .. }
        | UpdatePhase::VerifyFailed
        | UpdatePhase::InstallFailed { .. } => "down",
        UpdatePhase::Declined => "warn",
        UpdatePhase::Checking
        | UpdatePhase::Downloading { .. }
        | UpdatePhase::Verifying
        | UpdatePhase::WaitingForUac
        | UpdatePhase::Installing => "work",
        _ => {
            if u.available.is_some() {
                "work"
            } else {
                "ok"
            }
        }
    }
}

/// The panel title tone: what a heading of the changelog means, as a token
/// name (`noteTone`).
pub fn note_tone(title: &str) -> &'static str {
    let t = title.to_lowercase();
    if t.starts_with("add") {
        "ok"
    } else if t.starts_with("fix") {
        "warn"
    } else if t.starts_with("remov") || t.starts_with("deprecat") || t.starts_with("secur") {
        "down"
    } else {
        "work"
    }
}

/// A release-notes section: a heading and its items (`NoteBlock`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteBlock {
    pub title: String,
    pub items: Vec<String>,
}

/// A tiny reader for the release notes (`parseNotes`): the CHANGELOG
/// section of one release, written as `### Added` headings over `- item`
/// lists. `#` and `##` name the release itself and are skipped; a wrapped
/// line continues the previous item; a plain line is its own item.
pub fn parse_notes(md: &str) -> Vec<NoteBlock> {
    let md = md.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks: Vec<NoteBlock> = Vec::new();
    let mut cur: Option<usize> = None;
    let mut open = false;
    let ensure = |blocks: &mut Vec<NoteBlock>, cur: &mut Option<usize>| -> usize {
        match *cur {
            Some(i) => i,
            None => {
                blocks.push(NoteBlock {
                    title: "Notes".to_string(),
                    items: Vec::new(),
                });
                let i = blocks.len() - 1;
                *cur = Some(i);
                i
            }
        }
    };
    for raw in md.split('\n') {
        let line = raw.trim_end();
        if line.trim().is_empty() {
            open = false;
            continue;
        }
        if let Some(head) = heading(line) {
            let (level, rest) = head;
            if level >= 3 {
                // Strip trailing `#` and spaces; empty becomes "Notes".
                let title = rest.trim_end_matches('#').trim();
                let title = if title.is_empty() {
                    "Notes".to_string()
                } else {
                    title.to_string()
                };
                blocks.push(NoteBlock {
                    title,
                    items: Vec::new(),
                });
                cur = Some(blocks.len() - 1);
            }
            open = false;
            continue;
        }
        if let Some(bullet) = bullet(line) {
            let i = ensure(&mut blocks, &mut cur);
            blocks[i].items.push(bullet.trim().to_string());
            open = true;
            continue;
        }
        let i = ensure(&mut blocks, &mut cur);
        if open && raw.starts_with(char::is_whitespace) && !blocks[i].items.is_empty() {
            blocks[i].items.last_mut().unwrap().push(' ');
            blocks[i].items.last_mut().unwrap().push_str(line.trim());
        } else {
            blocks[i].items.push(line.trim().to_string());
            open = false;
        }
    }
    blocks.retain(|b| !b.items.is_empty());
    blocks
}

/// The heading a line names, if it is one: its level and the rest of the
/// line (`/^(#{1,6})\s+(.*)$/`).
fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some((hashes, rest.trim_start()))
}

/// The item a bullet line names, if it is one (`/^\s*[-*+]\s+(.*)$/`).
fn bullet(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let first = trimmed.chars().next()?;
    if first != '-' && first != '*' && first != '+' {
        return None;
    }
    let rest = &trimmed[1..];
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest.trim_start())
}

/// One inline part of a note item (`Inline`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(String),
}

/// Splits one item into text, `code` and **bold**. A link keeps its words
/// and loses its address (`parseInline`).
pub fn parse_inline(text: &str) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let bytes = text.as_bytes();
    let mut last = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        // Find the next of `` ` ``, `**`, or `[`.
        let rest = &text[i..];
        let code_at = rest.find('`');
        let strong_at = rest.find("**");
        let link_at = rest.find('[');
        let next = match (code_at, strong_at, link_at) {
            (Some(a), Some(b), Some(c)) => (a.min(b).min(c), a, b, c),
            (Some(a), Some(b), None) => (a.min(b), a, b, usize::MAX),
            (Some(a), None, Some(c)) => (a.min(c), a, usize::MAX, c),
            (None, Some(b), Some(c)) => (b.min(c), usize::MAX, b, c),
            (Some(a), None, None) => (a, a, usize::MAX, usize::MAX),
            (None, Some(b), None) => (b, usize::MAX, b, usize::MAX),
            (None, None, Some(c)) => (c, usize::MAX, usize::MAX, c),
            (None, None, None) => break,
        };
        let (at, code_i, strong_i, link_i) = next;
        if at > 0 {
            out.push(Inline::Text(text[i..i + at].to_string()));
        }
        let start = i + at;
        if code_i != usize::MAX {
            let after = start + 1;
            if let Some(end) = text[after..].find('`') {
                out.push(Inline::Code(text[after..after + end].to_string()));
                i = after + end + 1;
                last = i;
                continue;
            }
        } else if strong_i != usize::MAX {
            let after = start + 2;
            // `\*\*([^*]+)\*\*`: the words hold no `*`.
            if let Some(end) = text[after..].find("**") {
                let words = &text[after..after + end];
                if !words.contains('*') {
                    out.push(Inline::Strong(words.to_string()));
                    i = after + end + 2;
                    last = i;
                    continue;
                }
            }
        } else if link_i != usize::MAX {
            // `[words](address)` keeps the words; the address is dropped.
            if let Some(close) = text[start + 1..].find(']') {
                let words = &text[start + 1..start + 1 + close];
                let after = start + 1 + close + 1;
                if text[after..].starts_with('(') {
                    if let Some(paren) = text[after..].find(')') {
                        out.push(Inline::Text(words.to_string()));
                        i = after + paren + 1;
                        last = i;
                        continue;
                    }
                }
            }
        }
        // Not a valid inline span: treat the marker as text and move on.
        out.push(Inline::Text(text[start..start + 1].to_string()));
        i = start + 1;
        last = i;
    }
    if last < text.len() {
        out.push(Inline::Text(text[last..].to_string()));
    }
    out
}

use gpui::{Context, IntoElement, Window, div, prelude::*, px};

use crate::components::button::{Variant, button, button_sm, skirt_layers};
use crate::components::panel::{state_dot, tracked_title};
use crate::theme::{Theme, TokenRadius};

// ---------------------------------------------------------------------------
// The view.

pub struct UpdatesView {
    pub theme: Theme,
    /// The latest tray state, copied here by the shell from the settings
    /// view (the single reader of the stdio receiver).
    pub update: Option<UpdateState>,
    /// A tray is attached when stdin is a pipe (`canUpdate()`).
    pub tray_present: bool,
    /// Five seconds without an answer: the silent tray error.
    pub silent: bool,
    /// How many times the page has asked the tray (Try again).
    pub asked: u32,
    /// The installed release's notes, captured on arrival so they stay on
    /// screen after the dismiss clears `updated_to` (`seenNotes`).
    pub seen_notes: Option<UpdateInfo>,
    /// The release the dismiss was sent for (`dismissedFor`).
    pub dismissed_for: Option<String>,
    /// A failed command's words (always `None` today: the tray never replies).
    pub err: Option<String>,
    /// The Update now key's error (same).
    pub key_err: Option<String>,
    /// The loading placeholder has been visible for `LOAD_DELAY_MS`.
    pub loading_shown: bool,
    /// The current silent-timer generation.
    generation: u64,
    /// The window width, for the page's breakpoints.
    pub width: f32,
    /// The shell, for the notices that navigate away (unused today).
    #[allow(dead_code)]
    pub shell: Option<gpui::Entity<crate::shell::Shell>>,
}

impl UpdatesView {
    pub fn new(
        theme: Theme,
        cx: &mut Context<Self>,
        tray_present: bool,
        shell: Option<gpui::Entity<crate::shell::Shell>>,
    ) -> Self {
        let mut view = UpdatesView {
            theme,
            update: None,
            tray_present,
            silent: false,
            asked: 0,
            seen_notes: None,
            dismissed_for: None,
            err: None,
            key_err: None,
            loading_shown: false,
            generation: 0,
            width: 1280.0,
            shell,
        };
        view.start_silent_timer(cx);
        view.start_load_delay(cx);
        view
    }

    /// Takes a fresh tray state (the shell's copy from the settings view):
    /// stores it and, when `updated_to` is set for a release the dismiss
    /// was not sent for yet, sends the dismiss once and keeps the notes.
    pub fn set_update(&mut self, state: Option<UpdateState>, cx: &mut Context<Self>) {
        let changed = self.update != state;
        if !changed {
            return;
        }
        self.update = state.clone();
        if let Some(state) = state {
            if let Some(updated) = &state.updated_to {
                if self.dismissed_for.as_deref() != Some(updated.version.as_str()) {
                    self.dismissed_for = Some(updated.version.clone());
                    // The tray never replies to a command, so a failure is
                    // shown nowhere, as React does (the effect ignores it).
                    crate::send_update_dismiss_updated();
                    self.seen_notes = Some(updated.clone());
                }
            }
        }
        cx.notify();
    }

    /// The strip's one-line context (React `PageTitle` context):
    /// `<current_version>` plus the arrow and `<available version>` when a
    /// release is available. Empty when the state is not there yet.
    pub fn strip_context_text(&self) -> String {
        match &self.update {
            Some(u) => match &u.available {
                Some(available) => {
                    format!("{} → {}", u.current_version, available.version)
                }
                None => u.current_version.clone(),
            },
            None => String::new(),
        }
    }

    /// Try again (the silent error's button): run the 5 s deadline again
    /// and ask the tray for its state again. React asks through
    /// `refreshUpdate()` (`update_state`); the GPUI form of that ask is
    /// `ready`, which the tray answers with the current state and prefs.
    fn retry(&mut self, cx: &mut Context<Self>) {
        self.silent = false;
        self.asked += 1;
        self.generation += 1;
        let generation = self.generation;
        self.start_silent_timer_with(generation, cx);
        crate::send_ready();
        cx.notify();
    }

    fn start_silent_timer(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.start_silent_timer_with(generation, cx);
    }

    fn start_silent_timer_with(&mut self, generation: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(TRAY_TIMEOUT_MS))
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.generation == generation && this.update.is_none() {
                    this.silent = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn start_load_delay(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(LOAD_DELAY_MS))
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.update.is_none() {
                    this.loading_shown = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Update now (also Try again after an install failure): clears the
    /// key's error and sends `update_install`.
    fn install(&mut self, cx: &mut Context<Self>) {
        self.key_err = None;
        crate::send_update_install();
        cx.notify();
    }

    /// Check now: clears the page error and sends `update_check_now`.
    fn check_now(&mut self, cx: &mut Context<Self>) {
        self.err = None;
        crate::send_update_check_now();
        cx.notify();
    }

    /// Skip this version: clears the page error and sends `update_skip`.
    fn skip(&mut self, cx: &mut Context<Self>) {
        self.err = None;
        crate::send_update_skip();
        cx.notify();
    }

    /// Open logs folder: clears the page error and sends
    /// `open_logs_folder`. The tray logs a failure; there is no reply to
    /// show, as with every command here.
    fn open_logs(&mut self, cx: &mut Context<Self>) {
        self.err = None;
        crate::send_open_logs_folder();
        cx.notify();
    }
}

// ---------------------------------------------------------------------------
// The drawing.

impl UpdatesView {
    fn panel(
        &self,
        id: &str,
        title: &str,
        body: impl IntoElement,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(gpui::SharedString::from(id.to_string()))
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .p_4()
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .mb_3()
                    .child(tracked_title(
                        &title.to_uppercase(),
                        13.0,
                        0.08,
                        gpui::FontWeight::MEDIUM,
                        crate::fonts::MONO_FAMILY,
                        Theme::color(t.mute),
                    )),
            )
            .child(body)
    }

    fn error_line(&self, msg: &str, mt: bool) -> impl IntoElement {
        let t = &self.theme;
        let el = div()
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.down))
            .child(msg.to_string());
        if mt {
            el.mt_2()
        } else {
            el
        }
    }

    fn note(&self, words: &str) -> impl IntoElement {
        let t = &self.theme;
        div()
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.mute))
            .child(words.to_string())
    }

    /// The Update now key, busy and fully lit while the update runs
    /// (`.btn[aria-busy="true"]:disabled`), and the 4 px slot under it
    /// that fills from real bytes. No spinner.
    fn update_key(&self, u: &UpdateState, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let label = key_label(&u.phase);
        let fill = slot_fill(&u.phase);
        let busy = is_installing(&u.phase);
        let acts = tray_acts(self.tray_present);
        let entity = cx.entity();
        let key: gpui::AnyElement = if busy {
            // Fully lit and still: the progress reads as the key's own
            // state, not as a dead control. No click, no dimming.
            div()
                .id(gpui::SharedString::from("updates-key"))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .h(px(32.0))
                .px(px(12.0 + 1.0))
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.on_accent))
                .child(skirt_layers(
                    t.live_skirt(),
                    10.0,
                    32.0,
                    t.live(),
                    gpui::Rgba {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    },
                ))
                .child(label)
                .into_any_element()
        } else {
            button(
                t,
                "updates-key",
                &label,
                Variant::Primary,
                !acts,
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.install(cx));
                },
            )
            .into_any_element()
        };
        let slot_w = 0.0f32; // relative fill below
        let _ = slot_w;
        let fill_el = match fill {
            None => div()
                .absolute()
                .inset_0()
                .rounded(px(2.0))
                .bg(t.live_soft()),
            Some(p) => div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .rounded(px(2.0))
                .bg(t.live())
                .w(gpui::relative(p)),
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(key);
        col = col.child(
            div()
                .id(gpui::SharedString::from("updates-slot"))
                .relative()
                .h(px(4.0))
                .rounded(px(2.0))
                .bg(Theme::color(t.sunk))
                .border_1()
                .border_color(Theme::color(t.line))
                .overflow_hidden()
                .child(fill_el),
        );
        if let Some(err) = &self.key_err {
            col = col.child(self.error_line(err, false));
        }
        col
    }

    /// The state line. A hidden copy of the longest line shares its cell,
    /// so the line is as tall as it can get (`.upd-state`).
    fn state_line(&self, u: &UpdateState) -> impl IntoElement {
        let t = &self.theme;
        let line = state_line(u);
        let size = if line.len() > LONGEST_STATE.len() {
            line.clone()
        } else {
            LONGEST_STATE.to_string()
        };
        div()
            .relative()
            .mt_4()
            .child(
                // The hidden sizing copy: same dot and words, tone mute.
                state_dot(t, "mute", &size),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(state_dot(t, state_tone(u), &line)),
            )
    }

    /// The available panel: the key, the state line, the UAC sentence, Skip
    /// (or the skipped note), Open logs folder after an install failure,
    /// and the "Tray app only" note when there is no tray.
    fn available_panel(&self, u: &UpdateState, cx: &mut Context<Self>) -> impl IntoElement {
        let acts = tray_acts(self.tray_present);
        let busy = is_installing(&u.phase);
        let version = u
            .available
            .as_ref()
            .map(|a| a.version.clone())
            .unwrap_or_default();
        let title = format!("canvas-rgb {version}");
        let mut body = div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .child(self.update_key(u, cx))
            .child(self.state_line(u));
        body = body.child(
            div()
                .mt_3()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(self.theme.mute))
                .child(uac_line(&version)),
        );
        let mut more = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .mt_4()
            .min_h(px(28.0));
        if !is_skipped(u) {
            let entity = cx.entity();
            more = more.child(button_sm(
                &self.theme,
                "updates-skip",
                "Skip this version",
                Variant::Ghost,
                busy || !acts,
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.skip(cx));
                },
            ));
        } else {
            more = more.child(self.note(&format!(
                "{version} is skipped. Update now still works."
            )));
        }
        if matches!(u.phase, UpdatePhase::InstallFailed { .. }) {
            // The button is disabled exactly when there is no tray host.
            let entity = cx.entity();
            more = more.child(button_sm(
                &self.theme,
                "updates-logs",
                "Open logs folder",
                Variant::Ghost,
                !acts,
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.open_logs(cx));
                },
            ));
        }
        // The "Tray app only" note: shown when there is no tray, or when the
        // install failed with no tray host. In GPUI the second is the same
        // state as the first (no tray host means no `acts`), so one note
        // stands for both.
        if !acts {
            more = more.child(self.note("Tray app only"));
        }
        body = body.child(more);
        if let Some(err) = &self.err {
            body = body.child(self.error_line(err, true));
        }
        self.panel("updates-available", &title, body)
    }

    /// The "Up to date" panel: the state line, Check now, and the Last
    /// checked line. The only place Check now appears.
    fn up_to_date_panel(&self, u: &UpdateState, cx: &mut Context<Self>) -> impl IntoElement {
        let acts = tray_acts(self.tray_present);
        let checking = check_now_busy(u);
        let entity = cx.entity();
        let label = if checking { "Checking" } else { "Check now" };
        let check = button(
            &self.theme,
            "updates-check",
            label,
            Variant::Default,
            !check_now_enabled(u, acts),
            move |_, _, cx| {
                entity.update(cx, |this, cx| this.check_now(cx));
            },
        );
        let today = today_day_utc();
        let last = if acts {
            format!("Last checked {}", fmt_checked(u.last_check.as_deref(), today))
        } else {
            "Tray app only".to_string()
        };
        let mut body = div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .child(self.state_line(u));
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.0))
                .mt_4()
                .min_h(px(28.0))
                .child(check)
                .child(self.note(&last)),
        );
        if let Some(err) = &self.err {
            body = body.child(self.error_line(err, true));
        }
        self.panel("updates-up-to-date", "Up to date", body)
    }

    /// One notes panel per `###` heading: the title, then a list whose
    /// items carry an 8 x 2 px bar at `top: 0.7em` in the tone colour.
    fn note_panel(&self, block: &NoteBlock) -> impl IntoElement {
        let t = &self.theme;
        let tone_color = match note_tone(&block.title) {
            "ok" => Theme::color(t.ok),
            "warn" => Theme::color(t.warn),
            "down" => Theme::color(t.down),
            _ => Theme::color(t.work),
        };
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .min_w(px(0.0));
        for item in &block.items {
            let mut row = div()
                .flex()
                .min_w(px(0.0))
                .pl_4()
                .relative();
            // The bar: 8 x 2 px at top 0.7em (10.5 px at 15 px).
            row = row.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(10.5))
                    .w(px(8.0))
                    .h(px(2.0))
                    .bg(tone_color),
            );
            let mut line = div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(px(15.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim));
            for part in parse_inline(item) {
                line = match part {
                    Inline::Text(words) => line.child(words),
                    Inline::Code(words) => line.child(
                        div()
                            .px(px(4.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(Theme::color(t.sunk))
                            .text_sm()
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.dim))
                            .child(words),
                    ),
                    Inline::Strong(words) => line.child(
                        div()
                            .font_weight(gpui::FontWeight(600.0))
                            .text_color(Theme::color(t.ink))
                            .child(words),
                    ),
                };
            }
            list = list.child(row.child(line));
        }
        self.panel("updates-note", &block.title, list)
    }

    /// The loading shape (`UpdatesShape`): the update panel and the three
    /// note panels, as tall as the loaded page so nothing moves.
    fn loading_shape(&self, phone: bool) -> impl IntoElement {
        let t = &self.theme;
        let skel = |w: f32, h: f32| {
            div()
                .w(px(w))
                .h(px(h))
                .r_sm()
                .bg(Theme::color(t.raised))
        };
        let panel_box = |h: f32| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .r_lg()
                .border_1()
                .border_color(Theme::color(t.line))
                .bg(Theme::color(t.panel))
                .h(px(h))
                .child(skel(148.0, 32.0))
                .child(skel(180.0, 14.0))
        };
        // The React heights are measured at 1440 (260.1, 165.3, 113.8, 84)
        // and on a phone (321, 230.5, 157.3, 105.8). A GPUI box at the
        // same height keeps the frame still when the state arrives.
        let (h_main, h_a, h_b, h_c) = if phone {
            (321.0, 230.5, 157.3, 105.8)
        } else {
            (260.1, 165.3, 113.8, 84.0)
        };
        let left = div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_w(px(0.0))
            .child(panel_box(h_main));
        let right = div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_w(px(0.0))
            .child(panel_box(h_a))
            .child(panel_box(h_b))
            .child(panel_box(h_c));
        if phone || self.width <= 960.0 {
            left.child(right)
        } else {
            div().flex().gap_4().child(left).child(right)
        }
    }

    /// The silent tray error: the error panel with Try again (no Details).
    fn silent_error(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let retry = button(
            &self.theme,
            "updates-try-again",
            "Try again",
            Variant::Primary,
            false,
            move |_, _, cx| {
                entity.update(cx, |this, cx| this.retry(cx));
            },
        );
        let body = div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(self.theme.down))
                    .child(SILENT_SUMMARY.to_string()),
            )
            .child(div().mt_2().child(retry));
        let t = &self.theme;
        div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .p_4()
                    .r_lg()
                    .border_1()
                    .border_color(Theme::color(t.line))
                    .bg(Theme::color(t.panel))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .mb_3()
                            .child(tracked_title(
                                &SILENT_TITLE.to_uppercase(),
                                13.0,
                                0.08,
                                gpui::FontWeight::MEDIUM,
                                crate::fonts::MONO_FAMILY,
                                Theme::color(t.mute),
                            )),
                    )
                    .child(body),
            )
    }
}

impl Render for UpdatesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.width = f32::from(_window.window_bounds().get_bounds().size.width);
        let phone = self.width <= 720.0;
        let narrow = self.width <= 960.0;
        // The page title lives in the shell strip (React `PageTitle`); the
        // shell draws "Updates" and the version context. This view only
        // draws the frame: the update card and the notes.
        if self.update.is_none() {
            if self.silent {
                // The tray stayed quiet for 5 s: the error panel with Try
                // again. The shape stays behind it, as the frame's own.
                return div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .child(self.silent_error(cx));
            }
            // The placeholder is invisible for the first 150 ms, then shows.
            return div()
                .flex()
                .flex_col()
                .gap_4()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .child(if self.loading_shown {
                    self.loading_shape(phone).into_any_element()
                } else {
                    div().into_any_element()
                });
        }
        let u = self.update.clone().unwrap();
        let avail = u.available.clone();
        let notes_of = avail.clone().or_else(|| u.updated_to.clone()).or_else(|| self.seen_notes.clone());
        let blocks = match &notes_of {
            Some(info) => parse_notes(&info.notes),
            None => Vec::new(),
        };
        let card: gpui::AnyElement = match &avail {
            Some(_) => self.available_panel(&u, cx).into_any_element(),
            None => self.up_to_date_panel(&u, cx).into_any_element(),
        };
        let mut notes_col = div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_w(px(0.0));
        match &notes_of {
            Some(info) if !blocks.is_empty() => {
                for block in &blocks {
                    notes_col = notes_col.child(self.note_panel(block));
                }
            }
            Some(info) => {
                notes_col = notes_col.child(
                    self.panel(
                        "updates-whats-new",
                        "What's new",
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(self.theme.mute))
                            .child(format!("No notes were published for {}.", info.version)),
                    ),
                );
            }
            None => {}
        }
        let left = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .flex_1()
            .min_w(px(0.0))
            .child(card);
        let grid = if narrow {
            // One column: the grid itself is the pane (the card stacks
            // above the notes). On a phone the document scrolls instead.
            let mut grid = div()
                .id(gpui::SharedString::from("updates-grid-scroll"))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .gap_4()
                .child(left)
                .child(notes_col);
            if !phone {
                grid = grid.overflow_y_scroll();
            }
            grid.into_any_element()
        } else {
            // Two columns: the grid fills the height and only the notes
            // column scrolls (with the s1 bleed).
            div()
                .flex()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .gap_4()
                .child(left)
                .child(
                    div()
                        .id(gpui::SharedString::from("updates-notes-scroll"))
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .mx(px(-4.0))
                        .px(px(4.0))
                        .overflow_y_scroll()
                        .child(notes_col),
                )
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .child(grid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::tauri::UpdateInfo;

    fn info(version: &str, notes: &str) -> UpdateInfo {
        UpdateInfo {
            version: version.into(),
            notes: notes.into(),
            pub_date: None,
        }
    }

    fn state(phase: UpdatePhase, available: Option<UpdateInfo>) -> UpdateState {
        UpdateState {
            current_version: "0.1.0".into(),
            auto_check: true,
            last_check: Some("2026-10-29T09:14:00Z".into()),
            phase,
            available,
            skipped_version: None,
            snoozed: false,
            updated_to: None,
        }
    }

    #[test]
    fn key_steps_match_the_react_order() {
        assert_eq!(key_step(&UpdatePhase::Available), 0);
        assert_eq!(key_step(&UpdatePhase::Idle), 0);
        assert_eq!(key_step(&UpdatePhase::VerifyFailed), 0);
        assert_eq!(key_step(&UpdatePhase::Declined), 0);
        assert_eq!(key_step(&UpdatePhase::Downloading { downloaded: 0, total: None }), 1);
        assert_eq!(key_step(&UpdatePhase::Verifying), 2);
        assert_eq!(key_step(&UpdatePhase::WaitingForUac), 3);
        assert_eq!(key_step(&UpdatePhase::Installing), 4);
        assert_eq!(key_step(&UpdatePhase::InstallFailed { message: "x".into() }), 5);
        assert_eq!(KEY_LABELS, ["Update now", "Downloading 100%", "Checking download", "Waiting for Windows", "Installing", "Try again"]);
        assert_eq!(key_label(&UpdatePhase::Available), "Update now");
        assert_eq!(key_label(&UpdatePhase::InstallFailed { message: "x".into() }), "Try again");
        assert_eq!(key_label(&UpdatePhase::Verifying), "Checking download");
        assert_eq!(key_label(&UpdatePhase::WaitingForUac), "Waiting for Windows");
        assert_eq!(key_label(&UpdatePhase::Installing), "Installing");
    }

    #[test]
    fn downloading_label_floors_and_aria_rounds() {
        // The fixture: 4.2 of 11.8 MB is 35.59% → label 35, aria 36.
        let dl = 4_200_000u64;
        let total = 11_800_000u64;
        assert_eq!(downloading_label(&UpdatePhase::Downloading { downloaded: dl, total: Some(total) }), "Downloading 35%");
        assert_eq!(aria_percent(&UpdatePhase::Downloading { downloaded: dl, total: Some(total) }), Some(36));
        // Unknown length: megabytes and no aria value.
        assert_eq!(downloading_label(&UpdatePhase::Downloading { downloaded: 4_400_000, total: None }), "Downloading 4.2 MB");
        assert_eq!(aria_percent(&UpdatePhase::Downloading { downloaded: 4_400_000, total: None }), None);
        // Not downloading: the step-1 fallback label.
        assert_eq!(downloading_label(&UpdatePhase::Available), "Downloading 100%");
    }

    #[test]
    fn slot_fill_covers_every_phase() {
        assert_eq!(slot_fill(&UpdatePhase::Available), Some(0.0));
        assert_eq!(slot_fill(&UpdatePhase::Idle), Some(0.0));
        assert_eq!(slot_fill(&UpdatePhase::Verifying), Some(1.0));
        assert_eq!(slot_fill(&UpdatePhase::WaitingForUac), Some(1.0));
        assert_eq!(slot_fill(&UpdatePhase::Installing), Some(1.0));
        assert_eq!(slot_fill(&UpdatePhase::Downloading { downloaded: 4_400_000, total: None }), None);
        let fill = slot_fill(&UpdatePhase::Downloading { downloaded: 4_200_000, total: Some(11_800_000) }).unwrap();
        assert!((fill - 0.3559).abs() < 0.01, "fill {fill}");
        // Full downloading caps at one.
        assert_eq!(slot_fill(&UpdatePhase::Downloading { downloaded: 12_000_000, total: Some(11_800_000) }), Some(1.0));
    }

    #[test]
    fn installing_phases_match_the_react_list() {
        assert!(is_installing(&UpdatePhase::Downloading { downloaded: 0, total: None }));
        assert!(is_installing(&UpdatePhase::Verifying));
        assert!(is_installing(&UpdatePhase::WaitingForUac));
        assert!(is_installing(&UpdatePhase::Installing));
        assert!(!is_installing(&UpdatePhase::Available));
        assert!(!is_installing(&UpdatePhase::CheckFailed { reason: CheckFailReason::Offline }));
        assert!(!is_installing(&UpdatePhase::Declined));
    }

    #[test]
    fn state_lines_match_the_settled_words() {
        assert_eq!(state_line(&state(UpdatePhase::Checking, None)), "Checking for updates…");
        assert_eq!(
            state_line(&state(
                UpdatePhase::Downloading { downloaded: 4_400_000, total: Some(11_800_000) },
                Some(info("0.1.1", ""))
            )),
            "Downloading 0.1.1: 4.2 of 11.3 MB."
        );
        assert_eq!(
            state_line(&state(
                UpdatePhase::Downloading { downloaded: 4_400_000, total: None },
                Some(info("0.1.1", ""))
            )),
            "Downloading 0.1.1: 4.2 MB."
        );
        assert_eq!(state_line(&state(UpdatePhase::Verifying, None)), "Checking the download against canvas-rgb's release key.");
        assert_eq!(state_line(&state(UpdatePhase::VerifyFailed, None)), "The download failed its check and was deleted. Nothing was installed.");
        assert_eq!(state_line(&state(UpdatePhase::WaitingForUac, Some(info("0.1.1", "")))), "Windows will ask for permission to install 0.1.1.");
        assert_eq!(state_line(&state(UpdatePhase::Declined, None)), "Permission declined. Nothing was changed.");
        assert_eq!(state_line(&state(UpdatePhase::Installing, Some(info("0.1.1", "")))), "Installing 0.1.1.");
        assert_eq!(
            state_line(&state(UpdatePhase::CheckFailed { reason: CheckFailReason::Offline }, None)),
            "Could not check for updates: no connection."
        );
        assert_eq!(
            state_line(&state(UpdatePhase::CheckFailed { reason: CheckFailReason::RateLimited }, None)),
            "Could not check for updates: GitHub rate limit."
        );
        assert_eq!(
            state_line(&state(UpdatePhase::CheckFailed { reason: CheckFailReason::Other }, None)),
            "Could not check for updates."
        );
        assert_eq!(
            state_line(&state(
                UpdatePhase::InstallFailed { message: "the installer exited with code 5".into() },
                Some(info("0.1.1", ""))
            )),
            "The installer stopped before it finished. canvas-rgb is still on 0.1.0."
        );
        assert_eq!(
            state_line(&state(
                UpdatePhase::InstallFailed { message: "Setup could not replace canvas-rgb.exe because it is in use".into() },
                Some(info("0.1.1", ""))
            )),
            "Setup could not replace canvas-rgb.exe because it is in use. canvas-rgb is still on 0.1.0."
        );
        assert_eq!(
            state_line(&state(UpdatePhase::Available, Some(info("0.1.1", "")))),
            "0.1.1 available."
        );
        assert_eq!(state_line(&state(UpdatePhase::UpToDate, None)), "Up to date.");
        assert_eq!(state_line(&state(UpdatePhase::Idle, None)), "Up to date.");
        let mut updated = state(UpdatePhase::UpToDate, None);
        updated.updated_to = Some(info("0.1.1", ""));
        assert_eq!(state_line(&updated), "Updated to 0.1.1.");
    }

    #[test]
    fn state_tones_match_the_dot_rule() {
        assert_eq!(state_tone(&state(UpdatePhase::CheckFailed { reason: CheckFailReason::Offline }, None)), "down");
        assert_eq!(state_tone(&state(UpdatePhase::VerifyFailed, None)), "down");
        assert_eq!(state_tone(&state(UpdatePhase::InstallFailed { message: "x".into() }, None)), "down");
        assert_eq!(state_tone(&state(UpdatePhase::Declined, None)), "warn");
        assert_eq!(state_tone(&state(UpdatePhase::Checking, None)), "work");
        assert_eq!(state_tone(&state(UpdatePhase::Downloading { downloaded: 0, total: None }, None)), "work");
        assert_eq!(state_tone(&state(UpdatePhase::Verifying, None)), "work");
        assert_eq!(state_tone(&state(UpdatePhase::WaitingForUac, None)), "work");
        assert_eq!(state_tone(&state(UpdatePhase::Installing, None)), "work");
        assert_eq!(state_tone(&state(UpdatePhase::Available, Some(info("0.1.1", "")))), "work");
        assert_eq!(state_tone(&state(UpdatePhase::UpToDate, None)), "ok");
        assert_eq!(state_tone(&state(UpdatePhase::Idle, None)), "ok");
    }

    #[test]
    fn installer_sentence_replaces_raw_words() {
        assert_eq!(installer_sentence("the installer exited with code 5"), "The installer stopped before it finished.");
        assert_eq!(installer_sentence(""), "The installer stopped before it finished.");
        assert_eq!(installer_sentence("error"), "The installer stopped before it finished.");
        assert_eq!(installer_sentence("0x80070005 failed"), "The installer stopped before it finished.");
        assert_eq!(installer_sentence("42"), "The installer stopped before it finished.");
        assert_eq!(
            installer_sentence("Setup could not replace canvas-rgb.exe because it is in use"),
            "Setup could not replace canvas-rgb.exe because it is in use."
        );
        assert_eq!(installer_sentence("no trailing period here"), "No trailing period here.");
        // Failed for the current version: only the first sentence, no "still on".
        assert_eq!(
            install_failed_sentence("the installer exited with code 5", "0.1.0", Some("0.1.0")),
            "The installer stopped before it finished."
        );
        assert!(same_version("v0.3.0", "0.3.0"));
        assert!(!same_version("0.3.0", "0.3.1"));
    }

    #[test]
    fn uac_and_skipped_lines_match_the_words() {
        assert_eq!(
            uac_line("0.1.1"),
            "Windows will ask for permission to install 0.1.1. While it installs, canvas-rgb stops for a moment: the lights pause and the fans run on your motherboard's settings. Choosing No leaves everything as it is."
        );
        let mut skipped = state(UpdatePhase::Available, Some(info("0.1.1", "")));
        skipped.skipped_version = Some("0.1.1".into());
        assert!(is_skipped(&skipped));
        let fresh = state(UpdatePhase::Available, Some(info("0.1.1", "")));
        assert!(!is_skipped(&fresh));
        assert_eq!(LONGEST_STATE, "The download failed its check and was deleted. Nothing was installed.");
        assert!(state_line(&state(UpdatePhase::VerifyFailed, None)).len() <= LONGEST_STATE.len());
        assert_eq!(state_line(&state(UpdatePhase::VerifyFailed, None)), LONGEST_STATE);
    }

    #[test]
    fn check_now_and_gating_match_the_react_rules() {
        let up = state(UpdatePhase::UpToDate, None);
        assert!(!check_now_busy(&up));
        assert!(check_now_enabled(&up, true));
        assert!(!check_now_enabled(&up, false));
        let checking = state(UpdatePhase::Checking, None);
        assert!(check_now_busy(&checking));
        assert!(!check_now_enabled(&checking, true));
        let installing = state(
            UpdatePhase::Downloading { downloaded: 0, total: None },
            Some(info("0.1.1", "")),
        );
        assert!(!check_now_enabled(&installing, true));
        // Check now never appears with an available release: the view only
        // draws it in the up-to-date panel.
        assert!(state(UpdatePhase::Available, Some(info("0.1.1", ""))).available.is_some());
        // The silent error words.
        assert_eq!(SILENT_TITLE, "No answer from the tray");
        assert_eq!(SILENT_SUMMARY, "The tray app did not answer. Start it, then try again.");
        assert_eq!(LOADING_LABEL, "Reading the update state");
        assert_eq!(TRAY_TIMEOUT_MS, 5000);
        assert_eq!(LOAD_DELAY_MS, 150);
    }

    #[test]
    fn notes_parse_matches_the_demo_notes() {
        let notes = "### Added\n- Updates: canvas-rgb looks for a newer release at start and once a day, and installs it when you press **Update now**.\n- A What's new page that shows the notes of the release that is waiting.\n- `canvas-rgb.exe --after-update` starts the tray quietly after an install.\n\n### Changed\n- The tray menu gains an Update to 0.1.1 row while a release is waiting.\n- Settings gains an Updates panel with Check now.\n\n### Fixed\n- The editor no longer opens a window after the installer relaunches the tray.";
        let blocks = parse_notes(notes);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].title, "Added");
        assert_eq!(blocks[0].items.len(), 3);
        assert_eq!(blocks[1].title, "Changed");
        assert_eq!(blocks[1].items.len(), 2);
        assert_eq!(blocks[2].title, "Fixed");
        assert_eq!(blocks[2].items.len(), 1);
        // `#` and `##` name the release itself and are skipped.
        assert_eq!(parse_notes("## [0.1.1] - 2026-10-29\n### Added\n- one").len(), 1);
        assert_eq!(parse_notes("## [0.1.1] - 2026-10-29\n### Added\n- one")[0].title, "Added");
        // A wrapped indented line continues the previous item.
        let wrapped = parse_notes("### Added\n- first\n  continues here\n- second");
        assert_eq!(wrapped[0].items, vec!["first continues here", "second"]);
        // Blank notes parse to no items (the fallback panel).
        assert!(parse_notes("").is_empty());
        assert!(parse_notes("# [0.1.1]\n## notes").is_empty());
        // A heading alone with no items is dropped.
        assert!(parse_notes("### Empty").is_empty());
        // Trailing hashes are stripped.
        let hashed = parse_notes("### Added ##\n- one");
        assert_eq!(hashed[0].title, "Added");
    }

    #[test]
    fn inline_parse_keeps_words_and_drops_addresses() {
        assert_eq!(
            parse_inline("plain text"),
            vec![Inline::Text("plain text".into())]
        );
        assert_eq!(
            parse_inline("press **Update now** today"),
            vec![
                Inline::Text("press ".into()),
                Inline::Strong("Update now".into()),
                Inline::Text(" today".into())
            ]
        );
        assert_eq!(
            parse_inline("run `canvas-rgb.exe --up` now"),
            vec![
                Inline::Text("run ".into()),
                Inline::Code("canvas-rgb.exe --up".into()),
                Inline::Text(" now".into())
            ]
        );
        assert_eq!(
            parse_inline("see [the docs](https://example.com/a) here"),
            vec![
                Inline::Text("see ".into()),
                Inline::Text("the docs".into()),
                Inline::Text(" here".into())
            ]
        );
        // An unmatched marker stays as text.
        assert_eq!(parse_inline("a * b"), vec![Inline::Text("a * b".into())]);
        assert_eq!(parse_inline("`unclosed"), vec![Inline::Text("`".into()), Inline::Text("unclosed".into())]);
    }

    #[test]
    fn check_fail_reasons_serialize_as_the_react_strings() {
        use canvas_proto::tauri::CheckFailReason as R;
        for (reason, word) in [(R::Offline, "offline"), (R::RateLimited, "rate_limited"), (R::Other, "other")] {
            let json = serde_json::to_string(&reason).unwrap();
            assert_eq!(json, format!("\"{word}\""));
            assert_eq!(serde_json::from_str::<R>(&json).unwrap(), reason);
        }
        // The state line reads them back.
        assert_eq!(
            state_line(&state(UpdatePhase::CheckFailed { reason: R::Offline }, None)),
            "Could not check for updates: no connection."
        );
    }

    #[test]
    fn note_tones_match_the_heading_rule() {
        assert_eq!(note_tone("Added"), "ok");
        assert_eq!(note_tone("added more"), "ok");
        assert_eq!(note_tone("Fixed"), "warn");
        assert_eq!(note_tone("Removed"), "down");
        assert_eq!(note_tone("Deprecated"), "down");
        assert_eq!(note_tone("Security"), "down");
        assert_eq!(note_tone("Changed"), "work");
        assert_eq!(note_tone("Notes"), "work");
    }
}

