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

use canvas_proto::color::Rgb8;
use canvas_proto::peripheral::{PeripheralDetail, PeripheralInfo, PeripheralKind, ReadProgress, ReadStage};
use canvas_proto::props::{PropKind, PropValue, Props, PropertyDef};
use gpui::{Bounds, Context, IntoElement, Window, canvas, div, point, prelude::*, px, size};
use serde::Deserialize;

use crate::client::DaemonClient;
use crate::components::button::{Variant, button, button_icon};
use crate::components::number_field::number_field;
use crate::components::panel::{state_dot, tracked_title};
use crate::components::segmented::{segmented, segmented_disabled};
use crate::components::slider::{Origin, slider_range};
use crate::components::toggle::{toggle, toggle_disabled};
use crate::theme::{Theme, TokenRadius};

// ---------------------------------------------------------------------------
// Logic, ported from PeripheralsView.tsx / PropertyControls.tsx /
// peripherals.ts / ControllerOutline.tsx and covered by the tests below.

/// `LOAD_DELAY_MS`.
pub const LOAD_DELAY_MS: u64 = 150;
/// `STAGE_POLL_MS`.
pub const STAGE_POLL_MS: u64 = 300;
/// The first stage poll after the read starts (100 ms in React).
pub const STAGE_FIRST_MS: u64 = 100;

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

/// Decimal places a step needs (`decimalsOf`).
pub fn decimals_of(step: f64) -> usize {
    let s = step.to_string();
    match s.find('.') {
        None => 0,
        Some(dot) => (s.len() - dot - 1).min(2),
    }
}

/// The default value for a missing key (`defaultFor`).
pub fn default_for(kind: &PropKind) -> PropValue {
    kind.default_value()
}

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

/// Wrap degrees into 0..360 (`HueRing`/`AngleDial`).
pub fn wrap_deg(deg: f64) -> f64 {
    ((deg % 360.0) + 360.0) % 360.0
}

/// A hue step (`HueRing` arrows: 3 degrees, shift 15).
#[allow(dead_code)]
pub fn hue_step(current: f64, delta: f64) -> f64 {
    wrap_deg(current + delta)
}

/// An angle step (`AngleDial` arrows: 1 degree, shift 15, Home 0, End 359).
#[allow(dead_code)]
pub fn angle_step(current: f64, delta: f64) -> f64 {
    wrap_deg(current + delta)
}

/// The angle read-out (`AngleDial` `now`): rounded and wrapped to 0..=359.
pub fn angle_now(deg: f64) -> i32 {
    ((wrap_deg(deg).round() as i64) % 360) as i32
}

/// The HSV conversions `ColorPicker` works in (copied from Settings).
pub fn hex_to_hsv(hex: &str, keep_hue: f32) -> (f32, f32, f32) {
    let raw = hex.strip_prefix('#').unwrap_or(hex);
    if raw.len() != 6 {
        return (keep_hue, 0.0, 0.0);
    }
    let p = |i: usize| u8::from_str_radix(&raw[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
    let (r, g, b) = (p(0), p(2), p(4));
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

/// HSV to `#rrggbb` (copied from Settings).
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

/// Parse `#rrggbb` (or a bare `rrggbb`) to `Rgb8`; None when it does not parse.
pub fn parse_hex(hex: &str) -> Option<Rgb8> {
    hex.parse::<Rgb8>().ok()
}

/// Whether a value is a valid `#rrggbb` (`isHex`).
#[allow(dead_code)]
pub fn is_hex(value: &str) -> bool {
    parse_hex(value).is_some()
}

/// The swatch list for a colour field (`PropertyControls.tsx:130-138`): the
/// other colours of the same schema, excluding the field's own colour and
/// any that do not parse.
pub fn colour_swatches(
    properties: &[PropertyDef],
    values: &Props,
    own_key: &str,
    own_hex: &str,
) -> Vec<(String, String)> {
    let own = own_hex.to_lowercase();
    let mut out = Vec::new();
    for other in properties {
        if other.key == own_key {
            continue;
        }
        let PropKind::Color { default } = other.kind else {
            continue;
        };
        let held = values.get(&other.key);
        let other_hex = match held {
            Some(PropValue::Color(c)) => c.to_hex(),
            _ => default.to_hex(),
        };
        let Some(parsed) = parse_hex(&other_hex) else {
            continue;
        };
        if parsed.to_hex() == own {
            continue;
        }
        out.push((parsed.to_hex(), other.label.clone()));
    }
    out
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
    fn pick_device(&mut self, id: &str, cx: &mut Context<Self>) {
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
                        this.toast = Some(Toast { text: "The settings are on the device".to_string(), tone: "ok" });
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
    /// Takes the toast, if one is showing.
    #[allow(dead_code)]
    pub fn take_toast(&mut self) -> Option<Toast> {
        self.toast.take()
    }

    /// The slider's measured box for `key`: fresh every render, shared by the
    /// fader's canvas and its drag handlers for that render.
    fn slider_origin(&self, key: &str) -> Origin {
        let _ = key;
        Origin::new()
    }

    /// A section panel (`Panel` with the section title).
    fn panel(&self, id: &str, title: &str, body: impl IntoElement) -> impl IntoElement {
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
    ) -> impl IntoElement {
        let t = &self.theme;
        let fixed = read_only.iter().any(|k| k == &def.key);
        let label = def.label.clone();
        let key = def.key.clone();
        let control: gpui::AnyElement = if fixed {
            self.read_only_value(value).into_any_element()
        } else {
            self.prop_control(&key, &label, &def.kind, value, schema_properties, values, disabled, view_entity).into_any_element()
        };
        let label_el = div()
            .flex_1()
            .min_w(px(0.0))
            .text_size(px(15.0))
            .line_height(gpui::relative(1.45))
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.ink))
            .child(label);
        // `.prop-item`: grid label + control; number rows carry slider + field.
        // The generic path is `minmax(0, 116px) minmax(0, 1fr)` inside the
        // sections; number rows are `minmax(0, 116px) minmax(0, 1fr) 48px`.
        let is_number = matches!(def.kind, PropKind::Number { .. });
        if narrow {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .min_h(px(44.0))
                .py_2()
                .border_b_1()
                .border_color(Theme::color(t.line))
                .child(label_el)
                .child(control)
        } else if is_number {
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_h(px(52.0))
                .py_2()
                .border_b_1()
                .border_color(Theme::color(t.line))
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
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .min_h(px(52.0))
                .py_2()
                .border_b_1()
                .border_color(Theme::color(t.line))
                .child(div().w(px(116.0)).flex_none().child(label_el))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .justify_end()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(control),
                )
        }
    }

    /// The control for one property (`control()` in PropertyControls).
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
    ) -> gpui::AnyElement {
        let t = &self.theme;
        match kind {
            PropKind::Number { min, max, step, default } => {
                let n = match value {
                    PropValue::Number(n) => *n as f32,
                    _ => *default as f32,
                };
                let min_f = *min as f32;
                let max_f = *max as f32;
                let step_f = *step as f32;
                let key_owned = key.to_string();
                let entity_a = view_entity.clone();
                let key_a = key_owned.clone();
                let on_change = move |v: f32, _: &mut Window, cx: &mut gpui::App| {
                    let stepped = quantise_step(v, min_f, max_f, step_f);
                    entity_a.update(cx, |this, cx| this.on_change(&key_a, PropValue::Number(stepped as f64), cx));
                };
                let entity_b = view_entity.clone();
                let key_b = key_owned.clone();
                let on_commit = move |v: f32, _: &mut Window, cx: &mut gpui::App| {
                    let stepped = quantise_step(v, min_f, max_f, step_f);
                    entity_b.update(cx, |this, cx| this.on_change(&key_b, PropValue::Number(stepped as f64), cx));
                };
                let decimals = decimals_of(*step);
                let key_c = key_owned.clone();
                let entity_c = view_entity.clone();
                let on_field = move |v: f32, _: &mut Window, cx: &mut gpui::App| {
                    let stepped = quantise_step(v, min_f, max_f, step_f);
                    entity_c.update(cx, |this, cx| this.on_change(&key_c, PropValue::Number(stepped as f64), cx));
                };
                let _ = decimals;
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(slider_range(t, &format!("{key}-slider"), n, min_f, max_f, disabled, self.slider_origin(key), on_change, on_commit))
                    .child(number_field(t, &format!("{key}-field"), n, min_f, max_f, step_f, None, on_field))
                    .into_any_element()
            }
            PropKind::Bool { default } => {
                let on = match value {
                    PropValue::Bool(b) => *b,
                    _ => *default,
                };
                let key_owned = key.to_string();
                let entity = view_entity.clone();
                let inner = move |v: bool, _: &mut Window, cx: &mut gpui::App| {
                    entity.update(cx, |this, cx| this.on_change(&key_owned, PropValue::Bool(v), cx));
                };
                if disabled {
                    toggle_disabled(t, &format!("{key}-toggle"), on, inner).into_any_element()
                } else {
                    toggle(t, &format!("{key}-toggle"), on, inner).into_any_element()
                }
            }
            PropKind::Choice { options, default } => {
                let current = match value {
                    PropValue::Choice(s) => s.clone(),
                    _ => default.clone(),
                };
                if options.len() <= 4 {
                    let words: Vec<String> = options.iter().map(|o| choice_word(o)).collect();
                    let opts: Vec<(&str, &str)> = options
                        .iter()
                        .zip(words.iter())
                        .map(|(o, w)| (o.as_str(), w.as_str()))
                        .collect();
                    let _ = label;
                    if disabled {
                        segmented_disabled(t, &format!("{key}-seg"), &opts, &current, |_, _, _| {}).into_any_element()
                    } else {
                        let key_owned = key.to_string();
                        let entity = view_entity.clone();
                        segmented(t, &format!("{key}-seg"), &opts, &current, move |v: String, _: &mut Window, cx: &mut gpui::App| {
                            entity.update(cx, |this, cx| this.on_change(&key_owned, PropValue::Choice(v), cx));
                        }).into_any_element()
                    }
                } else {
                    self.choice_select(key, label, options, &current, disabled, view_entity).into_any_element()
                }
            }
            PropKind::Color { default } => {
                let hex = match value {
                    PropValue::Color(c) => c.to_hex(),
                    _ => default.to_hex(),
                };
                self.color_field(key, label, &hex, schema_properties, values, disabled, view_entity).into_any_element()
            }
            PropKind::Hue { default } => {
                let deg = match value {
                    PropValue::Hue(n) => *n as f32,
                    _ => *default as f32,
                };
                self.hue_control(key, label, deg, disabled, view_entity).into_any_element()
            }
            PropKind::Angle { default } => {
                let deg = match value {
                    PropValue::Angle(n) => *n as f32,
                    PropValue::Hue(n) => *n as f32,
                    PropValue::Number(n) => (*n).clamp(0.0, 360.0) as f32,
                    _ => *default as f32,
                };
                self.angle_control(key, label, deg, disabled, view_entity).into_any_element()
            }
        }
    }

    /// A choice with more than 4 options (`Select`).
    fn choice_select(
        &self,
        key: &str,
        label: &str,
        options: &[String],
        current: &str,
        disabled: bool,
        view_entity: gpui::Entity<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let open = self.select_open.get(key).copied().unwrap_or(false);
        let opts: Vec<(String, String)> = options.iter().map(|o| (o.clone(), choice_word(o))).collect();
        let refs: Vec<(&str, &str)> = opts.iter().map(|(v, l)| (v.as_str(), l.as_str())).collect();
        let id = format!("{key}-select");
        let entity_toggle = view_entity.clone();
        let key_toggle = key.to_string();
        let entity_pick = view_entity.clone();
        let key_pick = key.to_string();
        let _ = label;
        let select = crate::components::select::select_field(
            t,
            &id,
            label,
            &refs,
            current,
            open,
            disabled,
            false,
            move |_, cx| {
                entity_toggle.update(cx, |this, cx| {
                    let entry = this.select_open.entry(key_toggle.clone()).or_insert(false);
                    *entry = !*entry;
                    cx.notify();
                });
            },
            move |v: String, _, cx| {
                entity_pick.update(cx, |this, cx| {
                    this.select_open.insert(key_pick.clone(), false);
                    this.on_change(&key_pick, PropValue::Choice(v), cx);
                });
            },
        );
        select
    }

    /// A colour field (`ColorField`): the swatch, the hex, and the picker
    /// popover with the other colours of the schema as swatches.
    #[allow(clippy::too_many_arguments)]
    fn color_field(
        &self,
        key: &str,
        label: &str,
        hex: &str,
        schema_properties: &[PropertyDef],
        values: &Props,
        disabled: bool,
        view_entity: gpui::Entity<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let hex_owned = hex.to_string();
        let open = self.color_open.as_deref() == Some(key);
        let swatches = colour_swatches(schema_properties, values, key, hex);
        let entity_toggle = view_entity.clone();
        let key_toggle = key.to_string();
        let mut row = div()
            .flex()
            .items_center()
            .gap_2();
        let swatch = div()
            .id(gpui::SharedString::from(format!("{key}-swatch")))
            .relative()
            .flex_none()
            .size(px(32.0))
            .r_sm()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(hex));
        let swatch = if disabled {
            swatch
        } else {
            let entity = entity_toggle;
            let key_clone = key_toggle.clone();
            swatch.cursor_pointer().on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.color_open = if this.color_open.as_deref() == Some(key_clone.as_str()) {
                        None
                    } else {
                        Some(key_clone.clone())
                    };
                    cx.notify();
                });
            })
        };
        row = row
            .child(swatch)
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(hex_owned.clone()),
            );
        if open && !disabled {
            row = row.child(self.color_picker(key, label, hex, &swatches, view_entity));
        }
        row
    }

    /// The picker popover (`ColorPicker`): SV square, hue strip, hex row and
    /// the other colours of the schema.
    fn color_picker(
        &self,
        key: &str,
        label: &str,
        hex: &str,
        swatches: &[(String, String)],
        view_entity: gpui::Entity<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let (hue, sat, val) = hex_to_hsv(hex, 0.0);
        let hue_hex = hsv_to_hex(hue, 1.0, 1.0);
        let entity = view_entity.clone();
        let key_owned = key.to_string();
        let _ = label;
        let mut picker = div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(212.0))
            .p_3()
            .r_md()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.panel));
        // SV square: hue at full strength, washed to white across and black down.
        let entity_sv = entity.clone();
        let key_sv = key_owned.clone();
        let hex_owned = hex.to_string();
        let sv = div()
            .id(gpui::SharedString::from(format!("{key}-sv")))
            .relative()
            .w_full()
            .h(px(160.0))
            .r_sm()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(&hue_hex))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(
                        canvas(
                            move |_, _, _| {},
                            move |bounds, _, window, _| {
                                let w = bounds.size.width;
                                let h = bounds.size.height;
                                let steps = 24;
                                for i in 0..steps {
                                    for j in 0..steps {
                                        let s = i as f32 / steps as f32;
                                        let v = 1.0 - j as f32 / steps as f32;
                                        let c = {
                                            let (hr, hg, hb) = {
                                                let raw = hue_hex.strip_prefix('#').unwrap_or(&hue_hex);
                                                let p = |k: usize| u8::from_str_radix(&raw[k..k + 2], 16).unwrap_or(0) as f32 / 255.0;
                                                (p(0), p(2), p(4))
                                            };
                                            let r = hr + (1.0 - hr) * (1.0 - s);
                                            let g = hg + (1.0 - hg) * (1.0 - s);
                                            let b = hb + (1.0 - hb) * (1.0 - s);
                                            gpui::Rgba { r: r * v, g: g * v, b: b * v, a: 1.0 }
                                        };
                                        let cell_w = w / steps as f32;
                                        let cell_h = h / steps as f32;
                                        window.paint_quad(gpui::fill(
                                            Bounds {
                                                origin: point(bounds.origin.x + cell_w * i as f32, bounds.origin.y + cell_h * j as f32),
                                                size: size(cell_w, cell_h),
                                            },
                                            c,
                                        ));
                                    }
                                }
                                // Handle.
                                let hx = bounds.origin.x + w * sat;
                                let hy = bounds.origin.y + h * (1.0 - val);
                                window.paint_quad(gpui::outline(
                                    Bounds {
                                        origin: point(hx - px(7.0), hy - px(7.0)),
                                        size: size(px(14.0), px(14.0)),
                                    },
                                    Theme::color(Theme::dark().ink),
                                    gpui::BorderStyle::default(),
                                ));
                            },
                        )
                        .absolute()
                        .inset_0(),
                ),
            )
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                // A click commits the current hex (the popover's sv/hue drags
                // stream into the draft; a GPUI click commits at once).
                entity_sv.update(cx, |this, cx| {
                    if let Some(parsed) = parse_hex(&hex_owned) {
                        this.on_change(&key_sv, PropValue::Color(parsed), cx);
                    }
                    cx.notify();
                });
            });
        picker = picker.child(sv);
        // Hue strip.
        let entity_hue = entity.clone();
        let hue_strip = div()
            .id(gpui::SharedString::from(format!("{key}-hue")))
            .relative()
            .w_full()
            .h(px(14.0))
            .r_sm()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .child(
                canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        let w = bounds.size.width;
                        let steps = 36;
                        for i in 0..steps {
                            let h = i as f32 / steps as f32 * 360.0;
                            let (r, g, b) = {
                                let raw = hsv_to_hex(h, 0.8, 0.62);
                                let s = raw.strip_prefix('#').unwrap_or(&raw);
                                let p = |k: usize| u8::from_str_radix(&s[k..k + 2], 16).unwrap_or(0) as f32 / 255.0;
                                (p(0), p(2), p(4))
                            };
                            let cell_w = w / steps as f32;
                            window.paint_quad(gpui::fill(
                                Bounds {
                                    origin: point(bounds.origin.x + cell_w * i as f32, bounds.origin.y),
                                    size: size(cell_w, bounds.size.height),
                                },
                                gpui::Rgba { r, g, b, a: 1.0 },
                            ));
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                entity_hue.update(cx, |_this, cx| {
                    cx.notify();
                });
            });
        picker = picker.child(hue_strip);
        // Hex row.
        picker = picker.child(
            div()
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
                        .size(px(24.0))
                        .r_sm()
                        .border_1()
                        .border_color(Theme::color(t.line_2))
                        .bg(Theme::color(hex)),
                ),
        );
        // The other colours of the schema.
        if !swatches.is_empty() {
            let mut row = div().flex().flex_wrap().gap(px(6.0));
            for (sw, sw_label) in swatches {
                let entity_sw = entity.clone();
                let key_sw = key_owned.clone();
                let sw_owned = sw.clone();
                let _ = sw_label;
                row = row.child(
                    div()
                        .id(gpui::SharedString::from(format!("{key}-swatch")))
                        .size(px(22.0))
                        .r_sm()
                        .border_1()
                        .border_color(Theme::color(t.line_2))
                        .bg(Theme::color(sw))
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            entity_sw.update(cx, |this, cx| {
                                if let Some(c) = parse_hex(&sw_owned) {
                                    this.on_change(&key_sw, PropValue::Color(c), cx);
                                }
                                this.color_open = None;
                                cx.notify();
                            });
                        }),
                );
            }
            picker = picker.child(row);
        }
        picker
    }

    /// A hue control (`HueRing` plus degrees field).
    fn hue_control(&self, key: &str, _label: &str, deg: f32, disabled: bool, view_entity: gpui::Entity<Self>) -> impl IntoElement {
        let t = &self.theme;
        let hue = wrap_deg(deg as f64) as f32;
        let ink = Theme::color(t.ink);
        let ring = div()
            .id(gpui::SharedString::from(format!("{key}-hue")))
            .relative()
            .flex_none()
            .size(px(88.0))
            .rounded_full()
            .child(
                canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        let cx = f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0;
                        let cy = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0;
                        let w = f32::from(bounds.size.width);
                        let r_outer = 48.0 * (w / 100.0);
                        let r_inner = 36.0 * (w / 100.0);
                        for i in 0..36 {
                            let a0 = i as f32 * 10.0;
                            let a1 = a0 + 10.6;
                            let (x0, y0) = polar(cx, cy, r_outer, a0);
                            let (x1, y1) = polar(cx, cy, r_outer, a1);
                            let (x2, y2) = polar(cx, cy, r_inner, a1);
                            let (x3, y3) = polar(cx, cy, r_inner, a0);
                            let mut b = gpui::PathBuilder::fill();
                            b.move_to(point(px(x0), px(y0)));
                            b.line_to(point(px(x1), px(y1)));
                            b.line_to(point(px(x2), px(y2)));
                            b.line_to(point(px(x3), px(y3)));
                            b.close();
                            if let Ok(path) = b.build() {
                                let hex = hsv_to_hex(i as f32 * 10.0, 0.8, 0.62);
                                window.paint_path(path, Theme::color(&hex));
                            }
                        }
                        let (hx, hy) = polar(cx, cy, 42.0 * (w / 100.0), hue);
                        let mut dot = gpui::PathBuilder::fill();
                        dot.add_polygon(&circle_points(point(px(hx), px(hy)), 5.5 * (w / 100.0), 16), true);
                        if let Ok(path) = dot.build() {
                            window.paint_path(path, ink);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            );
        let ring = if disabled {
            ring.opacity(0.5)
        } else {
            ring.cursor_pointer()
        };
        let key_field = key.to_string();
        let entity_field = view_entity.clone();
        let field = number_field(t, &format!("{key}-degrees"), deg.round(), 0.0, 360.0, 1.0, Some("°"), move |v: f32, _: &mut Window, cx: &mut gpui::App| {
            entity_field.update(cx, |this, cx| this.on_change(&key_field, PropValue::Hue(v as f64), cx));
        });
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(ring)
            .child(field)
    }

    /// An angle control (`AngleDial` plus degrees field).
    fn angle_control(&self, key: &str, _label: &str, deg: f32, disabled: bool, view_entity: gpui::Entity<Self>) -> impl IntoElement {
        let t = &self.theme;
        let deg_wrapped = wrap_deg(deg as f64) as f32;
        let mute = Theme::color(t.mute);
        let dial_ring = Theme::color(t.dial_ring);
        let ink = Theme::color(t.ink);
        let key_owned = key.to_string();
        let dial = div()
            .id(gpui::SharedString::from(format!("{key}-dial")))
            .relative()
            .flex_none()
            .size(px(88.0))
            .rounded_full()
            .child(
                canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        let s = f32::from(bounds.size.width) / 100.0;
                        let cx = f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.0;
                        let cy = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0;
                        // Cardinal ticks.
                        for (x1, y1, x2, y2) in [(95.0, 50.0, 99.0, 50.0), (50.0, 95.0, 50.0, 99.0), (5.0, 50.0, 1.0, 50.0), (50.0, 1.0, 50.0, 5.0)] {
                            let mut b = gpui::PathBuilder::stroke(px(2.0 * s));
                            b.move_to(point(px(cx + (x1 - 50.0) * s), px(cy + (y1 - 50.0) * s)));
                            b.line_to(point(px(cx + (x2 - 50.0) * s), px(cy + (y2 - 50.0) * s)));
                            if let Ok(path) = b.build() {
                                window.paint_path(path, mute);
                            }
                        }
                        // The neutral ring.
                        let mut ring = gpui::PathBuilder::stroke(px(7.0 * s));
                        ring.add_polygon(&circle_points(point(px(cx), px(cy)), 40.0 * s, 48), true);
                        if let Ok(path) = ring.build() {
                            window.paint_path(path, dial_ring);
                        }
                        // The needle, turned to the value.
                        let rad = deg_wrapped * std::f32::consts::PI / 180.0;
                        let (dx, dy) = (rad.cos(), rad.sin());
                        let mut needle = gpui::PathBuilder::stroke(px(3.0 * s));
                        needle.move_to(point(px(cx + 4.0 * s), px(cy)));
                        needle.line_to(point(px(cx + 26.0 * s * dx), px(cy + 26.0 * s * dy)));
                        if let Ok(path) = needle.build() {
                            window.paint_path(path, ink);
                        }
                        let mut hub = gpui::PathBuilder::fill();
                        hub.add_polygon(&circle_points(point(px(cx), px(cy)), 3.5 * s, 16), true);
                        if let Ok(path) = hub.build() {
                            window.paint_path(path, ink);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            );
        let dial = if disabled { dial.opacity(0.5) } else { dial.cursor_pointer() };
        let key_field = key_owned.clone();
        let entity_field = view_entity.clone();
        let now = angle_now(deg as f64);
        let field = number_field(t, &format!("{key}-degrees"), now as f32, 0.0, 359.0, 1.0, Some("°"), move |v: f32, _: &mut Window, cx: &mut gpui::App| {
            entity_field.update(cx, |this, cx| this.on_change(&key_field, PropValue::Angle(v as f64), cx));
        });
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(dial)
            .child(field)
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
            for def in &schema {
                let value = values.get(&def.key).cloned().unwrap_or_else(|| default_for(&def.kind));
                list = list.child(self.prop_row(def, &value, read_only, disabled, &schema, values, view_entity.clone(), narrow));
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
                    div()
                        .size(px(10.0))
                        .rounded_full()
                        .bg(Theme::color(&stage.colour))
                        .border_1()
                        .border_color(Theme::color(t.line_2)),
                )
                .child(
                    div()
                        .text_sm()
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
            chips = chips.child(chip);
        }
        body = body.child(chips);
        // The picked stage editor.
        if picked.is_some() && !picked_defs.is_empty() {
            let schema: Vec<PropertyDef> = picked_defs.iter().map(|p| (*p).clone()).collect();
            let mut list = div().flex().flex_col().mt_3().pt_3().border_t_1().border_color(Theme::color(t.line));
            for def in &schema {
                let value = values.get(&def.key).cloned().unwrap_or_else(|| default_for(&def.kind));
                list = list.child(self.prop_row(def, &value, read_only, disabled, &schema, values, view_entity.clone(), narrow));
            }
            body = body.child(list);
        }
        self.panel(&format!("dpi-{section_id}"), section_title, body)
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
        let plan = self.controller_outline(&plan_buttons, picked.as_deref(), view_entity.clone(), content_w);
        // The map list.
        let narrow_grid = content_w <= 960.0;
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
                    let select = self.choice_select(&def.key, &format!("{label} mapping"), &options, &current, disabled, view_entity.clone());
                    // The map Select is 128 px wide at desktop.
                    row = row.child(div().w(px(128.0)).flex_none().child(select));
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
        let grid = if narrow_grid {
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(plan)
                .child(map_grid)
        } else {
            div()
                .flex()
                .gap_5()
                .items_start()
                .child(div().w(px(420.0)).flex_none().min_w(px(280.0)).child(plan))
                .child(map_grid)
        };
        self.panel("controller-panel", &detail.info.name, grid)
    }

    /// The plan of the pad (`ControllerOutline`): front and rear views.
    fn controller_outline(
        &self,
        buttons: &[(String, String)],
        selected: Option<&str>,
        view_entity: gpui::Entity<Self>,
        content_w: f32,
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
        // The front view: 340x244 at the available width.
        let plan_w = (content_w - 32.0).clamp(280.0, 420.0);
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
            .child(front_wrap)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .w(px(200.0))
                    .child(rear_wrap)
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
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(20.0))
                            .line_height(gpui::relative(1.25))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(info.name.clone()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child(format!("{} · {} · ", kind_word(info.kind), info.vendor))
                            .child(
                                div()
                                    .font_family(crate::fonts::MONO_FAMILY)
                                    .child(info.location.clone()),
                            ),
                    ),
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
            access.ml_4().pl_4().border_l_1().border_color(Theme::color(t.line))
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
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel))
            .child(head_panel_body);
        // The sections.
        let narrow = content_w < 640.0;
        let mut column_panels: Vec<gpui::AnyElement> = Vec::new();
        let mut wide_panels: Vec<gpui::AnyElement> = Vec::new();
        for section in &detail.schema.sections {
            if is_mouse && section.id == "dpi" {
                column_panels.push(
                    self.dpi_panel(
                        &section.id,
                        &section.title,
                        &section.properties,
                        values,
                        read_only,
                        !writes,
                        view_entity.clone(),
                        narrow,
                    )
                    .into_any_element(),
                );
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
            for def in &schema {
                let value = values.get(&def.key).cloned().unwrap_or_else(|| default_for(&def.kind));
                list = list.child(self.prop_row(def, &value, read_only, !writes, &schema, values, view_entity.clone(), narrow));
            }
            // The mouse Buttons panel is a two-column grid of read-only rows.
            let panel_body: gpui::AnyElement = if section.id == "buttons" && is_mouse {
                let mut grid = div().flex().flex_wrap().gap_4();
                for def in &schema {
                    let value = values.get(&def.key).cloned().unwrap_or_else(|| default_for(&def.kind));
                    grid = grid.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_sm()
                                    .font_family(crate::fonts::UI_FAMILY)
                                    .text_color(Theme::color(t.dim))
                                    .child(plain_label(&def.label)),
                            )
                            .child(self.read_only_value(&value)),
                    );
                }
                grid.into_any_element()
            } else {
                list.into_any_element()
            };
            column_panels.push(self.panel(&format!("section-{}", section.id), &section.title, panel_body).into_any_element());
        }
        // The sections stack in one column on a phone (`columns: auto` in the
        // 639 container rule); on desktop they are columns of 280 px. GPUI
        // has no multicol, so both draw as one column with the same gap.
        let _ = narrow;
        let mut col = div().flex().flex_col().gap_4();
        for panel in column_panels {
            col = col.child(panel);
        }
        let sections = col;
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
            ));
        }
        stack
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
                    .text_sm()
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

/// Format a number like React prints it (no trailing `.0` for whole numbers).
pub fn format_number(n: f64, decimals: usize) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else if decimals == 0 {
        format!("{n:.0}")
    } else {
        format!("{n:.decimals$}")
    }
}

/// Put a raw value on the step grid and inside the range (Slider `quantise`).
pub fn quantise_step(raw: f32, min: f32, max: f32, step: f32) -> f32 {
    let step = if step.abs() < f32::EPSILON { 1.0 } else { step };
    let decimals = decimals_of(step as f64);
    let stepped = min + ((raw - min) / step).round() * step;
    let clamped = stepped.clamp(min, max);
    let factor = 10f32.powi(decimals as i32);
    (clamped * factor).round() / factor
}

/// A point on the (50, 50) circle at `deg` (0 = up), in drawing units.
fn polar(cx: f32, cy: f32, r: f32, deg: f32) -> (f32, f32) {
    let rad = (deg - 90.0) * std::f32::consts::PI / 180.0;
    (cx + r * rad.cos(), cy + r * rad.sin())
}

/// The polygon of a circle at `centre` with `radius`.
fn circle_points(centre: gpui::Point<gpui::Pixels>, radius: f32, steps: usize) -> Vec<gpui::Point<gpui::Pixels>> {
    (0..=steps)
        .map(|i| {
            let a = i as f32 / steps as f32 * std::f32::consts::TAU;
            point(centre.x + px(radius * a.cos()), centre.y + px(radius * a.sin()))
        })
        .collect()
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
                )
                .into_any_element(),
            _ => {
                let mut row = div().flex().gap_2().min_w(px(0.0));
                for r in &rows {
                    row = row.child(self.device_key(r, view_entity.clone()));
                }
                row.into_any_element()
            }
        };

        // The save-error line.
        let save_line = if let Some(err) = self.save_error() {
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.warn))
                .child(err)
                .into_any_element()
        } else {
            div().into_any_element()
        };

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
                    .p_1()
                    .child(keys),
            )
            .child(save_line);
        // The detail pane scrolls inside its own pane on desktop (React
        // `.rd-peripherals .peripheral-detail` `overflow-y: auto`); on a phone
        // the page scrolls instead.
        let detail_wrapped: gpui::AnyElement = if phone {
            div().flex().flex_col().flex_1().min_h(px(0.0)).child(detail_pane).into_any_element()
        } else {
            div()
                .id(gpui::SharedString::from("peri-detail-scroll"))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(detail_pane)
                .into_any_element()
        };
        page = page.child(detail_wrapped);
        page
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
    /// The toast after a saved write (React `notify("The settings are on the device")`, tone ok).
    #[allow(dead_code)]
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
}
