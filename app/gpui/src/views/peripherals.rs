//! `PeripheralsView`, after `app/web/src/views/PeripheralsView.tsx`
//! (`ControllerOutline`, `PropertyControls`) and `docs/gpui-parity/peripherals.md`.
//!
//! The port keeps the React view's rules: the device keys row, the device
//! head with the Read/Change switches, the staged-edits/Apply flow, the DPI
//! panel with its stage chips (including #229: `dpi-stage-count` starts with
//! `dpi-stage-`, so the React `others` filter drops it and the control never
//! renders; the port reproduces that absence), the controller panel with the
//! plan of the pad and its map list, the profile/section panels in schema
//! order, the RGB-rings note, and the loading/empty/error/refreshing frames.
//!
//! The React demo-mode states (`?state=...`) are demo-backend behavior and
//! have no GPUI counterpart: the GPUI editor always talks to a real (here,
//! fake) daemon over HTTP. Keyboard paths the GPUI components cannot serve
//! (slider arrows, number typing, listbox arrows, popover Escape) stay
//! unticked in the checklist, as they do for Displays. Tooltips (hover/focus
//! bubbles) are not drawn; their labels stay visible.
//!
//! Known React behavior reproduced here, not fixed: the DPI stage-count
//! control never renders (#229), and the daemon never sends a list battery,
//! so the battery badge is drawn only when one arrives.

use std::collections::HashMap;
use std::sync::Mutex;

use canvas_proto::peripheral::{PeripheralDetail, PeripheralInfo, PeripheralKind, ReadProgress, ReadStage};
use canvas_proto::props::{PropKind, PropValue, Props, PropertyDef};
use gpui::{Context, IntoElement, Window, canvas, div, point, prelude::*, px};
use serde::Deserialize;

use crate::client::DaemonClient;
use crate::components::button::{Variant, button, button_icon};
use crate::components::panel::{state_dot, tracked_title};
use crate::components::prop_controls::{control, control_words};
use crate::components::slider::Origin;
use crate::components::toggle::{toggle, toggle_disabled};
use crate::theme::{Theme, TokenRadius};

use super::capture_scroll;

// ---------------------------------------------------------------------------
// Logic, ported from PeripheralsView.tsx / PropertyControls.tsx /
// peripherals.ts / ControllerOutline.tsx and covered by the tests below.

/// `LOAD_DELAY_MS`.
pub const LOAD_DELAY_MS: u64 = 150;
/// `STAGE_POLL_MS`.
pub const STAGE_POLL_MS: u64 = 300;
/// The first stage poll after the read starts (100 ms in React).
pub const STAGE_FIRST_MS: u64 = 100;

/// Debug only (#208): `CANVAS_RGB_CAPTURE_PROBE=1` prints the layout bounds of
/// the elements wrapped with [`pr`] to stderr, once per change, so a capture
/// run reports where GPUI really put a box (React's are probed in headless
/// Chromium by `tools/ui-capture/probe-peripherals.mjs`). The same mechanism
/// as `views::settings`; release builds never wrap anything.
fn probe_on() -> bool {
    cfg!(debug_assertions) && std::env::var_os("CANVAS_RGB_CAPTURE_PROBE").is_some()
}

fn probe_report(name: &str, b: gpui::Bounds<gpui::Pixels>) {
    use std::sync::OnceLock;
    static SEEN: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let line = format!(
        "{:.2},{:.2} {:.2}x{:.2}",
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height)
    );
    let mut seen = SEEN.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if seen.get(name) != Some(&line) {
        let target = std::env::var("CANVAS_RGB_CAPTURE_PROBE").unwrap_or_default();
        if target.len() > 1 {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&target) {
                let _ = writeln!(f, "{name} {line}");
            }
        } else {
            eprintln!("probe {name} {line}");
        }
        seen.insert(name.to_string(), line);
    }
}

/// Wraps `el` so its bounds are reported when the probe is on.
fn pr(name: impl Into<String>, el: impl IntoElement) -> gpui::AnyElement {
    if !probe_on() {
        return el.into_any_element();
    }
    let name: String = name.into();
    div()
        .relative()
        .child(el)
        .child(
            gpui::canvas(move |bounds, _window, _cx| probe_report(&name, bounds), |_, _, _, _| {})
                .absolute()
                .inset_0(),
        )
        .into_any_element()
}

/// Debug only (#208): the line metrics GPUI lays the page's text sizes out
/// with, written beside the box probe when `CANVAS_RGB_CAPTURE_PROBE` names a
/// file. React's are measured headlessly by `tools/ui-capture/probe-text.mjs`;
/// the two settle why the 13.33, 14 and 20 px runs' ink sits one pixel higher
/// than the browser's while 15 px matches. It writes once and does nothing
/// without the variable.
fn line_metrics_probe(window: &gpui::Window) {
    use std::sync::OnceLock;
    static ONCE: OnceLock<()> = OnceLock::new();
    if ONCE.set(()).is_err() {
        return;
    }
    let target = std::env::var("CANVAS_RGB_CAPTURE_PROBE").unwrap_or_default();
    if target.len() < 2 {
        return;
    }
    // (size, weight, family, line height, the run's text): the runs the page
    // draws at 13.333, 14, 15 and 20 px.
    let styles: &[(f32, gpui::FontWeight, &str, f32, &str)] = &[
        (20.0, gpui::FontWeight::SEMIBOLD, crate::fonts::UI_FAMILY, 25.0, "8BitDo Ultimate 2 Wireless (XInput)"),
        (14.0, gpui::FontWeight::NORMAL, crate::fonts::UI_FAMILY, 20.3, "Game controller"),
        (15.0, gpui::FontWeight::NORMAL, crate::fonts::UI_FAMILY, 21.75, "Polling rate (Hz)"),
        (13.333, gpui::FontWeight::SEMIBOLD, crate::fonts::UI_FAMILY, 19.333, "8BitDo Ultimate"),
        (13.333, gpui::FontWeight::NORMAL, crate::fonts::MONO_FAMILY, 19.333, "26000"),
    ];
    let mut out = String::new();
    for (size, weight, family, line_height, text) in styles {
        let font = gpui::Font {
            family: family.to_string().into(),
            weight: *weight,
            style: gpui::FontStyle::Normal,
            features: crate::fonts::kerning(),
            fallbacks: None,
        };
        let run = gpui::TextRun {
            len: text.len(),
            font,
            color: gpui::black(),
            underline: None,
            strikethrough: None,
            background_color: None,
        };
        let layout = window.text_system().layout_line(text, px(*size), &[run], None);
        let ascent = f32::from(layout.ascent);
        let descent = f32::from(layout.descent);
        // `paint_line`'s baseline: half the line's spare height over the
        // font's ascent (`gpui::text_system::line::paint_line`).
        let baseline = (line_height - ascent - descent) / 2.0 + ascent;
        out.push_str(&format!(
            "text size={size} weight={} family={family} lh={line_height} ascent={ascent:.4} descent={descent:.4} baseline={baseline:.4} w={:.4} text=\"{text}\"\n",
            weight.0.round() as u16,
            f32::from(layout.width)
        ));
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&target) {
        let _ = f.write_all(out.as_bytes());
    }
}

/// A peripheral as the list shows it. The spine's `PeripheralInfo` has no
/// battery field; the React row carries it as an extra the daemon may add.
/// The port keeps the same shape: when a battery arrives it is drawn.
#[derive(Debug, Clone, Deserialize)]
pub struct PeripheralRow {
    #[serde(flatten)]
    pub info: PeripheralInfo,
    #[serde(default)]
    pub battery: Option<u8>,
    #[serde(default)]
    pub charging: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ListResponse {
    peripherals: Vec<PeripheralRow>,
}

/// `plainLabel` (`PeripheralsView.tsx:42-44`).
pub fn plain_label(label: &str) -> String {
    let mut s = label.to_string();
    if let Some(idx) = s.rfind(" (profile ") {
        if s.ends_with(')') {
            s.truncate(idx);
            return s;
        }
    }
    if let Some(idx) = s.rfind(", profile ") {
        if s.ends_with(')') {
            s.truncate(idx);
            s.push(')');
            return s;
        }
    }
    s
}

/// The words for a kind (`peripherals.ts` `kindWord`).
pub fn kind_word(kind: PeripheralKind) -> &'static str {
    match kind {
        PeripheralKind::Mouse => "Mouse",
        PeripheralKind::GameController => "Game controller",
        PeripheralKind::Keyboard => "Keyboard",
    }
}

/// What a peripheral is called in a sentence about reading it (`nounOf`).
pub fn noun_of(kind: PeripheralKind) -> &'static str {
    match kind {
        PeripheralKind::Mouse => "mouse",
        PeripheralKind::GameController => "controller",
        PeripheralKind::Keyboard => "keyboard",
    }
}

/// The words for a choice id (`choiceWord`). A known id reads as its word;
/// an unknown hyphenated id opens its hyphens and capitalises the first
/// letter; anything else reads as itself.
pub fn choice_word(id: &str) -> String {
    const WORDS: &[(&str, &str)] = &[
        ("none", "None"),
        ("start", "Start"),
        ("select", "Select"),
        ("menu", "Menu / Share"),
        ("home", "Home"),
        ("screenshot", "Screenshot"),
        ("swap", "Swap"),
        ("other", "Other"),
        ("a", "A"),
        ("b", "B"),
        ("x", "X"),
        ("y", "Y"),
        ("left-stick-click", "Left stick click"),
        ("right-stick-click", "Right stick click"),
        ("left-bumper", "Left bumper (LB)"),
        ("right-bumper", "Right bumper (RB)"),
        ("left-trigger", "Left trigger"),
        ("right-trigger", "Right trigger"),
        ("dpad-up", "D-pad up"),
        ("dpad-down", "D-pad down"),
        ("dpad-left", "D-pad left"),
        ("dpad-right", "D-pad right"),
        ("paddle-1", "Paddle 1 (P1)"),
        ("paddle-2", "Paddle 2 (P2)"),
        ("paddle-3", "Paddle 3 (P3)"),
        ("paddle-4", "Paddle 4 (P4)"),
        ("turbo-single", "Turbo (single)"),
        ("turbo-auto", "Turbo (auto)"),
        ("left-click", "Left click"),
        ("right-click", "Right click"),
        ("middle-click", "Middle click"),
        ("dpi-cycle", "DPI cycle"),
        ("forward", "Forward"),
        ("back", "Back"),
        ("static", "Static"),
        ("breathing", "Breathing"),
        ("neon", "Neon"),
        ("off", "Off"),
        ("color-breathing", "Colour breathing"),
        ("static-dpi", "Static (DPI colour)"),
        ("breathing-dpi", "Breathing (DPI colour)"),
    ];
    for (key, word) in WORDS {
        if *key == id {
            return (*word).to_string();
        }
    }
    if id.contains('-') && id.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        let text = id.replace('-', " ");
        let mut chars = text.chars();
        return match chars.next() {
            Some(first) => first.to_uppercase().to_string() + chars.as_str(),
            None => text,
        };
    }
    id.to_string()
}

/// The words for a read stage (`readStage`): (label, done, total).
pub fn read_stage(progress: Option<&ReadProgress>, noun: &str) -> (String, u32, u32) {
    let stage = progress.map(|p| p.stage).unwrap_or(ReadStage::Idle);
    match stage {
        ReadStage::Opening => (format!("Opening the {noun}"), 0, 0),
        ReadStage::Waking => (format!("Waking the {noun}"), 0, 0),
        ReadStage::Reading => match progress {
            Some(p) if p.total > 0 => ("Reading settings".to_string(), p.done, p.total),
            _ => ("Reading settings".to_string(), 0, 0),
        },
        ReadStage::Idle => ("Reading settings".to_string(), 0, 0),
    }
}

/// The stage as one line (`stageLine`).
pub fn stage_line(label: &str, done: u32, total: u32) -> String {
    if total > 0 {
        format!("{label} · {done} of {total}")
    } else {
        label.to_string()
    }
}

/// `sameValue` (`PeripheralsView.tsx:69-73`).
pub fn same_value(a: Option<&PropValue>, b: &PropValue) -> bool {
    match a {
        None => false,
        Some(a) => std::mem::discriminant(a) == std::mem::discriminant(b) && a == b,
    }
}

/// `keepLive` (`PeripheralsView.tsx:85-88`): the entries of `prev` whose
/// device key is in `live`.
pub fn keep_live<T: Clone>(prev: &HashMap<String, T>, live: &std::collections::HashSet<String>) -> HashMap<String, T> {
    prev.iter().filter(|(id, _)| live.contains(*id)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// The property-control rules this view shares with the Effects page
/// (moved into `components/prop_controls`).
#[allow(unused_imports)]
pub use crate::components::prop_controls::{
    angle_now, angle_step, colour_swatches, decimals_of, default_for, format_number, hex_to_hsv,
    hsv_to_hex, hue_step, is_hex, parse_hex, quantise_step, wrap_deg,
};

/// The value a key holds, plain (`readStage`/`DpiPanel` `plain`): the number
/// or choice text, or None when absent.
pub fn plain_number(values: &Props, key: &str) -> Option<f64> {
    match values.get(key) {
        Some(PropValue::Number(n)) => Some(*n),
        Some(PropValue::Hue(n)) => Some(*n),
        Some(PropValue::Angle(n)) => Some(*n),
        _ => None,
    }
}

/// The choice text a key holds, or None.
pub fn plain_choice(values: &Props, key: &str) -> Option<String> {
    match values.get(key) {
        Some(PropValue::Choice(s)) => Some(s.clone()),
        Some(PropValue::Color(c)) => Some(c.to_hex()),
        Some(PropValue::Number(n)) => Some(n.to_string()),
        Some(PropValue::Bool(b)) => Some(b.to_string()),
        Some(PropValue::Hue(n)) => Some(n.to_string()),
        Some(PropValue::Angle(n)) => Some(n.to_string()),
        None => None,
    }
}

/// The DPI stage count (`DpiPanel` `count`): the `dpi-stage-count` value or
/// 6 when absent.
pub fn dpi_stage_count(values: &Props) -> usize {
    plain_number(values, "dpi-stage-count").map(|n| n as usize).unwrap_or(6)
}

/// The active DPI stage (`DpiPanel` `active`): the `dpi-active-stage` value
/// or "1" when absent.
pub fn dpi_active_stage(values: &Props) -> String {
    plain_choice(values, "dpi-active-stage").unwrap_or_else(|| "1".to_string())
}

/// One DPI stage chip: its number, its DPI and its colour.
#[derive(Debug, Clone, PartialEq)]
pub struct DpiStage {
    pub n: usize,
    pub dpi: f64,
    pub colour: String,
}

/// The stage chips (`DpiPanel` stages): `count` stages, each with its DPI or
/// 0 and its colour or `#ffffff` when the value is absent.
pub fn dpi_stages(values: &Props) -> Vec<DpiStage> {
    let count = dpi_stage_count(values);
    (1..=count)
        .map(|n| DpiStage {
            n,
            dpi: plain_number(values, &format!("dpi-stage-{n}")).unwrap_or(0.0),
            colour: plain_choice(values, &format!("dpi-stage-{n}-colour")).unwrap_or_else(|| "#ffffff".to_string()),
        })
        .collect()
}

/// The `others` of a DPI section (`PeripheralsView.tsx:758`): every key that
/// does not start with `dpi-stage-`. This is the #229 behaviour: the filter
/// drops `dpi-stage-count` too, so the stage-count control never renders.
pub fn dpi_others(properties: &[PropertyDef]) -> Vec<&PropertyDef> {
    properties.iter().filter(|p| !p.key.starts_with("dpi-stage-")).collect()
}

/// The picked stage's two defs (`DpiPanel` `pickedDefs`).
pub fn dpi_picked_defs(properties: &[PropertyDef], picked: Option<usize>) -> Vec<&PropertyDef> {
    match picked {
        None => Vec::new(),
        Some(n) => properties
            .iter()
            .filter(|p| p.key == format!("dpi-stage-{n}") || p.key == format!("dpi-stage-{n}-colour"))
            .collect(),
    }
}

/// The active profile (`PeripheralsView.tsx:629` and `ControllerPanel`): the
/// `active-profile` value or "1" when absent.
pub fn active_profile(values: &Props) -> String {
    plain_choice(values, "active-profile").unwrap_or_else(|| "1".to_string())
}

/// The changed keys: staged entries whose value differs from what the device
/// holds (`changed`, `PeripheralsView.tsx:326-332`).
pub fn changed_props(staged: &Props, held: &Props) -> Props {
    staged
        .iter()
        .filter(|(key, value)| !same_value(held.get(*key), value))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// The staged edits left after a successful Apply (`apply()`): the sent
/// values are gone, an edit made during the send stays.
pub fn stale_after_apply(staged: &Props, sent: &Props) -> Props {
    staged
        .iter()
        .filter(|(key, value)| !(sent.contains_key(*key) && same_value(Some(value), &sent[*key])))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Whether `section` is drawn (`PeripheralsView.tsx:628-631`): a controller
/// `profile-N` section shows only when N is the active profile.
pub fn section_visible(section_id: &str, kind: PeripheralKind, values: &Props) -> bool {
    if kind == PeripheralKind::GameController && section_id.starts_with("profile-") {
        let active = active_profile(values);
        return section_id == format!("profile-{active}");
    }
    true
}

/// The CSS multicol column count for a content width: `.rd-peri-sections`
/// is `columns: 280px auto; column-gap: 16px` (`app.css:5686-5689`); on a
/// phone the 639 container rule stacks them (`columns: auto`).
pub fn section_columns(content_w: f32) -> usize {
    ((content_w + 16.0) / 296.0).floor().max(1.0) as usize
}

/// Greedy multicol fill with a height budget (the balancing step of CSS
/// `column-fill: balance`): the smallest budget that fits every panel into
/// at most `cols` columns, then the sequential fill. Returns, per column,
/// panel indices (the same shape as `views::settings::distribute`).
pub fn distribute(heights: &[f32], cols: usize) -> Vec<Vec<usize>> {
    if cols <= 1 {
        return vec![(0..heights.len()).collect()];
    }
    let total: f32 = heights.iter().sum();
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
    true
}

/// One row height inside a section panel: number rows and plain rows are 52
/// (`min-height: 52px`), a stacked segmented row is the label line over the
/// control (measured 79 px at 1440), and on a phone every row is 44. A label
/// that wraps to two lines in the 116 px column makes its row 60/61 (measured
/// at 1440: `Key response time (ms)` and `Deep sleep after (min)` give 60,
/// `Left stick range start (%)` gives 61).
fn row_height(kind: &PropKind, narrow: bool, label: &str) -> f32 {
    let wrapped = label.chars().count() > 18;
    let tall = if wrapped { 61.0 } else { 52.0 };
    if narrow {
        return match kind {
            PropKind::Choice { options, .. } if options.len() <= 4 => 79.0,
            PropKind::Number { .. } => 82.0,
            _ => 44.0,
        };
    }
    match kind {
        PropKind::Choice { options, .. } if options.len() <= 4 => 79.0,
        _ => tall,
    }
}

/// Estimated panel height for the multicol balance: 32 px of panel padding
/// (p_4) plus the tracked title (19 px) and its 12 px bottom margin, the
/// rows, and the 16 px bottom margin the multicol adds between panels
/// (counted on every panel but the last would change nothing about the
/// fill). Calibrated to React's measured panel heights at 1440 phone-free
/// (DPI 211, Performance 201, Lighting 270, Power 174, Buttons 260,
/// Profile 114).
pub fn section_panel_height(properties: &[PropertyDef], buttons_pair: bool, narrow: bool) -> f32 {
    let pad = 32.0;
    let head = 31.0;
    let body = if buttons_pair {
        // Two columns of stacked key/value pairs, three rows tall
        // (`grid-auto-flow: column`), each pair 66 px.
        properties.len().div_ceil(2) as f32 * 66.0
    } else {
        properties.iter().map(|def| row_height(&def.kind, narrow, &def.label)).sum()
    };
    pad + head + body + 16.0
}

/// Estimated height of the DPI panel: its remaining rows plus the stage-chip
/// block (two rows of three chips at 248 px inner width, 32 px each with an
/// 8 px gap, plus 12 px top margin and 12 px top padding: 96 px measured).
pub fn dpi_panel_height(properties: &[PropertyDef], narrow: bool) -> f32 {
    let others: Vec<PropertyDef> = dpi_others(properties).iter().map(|p| (*p).clone()).collect();
    let pad = 32.0;
    let head = 31.0;
    let body: f32 = others.iter().map(|def| row_height(&def.kind, narrow, &def.label)).sum();
    let chips = if narrow {
        // Four chips a row at 332 px inner width: two rows, same 96 px block.
        96.0
    } else {
        96.0
    };
    pad + head + body + chips + 16.0
}

/// The body path (`ControllerOutline.tsx` BODY), in the 340x244 space.
pub const BODY_PATH: &str = "M 78 58 L 262 58 C 286 58 304 70 312 96 C 322 128 334 168 334 192 C 334 214 322 236 306 236 C 290 236 282 220 276 208 C 270 200 262 196 244 196 L 96 196 C 78 196 70 200 64 208 C 58 220 50 236 34 236 C 18 236 6 214 6 192 C 6 168 18 128 28 96 C 36 70 54 58 78 58 Z";

/// The D-pad cross (`DPAD`), in the 340x244 space.
pub const DPAD_PATH: &str = "M 105 126 L 131 126 L 131 139 L 157 139 L 157 165 L 131 165 L 131 178 L 105 178 L 105 165 L 79 165 L 79 139 L 105 139 Z";

/// The front view's box, in drawing units.
pub const FRONT_W: f32 = 340.0;
/// The rear view's box, in drawing units.
#[allow(dead_code)]
pub const REAR_W: f32 = 200.0;
/// The rear view's body transform (`translate(15 2) scale(0.5)`).
pub const REAR_TX: f32 = 15.0;
/// The rear view's body transform.
pub const REAR_TY: f32 = 2.0;
/// The rear view's body transform.
pub const REAR_SCALE: f32 = 0.5;
/// The minimum target hit area, in drawing units (`Math.max(spot.w, 34)`).
pub const MIN_HIT: f32 = 34.0;

/// One button spot on the plan (`Spot`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub round: bool,
}

/// The mark a spot prints: letters, or a glyph.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    Text(String),
    Glyph(&'static str),
}

/// One front-view spot with its mark.
#[derive(Debug, Clone)]
pub struct PlacedSpot {
    pub spot: Spot,
    pub mark: Mark,
}

/// The mirror of a spot across the centre line (`mirror`).
pub fn mirror(spot: Spot, mark: Mark) -> PlacedSpot {
    PlacedSpot { spot: Spot { cx: FRONT_W - spot.cx, cy: spot.cy, w: spot.w, h: spot.h, round: spot.round }, mark }
}

fn text_spot(cx: f32, cy: f32, w: f32, h: f32, round: bool, text: &str) -> PlacedSpot {
    PlacedSpot { spot: Spot { cx, cy, w, h, round }, mark: Mark::Text(text.to_string()) }
}

fn glyph_spot(cx: f32, cy: f32, w: f32, h: f32, glyph: &'static str) -> PlacedSpot {
    PlacedSpot { spot: Spot { cx, cy, w, h, round: true }, mark: Mark::Glyph(glyph) }
}

/// Where each button of the front view sits (`FRONT`).
pub fn front_spot(place: &str) -> Option<PlacedSpot> {
    let l2 = Spot { cx: 78.0, cy: 17.0, w: 60.0, h: 24.0, round: false };
    let l1 = Spot { cx: 80.0, cy: 51.0, w: 60.0, h: 24.0, round: false };
    let p3 = Spot { cx: 26.0, cy: 60.0, w: 34.0, h: 26.0, round: false };
    let l3 = Spot { cx: 72.0, cy: 100.0, w: 44.0, h: 44.0, round: true };
    match place {
        "l2" => Some(PlacedSpot { spot: l2, mark: Mark::Text("L2".into()) }),
        "r2" => Some(mirror(l2, Mark::Text("R2".into()))),
        "l1" => Some(PlacedSpot { spot: l1, mark: Mark::Text("L1".into()) }),
        "r1" => Some(mirror(l1, Mark::Text("R1".into()))),
        "paddle-3" => Some(PlacedSpot { spot: p3, mark: Mark::Text("P3".into()) }),
        "paddle-4" => Some(mirror(p3, Mark::Text("P4".into()))),
        "l3" => Some(PlacedSpot { spot: l3, mark: Mark::Text("L3".into()) }),
        "r3" => Some(text_spot(222.0, 152.0, 44.0, 44.0, true, "R3")),
        "select" => Some(glyph_spot(132.0, 86.0, 26.0, 26.0, "select")),
        "home" => Some(glyph_spot(170.0, 86.0, 30.0, 30.0, "home")),
        "start" => Some(glyph_spot(208.0, 86.0, 26.0, 26.0, "start")),
        "menu" => Some(glyph_spot(170.0, 128.0, 24.0, 24.0, "menu")),
        "dpad-up" => Some(glyph_spot(118.0, 126.0, 26.0, 26.0, "up")),
        "dpad-down" => Some(glyph_spot(118.0, 178.0, 26.0, 26.0, "down")),
        "dpad-left" => Some(glyph_spot(92.0, 152.0, 26.0, 26.0, "left")),
        "dpad-right" => Some(glyph_spot(144.0, 152.0, 26.0, 26.0, "right")),
        "y" => Some(text_spot(268.0, 80.0, 28.0, 28.0, true, "Y")),
        "x" => Some(text_spot(242.0, 106.0, 28.0, 28.0, true, "X")),
        "b" => Some(text_spot(294.0, 106.0, 28.0, 28.0, true, "B")),
        "a" => Some(text_spot(268.0, 132.0, 28.0, 28.0, true, "A")),
        _ => None,
    }
}

/// The two paddles of the rear view (`REAR`).
pub fn rear_spot(place: &str) -> Option<PlacedSpot> {
    match place {
        "paddle-1" => Some(text_spot(42.0, 82.0, 32.0, 46.0, false, "P1")),
        "paddle-2" => Some(text_spot(158.0, 82.0, 32.0, 46.0, false, "P2")),
        _ => None,
    }
}

/// The name a key carries on the plan (`placeOf`).
pub fn place_of(key: &str) -> String {
    if let Some(stripped) = key.strip_prefix("button-") {
        return stripped.to_string();
    }
    if let Some(idx) = key.rfind("-paddle-") {
        return key[idx + 1..].to_string();
    }
    key.to_string()
}

/// The hit rect (`Math.max(spot.w, 34)`): (x, y, width, height).
pub fn hit_rect(spot: &Spot) -> (f32, f32, f32, f32) {
    let hw = spot.w.max(MIN_HIT);
    let hh = spot.h.max(MIN_HIT);
    (spot.cx - hw / 2.0, spot.cy - hh / 2.0, hw, hh)
}

/// The page title context (`PageTitle` context): `"{n} found"`, with 0 while
/// the list is still loading (`rows = list ?? []`).
pub fn found_context(count: Option<usize>) -> String {
    format!("{} found", count.unwrap_or(0))
}

/// The list frame's state (`listState`, `PeripheralsView.tsx:452-453`).
///
/// `None` until the list answers; `Some(err)` while the list failed and no
/// retry runs; `Some(rows)` afterwards.
pub fn list_state(list: Option<&Vec<PeripheralRow>>, list_error: Option<&str>, list_busy: bool) -> &'static str {
    if list_error.is_some() && !list_busy {
        "error"
    } else if list.is_none() {
        "loading"
    } else if list.is_some_and(|rows| rows.is_empty()) {
        "empty"
    } else {
        "ready"
    }
}

/// The detail frame's state (`detailState`, `PeripheralsView.tsx:404`).
pub fn detail_state(detail: Option<&PeripheralDetail>, detail_error: Option<&str>) -> &'static str {
    if detail_error.is_some() {
        "error"
    } else if detail.is_some() {
        "ready"
    } else {
        "loading"
    }
}

/// Whether there is no page for the detail (`noDetail`, `PeripheralsView.tsx:407`).
pub fn no_detail(list: Option<&Vec<PeripheralRow>>, list_error: Option<&str>) -> bool {
    list_error.is_some() || list.is_some_and(|rows| rows.is_empty())
}

/// The selected id (`selectedId`, `PeripheralsView.tsx:160`): the pick, or the
/// first listed device.
pub fn selected_id(picked: Option<&str>, list: Option<&Vec<PeripheralRow>>) -> Option<String> {
    picked.map(String::from).or_else(|| list.and_then(|rows| rows.first()).map(|r| r.info.id.0.clone()))
}

// ---------------------------------------------------------------------------
// The view.

pub struct PeripheralsView {
    pub theme: Theme,
    pub list: Option<Vec<PeripheralRow>>,
    pub list_error: Option<String>,
    pub list_busy: bool,
    pub list_loading_shown: bool,
    pub detail_loading_shown: bool,
    pub refreshing_shown: bool,
    pub picked_id: Option<String>,
    pub loaded_id: Option<String>,
    pub loaded_detail: Option<PeripheralDetail>,
    pub detail_error: Option<String>,
    pub change_saved: bool,
    pub staged_by: HashMap<String, Props>,
    pub saving: bool,
    pub save_errors: HashMap<String, String>,
    pub picked_stage: Option<usize>,
    pub picked_button: Option<String>,
    pub reloading_id: Option<String>,
    pub detail_request: u64,
    pub rereading: bool,
    pub edit_count: HashMap<String, u64>,
    pub edits_at_failure: HashMap<String, u64>,
    pub failures: HashMap<String, u64>,
    pub progress: Option<ReadProgress>,
    pub progress_id: Option<String>,
    pub progress_generation: u64,
    pub color_open: Option<String>,
    pub select_open: HashMap<String, bool>,
    pub toast: Option<Toast>,
    pub width: f32,
}

/// The last read of each device in this session (React `lastRead`).
static LAST_READ: std::sync::Mutex<Option<HashMap<String, PeripheralDetail>>> =
    std::sync::Mutex::new(None);

fn last_read_get(id: &str) -> Option<PeripheralDetail> {
    LAST_READ.lock().ok().and_then(|guard| guard.as_ref().and_then(|m| m.get(id).cloned()))
}

fn last_read_set(id: &str, detail: PeripheralDetail) {
    if let Ok(mut guard) = LAST_READ.lock() {
        guard.get_or_insert_with(HashMap::new).insert(id.to_string(), detail);
    }
}

impl PeripheralsView {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        let mut view = PeripheralsView {
            theme,
            list: None,
            list_error: None,
            list_busy: false,
            list_loading_shown: false,
            detail_loading_shown: false,
            refreshing_shown: false,
            picked_id: None,
            loaded_id: None,
            loaded_detail: None,
            detail_error: None,
            change_saved: false,
            staged_by: HashMap::new(),
            saving: false,
            save_errors: HashMap::new(),
            picked_stage: None,
            picked_button: None,
            reloading_id: None,
            detail_request: 0,
            rereading: false,
            edit_count: HashMap::new(),
            edits_at_failure: HashMap::new(),
            failures: HashMap::new(),
            progress: None,
            progress_id: None,
            progress_generation: 0,
            color_open: None,
            select_open: HashMap::new(),
            toast: None,
            width: 1280.0,
        };
        view.load_list(cx);
        view.start_load_delay(cx);
        view
    }

    fn client() -> DaemonClient {
        DaemonClient::new()
    }

    fn edits_of(&self, id: &str) -> u64 {
        self.edit_count.get(id).copied().unwrap_or(0)
    }

    fn failures_of(&self, id: &str) -> u64 {
        self.failures.get(id).copied().unwrap_or(0)
    }

    fn keeps_staged(&self, id: &str) -> bool {
        self.edits_of(id) > self.edits_at_failure.get(id).copied().unwrap_or(0)
    }

    fn note_failure(&mut self, id: &str, carried_edits: bool) {
        if carried_edits {
            self.edits_at_failure.insert(id.to_string(), self.edits_of(id));
        }
        let next = self.failures_of(id) + 1;
        self.failures.insert(id.to_string(), next);
    }

    /// The selected id (`selectedId`).
    pub fn sel_id(&self) -> Option<String> {
        selected_id(self.picked_id.as_deref(), self.list.as_ref())
    }

    /// How far the keys row is scrolled sideways so the picked key is in
    /// view. React's keys row is a scroller (`.rd-peri-keys`
    /// `overflow-x: auto`), and clicking a key focuses it, so the browser
    /// scrolls it into view: at 390 the controller key shows with its left
    /// edge at 78 px, i.e. the row sits 122 px along. The scrollport is the
    /// row's padding box (4 px of padding each side against the -4 px
    /// margin), so a key counts as in view while 8 px of it hang past the
    /// content edge. GPUI has no focus scrolling, so the offset is computed
    /// from the keys' widths (the same 78 px of frame around the measured
    /// name).
    pub fn keys_scroll(&self, window: &Window, content_w: f32) -> f32 {
        let Some(sel) = self.sel_id() else { return 0.0 };
        let Some(rows) = self.list.as_ref() else { return 0.0 };
        let mut x = 0.0f32;
        for row in rows {
            let w = Self::key_width(window, row);
            if row.info.id.0 == sel {
                return (x + w - (content_w + 8.0)).max(0.0);
            }
            x += w + 8.0;
        }
        0.0
    }

    /// The device whose detail is on screen (`fresh` or the last read).
    pub fn detail(&self) -> Option<PeripheralDetail> {
        if let (Some(id), Some(detail)) = (&self.loaded_id, &self.loaded_detail) {
            if Some(id) == self.sel_id().as_ref() {
                return Some(detail.clone());
            }
        }
        self.sel_id().and_then(|id| last_read_get(&id))
    }

    /// The staged edits for the selected device.
    pub fn staged(&self) -> Props {
        self.sel_id()
            .and_then(|id| self.staged_by.get(&id).cloned())
            .unwrap_or_default()
    }

    /// The values the controls show: held values with staged edits on top.
    pub fn values(&self) -> Props {
        let mut out = self.detail().map(|d| d.values.clone()).unwrap_or_default();
        for (k, v) in self.staged() {
            out.insert(k, v);
        }
        out
    }

    /// The changed keys (`changed`).
    pub fn changed(&self) -> Props {
        let staged = self.staged();
        let held = self.detail().map(|d| d.values.clone()).unwrap_or_default();
        changed_props(&staged, &held)
    }

    /// The pending count (`changedCount`).
    pub fn pending_count(&self) -> usize {
        self.changed().len()
    }

    /// The selected device's failed-write message, if any (`saveError`).
    pub fn save_error(&self) -> Option<String> {
        self.sel_id().and_then(|id| self.save_errors.get(&id).cloned())
    }

    /// Whether the device is being read (`reading`).
    pub fn reading(&self) -> bool {
        let Some(id) = self.sel_id() else {
            return false;
        };
        if self.detail_error.is_some() {
            return false;
        }
        let fresh = self.loaded_id.as_ref() == Some(&id) && self.loaded_detail.is_some();
        let reloading = self.reloading_id.as_ref() == Some(&id);
        !fresh || reloading
    }

    /// The strip context (`PageTitle` context): `"{n} found"`, with 0 while
    /// the list is still loading (`rows = list ?? []`).
    pub fn strip_context_text(&self) -> String {
        found_context(self.list.as_ref().map(|r| r.len()))
    }

    fn start_load_delay(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(LOAD_DELAY_MS)).await;
            let _ = view.update(cx, |this, cx| {
                this.list_loading_shown = this.list.is_none();
                this.detail_loading_shown = this.detail().is_none() && this.detail_error.is_none();
                this.refreshing_shown = this.detail().is_some() && this.reading();
                cx.notify();
            });
        })
        .detach();
    }

    /// `GET /api/v1/peripherals` (`loadList`).
    fn load_list(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.get("/api/v1/peripherals")
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.list_busy = false;
                match result {
                    Ok(body) => match serde_json::from_str::<ListResponse>(&body) {
                        Ok(parsed) => {
                            let live: std::collections::HashSet<String> =
                                parsed.peripherals.iter().map(|r| r.info.id.0.clone()).collect();
                            this.staged_by = keep_live(&this.staged_by, &live);
                            this.save_errors = keep_live(&this.save_errors, &live);
                            this.list = Some(parsed.peripherals);
                            this.list_error = None;
                            // The `useEffect` on `selectedId` (`PeripheralsView.tsx`
                            // `:285-318`): the first read fires as soon as the list
                            // lands, and again when the selected device changes.
                            // A pick that arrived before the list (a capture's
                            // `--peri-device`) already read its device, so only
                            // load when the selected device has no detail held.
                            let needs_read = match this.sel_id() {
                                Some(id) => {
                                    this.detail().is_none() || this.loaded_id.as_deref() != Some(id.as_str())
                                }
                                None => false,
                            };
                            if needs_read {
                                if let Some(id) = this.sel_id() {
                                    this.load_selected(&id, cx);
                                }
                            }
                            this.start_progress_poll(cx);
                        }
                        Err(e) => {
                            this.list_error = Some(e.to_string());
                        }
                    },
                    Err(e) => {
                        this.list_error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Try again" (`retryList`).
    fn retry_list(&mut self, cx: &mut Context<Self>) {
        if self.list_busy {
            return;
        }
        self.list_error = None;
        self.list_busy = true;
        self.load_list(cx);
        cx.notify();
    }

    /// Asks for one device's detail again (`loadDetail`). `landed` says the
    /// change is already on the device; `force_read` asks for a fresh read.
    fn load_detail(&mut self, id: &str, landed: bool, force_read: bool, cx: &mut Context<Self>) {
        if force_read {
            if self.rereading {
                return;
            }
            self.rereading = true;
        }
        self.detail_request += 1;
        let n = self.detail_request;
        let failed = self.failures_of(id);
        self.reloading_id = Some(id.to_string());
        let id_owned = id.to_string();
        let id_for_bg = id_owned.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let path = if force_read {
                format!("/api/v1/peripherals/{id_for_bg}?fresh=true")
            } else {
                format!("/api/v1/peripherals/{id_for_bg}")
            };
            client.get(&path)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                if force_read {
                    this.rereading = false;
                }
                if n != this.detail_request {
                    return;
                }
                this.reloading_id = None;
                match result {
                    Ok(body) => match serde_json::from_str::<PeripheralDetail>(&body) {
                        Ok(detail) => {
                            last_read_set(&id_owned, detail.clone());
                            this.loaded_id = Some(id_owned.clone());
                            this.loaded_detail = Some(detail);
                            if !this.keeps_staged(&id_owned) {
                                this.staged_by.remove(&id_owned);
                            }
                            if force_read && this.failures_of(&id_owned) == failed {
                                this.save_errors.remove(&id_owned);
                            }
                            this.detail_error = None;
                            this.change_saved = false;
                        }
                        Err(e) => {
                            this.loaded_id = None;
                            this.loaded_detail = None;
                            this.detail_error = Some(e.to_string());
                            this.change_saved = landed;
                        }
                    },
                    Err(e) => {
                        this.loaded_id = None;
                        this.loaded_detail = None;
                        this.detail_error = Some(e.to_string());
                        this.change_saved = landed;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The read of a newly picked device (the `useEffect` on `selectedId`).
    fn load_selected(&mut self, id: &str, cx: &mut Context<Self>) {
        self.detail_request += 1;
        let n = self.detail_request;
        let id_owned = id.to_string();
        let id_for_bg = id_owned.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.get(&format!("/api/v1/peripherals/{id_for_bg}"))
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                if n != this.detail_request {
                    return;
                }
                this.reloading_id = None;
                match result {
                    Ok(body) => match serde_json::from_str::<PeripheralDetail>(&body) {
                        Ok(detail) => {
                            last_read_set(&id_owned, detail.clone());
                            this.loaded_id = Some(id_owned.clone());
                            this.loaded_detail = Some(detail);
                            if !this.keeps_staged(&id_owned) {
                                this.staged_by.remove(&id_owned);
                            }
                            this.detail_error = None;
                            this.change_saved = false;
                        }
                        Err(e) => {
                            this.loaded_id = None;
                            this.loaded_detail = None;
                            this.detail_error = Some(e.to_string());
                            this.change_saved = false;
                        }
                    },
                    Err(e) => {
                        this.loaded_id = None;
                        this.loaded_detail = None;
                        this.detail_error = Some(e.to_string());
                        this.change_saved = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Select a device key (`onClick` of `button.peripheral-row`).
    pub fn pick_device(&mut self, id: &str, cx: &mut Context<Self>) {
        self.detail_error = None;
        self.change_saved = false;
        self.picked_stage = None;
        self.picked_button = None;
        self.progress = None;
        self.progress_id = None;
        self.picked_id = Some(id.to_string());
        self.load_selected(id, cx);
        self.start_progress_poll(cx);
        cx.notify();
    }

    /// The stage poll (`useReadProgress`): every 300 ms while `reading`,
    /// first poll at 100 ms, failures ignored.
    fn start_progress_poll(&mut self, cx: &mut Context<Self>) {
        self.progress_generation += 1;
        let generation = self.progress_generation;
        let id = self.sel_id();
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(STAGE_FIRST_MS)).await;
            loop {
                let current = view.update(cx, |this, _| (this.sel_id(), this.reading())).unwrap_or((None, false));
                if current != (id.clone(), true) {
                    return;
                }
                if let Some(id) = &id {
                    let id_owned = id.clone();
                    let result = cx.background_executor().spawn(async move {
                        let client = Self::client();
                        client.get(&format!("/api/v1/peripherals/{id_owned}/read"))
                    }).await;
                    let _ = view.update(cx, |this, cx| {
                        if this.progress_generation != generation {
                            return;
                        }
                        if let Ok(body) = result {
                            if let Ok(parsed) = serde_json::from_str::<ReadProgress>(&body) {
                                this.progress = Some(parsed);
                                this.progress_id = Some(id.clone());
                            }
                        }
                        cx.notify();
                    });
                }
                cx.background_executor().timer(std::time::Duration::from_millis(STAGE_POLL_MS)).await;
            }
        })
        .detach();
    }

    /// An edit (`onChange`).
    fn on_change(&mut self, key: &str, value: PropValue, cx: &mut Context<Self>) {
        let Some(id) = self.sel_id() else {
            return;
        };
        let next = self.edits_of(&id) + 1;
        self.edit_count.insert(id.clone(), next);
        if let Some(entry) = self.staged_by.get_mut(&id) {
            entry.insert(key.to_string(), value);
        } else {
            let mut entry = Props::new();
            entry.insert(key.to_string(), value);
            self.staged_by.insert(id.clone(), entry);
        }
        self.save_errors.remove(&id);
        cx.notify();
    }

    /// Apply (`apply()`): PATCH the changed keys, then refresh and toast.
    pub fn apply(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sel_id() else {
            return;
        };
        if self.detail().is_none() {
            return;
        }
        let sent = self.changed();
        self.saving = true;
        self.save_errors.remove(&id);
        let sent_clone = sent.clone();
        let id_for_bg = id.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let body = serde_json::to_string(&sent_clone).unwrap_or_else(|_| "{}".into());
            client.patch_json(&format!("/api/v1/peripherals/{id_for_bg}"), &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(_) => {
                        if let Some(entry) = this.staged_by.get(&id) {
                            let next = stale_after_apply(entry, &sent);
                            if next.is_empty() {
                                this.staged_by.remove(&id);
                            } else {
                                this.staged_by.insert(id.clone(), next);
                            }
                        }
                        this.load_detail(&id, true, false, cx);
                        this.toast = Some(Self::toast_after_apply());
                    }
                    Err(e) => {
                        this.note_failure(&id, true);
                        this.save_errors.insert(id.clone(), e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Capture only (`--peri-toast`, #246): once the selected device's detail
    /// has landed, send the change (`apply`) so the fake daemon's answer
    /// raises React's toast. Nothing is staged, so the PATCH is empty and the
    /// device's settings do not change; the shell holds the lane open with
    /// `--toast-hold` so a still frame can compare it with React's.
    pub fn capture_apply_when_ready(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            for _ in 0..300 {
                let done = view
                    .update(cx, |this, cx| {
                        if this.detail().is_some() {
                            this.apply(cx);
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if done {
                    return;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
            }
        })
        .detach();
    }

    /// Revert (`revert()`): clears the device's staged edits and save error.
    pub fn revert(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sel_id() else {
            return;
        };
        self.staged_by.remove(&id);
        self.save_errors.remove(&id);
        cx.notify();
    }

    /// The access switches (`setAccess`).
    fn set_access(&mut self, next: access::AccessNext, cx: &mut Context<Self>) {
        let detail = self.detail();
        let Some(detail) = detail else {
            return;
        };
        let reads = next.reads.unwrap_or(detail.access.reads);
        let writes = next.writes.unwrap_or(detail.access.writes);
        let id = detail.info.id.0.clone();
        self.save_errors.remove(&id);
        let id_for_bg = id.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let access = canvas_proto::peripheral::PeripheralAccess { reads, writes };
            let body = serde_json::to_string(&access).unwrap_or_else(|_| "{}".into());
            client.put_json(&format!("/api/v1/peripherals/{id_for_bg}/access"), &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.load_detail(&id, true, false, cx);
                    }
                    Err(e) => {
                        this.note_failure(&id, false);
                        this.save_errors.insert(id.clone(), e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Read again (`loadDetail(id, false, true)`).
    fn read_again(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.sel_id() else {
            return;
        };
        self.load_detail(&id, false, true, cx);
        self.start_progress_poll(cx);
    }
}

pub mod access {
    /// The access change's next values (`setAccess`).
    #[derive(Debug, Clone, Copy)]
    pub struct AccessNext {
        pub reads: Option<bool>,
        pub writes: Option<bool>,
    }

    impl AccessNext {
        /// Turning reads off also turns writes off.
        pub fn reads_change(v: bool) -> Self {
            Self { reads: Some(v), writes: if v { None } else { Some(false) } }
        }

        /// Turning writes on or off alone.
        pub fn writes_change(v: bool) -> Self {
            Self { reads: None, writes: Some(v) }
        }
    }
}

/// A toast (`notify`): the words and the tone.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub text: String,
    pub tone: &'static str,
}

impl PeripheralsView {
    /// Takes the toast, if one is showing. The shell's notification lane is
    /// the only caller (`Shell::render`); React's `notify` is global chrome
    /// (`Notify.tsx:116`), so the toast is taken on every page, not just this
    /// one (#246).
    pub fn take_toast(&mut self) -> Option<Toast> {
        self.toast.take()
    }

    /// The slider's measured box for `key`: fresh every render, shared by the
    /// fader's canvas and its drag handlers for that render.
    #[allow(dead_code)]
    fn slider_origin(&self, key: &str) -> Origin {
        let _ = key;
        Origin::new()
    }

    /// A section panel (`Panel` with the section title). `narrow` is the
    /// phone container rule (`.rd-peripherals .panel { padding: var(--s3) }`
    /// at 639 px, against `p_4` elsewhere).
    fn panel(&self, id: &str, title: &str, body: impl IntoElement, narrow: bool) -> impl IntoElement {
        let t = &self.theme;
        div()
            .id(gpui::SharedString::from(id.to_string()))
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .p_4()
            .when(narrow, |d| d.p_3())
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

    /// The error panel (`ErrorPanel`): title, one-line summary, raw message
    /// behind "Details", and Try again.
    #[allow(dead_code)]
    fn error_panel(
        &self,
        id: &str,
        title: &str,
        summary: &str,
        message: &str,
        on_retry: gpui::Entity<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let entity = on_retry.clone();
        let retry = button(t, &format!("{id}-try-again"), "Try again", Variant::Default, false, move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.detail_error = None;
                this.change_saved = false;
                if let Some(id) = this.sel_id() {
                    this.load_detail(&id, false, false, cx);
                }
            });
        });
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
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.down))
                            .child(summary.to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                gpui::svg()
                                    .path("icons/CaretRight.svg")
                                    .size(px(14.0))
                                    .flex_none()
                                    .text_color(Theme::color(t.mute)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_family(crate::fonts::MONO_FAMILY)
                                    .text_color(Theme::color(t.mute))
                                    .child(message.to_string()),
                            ),
                    )
                    .child(div().flex().child(retry)),
            )
    }

    /// The loading shape (`PeripheralListShape`): two placeholder rows.
    fn list_shape(&self) -> impl IntoElement {
        let t = &self.theme;
        let skel = |w: f32, h: f32| {
            div()
                .w(px(w))
                .h(px(h))
                .r_sm()
                .bg(Theme::color(t.raised))
        };
        let mut rows = div().flex().flex_col().gap_2().min_w(px(0.0));
        for _ in 0..2 {
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .r_md()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .bg(Theme::color(t.panel))
                    .child(skel(28.0, 28.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(skel(120.0, 16.0))
                            .child(skel(160.0, 14.0)),
                    ),
            );
        }
        rows
    }

    /// The detail loading shape (`PeripheralDetailShape`): skeleton panels.
    fn detail_shape(&self, kind: PeripheralKind) -> impl IntoElement {
        let t = &self.theme;
        let skel = |w: f32, h: f32| {
            div()
                .w(px(w))
                .h(px(h))
                .r_sm()
                .bg(Theme::color(t.raised))
        };
        let panel_box = |h: f32, rows: usize| {
            let mut body = div().flex().flex_col().gap_2();
            for i in 0..rows {
                body = body.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(skel(110.0 + ((i * 41) % 70) as f32, 14.0))
                        .child(skel(if i % 2 == 0 { 150.0 } else { 110.0 }, 32.0)),
                );
            }
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
                .child(skel(96.0, 14.0))
                .child(body)
        };
        let mut stack = div().flex().flex_col().gap_4();
        stack = stack.child(panel_box(79.3, 1));
        if kind == PeripheralKind::GameController {
            stack = stack.child(panel_box(270.3, 4));
            stack = stack.child(panel_box(921.6, 0));
            stack = stack.child(panel_box(582.3, 10));
        } else {
            stack = stack.child(panel_box(171.3, 1));
            stack = stack.child(panel_box(166.3, 2));
            stack = stack.child(panel_box(270.3, 4));
            stack = stack.child(panel_box(166.3, 2));
            stack = stack.child(panel_box(374.3, 6));
        }
        stack
    }

    /// The stage line (`div.load-stage`): words and ticks (1..=12).
    fn stage_line_el(&self, label: &str, done: u32, total: u32) -> impl IntoElement {
        let t = &self.theme;
        let mut row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(stage_line(label, done, total)),
            );
        if total > 0 && total <= 12 {
            let mut ticks = div().flex().items_center().gap(px(4.0));
            for i in 0..total {
                ticks = ticks.child(
                    div()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(if i < done { Theme::color(t.live) } else { Theme::color(t.line) }),
                );
            }
            row = row.child(ticks);
        }
        row
    }

    /// A read-only row (`ReadOnlyValue`).
    fn read_only_value(&self, value: &PropValue) -> impl IntoElement {
        let t = &self.theme;
        match value {
            PropValue::Color(c) => {
                let hex = c.to_hex();
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(
                        div()
                            .size(px(14.0))
                            .r_sm()
                            .border_1()
                            .border_color(Theme::color(t.line_2))
                            .bg(Theme::color(&hex)),
                    )
                    .child(
                        div()
                            .font_family(crate::fonts::MONO_FAMILY)
                            .child(hex),
                    )
            }
            PropValue::Bool(on) => {
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(if *on { "on".to_string() } else { "off".to_string() })
            }
            PropValue::Number(n) => {
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(format_number(*n, 0))
            }
            PropValue::Choice(s) => {
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(choice_word(s))
            }
            PropValue::Hue(n) => {
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(format_number(*n, 0))
            }
            PropValue::Angle(n) => {
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(format_number(wrap_deg(*n), 0))
            }
        }
    }

    /// One property row (`div.prop-item`).
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn prop_row(
        &self,
        def: &PropertyDef,
        value: &PropValue,
        read_only: &[String],
        disabled: bool,
        schema_properties: &[PropertyDef],
        values: &Props,
        view_entity: gpui::Entity<Self>,
        narrow: bool,
        last: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        let fixed = read_only.iter().any(|k| k == &def.key);
        // `schemaOf` strips the profile repeat from every label
        // (`PeripheralsView.tsx:47-58`): the row draws the stripped label.
        let label = plain_label(&def.label);
        let key = def.key.clone();
        let control: gpui::AnyElement = if fixed {
            self.read_only_value(value).into_any_element()
        } else {
            self.prop_control(&key, &label, &def.kind, value, schema_properties, values, disabled, view_entity, narrow).into_any_element()
        };
        let label_el = div()
            .flex_1()
            .min_w(px(0.0))
            .text_size(px(15.0))
            .line_height(gpui::relative(1.45))
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.ink))
            .child(label.clone());
        // The generic path is `minmax(0, 116px) minmax(0, 1fr)` inside the
        // sections; number rows are `minmax(0, 116px) minmax(0, 1fr) 48px`.
        // Both number rows keep their parts at fixed columns: the slider
        // `flex: 1` (85 px measured at 1440 for the 1fr column, 64 px
        // effective with the `.num` field at 48 px), the field 48 px.
        let is_number = matches!(def.kind, PropKind::Number { .. });
        // Phone: only number and segmented rows stack (`.prop-item[data-kind=
        // "number"] .field-label` and `.prop-item:has(.seg) .field-label`
        // span the row at 639 px); a plain row keeps the 116 px label
        // column with the control after it (measured at 390: `Active DPI
        // stage` label 116 + control 200). `phone` is the param, before
        // the local `narrow` below shadows it for the stacking test.
        let phone = narrow;
        let is_seg = !fixed && matches!(&def.kind, PropKind::Choice { options, .. } if options.len() <= 4);
        // A segmented choice (4 or fewer options) drops under its label
        // (`.prop-item:has(.seg)`): the control sizes to its content and
        // spans the row.
        let narrow = is_seg || (narrow && is_number);
        // React's rows carry `border-bottom: 1px solid` and the last row of a
        // list drops it (`.prop-item:last-child { border-bottom: 0 }`). A
        // GPUI border adds to the box (React's is inside `border-box`), so
        // drawing it on the last row both painted a line React has not got
        // and made that row 1 px taller, which every panel below inherited
        // (measured at 390: the mouse's Performance panel 216 px against
        // React's 214.75, and the page 4 px out of step by the Buttons panel).
        let rule = !last;
        if is_seg {
            // The segmented control sizes to its content
            // (`.rd-peri-sections .seg { width: max-content }`), so the
            // stacked row shrink-wraps it.
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_2()
                .min_h(px(44.0))
                .py_2()
                .when(rule, |d| d.border_b_1().border_color(Theme::color(t.line)))
                .child(label_el)
                .child(control)
        } else if narrow {
            // Phone number rows: the label spans the row, then the slider
            // and the 48 px field share that line
            // (`.rd-peripherals .rd-peri-sections .prop-item[data-kind=
            // "number"] { grid-template-columns: minmax(0, 1fr) 48px }`
            // with `.field-label { grid-column: 1 / -1 }`). The control
            // stretches, so the slider flexes to the space left of the
            // field instead of collapsing to its cap. The label-to-control
            // gap is 12 px (measured on `.prop-item` at 390: `gap: 12px`),
            // not the 8 px `gap_2` the seg rows use.
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .min_h(px(44.0))
                .py_2()
                .when(rule, |d| d.border_b_1().border_color(Theme::color(t.line)))
                .child(pr(format!("lbl-{key}"), label_el))
                .child(pr(format!("ctl-{key}"), control))
        } else if is_number {
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_h(px(52.0))
                .py_2()
                .when(rule, |d| d.border_b_1().border_color(Theme::color(t.line)))
                .child(div().w(px(116.0)).flex_none().child(label_el))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(control),
                )
        } else {
            // Plain rows: the 116 px label column with the control after
            // it at every width, 52 px tall on desktop and 44 px on the
            // phone (`.rd-peri-sections .prop-item { min-height: 44px }`
            // inside the 639 container rule).
            div()
                .flex()
                .items_center()
                .gap_4()
                .min_h(px(if phone { 44.0 } else { 52.0 }))
                .py_2()
                .when(rule, |d| d.border_b_1().border_color(Theme::color(t.line)))
                .child(div().w(px(116.0)).flex_none().child(label_el))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(control),
                )
        }
    }

    /// The control for one property (`control()` in PropertyControls):
    /// delegated to the shared `components::prop_controls::control`, with
    /// this view's choice words remapped through `control_words`.
    #[allow(clippy::too_many_arguments)]
    fn prop_control(
        &self,
        key: &str,
        label: &str,
        kind: &PropKind,
        value: &PropValue,
        schema_properties: &[PropertyDef],
        values: &Props,
        disabled: bool,
        view_entity: gpui::Entity<Self>,
        narrow: bool,
    ) -> gpui::AnyElement {
        let t = &self.theme;
        if let PropKind::Number { min, max, step, default } = kind {
            // Number rows on this page: the `.num` is 48 px (`.rd-peripherals
            // .rd-peri-sections .prop-item[data-kind="number"] .num`), not the
            // shared 64 px, so the slider takes what is left of the 116 px
            // label column (measured 64 px at 1440).
            let n = match value {
                PropValue::Number(n) => *n as f32,
                _ => *default as f32,
            };
            let (min_f, max_f, step_f) = (*min as f32, *max as f32, *step as f32);
            let entity = view_entity.clone();
            let key_owned = key.to_string();
            let on = std::rc::Rc::new(move |v: PropValue, window: &mut Window, cx: &mut gpui::App| {
                entity.update(cx, |this, cx| this.on_change(&key_owned, v, cx));
                let _ = window;
            }) as crate::components::prop_controls::OnChange;
            let change = {
                let on = on.clone();
                move |v: f32, window: &mut Window, cx: &mut gpui::App| {
                    let stepped = crate::components::prop_controls::quantise_step(v, min_f, max_f, step_f);
                    on(PropValue::Number(stepped as f64), window, cx);
                }
            };
            return div()
                .flex()
                .items_center()
                .gap_3()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    // The slider is `flex: 1` in the 1fr column (measured 64
                    // px at 1440 where the 116 px label column and the 48 px
                    // `.num` field frame it, and 272 px at 390 where the
                    // label spans the row).
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(crate::components::slider::slider_range(
                            t,
                            &format!("{key}-slider"),
                            n,
                            min_f,
                            max_f,
                            disabled,
                            Origin::new(),
                            change.clone(),
                            change,
                        )),
                )
                .child(crate::components::number_field::number_field_width(
                    t,
                    &format!("{key}-field"),
                    n,
                    min_f,
                    max_f,
                    step_f,
                    None,
                    48.0,
                    {
                        let on = on.clone();
                        move |v: f32, window: &mut Window, cx: &mut gpui::App| {
                            let stepped = crate::components::prop_controls::quantise_step(v, min_f, max_f, step_f);
                            on(PropValue::Number(stepped as f64), window, cx);
                        }
                    },
                ))
                .into_any_element();
        }
        if let PropKind::Choice { options, .. } = kind {
            let pairs: Vec<(String, String)> = options
                .iter()
                .map(|o| (o.clone(), choice_word(o)))
                .collect();
            let open = self.select_open.get(key).copied().unwrap_or(false);
            let entity = view_entity.clone();
            let key_owned = key.to_string();
            let on = std::rc::Rc::new(move |v: PropValue, window: &mut Window, cx: &mut gpui::App| {
                entity.update(cx, |this, cx| this.on_change(&key_owned, v, cx));
                let _ = window;
            }) as crate::components::prop_controls::OnChange;
            let entity_toggle = view_entity.clone();
            let key_toggle = key.to_string();
            let toggle = std::rc::Rc::new(move |window: &mut Window, cx: &mut gpui::App| {
                entity_toggle.update(cx, |this, cx| {
                    let entry = this.select_open.entry(key_toggle.clone()).or_insert(false);
                    *entry = !*entry;
                    cx.notify();
                });
                let _ = window;
            }) as crate::components::prop_controls::OnToggle;
            let words = control_words(t, key, label, value, &pairs, disabled, open, on, toggle);
            // The trigger is `.select-trigger { min-width: 160px; max-width:
            // 100% }`: at the 248 px panel column the 120 px control column
            // is smaller than the min-width, so the trigger is 160 px wide
            // and overflows its column (measured at 1440); at 390 the
            // column is 200 px and the trigger fills it to 190 px.
            if options.len() > 4 {
                let width = if narrow { 190.0 } else { 160.0 };
                return div().w(px(width)).flex_none().child(words).into_any_element();
            }
            return words;
        }
        let color_open = self.color_open.as_deref() == Some(key);
        let select_open = self.select_open.get(key).copied().unwrap_or(false);
        let entity = view_entity.clone();
        let key_owned = key.to_string();
        let on = std::rc::Rc::new(move |v: PropValue, window: &mut Window, cx: &mut gpui::App| {
            entity.update(cx, |this, cx| this.on_change(&key_owned, v, cx));
            let _ = window;
        }) as crate::components::prop_controls::OnChange;
        let entity_color = view_entity.clone();
        let key_color = key.to_string();
        let toggle_color = std::rc::Rc::new(move |window: &mut Window, cx: &mut gpui::App| {
            entity_color.update(cx, |this, cx| {
                this.color_open = if this.color_open.as_deref() == Some(key_color.as_str()) {
                    None
                } else {
                    Some(key_color.clone())
                };
                cx.notify();
            });
            let _ = window;
        }) as crate::components::prop_controls::OnToggle;
        let entity_select = view_entity.clone();
        let key_select = key.to_string();
        let toggle_select = std::rc::Rc::new(move |window: &mut Window, cx: &mut gpui::App| {
            entity_select.update(cx, |this, cx| {
                let entry = this.select_open.entry(key_select.clone()).or_insert(false);
                *entry = !*entry;
                cx.notify();
            });
            let _ = window;
        }) as crate::components::prop_controls::OnToggle;
        control(
            t,
            key,
            label,
            kind,
            value,
            schema_properties,
            values,
            disabled,
            color_open,
            select_open,
            on,
            toggle_color,
            toggle_select,
        )
    }

    /// A choice with more than 4 options (`Select`), with this view's words.
    /// The trigger is `width` px (`120` in a section's 1fr column,
    /// `128` in a pad-map row): React's `.select-trigger` is capped by its
    /// column (`max-width: 100%` over a min-width that a 124 px column
    /// cannot satisfy), so it never sizes to its widest option.
    #[allow(clippy::too_many_arguments)]
    fn choice_select(
        &self,
        key: &str,
        label: &str,
        options: &[String],
        current: &str,
        disabled: bool,
        width: f32,
        view_entity: gpui::Entity<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let open = self.select_open.get(key).copied().unwrap_or(false);
        let pairs: Vec<(String, String)> = options
            .iter()
            .map(|o| (o.clone(), choice_word(o)))
            .collect();
        let entity = view_entity.clone();
        let key_owned = key.to_string();
        let on: crate::components::prop_controls::OnChange = std::rc::Rc::new(move |v: PropValue, window: &mut Window, cx: &mut gpui::App| {
            entity.update(cx, |this, cx| this.on_change(&key_owned, v, cx));
            let _ = window;
        });
        let entity_toggle = view_entity.clone();
        let key_toggle = key.to_string();
        let toggle: crate::components::prop_controls::OnToggle = std::rc::Rc::new(move |window: &mut Window, cx: &mut gpui::App| {
            entity_toggle.update(cx, |this, cx| {
                let entry = this.select_open.entry(key_toggle.clone()).or_insert(false);
                *entry = !*entry;
                cx.notify();
            });
            let _ = window;
        });
        let value = PropValue::Choice(current.to_string());
        div().w(px(width)).flex_none().child(control_words(t, key, label, &value, &pairs, disabled, open, on, toggle))
    }

    /// The DPI panel (`DpiPanel`).
    #[allow(clippy::too_many_arguments)]
    fn dpi_panel(
        &self,
        section_id: &str,
        section_title: &str,
        properties: &[PropertyDef],
        values: &Props,
        read_only: &[String],
        disabled: bool,
        view_entity: gpui::Entity<Self>,
        narrow: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        let stages = dpi_stages(values);
        let active = dpi_active_stage(values);
        let others = dpi_others(properties);
        let picked = self.picked_stage;
        let picked_defs = dpi_picked_defs(properties, picked);
        let mut body = div().flex().flex_col().min_w(px(0.0));
        // The remaining rows (`others`: here `dpi-active-stage` only).
        let others_defs: Vec<PropertyDef> = others.iter().map(|p| (*p).clone()).collect();
        if !others_defs.is_empty() {
            let mut list = div().flex().flex_col();
            let schema: Vec<PropertyDef> = others_defs.clone();
            for (i, def) in schema.iter().enumerate() {
                let value = values.get(&def.key).cloned().unwrap_or_else(|| default_for(&def.kind));
                let row = self.prop_row(def, &value, read_only, disabled, &schema, values, view_entity.clone(), narrow, i + 1 == schema.len());
                list = list.child(pr(format!("row-{}", def.key), row));
            }
            body = body.child(list);
        }
        // The stage chips.
        let mut chips = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .mt_3()
            .pt_3()
            .border_t_1()
            .border_color(Theme::color(t.line));
        for stage in &stages {
            let is_active = active == stage.n.to_string();
            let is_picked = picked == Some(stage.n);
            let n = stage.n;
            let label = format!("DPI stage {n}, {}", format_number(stage.dpi, 0));
            let full_label = if is_active { format!("{label}, the active stage") } else { label };
            let entity = view_entity.clone();
            let chip = div()
                .id(gpui::SharedString::from(format!("{section_id}-chip-{n}")))
                .flex()
                .items_center()
                .gap_2()
                .h(px(32.0))
                .px(px(12.0))
                .r_sm()
                .border_1()
                .border_color(if is_active { t.live() } else { Theme::color(t.line_2) })
                .bg(Theme::color(t.raised))
                .text_color(Theme::color(t.ink))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.picked_stage = if this.picked_stage == Some(n) { None } else { Some(n) };
                        cx.notify();
                    });
                })
                .child(
                    // The dot is `.dpi-chip-dot` (10 px, `box-shadow: 0 0 0
                    // 1px` ring: a 12 px footprint with 10 px ink). GPUI
                    // draws it as a 10 px border-box with `border_1`
                    // (8 px ink under the ring); a 12 px box wraps the
                    // widest chip (`26000`) onto a third row at 1440, so
                    // the 10 px box stays (measured: 4.66% vs 7.15%).
                    div()
                        .size(px(10.0))
                        .rounded_full()
                        .bg(Theme::color(&stage.colour))
                        .border_1()
                        .border_color(Theme::color(t.line_2)),
                )
                .child(
                    div()
                        .text_size(px(13.333))
                        .font_family(crate::fonts::MONO_FAMILY)
                        .child(format_number(stage.dpi, 0)),
                );
            // The skirtless key: the DPI chip carries no skirt (React's
            // `box-shadow` is the skirt; `data-on` flattens it).
            let chip = if is_picked {
                chip.border_2().border_color(t.live())
            } else {
                chip
            };
            let _ = full_label;
            chips = chips.child(pr(format!("chip-{section_id}-{n}"), chip));
        }
        body = body.child(chips);
        // The picked stage editor.
        if picked.is_some() && !picked_defs.is_empty() {
            let schema: Vec<PropertyDef> = picked_defs.iter().map(|p| (*p).clone()).collect();
            let mut list = div().flex().flex_col().mt_3().pt_3().border_t_1().border_color(Theme::color(t.line));
            for (i, def) in schema.iter().enumerate() {
                let value = values.get(&def.key).cloned().unwrap_or_else(|| default_for(&def.kind));
                let row = self.prop_row(def, &value, read_only, disabled, &schema, values, view_entity.clone(), narrow, i + 1 == schema.len());
                list = list.child(pr(format!("row-{}", def.key), row));
            }
            body = body.child(list);
        }
        self.panel(&format!("dpi-{section_id}"), section_title, body, narrow)
    }

    /// The controller panel (`ControllerPanel`).
    fn controller_panel(
        &self,
        detail: &PeripheralDetail,
        values: &Props,
        read_only: &[String],
        disabled: bool,
        view_entity: gpui::Entity<Self>,
        content_w: f32,
    ) -> impl IntoElement {
        let t = &self.theme;
        let buttons_section = detail.schema.sections.iter().find(|s| s.id == "buttons");
        let profile = active_profile(values);
        let profile_section = detail.schema.sections.iter().find(|s| s.id == format!("profile-{profile}"));
        let paddles: Vec<PropertyDef> = profile_section
            .map(|s| s.properties.iter().filter(|p| p.key.contains("-paddle-")).cloned().collect())
            .unwrap_or_default();
        let mut rows: Vec<PropertyDef> = buttons_section.map(|s| s.properties.clone()).unwrap_or_default();
        rows.extend(paddles);
        let picked = self.picked_button.clone();
        // The plan.
        let plan_buttons: Vec<(String, String)> = rows.iter().map(|p| (p.key.clone(), p.label.clone())).collect();
        let narrow_grid = content_w <= 960.0;
        // `.pad-plan-wrap` is the box the plan sits in: 12 px padding and a
        // 1 px border on a `--sunk` bed. At a desk the grid's first column is
        // 420 px wide (`minmax(280px, 420px)`); when the grid is one column
        // (960 and below) the wrap is the panel's whole content box (panel
        // padding 12 on a phone, 16 above it). The plan fills the wrap's
        // content box, so it is 394 px at a desk and 306 on a phone, not the
        // 326 the cell alone gave (React's `.pad-plan` is 306 at 390).
        let board_w = content_w - if content_w < 640.0 { 26.0 } else { 34.0 };
        let wrap_w = if narrow_grid { board_w } else { 420.0 };
        let plan = self.controller_outline(&plan_buttons, picked.as_deref(), view_entity.clone(), wrap_w - 26.0);
        let plan_wrap = pr(
            "pad-plan-wrap",
            div()
                .flex()
                .flex_col()
                .items_center()
                .w(px(wrap_w))
                .flex_none()
                .p_3()
                .r_md()
                .border_1()
                .border_color(Theme::color(t.line))
                .bg(Theme::color(t.sunk))
                .child(plan),
        );
        // The map list.
        let mut map = div().flex().flex_col().gap_2().flex_1().min_w(px(0.0));
        for def in &rows {
            let current = plain_choice(values, &def.key).unwrap_or_default();
            let fixed = read_only.iter().any(|k| k == &def.key);
            let is_picked = picked.as_deref() == Some(def.key.as_str());
            let label = plain_label(&def.label);
            let key_owned = def.key.clone();
            let entity = view_entity.clone();
            let mut row = div()
                .id(gpui::SharedString::from(format!("pad-row-{}", def.key)))
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .min_w(px(0.0))
                .py_1()
                .border_b_1()
                .border_color(if is_picked { t.live() } else { Theme::color(t.line) });
            row = row.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(label.clone()),
            );
            if fixed {
                row = row.child(
                    div()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child(choice_word(&current)),
                );
            } else {
                let options: Vec<String> = match &def.kind {
                    PropKind::Choice { options, .. } => options.clone(),
                    _ => Vec::new(),
                };
                if options.is_empty() {
                    row = row.child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.dim))
                            .child(choice_word(&current)),
                    );
                } else {
                    let select = self.choice_select(&def.key, &format!("{label} mapping"), &options, &current, disabled, 128.0, view_entity.clone());
                    // The map Select is 128 px wide at desktop.
                    row = row.child(div().flex_none().child(select));
                }
            }
            let _ = (key_owned, entity);
            map = map.child(row);
        }
        let map_grid = if content_w <= 720.0 {
            // Phone: one column.
            map
        } else if narrow_grid {
            // 960 and below the pad grid is one column; the map itself stays
            // `repeat(auto-fill, minmax(230px, 1fr))`.
            map
        } else {
            map
        };
        let map_grid = pr("pad-map", map_grid);
        let grid = if narrow_grid {
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(plan_wrap)
                .child(map_grid)
        } else {
            div()
                .flex()
                .gap_5()
                .items_start()
                .child(plan_wrap)
                .child(map_grid)
        };
        pr("panel-controller", self.panel("controller-panel", &detail.info.name, grid, content_w < 640.0))
    }

    /// The plan of the pad (`ControllerOutline`): front and rear views.
    fn controller_outline(
        &self,
        buttons: &[(String, String)],
        selected: Option<&str>,
        view_entity: gpui::Entity<Self>,
        plan_w: f32,
    ) -> impl IntoElement {
        let t = &self.theme;
        let front: Vec<(String, String, PlacedSpot)> = buttons
            .iter()
            .filter_map(|(key, label)| front_spot(&place_of(key)).map(|spot| (key.clone(), label.clone(), spot)))
            .collect();
        let rear: Vec<(String, String, PlacedSpot)> = buttons
            .iter()
            .filter_map(|(key, label)| rear_spot(&place_of(key)).map(|spot| (key.clone(), label.clone(), spot)))
            .collect();
        // The front view: 340x244 in the wrap's content box (`width: 100%`).
        let plan_h = plan_w * 244.0 / 340.0;
        let front_canvas = self.pad_canvas(BODY_PATH, &front, true, plan_w, plan_h);
        let mut front_wrap = div()
            .relative()
            .w(px(plan_w))
            .h(px(plan_h))
            .child(front_canvas);
        // Targets as absolutely positioned divs (clickable hit areas).
        let scale = plan_w / 340.0;
        for (key, label, placed) in &front {
            let (x, y, w, h) = hit_rect(&placed.spot);
            let on = selected == Some(key.as_str());
            let entity = view_entity.clone();
            let key_owned = key.clone();
            let target = div()
                .id(gpui::SharedString::from(format!("pad-front-{key}")))
                .absolute()
                .left(px(x * scale))
                .top(px(y * scale))
                .w(px(w * scale))
                .h(px(h * scale))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.picked_button = Some(key_owned.clone());
                        cx.notify();
                    });
                });
            let target = target
                .r_sm()
                .border_1()
                .border_color(if on {
                    t.live()
                } else {
                    Theme::color(t.line_2)
                })
                .bg(if on { t.live_soft() } else { Theme::color(t.panel) });
            let target = if let Mark::Text(text) = &placed.mark {
                target.child(
                    div()
                        .text_sm()
                        .font_family(crate::fonts::MONO_FAMILY)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Theme::color(t.dim))
                        .child(text.clone()),
                )
            } else {
                target
            };
            let _ = label;
            front_wrap = front_wrap.child(target);
        }
        // The rear view: 200x124, 200 px wide.
        let rear_w = 200.0f32;
        let rear_h = rear_w * 124.0 / 200.0;
        let rear_canvas = self.pad_canvas(BODY_PATH, &rear, false, rear_w, rear_h);
        let mut rear_wrap = div()
            .relative()
            .w(px(rear_w))
            .h(px(rear_h))
            .child(rear_canvas);
        let rear_scale = rear_w / 200.0;
        for (key, _, placed) in &rear {
            let (x, y, w, h) = hit_rect(&placed.spot);
            let on = selected == Some(key.as_str());
            let entity = view_entity.clone();
            let key_owned = key.clone();
            let target = div()
                .id(gpui::SharedString::from(format!("pad-rear-{key}")))
                .absolute()
                .left(px(x * rear_scale))
                .top(px(y * rear_scale))
                .w(px(w * rear_scale))
                .h(px(h * rear_scale))
                .flex()
                .items_center()
                .justify_center()
                .r_sm()
                .border_1()
                .border_color(if on { t.live() } else { Theme::color(t.line_2) })
                .bg(if on { t.live_soft() } else { Theme::color(t.panel) })
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.picked_button = Some(key_owned.clone());
                        cx.notify();
                    });
                });
            let target = if let Mark::Text(text) = &placed.mark {
                target.child(
                    div()
                        .text_sm()
                        .font_family(crate::fonts::MONO_FAMILY)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Theme::color(t.dim))
                        .child(text.clone()),
                )
            } else {
                target
            };
            rear_wrap = rear_wrap.child(target);
        }
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .w_full()
            .child(pr("pad-front", front_wrap))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .w(px(200.0))
                    .child(pr("pad-rear", rear_wrap))
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child("REAR"),
                    ),
            )
    }

    /// The pad body canvas: hairline paths, no fill (`pad-body`).
    fn pad_canvas(&self, _body: &str, _spots: &[(String, String, PlacedSpot)], front: bool, w: f32, h: f32) -> impl IntoElement {
        let body_color = Theme::color(self.theme.line_2);
        let glyph_color = Theme::color(self.theme.dim);
        let body = BODY_PATH;
        let dpad = DPAD_PATH;
        let is_front = front;
        let glyphs: Vec<(f32, f32, &'static str)> = if is_front {
            _spots
                .iter()
                .filter_map(|(_, _, placed)| match &placed.mark {
                    Mark::Glyph(name) => Some((placed.spot.cx, placed.spot.cy, *name)),
                    Mark::Text(_) => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        canvas(
            move |_, _, _| {},
            move |bounds, _, window, _| {
                let plot_w = f32::from(bounds.size.width);
                let plot_h = f32::from(bounds.size.height);
                let scale = if is_front { plot_w / 340.0 } else { plot_w / 200.0 };
                let ox = f32::from(bounds.origin.x);
                let oy = f32::from(bounds.origin.y);
                let at = |x: f32, y: f32| point(px(ox + x * scale), px(oy + y * scale));
                if is_front {
                    paint_body_path(window, &at, body, None, body_color, scale);
                    paint_body_path(window, &at, dpad, None, body_color, scale);
                    // Glyphs.
                    for (cx, cy, name) in &glyphs {
                        paint_glyphs(window, at(*cx, *cy), name, glyph_color, scale);
                    }
                } else {
                    paint_body_path(window, &at, body, Some((REAR_TX, REAR_TY, REAR_SCALE)), body_color, scale);
                }
                let _ = (plot_h, h);
            },
        )
        .w(px(w))
        .h(px(h))
    }

    /// The device detail (`PeripheralDetail` in React).
    fn detail_body(&self, detail: &PeripheralDetail, values: &Props, view_entity: gpui::Entity<Self>, content_w: f32) -> impl IntoElement {
        let t = &self.theme;
        let info = &detail.info;
        let read_only = &detail.schema.read_only;
        let writes = detail.access.writes;
        let reads = detail.access.reads;
        let is_controller = detail.schema.kind == PeripheralKind::GameController;
        let is_mouse = detail.schema.kind == PeripheralKind::Mouse;
        let icon = match info.kind {
            PeripheralKind::Mouse => "Mouse",
            PeripheralKind::GameController => "GameController",
            PeripheralKind::Keyboard => "Keyboard",
        };
        // The device head.
        let head = div()
            .flex()
            .items_center()
            .gap_3()
            .flex_wrap()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_none()
                    .size(px(44.0))
                    .r_md()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .bg(Theme::color(t.sunk))
                    .text_color(t.live())
                    .child(gpui::svg().path(format!("icons/{icon}.svg")).size(px(22.0)).text_color(t.live())),
            )
            .child(
                // `.peripheral-head-text { flex: 1 1 auto; min-width: 0 }`:
                // the name block takes the free space, so the StateDot sits
                // right before the access block (measured at 1440: the name
                // block is 638 px wide, `Connected` starts at 947).
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .child(pr(
                        "head-name",
                        div()
                            .text_size(px(20.0))
                            .line_height(gpui::relative(1.25))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(info.name.clone()),
                    ))
                    .child(pr(
                        "head-sub",
                        div()
                            // The subline wraps across the name block's width:
                            // at 390 the mono location drops to a second line
                            // beside the `Connected` state dot (`.peripheral-head
                            // -text { flex: 1 1 0 }`, `.peripheral-head { flex-
                            // wrap: wrap }`), not under the dot.
                            .text_sm()
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            // No gap: React's `<p>` is inline text, so a wrap
                            // lands line 2 directly under line 1. GPUI's
                            // `gap(2)` set BOTH axes and added a 2 px row-gap
                            // between the wrapped lines (measured: the sub
                            // box was 43 px vs React's 40.6), shifting
                            // everything below by 2 px. The space React has
                            // between `·` and the mono span is the text
                            // child's trailing space.
                            .child(format!("{} · {} · ", kind_word(info.kind), info.vendor))
                            .child(
                                div()
                                    .font_family(crate::fonts::MONO_FAMILY)
                                    .flex_none()
                                    .child(info.location.clone()),
                            ),
                    )),
            )
            .child(state_dot(t, if info.connected { "ok" } else { "mute" }, if info.connected { "Connected" } else { "Not connected" }));
        // The access switches.
        let reading = self.reading();
        let entity_read = view_entity.clone();
        let entity_change = view_entity.clone();
        let entity_again = view_entity.clone();
        let read_toggle: gpui::AnyElement = if reads {
            toggle(t, "peri-read", true, move |v, _, cx| {
                entity_read.update(cx, |this, cx| this.set_access(access::AccessNext::reads_change(v), cx));
            })
            .into_any_element()
        } else {
            toggle_disabled(t, "peri-read", false, move |v, _, cx| {
                entity_read.update(cx, |this, cx| this.set_access(access::AccessNext::reads_change(v), cx));
            })
            .into_any_element()
        };
        let change_toggle: gpui::AnyElement = if !reads {
            toggle_disabled(t, "peri-change", writes, move |v, _, cx| {
                entity_change.update(cx, |this, cx| this.set_access(access::AccessNext::writes_change(v), cx));
            })
            .into_any_element()
        } else if writes {
            toggle(t, "peri-change", true, move |v, _, cx| {
                entity_change.update(cx, |this, cx| this.set_access(access::AccessNext::writes_change(v), cx));
            })
            .into_any_element()
        } else {
            toggle(t, "peri-change", false, move |v, _, cx| {
                entity_change.update(cx, |this, cx| this.set_access(access::AccessNext::writes_change(v), cx));
            })
            .into_any_element()
        };
        let access = div()
            .flex()
            .items_center()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child("Read"),
                    )
                    .child(read_toggle),
            )
            .child(button_icon(
                t,
                "peri-read-again",
                "Read again",
                Variant::Default,
                !reads || reading,
                true,
                "ArrowsClockwise",
                "",
                16.0,
                move |_, _, cx| {
                    entity_again.update(cx, |this, cx| this.read_again(cx));
                },
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child("Change"),
                    )
                    .child(change_toggle),
            );
        let access = if content_w <= 720.0 {
            access.flex_1().w_full().justify_end()
        } else {
            // `.rd-peri-access { margin-left: auto; padding-left: var(--s4);
            // border-left: 1px solid var(--line) }`: `margin-left: auto`
            // pushes the access block against the padding, so it ends where
            // the head ends. Measured at 1440, the access block starts at
            // 1046.7 px (the head ends at 1399 px).
            access.ml_auto().pl_4().border_l_1().border_color(Theme::color(t.line))
        };
        let mut head_panel_body = div().flex().flex_col().gap_3().child(head.child(access));
        if let Some(problem) = &detail.problem {
            head_panel_body = head_panel_body.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.warn))
                    .child(problem.clone()),
            );
        }
        let head_panel = div()
            .flex()
            .flex_col()
            .p_4()
            .when(content_w < 640.0, |d| d.p_3())
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel))
            .child(head_panel_body);
        let head_panel = pr("panel-head", head_panel);
        // The sections.
        let narrow = content_w < 640.0;
        let mut column_panels: Vec<(f32, gpui::AnyElement)> = Vec::new();
        let mut wide_panels: Vec<gpui::AnyElement> = Vec::new();
        for section in &detail.schema.sections {
            if is_mouse && section.id == "dpi" {
                let panel = self.dpi_panel(
                    &section.id,
                    &section.title,
                    &section.properties,
                    values,
                    read_only,
                    !writes,
                    view_entity.clone(),
                    narrow,
                );
                column_panels.push((
                    dpi_panel_height(&section.properties, narrow),
                    pr(format!("panel-{}", section.id), panel),
                ));
                continue;
            }
            if is_controller && section.id == "buttons" {
                wide_panels.push(
                    self.controller_panel(detail, values, read_only, !writes, view_entity.clone(), content_w).into_any_element(),
                );
                continue;
            }
            if !section_visible(&section.id, detail.schema.kind, values) {
                continue;
            }
            let properties: Vec<PropertyDef> = if is_controller && section.id.starts_with("profile-") {
                section.properties.iter().filter(|p| !p.key.contains("-paddle-")).cloned().collect()
            } else {
                section.properties.clone()
            };
            let mut list = div().flex().flex_col();
            let schema = properties.clone();
            for (i, def) in schema.iter().enumerate() {
                let value = values.get(&def.key).cloned().unwrap_or_else(|| default_for(&def.kind));
                let row = self.prop_row(def, &value, read_only, !writes, &schema, values, view_entity.clone(), narrow, i + 1 == schema.len());
                list = list.child(pr(format!("row-{}", def.key), row));
            }
            // The mouse Buttons panel is a two-column grid of read-only rows,
            // filled down the first column (`grid-auto-flow: column`): rows
            // 1..3 in column one, 4..6 in column two.
            let panel_body: gpui::AnyElement = if section.id == "buttons" && is_mouse {
                let half = schema.len().div_ceil(2);
                let mut pair_grid = div().flex().gap_4();
                for col in 0..2 {
                    let mut column = div().flex().flex_col().flex_1().min_w(px(0.0));
                    for def in schema.iter().skip(col * half).take(half) {
                        let value = values.get(&def.key).cloned().unwrap_or_else(|| default_for(&def.kind));
                        column = column.child(
                            div()
                                .flex()
                                .flex_col()
                                .justify_center()
                                .gap_2()
                                .min_h(px(44.0))
                                .py_2()
                                .text_size(px(15.0))
                                .line_height(gpui::relative(1.45))
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child(plain_label(&def.label))
                                .child(self.read_only_value(&value)),
                        );
                    }
                    pair_grid = pair_grid.child(column);
                }
                pair_grid.into_any_element()
            } else {
                list.into_any_element()
            };
            column_panels.push((
                section_panel_height(&properties, section.id == "buttons" && is_mouse, narrow),
                pr(
                    format!("panel-{}", section.id),
                    self.panel(&format!("section-{}", section.id), &section.title, panel_body, narrow),
                ),
            ));
        }
        // The sections are CSS multicol columns of 280 px (`columns: 280px
        // auto`, `column-gap: 16px`), balanced across the columns. GPUI has
        // no multicol, so the panels are dealt into fixed-width columns in
        // DOM order by a greedy height budget, as Settings does.
        let cols = section_columns(content_w);
        let heights: Vec<f32> = column_panels.iter().map(|(h, _)| *h).collect();
        let distribution = distribute(&heights, cols);
        let sections = if cols <= 1 {
            let mut col = div().flex().flex_col().gap_4();
            for (_, panel) in column_panels {
                col = col.child(panel);
            }
            col.into_any_element()
        } else {
            let panels: Vec<Option<gpui::AnyElement>> = column_panels
                .into_iter()
                .map(|(_, panel)| Some(panel))
                .collect();
            let mut panels = panels;
            let mut grid = div().flex().flex_wrap().gap_4().min_w(px(0.0));
            for column in distribution {
                let mut col = div().flex().flex_col().gap_4().flex_1().min_w(px(0.0));
                for idx in column {
                    if let Some(panel) = panels[idx].take() {
                        col = col.child(panel);
                    }
                }
                grid = grid.child(col);
            }
            grid.into_any_element()
        };
        let mut stack = div().flex().flex_col().gap_4().child(head_panel).child(sections);
        for panel in wide_panels {
            stack = stack.child(panel);
        }
        if is_controller {
            stack = stack.child(self.panel(
                "rgb-rings",
                "RGB rings",
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("The rings on the pad are not read yet: this build cannot name them."),
                narrow,
            ));
        }
        stack
    }

    /// The width of one device key: a 1 px border each side, `px_3` (12 px)
    /// either side, the 28 px icon, two 8 px gaps and the 8 px state dot
    /// (`.rd-peri-keys .rd-devkey`, `.peripheral-icon`, `.state-dot`). The
    /// name is measured the way GPUI draws it, so the keys row can be
    /// scrolled to the picked key (see `keys_scroll`).
    fn key_width(window: &Window, row: &PeripheralRow) -> f32 {
        let name = crate::shell::text_width(
            window,
            &row.info.name,
            13.333,
            gpui::FontWeight::SEMIBOLD,
            crate::fonts::UI_FAMILY,
        );
        // A battery badge (never sent by the daemon today) adds its icon,
        // its gap and the word after it.
        let battery = if row.battery.is_some() { 4.0 + 14.0 + 4.0 } else { 0.0 };
        78.0 + name + battery
    }

    /// One device key (`button.peripheral-row.rd-devkey`).
    fn device_key(&self, row: &PeripheralRow, view_entity: gpui::Entity<Self>) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.sel_id().as_deref() == Some(row.info.id.0.as_str());
        let icon = match row.info.kind {
            PeripheralKind::Mouse => "Mouse",
            PeripheralKind::GameController => "GameController",
            PeripheralKind::Keyboard => "Keyboard",
        };
        let icon_color = if selected { t.live() } else { Theme::color(t.dim) };
        let id = row.info.id.0.clone();
        let entity = view_entity.clone();
        let key = div()
            .id(gpui::SharedString::from(format!("peri-key-{id}")))
            .flex()
            .items_center()
            .gap_2()
            .flex_none()
            .px_3()
            .py_2()
            .r_md()
            .border_1()
            .border_color(if selected { t.live() } else { Theme::color(t.line_2) })
            .bg(Theme::color(t.panel))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.pick_device(&id, cx));
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_none()
                    .size(px(28.0))
                    .r_sm()
                    .bg(Theme::color(t.sunk))
                    .text_color(icon_color)
                    .child(gpui::svg().path(format!("icons/{icon}.svg")).size(px(18.0)).text_color(icon_color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(13.333))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child(row.info.name.clone()),
            );
        let key = if let Some(battery) = row.battery {
            let battery_icon = if row.charging { "BatteryCharging" } else { "BatteryFull" };
            key.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .flex_none()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(gpui::svg().path(format!("icons/{battery_icon}.svg")).size(px(14.0)).text_color(Theme::color(t.dim)))
                    .child(format!("{battery}%")),
            )
        } else {
            key
        };
        key.child(
            div()
                .flex_none()
                .size(px(8.0))
                .rounded_full()
                .bg(if row.info.connected { Theme::color(t.ok) } else { Theme::color(t.mute) }),
        )
    }
}

/// Sample an SVG path (`M`, `L`, `C`, `Z`) into polyline points, expanding
/// cubic curves into 8 subdivisions each (exact geometry as a polyline).
fn sample_path(d: &str) -> Vec<(f32, f32)> {
    #[derive(Debug)]
    enum Tok {
        Cmd(char),
        Num(f32),
    }
    let mut toks = Vec::new();
    let s = d.as_bytes();
    let mut i = 0;
    while i < s.len() {
        let c = s[i] as char;
        if c.is_ascii_alphabetic() {
            toks.push(Tok::Cmd(c));
            i += 1;
        } else if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() {
            let start = i;
            if c == '-' || c == '+' {
                i += 1;
            }
            let mut seen_digit = false;
            let mut seen_dot = false;
            while i < s.len() {
                let c = s[i] as char;
                if c.is_ascii_digit() {
                    seen_digit = true;
                    i += 1;
                } else if c == '.' && !seen_dot {
                    seen_dot = true;
                    i += 1;
                } else {
                    break;
                }
            }
            if !seen_digit {
                // A bare sign/dot with no digit: skip it.
                continue;
            }
            let num: f32 = d[start..i].parse().unwrap_or(0.0);
            toks.push(Tok::Num(num));
        } else {
            i += 1;
        }
    }
    let mut out: Vec<(f32, f32)> = Vec::new();
    let mut idx = 0;
    let mut cur: Option<(f32, f32)> = None;
    let mut start: Option<(f32, f32)> = None;
    let mut cmd = 'M';
    while idx < toks.len() {
        if let Tok::Cmd(c) = &toks[idx] {
            cmd = *c;
            idx += 1;
            if cmd == 'Z' {
                if let (Some(cur_pt), Some(start_pt)) = (cur, start) {
                    if cur_pt != start_pt {
                        out.push(start_pt);
                    }
                }
                continue;
            }
        }
        match cmd {
            'M' => {
                let x = match toks.get(idx) { Some(Tok::Num(n)) => *n, _ => 0.0 };
                let y = match toks.get(idx + 1) { Some(Tok::Num(n)) => *n, _ => 0.0 };
                cur = Some((x, y));
                start = Some((x, y));
                out.push((x, y));
                idx += 2;
                cmd = 'L';
            }
            'L' => {
                let x = match toks.get(idx) { Some(Tok::Num(n)) => *n, _ => 0.0 };
                let y = match toks.get(idx + 1) { Some(Tok::Num(n)) => *n, _ => 0.0 };
                cur = Some((x, y));
                out.push((x, y));
                idx += 2;
            }
            'C' => {
                let nums: Vec<f32> = toks[idx..(idx + 6).min(toks.len())]
                    .iter()
                    .filter_map(|t| match t { Tok::Num(n) => Some(*n), _ => None })
                    .collect();
                if nums.len() < 6 {
                    break;
                }
                let p0 = cur.unwrap_or((0.0, 0.0));
                let (c1x, c1y, c2x, c2y, x, y) = (nums[0], nums[1], nums[2], nums[3], nums[4], nums[5]);
                for k in 1..=8 {
                    let t = k as f32 / 8.0;
                    let mt = 1.0 - t;
                    let px = mt * mt * mt * p0.0 + 3.0 * mt * mt * t * c1x + 3.0 * mt * t * t * c2x + t * t * t * x;
                    let py = mt * mt * mt * p0.1 + 3.0 * mt * mt * t * c1y + 3.0 * mt * t * t * c2y + t * t * t * y;
                    out.push((px, py));
                }
                cur = Some((x, y));
                idx += 6;
            }
            _ => {
                idx += 1;
                if idx >= toks.len() {
                    break;
                }
            }
        }
    }
    out
}

/// Paint a pad body path (`pad-body`: hairline, no fill).
#[allow(clippy::too_many_arguments)]
fn paint_body_path(
    window: &mut Window,
    at: &dyn Fn(f32, f32) -> gpui::Point<gpui::Pixels>,
    d: &str,
    transform: Option<(f32, f32, f32)>,
    colour: gpui::Rgba,
    scale: f32,
) {
    let pts = sample_path(d);
    let mut b = gpui::PathBuilder::stroke(px(2.0 * scale));
    let mut first = true;
    for (x, y) in pts {
        let (tx, ty) = match transform {
            Some((tx, ty, _)) => (x * REAR_SCALE + tx, y * REAR_SCALE + ty),
            None => (x, y),
        };
        if first {
            b.move_to(at(tx, ty));
            first = false;
        } else {
            b.line_to(at(tx, ty));
        }
    }
    if let Ok(path) = b.build() {
        window.paint_path(path, colour);
    }
}

/// Paint one glyph (`Glyph` in ControllerOutline) at `centre`.
fn paint_glyphs(window: &mut Window, centre: gpui::Point<gpui::Pixels>, name: &str, colour: gpui::Rgba, scale: f32) {
    let s = scale;
    let at = |x: f32, y: f32| point(centre.x + px(x * s), centre.y + px(y * s));
    match name {
        "select" => {
            let mut b = gpui::PathBuilder::stroke(px(1.5 * s));
            // Two rounded rects: approximate with a stroked rect outline.
            for (x, y, w, h) in [(-5.0, -3.0, 7.0, 7.0), (-2.0, -6.0, 7.0, 7.0)] {
                b.move_to(at(x, y));
                b.line_to(at(x + w, y));
                b.line_to(at(x + w, y + h));
                b.line_to(at(x, y + h));
                b.line_to(at(x, y));
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, colour);
            }
        }
        "start" => {
            let mut b = gpui::PathBuilder::stroke(px(1.5 * s));
            for (x1, y1, x2, y2) in [(-5.0, -4.0, 5.0, -4.0), (-5.0, 0.0, 5.0, 0.0), (-5.0, 4.0, 5.0, 4.0)] {
                b.move_to(at(x1, y1));
                b.line_to(at(x2, y2));
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, colour);
            }
        }
        "home" => {
            let mut b = gpui::PathBuilder::stroke(px(1.5 * s));
            for (x1, y1, x2, y2) in [(-6.0, 1.0, 0.0, -6.0), (0.0, -6.0, 6.0, 1.0), (-4.0, 0.0, -4.0, 6.0), (-4.0, 6.0, 4.0, 6.0), (4.0, 6.0, 4.0, 0.0)] {
                b.move_to(at(x1, y1));
                b.line_to(at(x2, y2));
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, colour);
            }
        }
        "menu" => {
            let mut b = gpui::PathBuilder::stroke(px(1.5 * s));
            for (x1, y1, x2, y2) in [(-5.0, 0.0, -4.0, 0.0), (-0.5, 0.0, 0.5, 0.0), (4.0, 0.0, 5.0, 0.0)] {
                b.move_to(at(x1, y1));
                b.line_to(at(x2, y2));
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, colour);
            }
        }
        "up" => {
            let mut b = gpui::PathBuilder::stroke(px(1.5 * s));
            for (x1, y1, x2, y2) in [(-4.0, 2.0, 0.0, -3.0), (0.0, -3.0, 4.0, 2.0)] {
                b.move_to(at(x1, y1));
                b.line_to(at(x2, y2));
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, colour);
            }
        }
        "down" => {
            let mut b = gpui::PathBuilder::stroke(px(1.5 * s));
            for (x1, y1, x2, y2) in [(-4.0, -2.0, 0.0, 3.0), (0.0, 3.0, 4.0, -2.0)] {
                b.move_to(at(x1, y1));
                b.line_to(at(x2, y2));
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, colour);
            }
        }
        "left" => {
            let mut b = gpui::PathBuilder::stroke(px(1.5 * s));
            for (x1, y1, x2, y2) in [(2.0, -4.0, -3.0, 0.0), (-3.0, 0.0, 2.0, 4.0)] {
                b.move_to(at(x1, y1));
                b.line_to(at(x2, y2));
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, colour);
            }
        }
        _ => {
            let mut b = gpui::PathBuilder::stroke(px(1.5 * s));
            for (x1, y1, x2, y2) in [(-2.0, -4.0, 3.0, 0.0), (3.0, 0.0, -2.0, 4.0)] {
                b.move_to(at(x1, y1));
                b.line_to(at(x2, y2));
            }
            if let Ok(path) = b.build() {
                window.paint_path(path, colour);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The page.

impl Render for PeripheralsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.width = f32::from(window.window_bounds().get_bounds().size.width);
        if probe_on() {
            line_metrics_probe(window);
        }
        let t = &self.theme;
        let view_entity = cx.entity();
        // The content box width (window minus rail and content padding), the
        // box React's @container page rules measure.
        let win_w = self.width;
        let rail_w = if win_w <= 720.0 { 0.0 } else if win_w < 1280.0 { 64.0 } else { 200.0 };
        let content_w = win_w - rail_w - if win_w <= 720.0 { 32.0 } else { 48.0 };
        let phone = win_w <= 720.0;

        let rows: Vec<PeripheralRow> = self.list.clone().unwrap_or_default();
        let list_state = list_state(self.list.as_ref(), self.list_error.as_deref(), self.list_busy).to_string();
        let detail = self.detail();
        let no_det = no_detail(self.list.as_ref(), self.list_error.as_deref());
        let reading = self.reading();
        let refreshing = detail.is_some() && reading;

        // The device keys row.
        let keys: gpui::AnyElement = match list_state.as_str() {
            "loading" => {
                if self.list_loading_shown {
                    self.list_shape().into_any_element()
                } else {
                    div().into_any_element()
                }
            }
            "error" => {
                let message = self.list_error.clone().unwrap_or_default();
                self.error_panel_list("peri-list-error", "The peripherals could not be listed", "canvas-rgb could not look for the peripherals. Nothing was changed.", &message, view_entity.clone())
                    .into_any_element()
            }
            "empty" => self
                .panel(
                    "peri-empty",
                    "No device found",
                    div()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child("None that canvas-rgb can configure is plugged in."),
                    content_w < 640.0,
                )
                .into_any_element(),
            _ => {
                let mut row = div().flex().gap_2().min_w(px(0.0));
                for r in &rows {
                    row = row.child(self.device_key(r, view_entity.clone()));
                }
                // The keys' boxes, so `keys_scroll` can bring the picked key
                // into view. Relative to the first key, which is the same
                // whether the row is scrolled or not.
                row = row.ml(px(-self.keys_scroll(window, content_w)));
                row.into_any_element()
            }
        };

        // The save-error line (drawn only when one is showing: an empty
        // element here would still collect the page gap and push the detail
        // pane 16 px down).
        let save_line = self.save_error().map(|err| {
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.warn))
                .child(err)
                .into_any_element()
        });

        // The detail pane.
        let detail_pane: gpui::AnyElement = if no_det {
            div().into_any_element()
        } else {
            let picked_kind = rows
                .iter()
                .find(|r| Some(&r.info.id.0) == self.sel_id().as_ref())
                .map(|r| r.info.kind)
                .unwrap_or_else(|| rows.first().map(|r| r.info.kind).unwrap_or(PeripheralKind::Mouse));
            let state = detail_state(detail.as_ref(), self.detail_error.as_deref());
            match state {
                "error" => {
                    let message = self.detail_error.clone().unwrap_or_default();
                    let summary = if self.change_saved {
                        "The change was saved, but the page could not be refreshed."
                    } else {
                        "canvas-rgb could not read that device. Nothing was changed."
                    };
                    self.error_panel_detail("peri-detail-error", "The device could not be read", summary, &message, view_entity.clone())
                        .into_any_element()
                }
                "ready" => {
                    let detail_unwrapped = detail.clone().unwrap();
                    let values = self.values();
                    let body = self.detail_body(&detail_unwrapped, &values, view_entity.clone(), content_w);
                    let body: gpui::AnyElement = if refreshing {
                        div().opacity(0.55).child(body).into_any_element()
                    } else {
                        body.into_any_element()
                    };
                    let stage = if reading {
                        let (label, done, total) = read_stage(
                            self.progress.as_ref(),
                            noun_of(picked_kind),
                        );
                        Some((label, done, total))
                    } else {
                        None
                    };
                    let mut col = div().flex().flex_col().gap_4().min_w(px(0.0));
                    if let Some((label, done, total)) = stage {
                        let shown = self.detail_loading_shown || self.refreshing_shown;
                        if shown {
                            col = col.child(self.stage_line_el(&label, done, total));
                        }
                    }
                    col.child(body).into_any_element()
                }
                _ => {
                    if self.detail_loading_shown {
                        self.detail_shape(picked_kind).into_any_element()
                    } else {
                        div().into_any_element()
                    }
                }
            }
        };

        let keys = pr("keys", keys);
        let mut page = div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .flex_none()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(keys),
            );
        if let Some(line) = save_line {
            page = page.child(line);
        }
        // The detail pane scrolls inside its own pane on desktop (React
        // `.rd-peripherals .peripheral-detail` `overflow-y: auto`); on a phone
        // the page scrolls instead.
        // #208: `capture-gpui.ps1 -Scroll` reaches the panels below the fold.
        // React's scrolled shot moves the detail pane at a desk and the
        // document on a phone; this does the same, so the keys row stays put
        // on a desk and moves with the page on a phone.
        let scroll = capture_scroll();
        let detail_wrapped: gpui::AnyElement = if phone {
            div().flex().flex_col().flex_1().min_h(px(0.0)).child(detail_pane).into_any_element()
        } else {
            let inner: gpui::AnyElement = if scroll > 0.0 {
                div().relative().top(px(-scroll)).child(detail_pane).into_any_element()
            } else {
                detail_pane
            };
            div()
                .id(gpui::SharedString::from("peri-detail-scroll"))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(inner)
                .into_any_element()
        };
        page = page.child(detail_wrapped);
        if phone && scroll > 0.0 {
            // The phone page scrolls as a document (React); the shift is the
            // whole of the scrolling a still capture needs (`capture_scroll`).
            return page.relative().top(px(-scroll)).into_any_element();
        }
        page.into_any_element()
    }
}

impl PeripheralsView {
    /// The list error panel with its own retry (`retryList`).
    fn error_panel_list(
        &self,
        id: &str,
        title: &str,
        summary: &str,
        message: &str,
        view_entity: gpui::Entity<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let retry = button(t, &format!("{id}-try-again"), "Try again", Variant::Default, self.list_busy, move |_, _, cx| {
            view_entity.update(cx, |this, cx| this.retry_list(cx));
        });
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
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.down))
                            .child(summary.to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child(message.to_string()),
                    )
                    .child(div().flex().child(retry)),
            )
    }

    /// The detail error panel with its own retry (the selected device now).
    fn error_panel_detail(
        &self,
        id: &str,
        title: &str,
        summary: &str,
        message: &str,
        view_entity: gpui::Entity<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let retry = button(t, &format!("{id}-try-again"), "Try again", Variant::Default, false, move |_, _, cx| {
            view_entity.update(cx, |this, cx| {
                this.detail_error = None;
                this.change_saved = false;
                if let Some(id) = this.sel_id() {
                    this.load_detail(&id, false, false, cx);
                }
            });
        });
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
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.down))
                            .child(summary.to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child(message.to_string()),
                    )
                    .child(div().flex().child(retry)),
            )
    }
}

impl PeripheralsView {
    /// The toast after a saved write (React `notify("The settings are on the device")`, tone ok;
    /// `PeripheralsView.tsx:371`).
    pub fn toast_after_apply() -> Toast {
        Toast { text: "The settings are on the device".to_string(), tone: "ok" }
    }
}

// ---------------------------------------------------------------------------
// Tests: the pure logic above, checked without a window or the network.

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::DeviceId;

    fn test_info(id: &str, kind: PeripheralKind, name: &str) -> PeripheralInfo {
        PeripheralInfo {
            id: DeviceId(id.into()),
            name: name.into(),
            vendor: "Beken".into(),
            kind,
            adapter: "test".into(),
            serial: String::new(),
            location: "hid:1d57:fa60".into(),
            connected: true,
        }
    }

    fn number(v: f64) -> PropValue {
        PropValue::Number(v)
    }

    #[test]
    fn the_apply_toast_is_reacts_words_and_tone() {
        // React `PeripheralsView.tsx:371`: notify("The settings are on the
        // device") with the default tone, ok (#246).
        let toast = PeripheralsView::toast_after_apply();
        assert_eq!(toast.text, "The settings are on the device");
        assert_eq!(toast.tone, "ok");
    }

    #[test]
    fn plain_label_drops_the_profile_repeat() {
        assert_eq!(plain_label("Left stick range start (%) (profile 1)"), "Left stick range start (%)");
        assert_eq!(plain_label("Left motor strength (%, profile 2)"), "Left motor strength (%)");
        assert_eq!(plain_label("Profile"), "Profile");
        assert_eq!(plain_label("Polling rate (Hz)"), "Polling rate (Hz)");
    }

    #[test]
    fn choice_words_cover_the_fixture_ids() {
        assert_eq!(choice_word("static"), "Static");
        assert_eq!(choice_word("static-dpi"), "Static (DPI colour)");
        assert_eq!(choice_word("color-breathing"), "Colour breathing");
        assert_eq!(choice_word("paddle-1"), "Paddle 1 (P1)");
        assert_eq!(choice_word("a"), "A");
        assert_eq!(choice_word("left-bumper"), "Left bumper (LB)");
        assert_eq!(choice_word("some-unknown-thing"), "Some unknown thing");
        assert_eq!(choice_word("1000"), "1000");
    }

    #[test]
    fn kind_words_and_nouns_match_react() {
        assert_eq!(kind_word(PeripheralKind::Mouse), "Mouse");
        assert_eq!(kind_word(PeripheralKind::GameController), "Game controller");
        assert_eq!(kind_word(PeripheralKind::Keyboard), "Keyboard");
        assert_eq!(noun_of(PeripheralKind::Mouse), "mouse");
        assert_eq!(noun_of(PeripheralKind::GameController), "controller");
        assert_eq!(noun_of(PeripheralKind::Keyboard), "keyboard");
    }

    #[test]
    fn read_stage_maps_daemon_stages_to_words() {
        assert_eq!(read_stage(None, "mouse").0, "Reading settings");
        let opening = ReadProgress { stage: ReadStage::Opening, done: 0, total: 0 };
        assert_eq!(read_stage(Some(&opening), "mouse").0, "Opening the mouse");
        let waking = ReadProgress { stage: ReadStage::Waking, done: 0, total: 0 };
        assert_eq!(read_stage(Some(&waking), "controller").0, "Waking the controller");
        let reading = ReadProgress::reading(3, 5);
        let (label, done, total) = read_stage(Some(&reading), "mouse");
        assert_eq!((label.as_str(), done, total), ("Reading settings", 3, 5));
        assert_eq!(stage_line(&label, done, total), "Reading settings · 3 of 5");
        assert_eq!(stage_line("Reading settings", 0, 0), "Reading settings");
    }

    #[test]
    fn decimals_of_matches_the_step() {
        assert_eq!(decimals_of(1.0), 0);
        assert_eq!(decimals_of(2.0), 0);
        assert_eq!(decimals_of(0.5), 1);
        assert_eq!(decimals_of(50.0), 0);
    }

    #[test]
    fn dpi_defaults_match_react_when_keys_are_absent() {
        let values = Props::new();
        assert_eq!(dpi_stage_count(&values), 6);
        assert_eq!(dpi_active_stage(&values), "1");
        let stages = dpi_stages(&values);
        assert_eq!(stages.len(), 6);
        assert_eq!(stages[0], DpiStage { n: 1, dpi: 0.0, colour: "#ffffff".into() });
        let mut values = Props::new();
        values.insert("dpi-stage-count".into(), number(6.0));
        values.insert("dpi-stage-1".into(), number(800.0));
        values.insert("dpi-stage-1-colour".into(), PropValue::Color("#ff0000".parse().unwrap()));
        assert_eq!(dpi_stages(&values)[0].dpi, 800.0);
        assert_eq!(dpi_stages(&values)[0].colour, "#ff0000");
    }

    #[test]
    fn dpi_stage_chips_use_the_fixture_values() {
        let mut values = Props::new();
        values.insert("dpi-stage-count".into(), number(6.0));
        values.insert("dpi-active-stage".into(), PropValue::Choice("1".into()));
        for (i, dpi) in [800, 1200, 1600, 3200, 5600, 26000].iter().enumerate() {
            values.insert(format!("dpi-stage-{}", i + 1), number(*dpi as f64));
        }
        let stages = dpi_stages(&values);
        assert_eq!(stages.len(), 6);
        assert_eq!(stages[0].dpi, 800.0);
        assert_eq!(stages[5].dpi, 26000.0);
    }

    #[test]
    fn dpi_others_reproduces_the_229_absence() {
        // #229: the filter drops `dpi-stage-count` too, so the control never
        // renders. The port must reproduce the absence, not fix it here.
        let properties = vec![
            PropertyDef { key: "dpi-stage-count".into(), label: "DPI stages in use".into(), kind: PropKind::Number { min: 1.0, max: 8.0, step: 1.0, default: 6.0 } },
            PropertyDef { key: "dpi-stage-1".into(), label: "DPI stage 1".into(), kind: PropKind::Number { min: 50.0, max: 26000.0, step: 50.0, default: 800.0 } },
            PropertyDef { key: "dpi-active-stage".into(), label: "Active DPI stage".into(), kind: PropKind::Choice { options: vec!["1".into()], default: "1".into() } },
        ];
        let others = dpi_others(&properties);
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].key, "dpi-active-stage");
    }

    #[test]
    fn dpi_chip_aria_label_names_the_active_stage() {
        let label = "DPI stage 1, 800";
        let active_label = format!("{label}, the active stage");
        assert_eq!(active_label, "DPI stage 1, 800, the active stage");
    }

    #[test]
    fn dpi_pick_toggles_back_to_none() {
        let mut picked: Option<usize> = None;
        let pick = |picked: &mut Option<usize>, n: usize| {
            *picked = if *picked == Some(n) { None } else { Some(n) };
        };
        pick(&mut picked, 3);
        assert_eq!(picked, Some(3));
        pick(&mut picked, 3);
        assert_eq!(picked, None);
        pick(&mut picked, 2);
        pick(&mut picked, 4);
        assert_eq!(picked, Some(4));
    }

    #[test]
    fn place_of_strips_the_button_and_paddle_prefixes() {
        assert_eq!(place_of("button-a"), "a");
        assert_eq!(place_of("button-dpad-up"), "dpad-up");
        assert_eq!(place_of("profile-1-paddle-2"), "paddle-2");
        assert_eq!(place_of("active-profile"), "active-profile");
    }

    #[test]
    fn hit_rect_is_never_less_than_34_units() {
        let spot = front_spot("select").unwrap().spot;
        // The drawn size is 26, the hit 34.
        let (x, y, w, h) = hit_rect(&spot);
        assert_eq!((w, h), (34.0, 34.0));
        assert_eq!((x, y), (132.0 - 17.0, 86.0 - 17.0));
        let l2 = front_spot("l2").unwrap().spot;
        let (_, _, w, h) = hit_rect(&l2);
        assert_eq!((w, h), (60.0, 34.0));
    }

    #[test]
    fn front_layout_matches_controller_outline() {
        let front_keys = ["a", "b", "x", "y", "l1", "r1", "l2", "r2", "l3", "r3", "select", "start", "menu", "home", "dpad-up", "dpad-down", "dpad-left", "dpad-right"];
        for key in front_keys {
            assert!(front_spot(key).is_some(), "{key} has a front spot");
        }
        assert!(front_spot("paddle-3").is_some());
        assert!(front_spot("paddle-4").is_some());
        assert!(rear_spot("paddle-1").is_some());
        assert!(rear_spot("paddle-2").is_some());
        assert!(front_spot("paddle-1").is_none());
        let a = front_spot("a").unwrap();
        assert!(matches!(a.mark, Mark::Text(ref t) if t == "A"));
        assert!(matches!(front_spot("select").unwrap().mark, Mark::Glyph("select")));
    }

    #[test]
    fn mirror_flips_across_the_centre_line() {
        let l2 = Spot { cx: 78.0, cy: 17.0, w: 60.0, h: 24.0, round: false };
        let r2 = mirror(l2, Mark::Text("R2".into()));
        assert_eq!(r2.spot.cx, 340.0 - 78.0);
        assert_eq!(r2.spot.cy, 17.0);
        assert!(matches!(r2.mark, Mark::Text(ref t) if t == "R2"));
    }

    #[test]
    fn body_and_dpad_paths_sample_without_panic() {
        let body = sample_path(BODY_PATH);
        assert!(!body.is_empty());
        assert!((body[0].0 - 78.0).abs() < 0.001);
        assert!((body[0].1 - 58.0).abs() < 0.001);
        let dpad = sample_path(DPAD_PATH);
        assert!(!dpad.is_empty());
        // The rear transform maps the body into the 200x124 box.
        let (x, y) = (body[0].0 * REAR_SCALE + REAR_TX, body[0].1 * REAR_SCALE + REAR_TY);
        assert!((0.0..=200.0).contains(&x) && (0.0..=124.0).contains(&y));
    }

    #[test]
    fn same_value_needs_same_type_and_value() {
        assert!(same_value(Some(&number(8.0)), &number(8.0)));
        assert!(!same_value(Some(&number(8.0)), &number(9.0)));
        assert!(!same_value(Some(&number(8.0)), &PropValue::Choice("8".into())));
        assert!(!same_value(None, &number(8.0)));
        assert!(same_value(Some(&PropValue::Choice("a".into())), &PropValue::Choice("a".into())));
    }

    #[test]
    fn changed_props_keeps_only_differing_staged_entries() {
        let mut held = Props::new();
        held.insert("polling".into(), PropValue::Choice("1000".into()));
        held.insert("debounce-ms".into(), number(8.0));
        let mut staged = Props::new();
        staged.insert("polling".into(), PropValue::Choice("1000".into()));
        staged.insert("debounce-ms".into(), number(10.0));
        let changed = changed_props(&staged, &held);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed.get("debounce-ms"), Some(&number(10.0)));
    }

    #[test]
    fn stale_after_apply_drops_only_the_sent_values() {
        let mut staged = Props::new();
        staged.insert("a".into(), number(1.0));
        staged.insert("b".into(), number(2.0));
        let mut sent = Props::new();
        sent.insert("a".into(), number(1.0));
        let left = stale_after_apply(&staged, &sent);
        assert_eq!(left.len(), 1);
        assert_eq!(left.get("b"), Some(&number(2.0)));
    }

    #[test]
    fn keep_live_drops_gone_devices() {
        let mut prev: HashMap<String, u32> = HashMap::new();
        prev.insert("a".into(), 1);
        prev.insert("gone".into(), 2);
        let live: std::collections::HashSet<String> = ["a".to_string()].into_iter().collect();
        let kept = keep_live(&prev, &live);
        assert_eq!(kept.len(), 1);
        assert!(kept.contains_key("a"));
    }

    #[test]
    fn active_profile_defaults_to_1() {
        assert_eq!(active_profile(&Props::new()), "1");
        let mut values = Props::new();
        values.insert("active-profile".into(), PropValue::Choice("2".into()));
        assert_eq!(active_profile(&values), "2");
    }

    #[test]
    fn only_the_active_profile_section_shows() {
        let mut values = Props::new();
        assert!(section_visible("profile-1", PeripheralKind::GameController, &values));
        assert!(!section_visible("profile-2", PeripheralKind::GameController, &values));
        assert!(section_visible("profiles", PeripheralKind::GameController, &values));
        values.insert("active-profile".into(), PropValue::Choice("2".into()));
        assert!(!section_visible("profile-1", PeripheralKind::GameController, &values));
        assert!(section_visible("profile-2", PeripheralKind::GameController, &values));
        // A mouse draws every section.
        assert!(section_visible("profile-2", PeripheralKind::Mouse, &values));
    }

    #[test]
    fn first_device_is_selected_with_no_pick() {
        let rows = vec![
            PeripheralRow { info: test_info("a", PeripheralKind::Mouse, "Mouse"), battery: None, charging: false },
            PeripheralRow { info: test_info("b", PeripheralKind::GameController, "Pad"), battery: None, charging: false },
        ];
        assert_eq!(selected_id(None, Some(&rows)).as_deref(), Some("a"));
        assert_eq!(selected_id(Some("b"), Some(&rows)).as_deref(), Some("b"));
        assert_eq!(selected_id(None, None), None);
        assert_eq!(selected_id(None, Some(&Vec::<PeripheralRow>::new())), None);
    }

    #[test]
    fn list_and_detail_states_match_react() {
        assert_eq!(list_state(None, None, false), "loading");
        assert_eq!(list_state(None, Some("x"), false), "error");
        assert_eq!(list_state(None, Some("x"), true), "loading");
        let empty = vec![];
        assert_eq!(list_state(Some(&empty), None, false), "empty");
        let rows = vec![PeripheralRow { info: test_info("a", PeripheralKind::Mouse, "Mouse"), battery: None, charging: false }];
        assert_eq!(list_state(Some(&rows), None, false), "ready");
        assert!(no_detail(Some(&empty), None));
        assert!(no_detail(None, Some("x")));
        assert!(!no_detail(Some(&rows), None));
        assert_eq!(detail_state(None, Some("x")), "error");
        assert_eq!(detail_state(None, None), "loading");
        let detail = PeripheralDetail {
            info: test_info("a", PeripheralKind::Mouse, "Mouse"),
            schema: canvas_proto::peripheral::PeripheralSchema {
                device: DeviceId("a".into()),
                kind: PeripheralKind::Mouse,
                sections: vec![],
                read_only: vec![],
                unreported: vec![],
            },
            values: Props::new(),
            access: canvas_proto::peripheral::PeripheralAccess { reads: true, writes: false },
            problem: None,
        };
        assert_eq!(detail_state(Some(&detail), None), "ready");
    }

    #[test]
    fn quantise_step_clamps_and_snaps() {
        assert_eq!(quantise_step(999.0, 1.0, 8.0, 1.0), 8.0);
        assert_eq!(quantise_step(0.0, 0.5, 30.0, 0.5), 0.5);
        assert_eq!(quantise_step(830.0, 50.0, 26000.0, 50.0), 850.0);
        assert_eq!(quantise_step(825.0, 50.0, 26000.0, 50.0), 850.0);
        assert_eq!(quantise_step(810.0, 50.0, 26000.0, 50.0), 800.0);
    }

    #[test]
    fn hue_and_angle_wrap_to_a_turn() {
        assert_eq!(hue_step(359.0, 3.0), 2.0);
        assert_eq!(hue_step(0.0, -3.0), 357.0);
        assert_eq!(angle_step(359.0, 1.0), 0.0);
        assert_eq!(angle_now(359.6), 0);
        assert_eq!(angle_now(359.0), 359);
        assert_eq!(angle_now(-1.0), 359);
    }

    #[test]
    fn colour_swatches_exclude_the_own_colour() {
        let properties = vec![
            PropertyDef { key: "led-colour".into(), label: "LED colour".into(), kind: PropKind::Color { default: "#ef3fff".parse().unwrap() } },
            PropertyDef { key: "dpi-stage-1-colour".into(), label: "DPI stage 1 colour".into(), kind: PropKind::Color { default: "#ff0000".parse().unwrap() } },
            PropertyDef { key: "polling".into(), label: "Polling rate (Hz)".into(), kind: PropKind::Choice { options: vec!["1000".into()], default: "1000".into() } },
        ];
        let mut values = Props::new();
        values.insert("led-colour".into(), PropValue::Color("#ef3fff".parse().unwrap()));
        values.insert("dpi-stage-1-colour".into(), PropValue::Color("#ff0000".parse().unwrap()));
        let swatches = colour_swatches(&properties, &values, "led-colour", "#ef3fff");
        assert_eq!(swatches.len(), 1);
        assert_eq!(swatches[0].0, "#ff0000");
    }

    #[test]
    fn format_number_drops_the_point_for_whole_numbers() {
        assert_eq!(format_number(800.0, 0), "800");
        assert_eq!(format_number(0.5, 1), "0.5");
        assert_eq!(format_number(26000.0, 0), "26000");
    }

    #[test]
    fn access_reads_off_also_turns_writes_off() {
        let next = access::AccessNext::reads_change(false);
        assert_eq!(next.reads, Some(false));
        assert_eq!(next.writes, Some(false));
        let next = access::AccessNext::reads_change(true);
        assert_eq!(next.reads, Some(true));
        assert_eq!(next.writes, None);
        let next = access::AccessNext::writes_change(true);
        assert_eq!(next.reads, None);
        assert_eq!(next.writes, Some(true));
    }

    #[test]
    fn row_deserialises_with_and_without_a_battery() {
        let without = r#"{"id":"a","name":"Mouse","vendor":"Beken","kind":"mouse","adapter":"x","serial":"","location":"hid:1","connected":true}"#;
        let row: PeripheralRow = serde_json::from_str(without).unwrap();
        assert_eq!(row.battery, None);
        assert!(!row.charging);
        let with = r#"{"id":"a","name":"Mouse","vendor":"Beken","kind":"mouse","adapter":"x","serial":"","location":"hid:1","connected":true,"battery":87,"charging":false}"#;
        let row: PeripheralRow = serde_json::from_str(with).unwrap();
        assert_eq!(row.battery, Some(87));
    }

    #[test]
    fn found_context_reports_the_count_or_zero() {
        assert_eq!(found_context(None), "0 found");
        assert_eq!(found_context(Some(0)), "0 found");
        assert_eq!(found_context(Some(2)), "2 found");
    }

    #[test]
    fn section_columns_follow_the_280px_multicol() {
        assert_eq!(section_columns(1192.0), 4);
        assert_eq!(section_columns(1672.0), 5);
        assert_eq!(section_columns(912.0), 3);
        assert_eq!(section_columns(720.0), 2);
        assert_eq!(section_columns(358.0), 1);
    }

    #[test]
    fn distribute_fills_columns_in_dom_order() {
        // Six mouse panels at 1440: DPI+Performance, Lighting+Power, Buttons+Profile, empty.
        let heights = [188.0, 168.0, 272.0, 168.0, 196.0, 116.0];
        let cols = distribute(&heights, 4);
        assert_eq!(cols.len(), 4);
        let flat: Vec<usize> = cols.into_iter().flatten().collect();
        assert_eq!(flat, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn section_heights_cover_the_panels() {
        let number = vec![PropertyDef { key: "k".into(), label: "l".into(), kind: PropKind::Number { min: 0.0, max: 1.0, step: 1.0, default: 0.0 } }];
        assert!(section_panel_height(&number, false, false) > 100.0);
        let buttons: Vec<PropertyDef> = (0..6)
            .map(|i| PropertyDef { key: format!("button-{i}"), label: "l".into(), kind: PropKind::Choice { options: vec!["a".into()], default: "a".into() } })
            .collect();
        assert!(section_panel_height(&buttons, true, false) > section_panel_height(&number, false, false));
    }
}
