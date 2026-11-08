//! `SettingsView`, after `app/web/src/views/SettingsView.tsx` (+
//! `UpdatesPanel.tsx`, `AudioSourcePanel.tsx`).
//!
//! The port keeps the React view's rules (the checklist is
//! `docs/gpui-parity/settings.md`):
//! - Rendering: frame-rate and brightness slider + number pairs, the fps
//!   hint from `status.fps_measured`, PATCH commits and the render error
//!   line;
//! - Appearance: theme and motion segmented controls, kept as GPUI
//!   editor prefs (its own file; never the tray's);
//! - Lock and quit: black-on-lock toggle, on-quit segmented, boot colour
//!   field + Clear, whole-object PUT with optimistic show and GET rollback;
//! - Audio source: the grouped picker, the live test (lease + 100 ms
//!   poll), the LED meter, dB figure, bands, and the compact popover form;
//! - Setup: map again, export/import through the native file dialogs, the
//!   note and the shared error line;
//! - Version: the daemon version, and the tray-only update controls in
//!   React's tray state when a tray is attached (stdin is a pipe) and in
//!   React's no-tray state otherwise (disabled; "Tray app only").
//!
//! The React demo-mode states (`?state=...`, `?update=...`) are
//! demo-backend behavior and have no GPUI counterpart: the GPUI editor
//! always talks to a real (here, fake) daemon over HTTP. Keyboard paths
//! the GPUI components cannot serve (slider arrows, number typing,
//! popover Escape, listbox arrows) stay unticked in the checklist, as
//! they do for Displays.

use std::sync::{Arc, Mutex};

use canvas_proto::api::{OutputState, RenderState, Status};
use canvas_proto::audio::{
    AudioFlow, AudioSource, AudioSources, AudioTestState, AudioTestStatus,
};
use canvas_proto::color::Rgb8;
use canvas_proto::config::Settings;
use canvas_proto::tauri::{
    CheckFailReason, EditorChoice, TrayPrefs, TrayToEditor, UpdateChannel, UpdatePhase,
    UpdateState,
};
use gpui::{Context, IntoElement, Window, div, prelude::*, px};

use crate::client::DaemonClient;
use crate::components::button::{Variant, button, button_icon, button_sm};
use crate::components::panel::{field_hint, field_stack_hint, state_dot, tracked_title};
use crate::components::segmented::segmented;
use crate::components::select::select_grouped;
use crate::components::slider::{Origin, slider_range};
use crate::components::toggle::{toggle, toggle_disabled};
use crate::theme::{Theme, TokenRadius};

// ---------------------------------------------------------------------------
// Logic, ported from the React view and covered by the tests below.

/// `audioStatusWords` (`AudioSourcePanel.tsx:28-59`).
pub fn audio_status_words(state: Option<&AudioTestState>) -> String {
    let Some(state) = state else {
        return "Live while this panel is open".to_string();
    };
    let name = state.source.name.as_deref().unwrap_or("the default output");
    match state.status {
        AudioTestStatus::Off => "Live while this panel is open".to_string(),
        AudioTestStatus::Waiting => "Waiting for the device…".to_string(),
        AudioTestStatus::NotListening => "Press Listen to test this input".to_string(),
        AudioTestStatus::NoDevice => "No output device".to_string(),
        AudioTestStatus::Missing => format!("Saved device not connected: using {name}"),
        AudioTestStatus::HeldExclusive => "Held by another app in exclusive mode".to_string(),
        AudioTestStatus::NoPackets => "No audio packets from the device".to_string(),
        AudioTestStatus::Silent => "Silent: play something".to_string(),
        AudioTestStatus::Live => "Live".to_string(),
        AudioTestStatus::DefaultChanged => format!("Windows default changed: now {name}"),
        AudioTestStatus::Reopening => format!("Reopening {name}"),
        AudioTestStatus::UnsupportedFormat => {
            format!("Format not supported (tag {})", state.format_tag.unwrap_or(0))
        }
    }
}

/// Whether the current pick resolves to an input (`isInputSelection`).
pub fn is_input_selection(sources: Option<&AudioSources>) -> bool {
    let Some(sources) = sources else { return false };
    match &sources.current.requested {
        AudioSource::Endpoint { flow, .. } => *flow == AudioFlow::Capture,
        AudioSource::FollowDefault => sources.current.flow == Some(AudioFlow::Capture),
    }
}

/// The lit LED cells and their tones (the meter's rule: 24 cells, ok to
/// 14, work to 20, warn above).
pub fn led_cells(peak: Option<f32>) -> Vec<Option<&'static str>> {
    let lit = match peak {
        Some(p) if p.is_finite() => (p.clamp(0.0, 1.0) * 24.0).round() as usize,
        _ => 0,
    };
    (0..24)
        .map(|i| {
            if i < lit {
                Some(if i < 14 {
                    "ok"
                } else if i < 20 {
                    "work"
                } else {
                    "warn"
                })
            } else {
                None
            }
        })
        .collect()
}

/// `isExperimentalVersion` (`updateWords.ts:14-19`): a `-` before any `+`.
#[allow(dead_code)]
pub fn is_experimental_version(version: &str) -> bool {
    let Some(dash) = version.find('-') else { return false };
    match version.find('+') {
        Some(plus) => dash < plus,
        None => true,
    }
}

/// `isSkipped` (`updateWords.ts:189-191`).
#[allow(dead_code)]
pub fn is_skipped(available: Option<&str>, skipped_version: Option<&str>) -> bool {
    match (available, skipped_version) {
        (Some(a), Some(s)) => a == s,
        _ => false,
    }
}

/// `isInstalling` (`updateWords.ts:41-44`).
#[allow(dead_code)]
pub fn is_installing_phase(kind: &str) -> bool {
    matches!(kind, "downloading" | "verifying" | "waiting_for_uac" | "installing")
}

/// The Settings status words (`updateWords.ts:194-228`): (tone, text).
/// `phase` is the UpdatePhase `kind` string; `reason` the check failure's.
#[allow(dead_code)]
pub fn settings_status(
    phase: &str,
    reason: Option<&str>,
    available: Option<&str>,
    skipped_version: Option<&str>,
) -> (String, String) {
    match phase {
        "checking" => ("work".into(), "Checking for updates…".into()),
        "downloading" => ("work".into(), format!("Downloading {}", available.unwrap_or(""))),
        "verifying" => ("work".into(), "Checking the download".into()),
        "waiting_for_uac" => ("work".into(), "Waiting for Windows".into()),
        "installing" => ("work".into(), format!("Installing {}", available.unwrap_or(""))),
        "declined" => ("warn".into(), "Permission declined".into()),
        "verify_failed" => ("down".into(), "The download failed its check".into()),
        "install_failed" => ("down".into(), "The install failed".into()),
        "check_failed" => (
            "down".into(),
            match reason {
                Some("offline") => "Check failed: offline".into(),
                Some("rate_limited") => "Check failed: GitHub rate limit".into(),
                _ => "Check failed".into(),
            },
        ),
        _ => {
            if let Some(v) = available {
                if is_skipped(Some(v), skipped_version) {
                    ("warn".into(), format!("{v} skipped"))
                } else {
                    ("work".into(), format!("{v} available"))
                }
            } else {
                ("ok".into(), "Up to date".into())
            }
        }
    }
}

/// `fmt1` (`format.ts:8-11`).
pub fn fmt1(n: Option<f32>) -> String {
    match n {
        Some(v) if v.is_finite() => format!("{v:.1}"),
        _ => "—".to_string(),
    }
}

/// The export file name (`config-transfer.ts`: stamp is the UTC date).
pub fn export_file_name(today: (u32, u32, u32)) -> String {
    format!("canvas-rgb-config-{:04}-{:02}-{:02}.json", today.0, today.1, today.2)
}

/// The UTC civil date for a Unix day count (Howard Hinnant's algorithm).
pub fn civil_from_days(z: i64) -> (u32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { (y + 1) as u32 } else { y as u32 }, m, d)
}

/// Today's UTC date, as the export stamp uses it.
pub fn today_utc() -> (u32, u32, u32) {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    civil_from_days(days)
}

/// `fmtChecked` (`updateWords.ts:27-34`): "Never", "HH:MM" today, or
/// "Mon D, HH:MM" another day. `today_day` is today's UTC day count (UTC
/// civil; the browser formats in local time — GPUI has no local offset).
#[allow(dead_code)]
pub fn fmt_checked(iso: Option<&str>, today_day: i64) -> String {
    let Some(iso) = iso else { return "Never".to_string() };
    let Some((day, hh, mm)) = parse_rfc3339_day(iso) else {
        return "Never".to_string();
    };
    let time = format!("{hh:02}:{mm:02}");
    if day == today_day {
        time
    } else {
        let (_, m, d) = civil_from_days(day);
        let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
            [m.clamp(1, 12) as usize - 1];
        format!("{month} {d}, {time}")
    }
}

/// The UTC day count and clock from an RFC 3339 string
/// (`2026-10-29T09:14:00Z`); None when it does not parse.
#[allow(dead_code)]
pub fn parse_rfc3339_day(iso: &str) -> Option<(i64, u32, u32)> {
    let date = iso.get(..10)?;
    let mut it = date.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    // Days from civil (Hinnant, inverse of civil_from_days).
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + (d as u64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let day = era * 146_097 + doe as i64 - 719_468;
    let time = iso.get(11..16)?;
    let (hh, mm) = time.split_once(':')?;
    Some((day, hh.parse().ok()?, mm.parse().ok()?))
}

/// The six panel titles, in DOM order (`SettingsView.tsx:159-313`).
#[allow(dead_code)]
pub const PANEL_TITLES: [&str; 6] = [
    "Rendering",
    "Appearance",
    "Lock and quit",
    "Audio source",
    "Setup",
    "Version",
];

/// The CSS multicol column count for a content width: `columns: 340px
/// auto; column-gap: 16px` (`app.css:4803`).
pub fn settings_columns(content_w: f32) -> usize {
    ((content_w + 16.0) / 356.0).floor().max(1.0) as usize
}

/// Greedy multicol fill with a height budget (the balancing step of CSS
/// `column-fill: balance`): the smallest budget that fits every panel
/// into at most `cols` columns. Returns, per column, panel indices.
pub fn distribute(heights: &[f32], cols: usize) -> Vec<Vec<usize>> {
    if cols <= 1 {
        return vec![(0..heights.len()).collect()];
    }
    let total: f32 = heights.iter().sum();
    // Binary-search the budget (0.5 px precision is past subpixel).
    let (mut lo, mut hi) = (0.0f32, total);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if fits(heights, cols, mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let mut columns: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_h = 0.0f32;
    for (i, h) in heights.iter().enumerate() {
        if !cur.is_empty() && cur_h + h > hi {
            columns.push(std::mem::take(&mut cur));
            cur_h = 0.0;
        }
        cur.push(i);
        cur_h += h;
    }
    if !cur.is_empty() {
        columns.push(cur);
    }
    while columns.len() < cols {
        columns.push(Vec::new());
    }
    columns
}

fn fits(heights: &[f32], cols: usize, budget: f32) -> bool {
    let mut used = 1usize;
    let mut cur = 0.0f32;
    for h in heights {
        if cur > 0.0 && cur + h > budget {
            used += 1;
            if used > cols {
                return false;
            }
            cur = 0.0;
        }
        cur += h;
    }
    used <= cols
}

/// The HSV conversions `ColorPicker` works in (`lib/color.ts`).
pub fn hex_to_hsv(hex: &str, keep_hue: f32) -> (f32, f32, f32) {
    let (r, g, b) = hex_to_rgb(hex);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let mut h = keep_hue;
    if d > 0.0 {
        if max == r {
            h = (g - b) / d;
        } else if max == g {
            h = (b - r) / d + 2.0;
        } else {
            h = (r - g) / d + 4.0;
        }
        h = (h * 60.0).rem_euclid(360.0);
    }
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

pub fn hsv_to_hex(h: f32, s: f32, v: f32) -> String {
    let hue = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let seg = ((hue / 60.0).floor() as usize) % 6;
    let (r, g, b) = match seg {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |n: f32| ((n + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

pub fn hex_to_rgb(hex: &str) -> (f32, f32, f32) {
    let raw = hex.strip_prefix('#').unwrap_or(hex);
    if raw.len() != 6 {
        return (0.0, 0.0, 0.0);
    }
    let p = |i: usize| u8::from_str_radix(&raw[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
    (p(0), p(2), p(4))
}

/// Whether `text` is a JSON document (the import refusal).
pub fn is_json(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text).is_ok()
}

/// The tray-only controls' shared state in a standalone GPUI run: no
/// tray, so `canUpdate()` is false and the update state is missing.
pub struct TrayAbsence;
#[allow(dead_code)]
impl TrayAbsence {
    pub const CHECKS_AUTOMATICALLY: bool = true;
    pub const LAST_CHECKED: &str = "—";
    pub const STATUS: &str = "—";
}

/// `canUpdate()` (`host.ts:95-97`): the tray is attached (stdin is a pipe).
pub fn tray_acts(tray_present: bool) -> bool {
    tray_present
}

/// Whether the Editor row is there: only an experimental version earns it
/// (`UpdatesPanel.tsx:63`).
#[allow(dead_code)]
pub fn editor_row_visible(u: Option<&UpdateState>) -> bool {
    u.is_some_and(|u| is_experimental_version(&u.current_version))
}

/// Check automatically's checked state (`UpdatesPanel.tsx:79`).
#[allow(dead_code)]
pub fn auto_checked(u: Option<&UpdateState>) -> bool {
    u.map(|u| u.auto_check).unwrap_or(true)
}

/// Experimental builds' checked state (`UpdatesPanel.tsx:98`).
#[allow(dead_code)]
pub fn experimental_checked(prefs: Option<&TrayPrefs>) -> bool {
    prefs.is_some_and(|p| p.channel == UpdateChannel::Experimental)
}

/// The Editor radio's value (`UpdatesPanel.tsx:108`).
#[allow(dead_code)]
pub fn editor_value(prefs: Option<&TrayPrefs>) -> EditorChoice {
    prefs.map(|p| p.editor).unwrap_or(EditorChoice::React)
}

/// Whether the tray is checking now (`UpdatesPanel.tsx:72`).
#[allow(dead_code)]
pub fn is_checking(u: Option<&UpdateState>) -> bool {
    u.is_some_and(|u| matches!(u.phase, UpdatePhase::Checking))
}

/// Whether the tray says an install is running (`updateWords.ts:41-44`).
#[allow(dead_code)]
pub fn is_installing(u: Option<&UpdateState>) -> bool {
    u.is_some_and(|u| {
        matches!(
            u.phase,
            UpdatePhase::Downloading { .. }
                | UpdatePhase::Verifying
                | UpdatePhase::WaitingForUac
                | UpdatePhase::Installing
        )
    })
}

/// Check now's disabled state (`UpdatesPanel.tsx:89`).
#[allow(dead_code)]
pub fn check_now_disabled(acts: bool, u: Option<&UpdateState>) -> bool {
    !acts || u.is_none() || is_checking(u) || is_installing(u)
}

/// Check automatically's disabled state (`UpdatesPanel.tsx:80`).
#[allow(dead_code)]
pub fn auto_disabled(acts: bool, u: Option<&UpdateState>) -> bool {
    !acts || u.is_none()
}

/// Last checked's words (`UpdatesPanel.tsx:85`).
#[allow(dead_code)]
pub fn last_checked_word(acts: bool, u: Option<&UpdateState>, today_day: i64) -> String {
    if acts {
        if let Some(u) = u {
            return fmt_checked(u.last_check.as_deref(), today_day);
        }
    }
    "—".to_string()
}

/// The `kind` string and check-failure reason the Rails status words take.
/// Maps the spine's `UpdatePhase` onto `settings_status`'s strings.
#[allow(dead_code)]
pub fn phase_kind_and_reason(phase: &UpdatePhase) -> (&'static str, Option<&'static str>) {
    match phase {
        UpdatePhase::Idle => ("idle", None),
        UpdatePhase::Checking => ("checking", None),
        UpdatePhase::UpToDate => ("up_to_date", None),
        UpdatePhase::Available => ("available", None),
        UpdatePhase::Downloading { .. } => ("downloading", None),
        UpdatePhase::Verifying => ("verifying", None),
        UpdatePhase::VerifyFailed => ("verify_failed", None),
        UpdatePhase::WaitingForUac => ("waiting_for_uac", None),
        UpdatePhase::Declined => ("declined", None),
        UpdatePhase::Installing => ("installing", None),
        UpdatePhase::CheckFailed { reason } => (
            "check_failed",
            match reason {
                CheckFailReason::Offline => Some("offline"),
                CheckFailReason::RateLimited => Some("rate_limited"),
                CheckFailReason::Other => None,
            },
        ),
        UpdatePhase::InstallFailed { .. } => ("install_failed", None),
    }
}

/// The status row's (tone, words) when the state is there
/// (`UpdatesPanel.tsx:71`).
#[allow(dead_code)]
pub fn tray_status(u: Option<&UpdateState>) -> Option<(String, String)> {
    let u = u?;
    let (kind, reason) = phase_kind_and_reason(&u.phase);
    let available = u.available.as_ref().map(|a| a.version.as_str());
    let skipped = u.skipped_version.as_deref();
    Some(settings_status(kind, reason, available, skipped))
}

/// Whether Undo is there (`UpdatesPanel.tsx:119`).
#[allow(dead_code)]
pub fn undo_visible(u: Option<&UpdateState>) -> bool {
    u.is_some_and(|u| {
        is_skipped(
            u.available.as_ref().map(|a| a.version.as_str()),
            u.skipped_version.as_deref(),
        ) && !matches!(u.phase, UpdatePhase::Checking)
    })
}

/// Whether What's new is there (`UpdatesPanel.tsx:124`).
#[allow(dead_code)]
pub fn whats_new_visible(u: Option<&UpdateState>) -> bool {
    u.is_some_and(|u| {
        u.available.is_some()
            && !is_skipped(
                u.available.as_ref().map(|a| a.version.as_str()),
                u.skipped_version.as_deref(),
            )
    })
}

/// Today's UTC day count (the same count `fmt_checked` compares against).
#[allow(dead_code)]
pub fn today_day_utc() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64
}

// ---------------------------------------------------------------------------
// The view.

/// The theme the shell and this view share (see `shell.rs`).
pub type ThemeCell = Arc<Mutex<Theme>>;

pub struct SettingsView {
    pub theme: Theme,
    pub theme_cell: ThemeCell,
    pub settings: Option<Settings>,
    /// A refresh failure after the settings read (the stale banner).
    pub settings_error: Option<String>,
    /// The first read failed (the error panel).
    pub load_error: Option<String>,
    pub retrying: bool,
    pub render: Option<RenderState>,
    pub status: Option<Status>,
    pub fps_draft: Option<f32>,
    pub brightness_draft: Option<f32>,
    pub render_err: Option<String>,
    pub err: Option<String>,
    pub note: Option<String>,
    pub theme_pref: String,
    pub motion_pref: String,
    pub boot_draft: Option<String>,
    pub boot_open: bool,
    pub audio_sources: Option<AudioSources>,
    pub audio_err: Option<String>,
    pub listen: bool,
    pub audio_test: Option<AudioTestState>,
    pub lease_id: Option<String>,
    pub audio_select_open: bool,
    pub popover_open: bool,
    pub visible: bool,
    pub audio_use_popover: bool,
    pub audio_polling: bool,
    pub settings_polling: bool,
    pub shell: Option<gpui::Entity<crate::shell::Shell>>,
    pub fps_origin: Origin,
    pub bri_origin: Origin,
    pub sv_origin: Origin,
    pub hue_origin: Origin,
    /// A tray is attached when stdin is a pipe (`canUpdate()`).
    pub tray_present: bool,
    /// The tray's update state, pushed over stdio after `ready` and on every change.
    pub tray_update: Option<UpdateState>,
    /// The tray's prefs, pushed over stdio after `ready` and after every change.
    pub tray_prefs: Option<TrayPrefs>,
    /// A failed tray command's words (Settings > Version error line).
    pub tray_err: Option<String>,
    /// Stdin lines in, at `TRAY_TO_EDITOR_LINE_MAX` a line.
    pub tray_rx: Option<std::sync::mpsc::Receiver<TrayToEditor>>,
    pub tray_polling: bool,
}

impl SettingsView {
    pub fn new(
        theme: Theme,
        theme_cell: ThemeCell,
        shell: Option<gpui::Entity<crate::shell::Shell>>,
        visible: bool,
        cx: &mut Context<Self>,
        tray_present: bool,
        tray_rx: Option<std::sync::mpsc::Receiver<TrayToEditor>>,
    ) -> Self {
        let (theme_pref, motion_pref) = read_prefs();
        // Without a stored pref the radio names the theme the shell
        // opened in (React's `readTheme` default is "system", but the
        // capture flag fixes dark/light, and the radio must agree).
        let theme_pref = if prefs_path().is_some_and(|p| p.exists()) {
            theme_pref
        } else if theme.is_dark() {
            "dark".to_string()
        } else {
            "light".to_string()
        };
        let mut view = SettingsView {
            theme,
            theme_cell,
            settings: None,
            settings_error: None,
            load_error: None,
            retrying: false,
            render: None,
            status: None,
            fps_draft: None,
            brightness_draft: None,
            render_err: None,
            err: None,
            note: None,
            theme_pref,
            motion_pref,
            boot_draft: None,
            boot_open: false,
            audio_sources: None,
            audio_err: None,
            listen: false,
            audio_test: None,
            lease_id: None,
            audio_select_open: false,
            popover_open: false,
            visible,
            audio_use_popover: false,
            audio_polling: false,
            settings_polling: false,
            shell,
            fps_origin: Origin::new(),
            bri_origin: Origin::new(),
            sv_origin: Origin::new(),
            hue_origin: Origin::new(),
            tray_present,
            tray_update: None,
            tray_prefs: None,
            tray_err: None,
            tray_rx,
            tray_polling: false,
        };
        // The constructor theme is the capture flag's answer; the stored
        // pref only names the radio choice. Apply it when it differs from
        // the flag (a real run, not a capture): the flag's theme wins.
        let stored = match view.theme_pref.as_str() {
            "dark" => Theme::dark(),
            "light" => Theme::light(),
            _ => view.theme,
        };
        if stored != view.theme {
            view.theme = stored;
            *view.theme_cell.lock().unwrap() = stored;
        }
        view.load_all(cx);
        view.load_audio_sources(cx);
        view.start_settings_poll(cx);
        view.start_tray_poll(cx);
        view
    }

    fn client() -> DaemonClient {
        DaemonClient::new()
    }

    /// `GET /api/v1/settings` + `/render` + `/status` (the reads the view
    /// makes; the shell polls the same answers for its own strip).
    fn load_all(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let settings = client.get("/api/v1/settings");
            let render = client.get("/api/v1/render");
            let status = client.get("/api/v1/status");
            (settings, render, status)
        });
        cx.spawn(async move |view, cx| {
            let (settings, render, status) = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.retrying = false;
                if let Ok(body) = render {
                    if let Ok(parsed) = serde_json::from_str::<RenderState>(&body) {
                        this.render = Some(parsed);
                    }
                }
                if let Ok(body) = status {
                    if let Ok(parsed) = serde_json::from_str::<Status>(&body) {
                        this.status = Some(parsed);
                    }
                }
                match settings {
                    Ok(body) => {
                        if let Ok(parsed) = serde_json::from_str::<Settings>(&body) {
                            this.settings = Some(parsed);
                            this.load_error = None;
                            this.settings_error = None;
                        }
                    }
                    Err(e) => {
                        if this.settings.is_none() {
                            this.load_error = Some(e.to_string());
                        } else {
                            this.settings_error = Some(e.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The 4 s settings poll (`store.ts` `refreshSettings` cadence).
    fn start_settings_poll(&mut self, cx: &mut Context<Self>) {
        if self.settings_polling {
            return;
        }
        self.settings_polling = true;
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(4))
                    .await;
                if view.update(cx, |_this, _cx| ()).is_err() {
                    break;
                }
                let result = cx.background_executor().spawn(async move {
                    DaemonClient::new().get("/api/v1/settings")
                }).await;
                let _ = view.update(cx, |this, cx| {
                    match result {
                        Ok(body) => {
                            if let Ok(parsed) = serde_json::from_str::<Settings>(&body) {
                                this.settings = Some(parsed);
                                this.settings_error = None;
                                this.load_error = None;
                            }
                        }
                        Err(e) => {
                            if this.settings.is_none() {
                                this.load_error = Some(e.to_string());
                            } else {
                                this.settings_error = Some(e.to_string());
                            }
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Drains the tray's stdio lines that are ready. Returns true when anything
    /// changed (a new update state, new prefs, or a navigate/close).
    fn drain_tray(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.tray_rx {
            loop {
                match rx.try_recv() {
                    Ok(TrayToEditor::UpdateState { state }) => {
                        self.tray_update = Some(*state);
                        changed = true;
                    }
                    Ok(TrayToEditor::Prefs { prefs }) => {
                        self.tray_prefs = Some(prefs);
                        changed = true;
                    }
                    Ok(TrayToEditor::Navigate { page }) => {
                        if crate::shell::is_valid_page(&page) {
                            if let Some(shell) = self.shell.clone() {
                                shell.update(cx, |s, cx| {
                                    s.navigate(&page);
                                    cx.notify();
                                });
                            }
                        }
                        changed = true;
                    }
                    Ok(TrayToEditor::Close) => {
                        std::process::exit(0);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }
        changed
    }

    /// The tray poll: drain stdin lines as they arrive (the tray pushes the
    /// update state and the prefs after `ready` and on every change).
    fn start_tray_poll(&mut self, cx: &mut Context<Self>) {
        if self.tray_polling || !self.tray_present {
            return;
        }
        self.tray_polling = true;
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
                let changed = view
                    .update(cx, |this, cx| this.drain_tray(cx))
                    .unwrap_or(false);
                if changed {
                    let _ = view.update(cx, |_this, cx| {
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// Whole-object PUT with the optimistic show and the GET rollback
    /// (`SettingsView.tsx:78-90`).
    fn save(&mut self, patch: Settings, cx: &mut Context<Self>) {
        self.err = None;
        self.note = None;
        self.settings = Some(patch.clone());
        let body = serde_json::to_string(&patch).unwrap_or_default();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let put = client.put_json("/api/v1/settings", &body);
            let fresh = client.get("/api/v1/settings");
            (put, fresh)
        });
        cx.spawn(async move |view, cx| {
            let (put, fresh) = bg.await;
            let _ = view.update(cx, |this, cx| {
                if let Ok(body) = fresh {
                    if let Ok(parsed) = serde_json::from_str::<Settings>(&body) {
                        this.settings = Some(parsed);
                    }
                }
                if let Err(e) = put {
                    this.err = Some(e.to_string());
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// `PATCH /api/v1/render` with `{fps, scale: null, sampling: null}`.
    fn commit_fps(&mut self, v: u32, cx: &mut Context<Self>) {
        self.render_err = None;
        let body = format!(r#"{{"fps":{v},"scale":null,"sampling":null}}"#);
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let patch = client.patch_json("/api/v1/render", &body);
            let fresh = client.get("/api/v1/render");
            (patch, fresh)
        });
        cx.spawn(async move |view, cx| {
            let (patch, fresh) = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.fps_draft = None;
                match patch {
                    Ok(body) => {
                        if let Ok(parsed) = serde_json::from_str::<RenderState>(&body) {
                            this.render = Some(parsed);
                        }
                        if let Ok(body) = fresh {
                            if let Ok(parsed) = serde_json::from_str::<RenderState>(&body) {
                                this.render = Some(parsed);
                            }
                        }
                    }
                    Err(e) => {
                        this.render_err = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `PATCH /api/v1/output` with `{power: null, brightness: v}`; the
    /// answer's brightness goes into the settings, then a settings GET.
    fn commit_brightness(&mut self, v: u8, cx: &mut Context<Self>) {
        self.render_err = None;
        let body = format!(r#"{{"power":null,"brightness":{v}}}"#);
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let patch = client.patch_json("/api/v1/output", &body);
            let fresh = client.get("/api/v1/settings");
            (patch, fresh)
        });
        cx.spawn(async move |view, cx| {
            let (patch, fresh) = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.brightness_draft = None;
                match patch {
                    Ok(body) => {
                        if let Ok(out) = serde_json::from_str::<OutputState>(&body) {
                            if let Some(s) = &mut this.settings {
                                s.brightness = out.brightness;
                            }
                        }
                        if let Ok(body) = fresh {
                            if let Ok(parsed) = serde_json::from_str::<Settings>(&body) {
                                this.settings = Some(parsed);
                            }
                        }
                    }
                    Err(e) => {
                        this.render_err = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_audio_sources(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            DaemonClient::new().get("/api/v1/audio/sources")
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(body) => {
                        if let Ok(parsed) = serde_json::from_str::<AudioSources>(&body) {
                            this.audio_sources = Some(parsed);
                            this.audio_err = None;
                        }
                    }
                    Err(e) => {
                        this.audio_err = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `PUT /api/v1/audio/source`, then the sources list again.
    fn pick_audio(&mut self, next: String, cx: &mut Context<Self>) {
        let Some(sources) = &self.audio_sources else { return };
        let body = if next == "follow-default" {
            r#"{"kind":"follow_default"}"#.to_string()
        } else {
            let ep = sources.endpoints.iter().find(|e| e.id == next);
            let (flow, name) = match ep {
                Some(e) => (
                    if e.flow == AudioFlow::Capture { "capture" } else { "render" },
                    e.name.clone(),
                ),
                None => ("render", next.clone()),
            };
            format!(
                r#"{{"kind":"endpoint","id":{},"flow":"{flow}","name_hint":{}}}"#,
                serde_json::to_string(&next).unwrap_or_default(),
                serde_json::to_string(&name).unwrap_or_default(),
            )
        };
        self.audio_err = None;
        self.audio_select_open = false;
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            let put = client.put_json("/api/v1/audio/source", &body);
            let fresh = client.get("/api/v1/audio/sources");
            (put, fresh)
        });
        cx.spawn(async move |view, cx| {
            let (put, fresh) = bg.await;
            let _ = view.update(cx, |this, cx| {
                match put {
                    Ok(body) => {
                        if let Ok(resolved) =
                            serde_json::from_str::<canvas_proto::audio::ResolvedAudioSource>(&body)
                        {
                            if let Some(s) = &mut this.audio_sources {
                                s.current = resolved;
                            }
                        }
                        if let Ok(body) = fresh {
                            if let Ok(parsed) = serde_json::from_str::<AudioSources>(&body) {
                                this.audio_sources = Some(parsed);
                            }
                        }
                    }
                    Err(e) => {
                        this.audio_err = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The live-test poll: a lease while the readings are on screen, a
    /// 100 ms GET poll, a 410 takes a new lease, failures are silent.
    /// `active` gates the lease (inline: visible; compact: popover open).
    fn start_audio_poll(&mut self, cx: &mut Context<Self>) {
        if self.audio_polling {
            return;
        }
        self.audio_polling = true;
        cx.spawn(async move |view, cx| {
            loop {
                let active = view
                    .update(cx, |this, _| {
                        this.visible && (!this.audio_use_popover || this.popover_open)
                    })
                    .unwrap_or(false);
                if !active {
                    // Release the lease and stop; the daemon would expire
                    // it anyway, React releases at once (S2).
                    let lease = view.update(cx, |this, _| this.lease_id.take()).ok().flatten();
                    if let Some(id) = lease {
                        let _ = cx.background_executor().spawn(async move {
                            let _ = DaemonClient::new()
                                .delete(&format!("/api/v1/audio/test?lease={id}"));
                        }).await;
                    }
                    let _ = view.update(cx, |this, cx| {
                        this.audio_polling = false;
                        cx.notify();
                    });
                    // Re-check shortly: the pill may come back (a popover
                    // reopen, or a navigate back to Settings).
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(200))
                        .await;
                    let still = view
                        .update(cx, |this, _| {
                            this.audio_polling = false;
                            this.visible && (!this.audio_use_popover || this.popover_open)
                        })
                        .unwrap_or(false);
                    if !still {
                        // Mark stopped so a later activation restarts.
                        let _ = view.update(cx, |this, _| this.audio_polling = false);
                        // Sleep before exiting so a quick reactivation
                        // doesn't miss the loop; if reactivated, this
                        // loop keeps serving it.
                        loop {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(500))
                                .await;
                            let on = view
                                .update(cx, |this, _| {
                                    this.visible
                                        && (!this.audio_use_popover || this.popover_open)
                                })
                                .unwrap_or(false);
                            if on {
                                break;
                            }
                            if view.update(cx, |_this, _| ()).is_err() {
                                return;
                            }
                        }
                    }
                }
                let (lease, listen) = view
                    .update(cx, |this, _| (this.lease_id.clone(), this.listen))
                    .unwrap_or((None, false));
                let result = cx.background_executor().spawn(async move {
                    let client = DaemonClient::new();
                    match lease {
                        None => {
                            let body = format!(r#"{{"listen":{listen}}}"#);
                            client.put_json("/api/v1/audio/test", &body)
                        }
                        Some(id) => client.get(&format!("/api/v1/audio/test?lease={id}")),
                    }
                }).await;
                let _ = view.update(cx, |this, cx| {
                    match result {
                        Ok(body) => {
                            if this.lease_id.is_none() {
                                if let Ok(lease) =
                                    serde_json::from_str::<canvas_proto::audio::AudioTestLease>(&body)
                                {
                                    this.lease_id = Some(lease.lease_id);
                                }
                            } else if let Ok(parsed) =
                                serde_json::from_str::<AudioTestState>(&body)
                            {
                                this.audio_test = Some(parsed);
                            }
                        }
                        Err(crate::client::ClientError::Daemon { status: 410, .. }) => {
                            this.lease_id = None;
                        }
                        Err(_) => {}
                    }
                    cx.notify();
                });
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
            }
        })
        .detach();
    }

    fn set_listen(&mut self, on: bool, cx: &mut Context<Self>) {
        self.listen = on;
        // S2: the old lease ends now, so the mic closes at once.
        if let Some(id) = self.lease_id.take() {
            let bg = cx.background_executor().spawn(async move {
                let _ = DaemonClient::new().delete(&format!("/api/v1/audio/test?lease={id}"));
            });
            cx.spawn(async move |_, _| {
                let _ = bg.await;
            })
            .detach();
        }
        cx.notify();
    }

    fn apply_theme(&mut self, pref: String, cx: &mut Context<Self>) {
        self.theme_pref = pref.clone();
        let theme = match pref.as_str() {
            "dark" => Theme::dark(),
            "light" => Theme::light(),
            _ => self.theme,
        };
        self.theme = theme;
        *self.theme_cell.lock().unwrap() = theme;
        write_prefs(&self.theme_pref, &self.motion_pref);
        cx.notify();
    }

    fn apply_motion(&mut self, pref: String, cx: &mut Context<Self>) {
        self.motion_pref = pref;
        write_prefs(&self.theme_pref, &self.motion_pref);
        cx.notify();
    }

    /// The command palette's "Switch the theme" (`Shell.tsx:172-181`):
    /// cycles dark, light, system, dark (`prefs.ts` `nextTheme`).
    pub fn palette_next_theme(&mut self, cx: &mut Context<Self>) {
        let next = match self.theme_pref.as_str() {
            "dark" => "light",
            "light" => "system",
            _ => "dark",
        };
        self.apply_theme(next.to_string(), cx);
    }

    /// The command palette's "Toggle interface motion"
    /// (`Shell.tsx:182-192`): flips full/reduced.
    pub fn palette_toggle_motion(&mut self, cx: &mut Context<Self>) {
        let next = if self.motion_pref == "reduced" { "full" } else { "reduced" };
        self.apply_motion(next.to_string(), cx);
    }

    /// Export: GET the bundle, ask where to save it, write the file.
    fn do_export(&mut self, cx: &mut Context<Self>) {
        self.err = None;
        self.note = None;
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::with_base_timeout(
                crate::client::base_url(),
                std::time::Duration::from_secs(60),
            );
            client.get("/api/v1/config/export")
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            match result {
                Ok(body) => {
                    let name = export_file_name(today_utc());
                    let dir = std::env::temp_dir();
                    let rx = match cx.update(|cx| cx.prompt_for_new_path(&dir, Some(&name))) {
                        Ok(rx) => rx,
                        Err(_) => return,
                    };
                    if let Ok(Ok(Some(path))) = rx.await {
                        let pretty = serde_json::from_str::<serde_json::Value>(&body)
                            .ok()
                            .and_then(|v| serde_json::to_string_pretty(&v).ok())
                            .unwrap_or(body);
                        match std::fs::write(&path, pretty) {
                            Ok(()) => view
                                .update(cx, |this, cx| {
                                    let file = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or(name);
                                    this.note = Some(format!("Downloaded {file}"));
                                    cx.notify();
                                })
                                .ok(),
                            Err(e) => view
                                .update(cx, |this, cx| {
                                    this.err = Some(e.to_string());
                                    cx.notify();
                                })
                                .ok(),
                        };
                    }
                }
                Err(e) => {
                    view.update(cx, |this, cx| {
                        this.err = Some(e.to_string());
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// Import: pick a file, refuse a non-JSON one, POST it, refresh all.
    fn do_import(&mut self, cx: &mut Context<Self>) {
        self.err = None;
        self.note = None;
        cx.spawn(async move |view, cx| {
            let rx = match cx.update(|cx| {
                cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Import canvas-rgb config".into()),
                })
            }) {
                Ok(rx) => rx,
                Err(_) => return,
            };
            let paths = match rx.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => paths,
                _ => return,
            };
            let path = paths[0].clone();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    view.update(cx, |this, cx| {
                        this.err = Some(e.to_string());
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            if !is_json(&text) {
                view.update(cx, |this, cx| {
                    this.err = Some(format!(
                        "{name} is not a canvas-rgb config file (it is not valid JSON)."
                    ));
                    cx.notify();
                })
                .ok();
                return;
            }
            let result = cx.background_executor().spawn(async move {
                let client = DaemonClient::with_base_timeout(
                    crate::client::base_url(),
                    std::time::Duration::from_secs(60),
                );
                client.post_json("/api/v1/config/import", &text)
            }).await;
            match result {
                Ok(_) => {
                    view.update(cx, |this, cx| {
                        this.note = Some(format!("Imported {name}"));
                        this.load_all(cx);
                        cx.notify();
                    })
                    .ok();
                }
                Err(e) => {
                    view.update(cx, |this, cx| {
                        this.err = Some(e.to_string());
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// Map my PC again: navigate to the Layout page and open the
    /// onboarding with `again: true` (React `/?map=1` + remap flag).
    fn map_pc(&mut self, cx: &mut Context<Self>) {
        if let Some(shell) = &self.shell {
            let shell = shell.clone();
            shell.update(cx, |s, cx| {
                s.navigate("layout");
                s.layout_view.update(cx, |v, cx| {
                    v.remap_all();
                    cx.notify();
                });
                cx.notify();
            });
        }
        cx.notify();
    }

    // -- rendering pieces -------------------------------------------------

    fn panel(
        &self,
        id: &str,
        title: &str,
        pad: f32,
        actions: impl IntoElement,
        body: impl IntoElement,
    ) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(gpui::SharedString::from(id.to_string()))
            .flex()
            .flex_col()
            .p(px(pad))
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
                    ))
                    .child(actions),
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
        if mt { el.mt_2() } else { el }
    }

    /// The number well (`.num` + `.unit`): 72 px, mono right-aligned,
    /// the unit beside it. GPUI has no text input, so the well is a
    /// display; the slider drag is the way to change the value (React's
    /// field is typed into — the checklist's NumberField keyboard items
    /// stay unticked, as they do for Displays).
    fn number_well(
        &self,
        _id: &str,
        value: u32,
        _min: u32,
        _max: u32,
        unit: &str,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        // React `.field-slider .row-wrap`: the 72 px `.num` plus the unit's
        // advance, so the row is 72 + 6 + unit wide (103 for "fps", 86 for
        // "%"). A fixed 103 put Brightness 17 px left at every size.
        let unit_w = unit.chars().count() as f32 * 14.0 * 0.6;
        div()
            .flex()
            .w(px(72.0 + 6.0 + unit_w))
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .w(px(72.0))
                    .h(px(32.0))
                    .px_2()
                    .r_md()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .bg(Theme::color(t.sunk))
                    .text_color(Theme::color(t.ink))
                    .text_size(px(15.0))
                    .font_family(crate::fonts::MONO_FAMILY)
                    .child(value.to_string()),
            )
            .child(
                // GPUI does not give a bare text div its text width in a
                // flex row (the slider ate it at 390), so the unit cell
                // takes its mono advance (0.6em per char) explicitly.
                div()
                    .flex_none()
                    .w(px(unit.chars().count() as f32 * 14.0 * 0.6))
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(unit.to_string()),
            )
    }

    /// A slider + number pair row (`.field-slider`): the slider takes the
    /// room, the number keeps 72 px. `commit` runs on release.
    #[allow(clippy::too_many_arguments)]
    fn slider_pair(
        &self,
        id: &str,
        value: f32,
        min: f32,
        max: f32,
        num_value: u32,
        num_min: u32,
        num_max: u32,
        unit: &str,
        origin: Origin,
        is_fps: bool,
        slot_w: Option<f32>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view_entity = cx.entity();
        let change_entity = view_entity.clone();
        let commit_entity = view_entity.clone();
        let slider = slider_range(
            &self.theme,
            id,
            value,
            min,
            max,
            false,
            origin,
            move |v, _, cx| {
                change_entity.update(cx, |this, cx| {
                    if is_fps {
                        this.fps_draft = Some(v);
                    } else {
                        this.brightness_draft = Some(v);
                    }
                    cx.notify();
                });
            },
            move |v, _, cx| {
                commit_entity.update(cx, |this, cx| {
                    if is_fps {
                        this.commit_fps(v as u32, cx);
                    } else {
                        this.commit_brightness(v as u8, cx);
                    }
                });
            },
        );
        let well = self.number_well(&format!("{id}-num"), num_value, num_min, num_max, unit, cx);
        let slider = match slot_w {
            // Wrapped fields (phone and narrow columns): the slot takes
            // its React width (inner minus the number row) directly —
            // GPUI's flex mis-measures this nested row at 390.
            Some(w) => div().flex_none().w(px(w)).child(slider).into_any_element(),
            None => slider.into_any_element(),
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .flex_1()
            .min_w(px(0.0))
            .child(slider)
            .child(well)
    }

    /// A Rendering field: the label column and the slider pair. On a
    /// phone the pair takes the full row under the label.
    fn slider_field(
        &self,
        id: &str,
        label: &str,
        hint: Option<String>,
        control: impl IntoElement,
        last: bool,
        slot_w: Option<f32>,
    ) -> impl IntoElement {
        let t = &self.theme;
        // React `.fig` hint (Frame rate's "<n> fps measured") is mono.
        let hint_is_fig = id == "settings-fps-field";
        if slot_w.is_some() {
            // Wrapped (phone and narrow columns): the pair takes the
            // full row under the label.
            if hint_is_fig {
                crate::components::panel::field_stack_hint_fig(t, id, label, hint.as_deref(), control, last).into_any_element()
            } else {
                field_stack_hint(t, id, label, hint.as_deref(), control, last).into_any_element()
            }
        } else {
            // The control takes the room (React `.field-slider` flex
            // 1 1 240, min 240); the label keeps the rest.
            let control = div().flex_1().min_w(px(240.0)).child(control);
            if hint_is_fig {
                crate::components::panel::field_hint_fig(t, id, label, hint.as_deref(), control, last).into_any_element()
            } else {
                field_hint(t, id, label, hint.as_deref(), control, last).into_any_element()
            }
        }
    }

    // -- panels -----------------------------------------------------------

    fn rendering_panel(&self, pad: f32, _phone: bool, inner_w: f32, wrapped: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(settings) = &self.settings else {
            return div().into_any_element();
        };
        let shown_fps = self
            .fps_draft
            .map(|v| v as u32)
            .unwrap_or_else(|| self.render.as_ref().map(|r| r.fps).unwrap_or(settings.fps));
        let shown_bri = self
            .brightness_draft
            .map(|v| v as u8)
            .unwrap_or(settings.brightness);
        let hint = Some(format!(
            "{} fps measured",
            fmt1(self.status.as_ref().map(|s| s.fps_measured))
        ));
        // Wrapped rows (phone and narrow columns): the slider slot
        // takes the inner width minus the number row (72 + gap 6 +
        // the unit's advance + the row gap 12 = 115 for "fps").
        let slot_w = if wrapped {
            Some((inner_w - 115.0).max(80.0))
        } else {
            None
        };
        let bri_slot_w = slot_w;
        let mut body = div()
            .flex()
            .flex_col()
            .child(self.slider_field(
                "settings-fps-field",
                "Frame rate",
                hint,
                self.slider_pair(
                    "settings-fps-slider",
                    shown_fps as f32,
                    10.0,
                    60.0,
                    shown_fps,
                    10,
                    60,
                    "fps",
                    self.fps_origin.clone(),
                    true,
                    slot_w,
                    cx,
                ),
                false,
                slot_w,
            ))
            .child(self.slider_field(
                "settings-bri-field",
                "Brightness",
                None,
                self.slider_pair(
                    "settings-bri-slider",
                    shown_bri as f32,
                    0.0,
                    100.0,
                    shown_bri as u32,
                    0,
                    100,
                    "%",
                    self.bri_origin.clone(),
                    false,
                    bri_slot_w,
                    cx,
                ),
                true,
                bri_slot_w,
            ));
        if let Some(msg) = &self.render_err {
            body = body.child(self.error_line(msg, true));
        }
        self.panel("settings-rendering", "Rendering", pad, div(), body)
            .into_any_element()
    }

    fn appearance_panel(&self, pad: f32, _phone: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let view_entity = cx.entity();
        let theme_entity = view_entity.clone();
        let theme_control = segmented(
            t,
            "settings-theme",
            &[("dark", "Dark"), ("light", "Light"), ("system", "System")],
            &self.theme_pref.clone(),
            move |v, _, cx| {
                theme_entity.update(cx, |this, cx| this.apply_theme(v, cx));
            },
        );
        let motion_entity = view_entity.clone();
        let motion_control = segmented(
            t,
            "settings-motion",
            &[("full", "Full"), ("reduced", "Reduced")],
            &self.motion_pref.clone(),
            move |v, _, cx| {
                motion_entity.update(cx, |this, cx| this.apply_motion(v, cx));
            },
        );
        let body = div()
            .flex()
            .flex_col()
            .child(field_hint(t, "settings-theme-field", "Theme", None, theme_control, false))
            .child(field_hint(t, "settings-motion-field", "Motion", None, motion_control, true));
        self.panel("settings-appearance", "Appearance", pad, div(), body)
            .into_any_element()
    }

    fn boot_field(&self, pad: f32, _phone: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let Some(settings) = &self.settings else {
            return div().into_any_element();
        };
        let hex = self
            .boot_draft
            .clone()
            .unwrap_or_else(|| settings.boot_color.map(|c| c.to_hex()).unwrap_or("#000000".into()));
        let view_entity = cx.entity();
        let swatch_entity = view_entity.clone();
        let swatch = div()
            .id(gpui::SharedString::from("settings-boot-swatch"))
            .relative()
            .w(px(32.0))
            .h(px(32.0))
            .r_md()
            .r_sm()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(&hex))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                swatch_entity.update(cx, |this, cx| {
                    if this.boot_open {
                        this.boot_open = false;
                        let last = this.boot_draft.clone();
                        if let Some(hex) = last {
                            if let Some(s) = &this.settings {
                                if s.boot_color.map(|c| c.to_hex()) != Some(hex.clone()) {
                                    let mut next = s.clone();
                                    next.boot_color = hex.parse::<Rgb8>().ok();
                                    this.save(next, cx);
                                    return;
                                }
                            }
                        }
                    } else {
                        this.boot_open = true;
                    }
                    cx.notify();
                });
            });
        let clear_entity = view_entity.clone();
        let clear_disabled = settings.boot_color.is_none();
        let control = div()
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .child(swatch)
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(hex.clone()),
            )
            .child(button_sm(
                t,
                "settings-boot-clear",
                "Clear",
                Variant::Ghost,
                clear_disabled,
                move |_, _, cx| {
                    clear_entity.update(cx, |this, cx| {
                        if let Some(s) = &this.settings {
                            let mut next = s.clone();
                            next.boot_color = None;
                            this.save(next, cx);
                        }
                    });
                },
            ));
        let control = if self.boot_open {
            control.child(self.color_picker_popover(&hex, cx))
        } else {
            control
        };
        let field = field_hint(t, "settings-boot-field", "Boot colour", None, control, true);
        let _ = pad;
        field.into_any_element()
    }

    /// The colour picker popover (`.popover.picker-pop`): the SV square,
    /// the hue strip and the hex row. Drags stream the colour into
    /// `boot_draft` only; closing commits.
    fn color_picker_popover(&self, hex: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let (h, s, v) = hex_to_hsv(hex, 0.0);
        let view_entity = cx.entity();
        // SV square.
        let sv_origin = self.sv_origin.clone();
        let sv_entity = view_entity.clone();
        let hue_rgb = hsv_to_hex(h, 1.0, 1.0);
        let sv = div()
            .relative()
            .w(px(212.0))
            .h(px(212.0))
            .r_sm()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .child(
                gpui::canvas(
                    {
                        let origin = sv_origin.clone();
                        move |bounds, _window, _cx| {
                            origin.set(
                                f32::from(bounds.origin.x),
                                f32::from(bounds.origin.y),
                                f32::from(bounds.size.width),
                                f32::from(bounds.size.height),
                            );
                        }
                    },
                    move |bounds, _, window, _| {
                        let (hue_r, hue_g, hue_b) = hex_to_rgb(&hue_rgb);
                        let w: f32 = bounds.size.width.into();
                        let hh: f32 = bounds.size.height.into();
                        let step = 4.0f32;
                        let mut y = 0.0f32;
                        while y < hh {
                            let vv = 1.0 - (y + step / 2.0) / hh;
                            let mut x = 0.0f32;
                            while x < w {
                                let ss = (x + step / 2.0) / w;
                                // white wash across, black wash down
                                let r = hue_r + (1.0 - hue_r) * (1.0 - ss);
                                let g = hue_g + (1.0 - hue_g) * (1.0 - ss);
                                let b = hue_b + (1.0 - hue_b) * (1.0 - ss);
                                let color = gpui::Rgba {
                                    r: r * vv,
                                    g: g * vv,
                                    b: b * vv,
                                    a: 1.0,
                                };
                                window.paint_quad(gpui::fill(
                                    gpui::Bounds {
                                        origin: gpui::point(
                                            bounds.origin.x + gpui::px(x),
                                            bounds.origin.y + gpui::px(y),
                                        ),
                                        size: gpui::size(
                                            gpui::px(step.min(w - x)),
                                            gpui::px(step.min(hh - y)),
                                        ),
                                    },
                                    color,
                                ));
                                x += step;
                            }
                            y += step;
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            // The handle ring at (s, v).
            .child(
                div()
                    .absolute()
                    .top(px((1.0 - v) * 212.0 - 7.0))
                    .left(px(s * 212.0 - 7.0))
                    .size(px(14.0))
                    .rounded_full()
                    .border_2()
                    .border_color(Theme::color(t.ink)),
            );
        let sv = sv
            .cursor_crosshair()
            .on_mouse_down(gpui::MouseButton::Left, {
                let origin = sv_origin.clone();
                let entity = sv_entity.clone();
                move |event: &gpui::MouseDownEvent, _window, cx| {
                    let (ox, oy, w, hh) = origin.get();
                    let x = f32::from(event.position.x);
                    let y = f32::from(event.position.y);
                    let ss = ((x - ox) / w.max(1.0)).clamp(0.0, 1.0);
                    let vv = (1.0 - (y - oy) / hh.max(1.0)).clamp(0.0, 1.0);
                    entity.update(cx, |this, cx| {
                        let (ch, _, _) = hex_to_hsv(
                            this.boot_draft.as_deref().unwrap_or("#000000"),
                            0.0,
                        );
                        this.boot_draft = Some(hsv_to_hex(ch, ss, vv));
                        cx.notify();
                    });
                }
            })
            .on_mouse_move({
                let origin = sv_origin.clone();
                let entity = sv_entity.clone();
                move |event: &gpui::MouseMoveEvent, _window, cx| {
                    let (ox, oy, w, hh) = origin.get();
                    let x = f32::from(event.position.x);
                    let y = f32::from(event.position.y);
                    let ss = ((x - ox) / w.max(1.0)).clamp(0.0, 1.0);
                    let vv = (1.0 - (y - oy) / hh.max(1.0)).clamp(0.0, 1.0);
                    entity.update(cx, |this, cx| {
                        let (ch, _, _) = hex_to_hsv(
                            this.boot_draft.as_deref().unwrap_or("#000000"),
                            0.0,
                        );
                        this.boot_draft = Some(hsv_to_hex(ch, ss, vv));
                        cx.notify();
                    });
                }
            });
        // Hue strip.
        let hue_origin = self.hue_origin.clone();
        let hue_entity = view_entity.clone();
        let hue_strip = div()
            .relative()
            .w_full()
            .h(px(14.0))
            .r_sm()
            .r_xs()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .child(
                gpui::canvas(
                    {
                        let origin = hue_origin.clone();
                        move |bounds, _window, _cx| {
                            origin.set(
                                f32::from(bounds.origin.x),
                                f32::from(bounds.origin.y),
                                f32::from(bounds.size.width),
                                f32::from(bounds.size.height),
                            );
                        }
                    },
                    move |bounds, _, window, _| {
                        let w: f32 = bounds.size.width.into();
                        let hh: f32 = bounds.size.height.into();
                        let step = 2.0f32;
                        let mut x = 0.0f32;
                        while x < w {
                            let hue = (x + step / 2.0) / w * 360.0;
                            let hex = hsv_to_hex(hue, 1.0, 1.0);
                            let (r, g, b) = hex_to_rgb(&hex);
                            window.paint_quad(gpui::fill(
                                gpui::Bounds {
                                    origin: gpui::point(bounds.origin.x + gpui::px(x), bounds.origin.y),
                                    size: gpui::size(gpui::px(step.min(w - x)), gpui::px(hh)),
                                },
                                gpui::Rgba { r, g, b, a: 1.0 },
                            ));
                            x += step;
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .absolute()
                    .top(px(-3.0))
                    .left(px(h / 360.0 * 212.0 - 5.0))
                    .w(px(10.0))
                    .h(px(20.0))
                    .r_sm()
                    .r_xs()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .bg(Theme::color(t.raised)),
            );
        let hue_strip = hue_strip
            .cursor_pointer()
            .on_mouse_down(gpui::MouseButton::Left, {
                let origin = hue_origin.clone();
                let entity = hue_entity.clone();
                move |event: &gpui::MouseDownEvent, _window, cx| {
                    let (ox, _, w, _) = origin.get();
                    let hue = ((f32::from(event.position.x) - ox) / w.max(1.0)).clamp(0.0, 1.0) * 360.0;
                    entity.update(cx, |this, cx| {
                        let cur = this.boot_draft.as_deref().unwrap_or("#000000").to_string();
                        let (_, ss, vv) = hex_to_hsv(&cur, 0.0);
                        this.boot_draft = Some(hsv_to_hex(hue, ss, vv));
                        cx.notify();
                    });
                }
            })
            .on_mouse_move({
                let origin = hue_origin.clone();
                let entity = hue_entity.clone();
                move |event: &gpui::MouseMoveEvent, _window, cx| {
                    let (ox, _, w, _) = origin.get();
                    let hue = ((f32::from(event.position.x) - ox) / w.max(1.0)).clamp(0.0, 1.0) * 360.0;
                    entity.update(cx, |this, cx| {
                        let cur = this.boot_draft.as_deref().unwrap_or("#000000").to_string();
                        let (_, ss, vv) = hex_to_hsv(&cur, 0.0);
                        this.boot_draft = Some(hsv_to_hex(hue, ss, vv));
                        cx.notify();
                    });
                }
            });
        // Hex row: the mono hex and the preview.
        let hex_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child(hex.to_string()),
            )
            .child(
                div()
                    .w(px(24.0))
                    .h(px(24.0))
                    .r_md()
                    .r_sm()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .bg(Theme::color(hex)),
            );
        // The popover panel (min-width 236 = 212 + padding).
        div()
            .id(gpui::SharedString::from("settings-boot-picker"))
            .absolute()
            .top(px(38.0))
            .left_0()
            .w(px(236.0))
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .r_md()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.panel))
            .child(sv)
            .child(hue_strip)
            .child(hex_row)
    }

    fn lock_panel(&self, pad: f32, _phone: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let Some(settings) = &self.settings else {
            return div().into_any_element();
        };
        let view_entity = cx.entity();
        let black_entity = view_entity.clone();
        let black_control = toggle(
            t,
            "settings-black-lock",
            settings.black_on_lock,
            move |v, _, cx| {
                black_entity.update(cx, |this, cx| {
                    if let Some(s) = &this.settings {
                        let mut next = s.clone();
                        next.black_on_lock = v;
                        this.save(next, cx);
                    }
                });
            },
        );
        let quit_entity = view_entity.clone();
        let quit_control = segmented(
            t,
            "settings-on-quit",
            &[("black", "Black"), ("leave_last_frame", "Leave last frame")],
            match settings.on_quit {
                canvas_proto::config::OnQuit::Black => "black",
                canvas_proto::config::OnQuit::LeaveLastFrame => "lead",
            },
            move |v, _, cx| {
                quit_entity.update(cx, |this, cx| {
                    if let Some(s) = &this.settings {
                        let mut next = s.clone();
                        next.on_quit = if v == "black" {
                            canvas_proto::config::OnQuit::Black
                        } else {
                            canvas_proto::config::OnQuit::LeaveLastFrame
                        };
                        this.save(next, cx);
                    }
                });
            },
        );
        let body = div()
            .flex()
            .flex_col()
            .child(field_hint(t, "settings-black-field", "Black on lock", None, black_control, false))
            .child(field_hint(t, "settings-quit-field", "On quit", None, quit_control, false))
            .child(self.boot_field(pad, false, cx));
        self.panel("settings-lock", "Lock and quit", pad, div(), body)
            .into_any_element()
    }

    // -- audio --------------------------------------------------------------

    fn audio_groups(&self) -> Vec<(String, Vec<(String, String)>)> {
        let mut groups = vec![(String::new(), Vec::new())];
        let resolved = self
            .audio_sources
            .as_ref()
            .and_then(|s| s.current.name.clone());
        let follow = match resolved {
            Some(name) => format!("Follow Windows default output: {name}"),
            None => "Follow Windows default output".to_string(),
        };
        groups[0].1.push(("follow-default".into(), follow));
        if let Some(sources) = &self.audio_sources {
            let mut outputs = Vec::new();
            let mut inputs = Vec::new();
            for e in &sources.endpoints {
                let label = if e.state == canvas_proto::audio::AudioEndpointState::NotConnected {
                    format!("{} (not connected)", e.name)
                } else {
                    e.name.clone()
                };
                match e.flow {
                    AudioFlow::Render => outputs.push((e.id.clone(), label)),
                    AudioFlow::Capture => inputs.push((e.id.clone(), label)),
                }
            }
            groups.push(("Outputs (loopback)".into(), outputs));
            groups.push(("Inputs".into(), inputs));
        }
        groups
    }

    fn audio_value(&self) -> String {
        match self
            .audio_sources
            .as_ref()
            .map(|s| &s.current.requested)
        {
            Some(AudioSource::Endpoint { id, .. }) => id.clone(),
            _ => "follow-default".to_string(),
        }
    }

    fn audio_select(
        &self,
        id: &str,
        full_width: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let groups_owned = self.audio_groups();
        let owned_refs: Vec<(&str, Vec<(&str, &str)>)> = groups_owned
            .iter()
            .map(|(h, opts)| {
                (
                    h.as_str(),
                    opts.iter().map(|(v, l)| (v.as_str(), l.as_str())).collect::<Vec<_>>(),
                )
            })
            .collect();
        let refs: Vec<(&str, &[(&str, &str)])> = owned_refs
            .iter()
            .map(|(h, opts)| (*h, opts.as_slice()))
            .collect();
        let view_entity = cx.entity();
        let toggle_entity = view_entity.clone();
        let pick_entity = view_entity;
        let value = self.audio_value();
        let open = self.audio_select_open;
        let disabled = self.audio_sources.is_none();
        select_grouped(
            &self.theme,
            id,
            &refs,
            &value,
            open,
            disabled,
            full_width,
            move |_, cx| {
                toggle_entity.update(cx, |this, cx| {
                    this.audio_select_open = !this.audio_select_open;
                    cx.notify();
                });
            },
            move |v, _, cx| {
                pick_entity.update(cx, |this, cx| this.pick_audio(v, cx));
            },
        )
        .into_any_element()
    }

    fn audio_readings(&self) -> impl IntoElement {
        let t = &self.theme;
        let peak = self.audio_test.as_ref().and_then(|s| s.peak);
        let peak_db = self.audio_test.as_ref().and_then(|s| s.peak_db);
        let bands = self.audio_test.as_ref().map(|s| s.bands.clone()).unwrap_or_default();
        let cells = led_cells(peak);
        let mut meter = div()
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .gap(px(2.0))
            .h(px(16.0));
        for tone in cells {
            let bg = match tone {
                Some("ok") => Theme::color(t.ok),
                Some("work") => Theme::color(t.work),
                Some("warn") => Theme::color(t.warn),
                _ => Theme::color(t.line_2),
            };
            meter = meter.child(div().flex_1().r_sm().bg(bg));
        }
        let db = div()
            .min_w(px(48.0))
            .text_right()
            .text_size(px(16.0))
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.ink))
            .child(match peak_db {
                Some(v) if v.is_finite() => format!("{v:.1}"),
                _ => "—".to_string(),
            });
        let mut band_row = div()
            .flex()
            .flex_1()
            .items_end()
            .gap(px(2.0))
            .min_h(px(0.0));
        for i in 0..32 {
            let v = bands.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            band_row = band_row.child(
                div()
                    .flex_1()
                    .h(px(v * 100.0 + 2.0))
                    .r_sm()
                    .bg(Theme::color(t.line_2)),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            // `.audio-readings`: the block is 96 px in every state.
            .h(px(96.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(meter)
                    .child(db),
            )
            .child(band_row)
    }

    const EFFECT_NOTE: &'static str =
        "Audio effects read this input whenever an audio effect is active (not only while this panel is open).";
    const EFFECT_NOTE_SHORT: &'static str = "Audio effects use this input";

    fn audio_panel(&self, pad: f32, compact: bool, inner_w: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let input_selected = is_input_selection(self.audio_sources.as_ref());
        let words = audio_status_words(self.audio_test.as_ref());
        let view_entity = cx.entity();
        if compact {
            // The select sits in the head's action slot (React
            // `.audio-picker-head`: flex 0 1 300 px, min 160). The title
            // ("AUDIO SOURCE" tracked, about 105 px) keeps its width and
            // the select takes the rest, clamped [160, 300]: a content-sized
            // flex_1 let the select overlap the title at 390.
            let select_w = (inner_w - 105.0 - 12.0).clamp(160.0, 300.0);
            let select = self.audio_select("settings-audio-select", true, cx);
            let head_actions = div()
                .flex_none()
                .w(px(select_w))
                .justify_end()
                .child(select);
            let listen_entity = view_entity.clone();
            let listen_control = toggle(t, "settings-audio-listen", self.listen, move |v, _, cx| {
                listen_entity.update(cx, |this, cx| this.set_listen(v, cx));
            });
            let pop_entity = view_entity.clone();
            let show_btn = button_sm(
                t,
                "settings-audio-show",
                if self.popover_open { "Hide readings" } else { "Show readings" },
                Variant::Ghost,
                false,
                move |_, _, cx| {
                    pop_entity.update(cx, |this, cx| {
                        this.popover_open = !this.popover_open;
                        this.start_audio_poll(cx);
                        cx.notify();
                    });
                },
            );
            let compact_words = if input_selected && !self.listen {
                Self::EFFECT_NOTE_SHORT.to_string()
            } else {
                words.clone()
            };
            let mut row = div()
                .flex()
                .items_center()
                .gap_3()
                .min_h(px(28.0))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child(compact_words),
                );
            if input_selected {
                row = row.child(listen_control);
            }
            row = row.child(show_btn);
            let mut body = div().flex().flex_col().child(row);
            if let Some(msg) = &self.audio_err {
                body = body.child(self.error_line(msg, false));
            }
            if self.popover_open {
                let mut pop = div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.audio_readings_rows());
                if input_selected {
                    pop = pop.child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.dim))
                            .child(Self::EFFECT_NOTE),
                    );
                }
                body = body.child(
                    div()
                        .absolute()
                        .top(px(38.0))
                        .right_0()
                        .w(px(320.0))
                        .p_3()
                        .r_md()
                        .border_1()
                        .border_color(Theme::color(t.line_2))
                        .bg(Theme::color(t.panel))
                        .child(pop),
                );
            }
            // Compact: padding-block s2 (8), others s4 (16) — the panel
            // builder takes one pad; override the vertical after.
            let panel = div()
                .id(gpui::SharedString::from("settings-audio"))
                .flex()
                .flex_col()
                .px(px(pad))
                .py_2()
                .r_lg()
                .border_1()
                .border_color(Theme::color(t.line))
                .bg(Theme::color(t.panel))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .mb_2()
                        .child(
                            div()
                                .flex_none()
                                .child(crate::components::panel::tracked_title_nowrap(
                                    "AUDIO SOURCE",
                                    13.0,
                                    0.08,
                                    gpui::FontWeight::MEDIUM,
                                    crate::fonts::MONO_FAMILY,
                                    Theme::color(t.mute),
                                )),
                        )
                        .child(head_actions),
                )
                .child(body);
            panel.into_any_element()
        } else {
            let select = self.audio_select("settings-audio-select", true, cx);
            let listen_entity = view_entity.clone();
            let picker_row = {
                let mut r = div().flex().items_center().gap_3().flex_1().min_w(px(0.0)).child(select);
                if input_selected {
                    r = r.child(toggle(
                        t,
                        "settings-audio-listen",
                        self.listen,
                        move |v, _, cx| {
                            listen_entity.update(cx, |this, cx| this.set_listen(v, cx));
                        },
                    ));
                }
                r
            };
            let hint = if input_selected { Some(Self::EFFECT_NOTE) } else { None };
            // The audio content sits 4 px high against React at every
            // non-compact size (1440 column 800: select top 407 vs 411,
            // panel bottom 600 vs 604): the body carries 4 px more.
            let mut body = div()
                .flex()
                .flex_col()
                .mt_1()
                .child(field_hint(t, "settings-audio-field", "Source", hint, picker_row, false))
                .child(
                    div()
                        .mt_2()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child(words),
                );
            if let Some(msg) = &self.audio_err {
                body = body.child(self.error_line(msg, false));
            }
            body = body.child(div().mt_3().child(self.audio_readings_rows()));
            let speaker = gpui::svg()
                .path("icons/SpeakerHigh.svg")
                .size(px(16.0))
                .text_color(Theme::color(t.ink));
            self.panel("settings-audio", "Audio source", pad, speaker, body)
                .into_any_element()
        }
    }

    /// The fixed-height readings block (`.audio-readings`, 96 px).
    fn audio_readings_rows(&self) -> impl IntoElement {
        self.audio_readings()
    }

    // -- setup & version ----------------------------------------------------

    fn setup_panel(&self, pad: f32, _phone: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let view_entity = cx.entity();
        let map_entity = view_entity.clone();
        let map_control = button_icon(
            t,
            "settings-map",
            "Map my PC again",
            Variant::Default,
            false,
            false,
            "CompassTool",
            "",
            16.0,
            move |_, _, cx| {
                map_entity.update(cx, |this, cx| this.map_pc(cx));
            },
        );
        let export_entity = view_entity.clone();
        let export_control = button_icon(
            t,
            "settings-export",
            "Export",
            Variant::Default,
            false,
            false,
            "DownloadSimple",
            "",
            16.0,
            move |_, _, cx| {
                export_entity.update(cx, |this, cx| this.do_export(cx));
            },
        );
        let import_entity = view_entity.clone();
        let import_control = button_icon(
            t,
            "settings-import",
            "Import",
            Variant::Default,
            false,
            false,
            "UploadSimple",
            "",
            16.0,
            move |_, _, cx| {
                import_entity.update(cx, |this, cx| this.do_import(cx));
            },
        );
        let mut body = div()
            .flex()
            .flex_col()
            .child(field_hint(t, "settings-map-field", "Map my PC", Some("Walk through every light again"), map_control, false))
            .child(field_hint(t, "settings-export-field", "Export", None, export_control, false))
            .child(field_hint(t, "settings-import-field", "Import", None, import_control, true));
        if let Some(note) = &self.note {
            body = body.child(state_dot(t, "ok", note));
        }
        if let Some(msg) = &self.err {
            body = body.child(self.error_line(msg, true));
        }
        self.panel("settings-setup", "Setup", pad, div(), body)
            .into_any_element()
    }

    fn version_panel(&self, pad: f32, _phone: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let version = self.status.as_ref().map(|s| s.version.clone()).unwrap_or_else(|| "—".to_string());
        // React `.fig` inherits the body 15 px (`--t15`): "0.9.0" is 46 px
        // wide there, 41 at GPUI's old 14 px.
        let version_fig = div()
            .text_size(px(15.0))
            .line_height(gpui::relative(1.45))
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.ink))
            .child(version);
        // Tray-only controls, in React's tray or no-tray state (`UpdatesPanel.tsx`):
        // `acts` is `canUpdate()` (a tray is attached when stdin is a pipe).
        let acts = tray_acts(self.tray_present);
        let u = self.tray_update.as_ref();
        let prefs = self.tray_prefs.as_ref();
        let checking = is_checking(u);
        let today = today_day_utc();
        let last_word = last_checked_word(acts, u, today);
        let view_entity = cx.entity();

        // Check automatically (`UpdatesPanel.tsx:76-83`).
        let auto_entity = view_entity.clone();
        let auto_checked_state = auto_checked(u);
        let auto_disabled_state = auto_disabled(acts, u);
        let auto_control = if auto_disabled_state {
            toggle_disabled(t, "settings-update-auto", auto_checked_state, |_, _, _| {}).into_any_element()
        } else {
            toggle(t, "settings-update-auto", auto_checked_state, move |v, _, cx| {
                auto_entity.update(cx, |this, cx| {
                    this.tray_err = None;
                    cx.notify();
                });
                crate::send_update_set_auto(v);
            }).into_any_element()
        };
        let last_checked = div()
            .text_size(px(15.0))
            .line_height(gpui::relative(1.45))
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.ink))
            .child(last_word);

        // Check now (`UpdatesPanel.tsx:87-94`): the `Swap` keeps the widest
        // label's width, so the key never resizes; here the visible label is
        // centred in that cell.
        let check_entity = view_entity.clone();
        let check_label = if checking { "Checking" } else { "Check now" };
        let check_control = button(
            t,
            "settings-update-check",
            check_label,
            Variant::Default,
            check_now_disabled(acts, u),
            move |_, _, cx| {
                check_entity.update(cx, |this, cx| {
                    this.tray_err = None;
                    cx.notify();
                });
                crate::send_update_check_now();
            },
        );

        // Experimental builds (`UpdatesPanel.tsx:95-102`).
        let experimental_entity = view_entity.clone();
        let experimental_checked_state = experimental_checked(prefs);
        let experimental_disabled = prefs.is_none();
        let experimental_control = if experimental_disabled {
            toggle_disabled(
                t,
                "settings-update-experimental",
                experimental_checked_state,
                |_, _, _| {},
            ).into_any_element()
        } else {
            toggle(t, "settings-update-experimental", experimental_checked_state, move |v, _, cx| {
                experimental_entity.update(cx, |this, cx| {
                    this.tray_err = None;
                    cx.notify();
                });
                crate::send_set_channel(if v {
                    UpdateChannel::Experimental
                } else {
                    UpdateChannel::Stable
                });
            }).into_any_element()
        };

        // Editor (`UpdatesPanel.tsx:103-113`), only on an experimental version.
        let editor_field = if editor_row_visible(u) {
            let editor_entity = view_entity.clone();
            let editor_selected = match editor_value(prefs) {
                EditorChoice::React => "react",
                EditorChoice::Gpui => "gpui",
            };
            let opts: &[(&str, &str)] = &[("react", "React"), ("gpui", "GPUI")];
            let editor_control = if prefs.is_none() {
                crate::components::segmented::segmented_disabled(
                    t,
                    "settings-update-editor",
                    opts,
                    editor_selected,
                    |_, _, _| {},
                ).into_any_element()
            } else {
                segmented(t, "settings-update-editor", opts, editor_selected, move |v, _, cx| {
                    editor_entity.update(cx, |this, cx| {
                        this.tray_err = None;
                        cx.notify();
                    });
                    crate::send_switch_editor(if v == "gpui" {
                        EditorChoice::Gpui
                    } else {
                        EditorChoice::React
                    });
                }).into_any_element()
            };
            Some(field_hint(t, "settings-editor-field", "Editor", Some("GPUI is experimental"), editor_control, false))
        } else {
            None
        };

        // Status row (`UpdatesPanel.tsx:114-134`).
        let status_row = match tray_status(u) {
            Some((tone, words)) => {
                let mut acts_row = div().flex().items_center().gap_2();
                if undo_visible(u) {
                    let undo_entity = view_entity.clone();
                    acts_row = acts_row.child(button_sm(
                        t,
                        "settings-update-undo",
                        "Undo",
                        Variant::Ghost,
                        !acts,
                        move |_, _, cx| {
                            undo_entity.update(cx, |this, cx| {
                                this.tray_err = None;
                                cx.notify();
                            });
                            crate::send_update_unskip();
                        },
                    ));
                }
                if whats_new_visible(u) {
                    let nav_entity = view_entity.clone();
                    acts_row = acts_row.child(button_sm(
                        t,
                        "settings-update-whatsnew",
                        "What’s new",
                        Variant::Ghost,
                        false,
                        move |_, _, cx| {
                            nav_entity.update(cx, |this, cx| {
                                if let Some(shell) = this.shell.clone() {
                                    shell.update(cx, |s, cx| {
                                        s.navigate("updates");
                                        cx.notify();
                                    });
                                }
                            });
                        },
                    ));
                }
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .min_h(px(48.0))
                    .pt_2()
                    .child(state_dot(t, &tone, &words))
                    .child(acts_row)
            }
            None => {
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .min_h(px(48.0))
                    .pt_2()
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child(TrayAbsence::STATUS.to_string()),
                    )
            }
        };

        // Open logs folder (`SettingsView.tsx:299-309`).
        let logs_entity = cx.entity();
        let logs_control = button_icon(
            t,
            "settings-logs",
            "Open logs folder",
            Variant::Default,
            !acts,
            false,
            "FolderOpen",
            "",
            16.0,
            move |_, _, cx| {
                logs_entity.update(cx, |this, cx| {
                    this.err = None;
                    cx.notify();
                });
                crate::send_open_logs_folder();
            },
        );
        let logs_row = if acts {
            div().flex().flex_wrap().items_center().gap_2().mt_3().child(logs_control)
        } else {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .mt_3()
                .child(logs_control)
                .child(
                    div()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child("Tray app only"),
                )
        };

        let mut body = div()
            .flex()
            .flex_col()
            .child(field_hint(t, "settings-auto-field", "Check automatically", Some("At start, then once a day"), auto_control, false))
            .child(field_hint(t, "settings-last-field", "Last checked", None, last_checked, false))
            .child(field_hint(t, "settings-check-field", "Check now", None, check_control, false))
            .child(field_hint(t, "settings-experimental-field", "Experimental builds", Some("Unstable test builds, offered early"), experimental_control, editor_field.is_some()));
        if let Some(editor) = editor_field {
            body = body.child(editor);
        }
        body = body.child(status_row);
        if !acts {
            body = body.child(
                div()
                    .mt_2()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("Tray app only"),
            );
        }
        if let Some(msg) = &self.tray_err {
            body = body.child(self.error_line(msg, true));
        }
        body = body.child(logs_row);
        self.panel("settings-version", "Version", pad, version_fig, body)
            .into_any_element()
    }

    /// The loading skeleton (React `SettingsShape`: two stacks of
    /// skeleton panels; GPUI draws the same rhythm as plain boxes).
    fn loading_shape(&self, cols: usize, pad: f32) -> impl IntoElement {
        let t = &self.theme;
        let skel = |w: f32, h: f32| {
            div()
                .w(px(w))
                .h(px(h))
                .r_sm()
                .bg(Theme::color(t.raised))
        };
        let panel_box = |bars: &[(f32, f32)]| {
            let mut p = div()
                .flex()
                .flex_col()
                .gap_2()
                .p(px(pad))
                .r_lg()
                .border_1()
                .border_color(Theme::color(t.line))
                .bg(Theme::color(t.panel));
            for (w, h) in bars {
                p = p.child(skel(*w, *h));
            }
            p
        };
        let left = div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .child(panel_box(&[(169.3, 46.0), (169.3, 46.0)]))
            .child(panel_box(&[(207.3, 46.0), (207.3, 46.0), (207.3, 46.0)]));
        let right = div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .child(panel_box(&[(120.3, 46.0)]))
            .child(panel_box(&[(159.3, 46.0), (159.3, 46.0)]))
            .child(panel_box(&[(159.3, 46.0), (159.3, 46.0)]));
        if cols <= 1 {
            left.child(right)
        } else {
            div().flex().gap_4().child(left).child(right)
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The tray's lines that are already waiting (a `ready` push): take them
        // before this draw, so the first frame with a tray is already tray state.
        self.drain_tray(cx);
        let win_w = f32::from(window.window_bounds().get_bounds().size.width);
        let win_h = f32::from(window.window_bounds().get_bounds().size.height);
        let rail_w = if win_w <= 720.0 {
            0.0
        } else if win_w < 1280.0 {
            64.0
        } else {
            200.0
        };
        let content_w = win_w - rail_w - if win_w <= 720.0 { 32.0 } else { 48.0 };
        let phone = win_w <= 720.0;
        let pad = if content_w <= 639.0 { 12.0 } else { 16.0 };
        let compact = win_h <= 899.0 || win_w <= 639.0;
        self.audio_use_popover = compact;
        self.start_audio_poll(cx);
        let t = &self.theme;

        let mut page = div().flex().flex_col().gap_4();
        // The stale banner (settings read, then a refresh failed).
        if self.settings.is_some() {
            if let Some(stale) = &self.settings_error {
                page = page.child(
                    div()
                        .flex()
                        .gap_2()
                        .p_2()
                        .r_md()
                        .border_1()
                        .border_color(Theme::color(t.warn))
                        .bg(Theme::color(t.warn_wash))
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.ink))
                        .child("The settings may be out of date.")
                        .child(
                            div()
                                .text_sm()
                                .font_family(crate::fonts::MONO_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child(stale.clone()),
                        ),
                );
            }
        }
        if self.settings.is_none() {
            if self.retrying || self.load_error.is_none() {
                let cols = settings_columns(content_w);
                page = page.child(self.loading_shape(cols, pad));
            } else {
                let entity = cx.entity();
                let retry = button(
                    t,
                    "settings-try-again",
                    "Try again",
                    Variant::Primary,
                    self.retrying,
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.retrying = true;
                            cx.notify();
                            this.load_all(cx);
                        });
                    },
                );
                page = page.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .p_8()
                        .r_lg()
                        .border_1()
                        .border_color(Theme::color(t.line))
                        .bg(Theme::color(t.panel))
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child("The settings could not be read"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.dim))
                                .child("canvas-rgb could not read the settings. Nothing was changed."),
                        )
                        .child(retry)
                        .child(
                            div()
                                .text_sm()
                                .font_family(crate::fonts::MONO_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child(self.load_error.clone().unwrap_or_default()),
                        ),
                );
            }
            return page;
        }

        // The six panels in CSS multicol order.
        let cols = settings_columns(content_w);
        let heights = estimated_heights(compact, phone, self.audio_err.is_some(), content_w, cols);
        let distribution = distribute(&heights, cols);
        let mut grid = div().flex().gap_4();
        for column in distribution {
            // `min_w(0)`: a flex item's minimum is its content's width by default, so the
            // audio row ("Follow Windows default output" plus its label) pushed the phone
            // column about 30 px past the window. React's multicol columns never grow past
            // their share.
            let mut col = div().flex().flex_col().gap_4().flex_1().min_w(px(0.0));
            for idx in column {
                col = match idx {
                    0 => {
                        let col_w = (content_w - (cols - 1) as f32 * 16.0) / cols as f32;
                        let inner = col_w - pad * 2.0;
                        // Forced slot widths only on the phone: on the
                        // desktop the flex row already lands where React
                        // puts it (measured at 1440).
                        let wrapped = phone;
                        col.child(self.rendering_panel(pad, phone, inner, wrapped, cx).into_any_element())
                    }
                    1 => col.child(self.appearance_panel(pad, phone, cx).into_any_element()),
                    2 => col.child(self.lock_panel(pad, phone, cx).into_any_element()),
                    3 => {
                        let col_w = (content_w - (cols - 1) as f32 * 16.0) / cols as f32;
                        let inner = col_w - pad * 2.0;
                        col.child(self.audio_panel(pad, compact, inner, cx).into_any_element())
                    }
                    4 => col.child(self.setup_panel(pad, phone, cx).into_any_element()),
                    _ => col.child(self.version_panel(pad, phone, cx).into_any_element()),
                };
            }
            grid = grid.child(col);
        }
        let page = page.child(grid);
        match super::capture_scroll() {
            s if s > 0.0 => page.relative().top(px(-s)),
            _ => page,
        }
    }
}

/// Estimated panel heights for the multicol balance, measured from the
/// CSS: panel padding 32, head 30, a plain field 48, a hinted field 58,
/// a wrapped slider field 98/78 (the label stacks over the control once
/// the panel inner width drops below label 140 + control min 240 + gap
/// 16), each panel carrying its 16 px bottom margin except the last.
/// (At 1440: [R,A],[L,Audio],[Setup,Version] — the React flow.)
fn estimated_heights(compact: bool, phone: bool, audio_err: bool, content_w: f32, cols: usize) -> Vec<f32> {
    let head = 30.0;
    let pad = 32.0;
    let col_w = (content_w - (cols as f32 - 1.0).max(0.0) * 16.0) / cols.max(1) as f32;
    let wrapped = col_w - 32.0 < 396.0 || phone;
    let fps_field = if wrapped { 98.0 } else { 58.0 };
    let bri_field = if wrapped { 78.0 } else { 48.0 };
    let rendering = pad + head + fps_field + bri_field;
    let appearance = pad + head + 96.0;
    let lock = pad + head + 144.0;
    let audio = if compact {
        16.0 + 26.0 + 28.0 + if audio_err { 20.0 } else { 0.0 }
    } else {
        pad + head + 48.0 + 28.0 + 108.0 + if audio_err { 20.0 } else { 0.0 }
    };
    let setup = pad + head + 154.0;
    let version = pad + head + 212.0 + 48.0 + 28.0 + 44.0;
    let mut heights = vec![rendering, appearance, lock, audio, setup, version];
    for h in heights.iter_mut().take(5) {
        *h += 16.0;
    }
    heights
}

// ---------------------------------------------------------------------------
// The GPUI editor's own prefs file (`%LOCALAPPDATA%\canvas-rgb\`).

fn prefs_path() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| {
        std::path::PathBuf::from(base)
            .join("canvas-rgb")
            .join("gpui-prefs.json")
    })
}

fn read_prefs() -> (String, String) {
    if let Some(path) = prefs_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                let theme = v
                    .get("theme")
                    .and_then(|x| x.as_str())
                    .unwrap_or("system")
                    .to_string();
                let motion = v
                    .get("motion")
                    .and_then(|x| x.as_str())
                    .unwrap_or("full")
                    .to_string();
                return (theme, motion);
            }
        }
    }
    ("system".to_string(), "full".to_string())
}

fn write_prefs(theme: &str, motion: &str) {
    if let Some(path) = prefs_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body = format!(r#"{{"theme":"{theme}","motion":"{motion}"}}"#);
        let _ = std::fs::write(&path, body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_order_is_the_react_dom_order() {
        assert_eq!(
            PANEL_TITLES,
            ["Rendering", "Appearance", "Lock and quit", "Audio source", "Setup", "Version"]
        );
    }

    #[test]
    fn experimental_version_needs_a_dash_before_a_plus() {
        assert!(is_experimental_version("0.8.5-dev.12"));
        assert!(!is_experimental_version("0.8.5+abc"));
        assert!(!is_experimental_version("0.8.5"));
        assert!(is_experimental_version("1.0-dev+build"));
        assert!(!is_experimental_version("1.0+build-dev"));
    }

    #[test]
    fn audio_words_cover_every_status() {
        assert_eq!(audio_status_words(None), "Live while this panel is open");
        let mk = |status, name: Option<&str>, tag: Option<u16>| AudioTestState {
            status,
            source: canvas_proto::audio::ResolvedAudioSource {
                requested: AudioSource::FollowDefault,
                endpoint_id: None,
                name: name.map(|s| s.to_string()),
                flow: None,
                fallback: None,
            },
            sample_rate: None,
            channels: None,
            bits_per_sample: None,
            format_tag: tag,
            peak: None,
            peak_db: None,
            bands: Vec::new(),
            packets: 0,
            silent_packets: 0,
        };
        assert_eq!(audio_status_words(Some(&mk(AudioTestStatus::Off, None, None))), "Live while this panel is open");
        assert_eq!(audio_status_words(Some(&mk(AudioTestStatus::Waiting, None, None))), "Waiting for the device…");
        assert_eq!(audio_status_words(Some(&mk(AudioTestStatus::NoDevice, None, None))), "No output device");
        assert_eq!(audio_status_words(Some(&mk(AudioTestStatus::Live, None, None))), "Live");
        assert_eq!(
            audio_status_words(Some(&mk(AudioTestStatus::Missing, Some("Mic"), None))),
            "Saved device not connected: using Mic"
        );
        // The name falls back to the default output.
        assert_eq!(
            audio_status_words(Some(&mk(AudioTestStatus::DefaultChanged, None, None))),
            "Windows default changed: now the default output"
        );
        assert_eq!(
            audio_status_words(Some(&mk(AudioTestStatus::UnsupportedFormat, None, Some(7)))),
            "Format not supported (tag 7)"
        );
        assert_eq!(
            audio_status_words(Some(&mk(AudioTestStatus::UnsupportedFormat, None, None))),
            "Format not supported (tag 0)"
        );
    }

    #[test]
    fn led_cells_match_the_meter_rule() {
        let cells = led_cells(Some(1.0));
        assert_eq!(cells.len(), 24);
        assert_eq!(cells[13], Some("ok"));
        assert_eq!(cells[14], Some("work"));
        assert_eq!(cells[19], Some("work"));
        assert_eq!(cells[20], Some("warn"));
        assert_eq!(cells[23], Some("warn"));
        assert_eq!(led_cells(Some(0.5))[11], Some("ok"));
        assert_eq!(led_cells(Some(0.5))[12], None);
        assert_eq!(led_cells(None)[0], None);
        assert_eq!(led_cells(Some(f32::NAN))[0], None);
    }

    #[test]
    fn fmt_checked_matches_the_words() {
        // 2026-10-29 09:14 UTC; today the same day.
        let today = parse_rfc3339_day("2026-10-29T00:00:00Z").unwrap().0;
        assert_eq!(fmt_checked(Some("2026-10-29T09:14:00Z"), today), "09:14");
        assert_eq!(fmt_checked(Some("2026-10-28T09:14:00Z"), today), "Oct 28, 09:14");
        assert_eq!(fmt_checked(None, today), "Never");
        assert_eq!(fmt_checked(Some("not a date"), today), "Never");
        // A known day count converts both ways.
        assert_eq!(civil_from_days(today), (2026, 10, 29));
    }

    #[test]
    fn settings_status_words_match_update_words() {
        assert_eq!(
            settings_status("checking", None, None, None),
            ("work".into(), "Checking for updates…".into())
        );
        assert_eq!(
            settings_status("downloading", None, None, None),
            ("work".into(), "Downloading ".into())
        );
        assert_eq!(
            settings_status("check_failed", Some("offline"), None, None),
            ("down".into(), "Check failed: offline".into())
        );
        assert_eq!(
            settings_status("check_failed", Some("rate_limited"), None, None),
            ("down".into(), "Check failed: GitHub rate limit".into())
        );
        assert_eq!(
            settings_status("available", None, Some("0.1.1"), Some("0.1.1")),
            ("warn".into(), "0.1.1 skipped".into())
        );
        assert_eq!(
            settings_status("idle", None, None, None),
            ("ok".into(), "Up to date".into())
        );
    }

    #[test]
    fn hsv_round_trips_through_hex() {
        for hex in ["#000000", "#ff0000", "#00ff00", "#0000ff", "#123456", "#abcdef"] {
            let (h, s, v) = hex_to_hsv(hex, 0.0);
            assert_eq!(hsv_to_hex(h, s, v), hex, "round trip {hex}");
        }
        assert_eq!(hex_to_hsv("#ff0000", 0.0), (0.0, 1.0, 1.0));
        assert_eq!(hex_to_hsv("#00ff00", 0.0), (120.0, 1.0, 1.0));
    }

    #[test]
    fn export_name_uses_the_utc_stamp() {
        assert_eq!(export_file_name((2026, 11, 6)), "canvas-rgb-config-2026-11-06.json");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
    }

    #[test]
    fn columns_follow_the_multicol_width() {
        assert_eq!(settings_columns(1672.0), 4);
        assert_eq!(settings_columns(1192.0), 3);
        assert_eq!(settings_columns(1168.0), 3);
        assert_eq!(settings_columns(912.0), 2);
        assert_eq!(settings_columns(848.0), 2);
        assert_eq!(settings_columns(358.0), 1);
    }

    #[test]
    fn distribute_fills_columns_in_order() {
        let heights = [166.0, 156.0, 204.0, 244.0, 204.0, 392.0];
        let cols = distribute(&heights, 3);
        assert_eq!(cols.len(), 3);
        let flat: Vec<usize> = cols.concat();
        assert_eq!(flat, vec![0, 1, 2, 3, 4, 5]);
        assert!(cols.iter().all(|c| !c.is_empty()));
        let one = distribute(&heights, 1);
        assert_eq!(one, vec![vec![0, 1, 2, 3, 4, 5]]);
    }

    #[test]
    fn fmt1_reads_the_hint_words() {
        assert_eq!(fmt1(Some(59.6)), "59.6");
        assert_eq!(fmt1(None), "—");
        assert_eq!(fmt1(Some(f32::NAN)), "—");
    }

    #[test]
    fn input_selection_reads_the_requested_pick() {
        let mk = |requested: AudioSource, flow: Option<AudioFlow>| AudioSources {
            endpoints: Vec::new(),
            current: canvas_proto::audio::ResolvedAudioSource {
                requested,
                endpoint_id: None,
                name: None,
                flow,
                fallback: None,
            },
        };
        assert!(!is_input_selection(None));
        assert!(is_input_selection(Some(&mk(
            AudioSource::Endpoint { id: "mic".into(), flow: AudioFlow::Capture, name_hint: "Mic".into() },
            Some(AudioFlow::Capture)
        ))));
        assert!(!is_input_selection(Some(&mk(
            AudioSource::Endpoint { id: "spk".into(), flow: AudioFlow::Render, name_hint: "Spk".into() },
            Some(AudioFlow::Render)
        ))));
        // Follow-default resolves through the current flow.
        assert!(is_input_selection(Some(&mk(AudioSource::FollowDefault, Some(AudioFlow::Capture)))));
        assert!(!is_input_selection(Some(&mk(AudioSource::FollowDefault, Some(AudioFlow::Render)))));
    }

    #[test]
    fn tray_state_maps_like_react_updates_panel() {
        use canvas_proto::tauri::{TrayPrefs, UpdateInfo};
        let mk_state = |version: &str, phase: UpdatePhase, available: Option<UpdateInfo>, skipped: Option<&str>| UpdateState {
            current_version: version.to_string(),
            auto_check: true,
            last_check: Some("2026-10-29T09:14:00Z".to_string()),
            phase,
            available,
            skipped_version: skipped.map(|s| s.to_string()),
            snoozed: false,
            updated_to: None,
        };
        let today = parse_rfc3339_day("2026-10-29T00:00:00Z").unwrap().0;
        // No tray: `canUpdate()` false, nothing acts.
        assert!(!tray_acts(false));
        assert!(tray_acts(true));
        // Stable version: no Editor row; dev version: Editor row.
        assert!(!editor_row_visible(Some(&mk_state("0.1.0", UpdatePhase::UpToDate, None, None))));
        assert!(editor_row_visible(Some(&mk_state("0.8.5-dev.12", UpdatePhase::UpToDate, None, None))));
        assert!(!editor_row_visible(None));
        // Check automatically reads on when the state is missing.
        assert!(auto_checked(None));
        assert!(auto_disabled(false, None));
        assert!(auto_disabled(true, None));
        // Last checked: dash without the tray or the state, the words with both.
        assert_eq!(last_checked_word(false, Some(&mk_state("0.1.0", UpdatePhase::UpToDate, None, None)), today), "—");
        assert_eq!(last_checked_word(true, None, today), "—");
        assert_eq!(last_checked_word(true, Some(&mk_state("0.1.0", UpdatePhase::UpToDate, None, None)), today), "09:14");
        // Check now disables while checking or installing.
        assert!(check_now_disabled(true, Some(&mk_state("0.1.0", UpdatePhase::Checking, None, None))));
        assert!(!check_now_disabled(true, Some(&mk_state("0.1.0", UpdatePhase::UpToDate, None, None))));
        // Status: the three tones and the offline words ride the spine phase.
        assert_eq!(
            tray_status(Some(&mk_state("0.1.0", UpdatePhase::Checking, None, None))),
            Some(("work".to_string(), "Checking for updates…".to_string()))
        );
        assert_eq!(
            tray_status(Some(&mk_state(
                "0.1.0",
                UpdatePhase::CheckFailed { reason: CheckFailReason::Offline },
                None,
                None
            ))),
            Some(("down".to_string(), "Check failed: offline".to_string()))
        );
        assert_eq!(tray_status(None), None);
        // Undo and What's new.
        let available = UpdateInfo { version: "0.1.1".into(), notes: "notes".into(), pub_date: None };
        let skipped = mk_state("0.1.0", UpdatePhase::Available, Some(available.clone()), Some("0.1.1"));
        assert!(undo_visible(Some(&skipped)));
        assert!(!whats_new_visible(Some(&skipped)));
        let fresh = mk_state("0.1.0", UpdatePhase::Available, Some(available), None);
        assert!(!undo_visible(Some(&fresh)));
        assert!(whats_new_visible(Some(&fresh)));
        // The Editor radio and the Experimental switch read the prefs.
        assert_eq!(editor_value(None), EditorChoice::React);
        assert_eq!(
            editor_value(Some(&TrayPrefs { editor: EditorChoice::Gpui, channel: UpdateChannel::Experimental })),
            EditorChoice::Gpui
        );
        assert!(experimental_checked(Some(&TrayPrefs { editor: EditorChoice::React, channel: UpdateChannel::Experimental })));
        assert!(!experimental_checked(None));
    }
}
