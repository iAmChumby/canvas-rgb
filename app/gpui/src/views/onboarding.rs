//! Onboarding ("Map my PC"): the page-level takeover the Layout page renders.
//!
//! Port of `app/web/src/views/editor/{Onboarding,HelloStep,Greeting,PartsStep,
//! MatchStep,FinishStep,OnboardStep,ProgressDots,HardwareLine}.tsx` and
//! `{flow,setup,takeover,parts,newLights,onboardEffects}.ts`. The checklist is
//! `docs/gpui-parity/onboarding.md`.
//!
//! The drawing of the PC stays on screen at every step (the Layout stage is
//! reused, not forked), one panel beside it says what to do next, and on a
//! phone the panel is a bottom sheet. The shell hides the rail/strip while
//! this view is open (React `takeover.ts`).

use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneKind};
use canvas_proto::layout::{Facing, Layout, LedSlice, Part, PartKind, ZoneShape};
use gpui::{Context, IntoElement, Window, div, prelude::*, px};

use crate::client::DaemonClient;
use crate::components::button::{Variant, button_icon, icon_button};
use crate::components::segmented::segmented;
use crate::theme::{Theme, TokenRadius};
use crate::views::layout::geometry::{LengthUnit, format_length};
use crate::views::layout::stage::{DeskInput, Sel, desk_with_hint};
use crate::views::layout::tokens::DeskTokens;
use crate::views::layout::LayoutView;

/// The five screens of one run (flow.ts `Step`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Hello,
    Case,
    Parts,
    Match,
    Finish,
}

/// The part the flow is limited to when one device is redone (flow.ts `Scope`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub device_id: String,
    pub zone_index: u32,
}

/// The five screens of a full run; a device redo is Match + Finish.
pub fn build_steps(scope: Option<&Scope>) -> Vec<StepKind> {
    if scope.is_some() {
        vec![StepKind::Match, StepKind::Finish]
    } else {
        vec![StepKind::Hello, StepKind::Case, StepKind::Parts, StepKind::Match, StepKind::Finish]
    }
}

/// Whether the window opens by itself (flow.ts `shouldOpen`).
pub fn should_open(onboarding_complete: bool, later_this_launch: bool) -> bool {
    !onboarding_complete && !later_this_launch
}

/// A case length as a person says it (Onboarding.tsx `caseText`).
pub fn case_text(units: f64, unit: LengthUnit) -> String {
    match unit {
        LengthUnit::In => format!("{:.1} in", units / 6.35),
        LengthUnit::Cm => format!("{} cm", (units / 2.5).round() as i64),
    }
}

/// "37.0 cm" reads as "37 cm"; inches keep their decimals (Onboarding.tsx `trimCm`).
#[allow(dead_code)]
pub fn trim_cm(text: &str) -> String {
    text.replace(".0 cm", " cm")
}

/// The case guides (parts.ts `CASE_GUIDES`): id, name, cm wide/tall.
#[allow(dead_code)]
pub fn case_guides() -> Vec<(&'static str, &'static str, f64, f64)> {
    vec![("small", "Small tower", 37.0, 34.0), ("mid", "Mid tower", 48.0, 49.0), ("full", "Full tower", 58.0, 58.0)]
}

/// Where a case stands: bottom edge on the floor line, middle on this x (parts.ts `CASE_ANCHOR`).
pub const CASE_ANCHOR_X: f64 = 160.0;
pub const CASE_ANCHOR_Y: f64 = 149.0;

/// The hello word for a BCP 47 tag (Greeting.tsx `helloFor`).
pub fn hello_for(tag: &str) -> &'static str {
    let lang = tag.to_lowercase().split('-').next().unwrap_or("").to_string();
    match lang.as_str() {
        "en" => "Hello",
        "es" => "Hola",
        "fr" => "Bonjour",
        "de" => "Hallo",
        "it" => "Ciao",
        "pt" => "Olá",
        "nl" => "Hallo",
        "sv" => "Hej",
        "da" => "Hej",
        "nb" => "Hei",
        "no" => "Hei",
        "fi" => "Hei",
        "pl" => "Cześć",
        "cs" => "Ahoj",
        "sk" => "Ahoj",
        "hu" => "Szia",
        "ro" => "Salut",
        "tr" => "Merhaba",
        "el" => "Γεια σας",
        "ru" => "Привет",
        "uk" => "Привіт",
        "ar" => "مرحبا",
        "he" => "שלום",
        "hi" => "नमस्ते",
        "ja" => "こんにちは",
        "ko" => "안녕하세요",
        "zh" => "你好",
        "vi" => "Xin chào",
        "th" => "สวัสดี",
        "id" => "Halo",
        _ => "Hello",
    }
}

/// One part with lights, for the progress dots (setup.ts `PartDot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartDot {
    pub index: usize,
    pub mapped: bool,
}

/// The parts with lights, in layout order (setup.ts `progressParts`).
pub fn progress_parts(layout: &Layout, devices: &[DeviceInfo]) -> Vec<PartDot> {
    let mut out = Vec::new();
    for (index, z) in layout.zones.iter().enumerate() {
        let device = devices.iter().find(|d| d.id.0 == z.device_id.0);
        let zone = device.and_then(|d| d.zones.iter().find(|x| x.index == z.zone_index));
        let led_count = zone.map(|z| z.led_count).unwrap_or(0);
        if led_count == 0 {
            continue;
        }
        out.push(PartDot { index, mapped: z.part.as_ref().map(|p| p.mapped).unwrap_or(false) });
    }
    out
}

/// {mapped, total} (setup.ts `progress`).
#[allow(dead_code)]
pub fn progress(layout: &Layout, devices: &[DeviceInfo]) -> (usize, usize) {
    let parts = progress_parts(layout, devices);
    let mapped = parts.iter().filter(|p| p.mapped).count();
    (mapped, parts.len())
}

/// The parts whose device is not there now (HardwareLine.tsx `missingParts`).
pub fn missing_parts(layout: &Layout, devices: Option<&[DeviceInfo]>) -> usize {
    let Some(devices) = devices else { return 0 };
    layout
        .zones
        .iter()
        .filter(|z| {
            let d = devices.iter().find(|x| x.id.0 == z.device_id.0);
            !d.is_some_and(|d| d.connected)
        })
        .count()
}

/// The zone is worth walking when its count can change or it is a long line (setup.ts `isDoubtful`).
pub fn is_doubtful(zone: &canvas_proto::device::ZoneInfo) -> bool {
    if zone.resizable {
        return true;
    }
    matches!(zone.kind, ZoneKind::Linear) && zone.led_count > 16
}

/// Whether a device kind is fixed (setup.ts `isFixedKind`).
pub fn is_fixed_kind(kind: DeviceKind) -> bool {
    matches!(kind, DeviceKind::Keyboard | DeviceKind::Mouse | DeviceKind::Dram | DeviceKind::Headset | DeviceKind::Microphone)
}

/// Default LEDs in one fan-sized run (setup.ts `RUN_LEDS`).
pub const RUN_LEDS: u32 = 12;

/// The length of the run that starts at `off` (setup.ts `runLen`).
pub fn run_len(led_count: u32, doubtful: bool, fixed: bool, placements_len: usize, off: u32) -> u32 {
    let left = led_count.saturating_sub(off);
    if !doubtful || fixed {
        return left;
    }
    let size = RUN_LEDS.max(led_count.div_ceil(placements_len.max(1) as u32));
    if left.saturating_sub(size) < 4 {
        left
    } else {
        size
    }
}

/// A zone as the setup sees it (setup.ts `SetupZone`).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SetupZone {
    pub device_id: String,
    pub zone_index: u32,
    pub device_name: String,
    pub name: String,
    pub device_kind: DeviceKind,
    pub led_count: u32,
    pub resizable: bool,
    pub min_leds: u32,
    pub max_leds: u32,
    pub placements: Vec<usize>,
    pub doubtful: bool,
    pub mapped: bool,
}

/// Every zone of every device, with the layout's state (setup.ts `setupZones`).
pub fn setup_zones(layout: &Layout, devices: Option<&[DeviceInfo]>) -> Vec<SetupZone> {
    let mut out = Vec::new();
    for device in devices.unwrap_or(&[]) {
        for zone in &device.zones {
            let placements: Vec<usize> = layout
                .zones
                .iter()
                .enumerate()
                .filter(|(_, z)| z.device_id.0 == device.id.0 && z.zone_index == zone.index)
                .map(|(i, _)| i)
                .collect();
            let mapped = !placements.is_empty()
                && placements.iter().all(|i| layout.zones[*i].part.as_ref().map(|p| p.mapped).unwrap_or(false));
            out.push(SetupZone {
                device_id: device.id.0.clone(),
                zone_index: zone.index,
                device_name: device.name.clone(),
                name: zone.name.clone(),
                device_kind: device.kind,
                led_count: zone.led_count,
                resizable: zone.resizable,
                min_leds: zone.min_leds,
                max_leds: zone.max_leds,
                placements,
                doubtful: is_doubtful(zone),
                mapped,
            });
        }
    }
    out
}

/// One run of LEDs to light (setup.ts `Run`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub zi: usize,
    pub start: u32,
    pub count: u32,
    pub whole: bool,
}

/// Whether a placement owns some of the run's LEDs (setup.ts `owns`).
fn owns(p: &canvas_proto::layout::PlacedZone, start: u32, count: u32) -> bool {
    match &p.leds {
        None => true,
        Some(s) => s.start < start + count && start < s.start + s.count,
    }
}

/// The placements of a zone that own some of the run (setup.ts `placementsOfRun`).
pub fn placements_of_run(layout: &Layout, zone: &SetupZone, start: u32, count: u32) -> Vec<usize> {
    layout
        .zones
        .iter()
        .enumerate()
        .filter(|(_, p)| p.device_id.0 == zone.device_id && p.zone_index == zone.zone_index && owns(p, start, count))
        .map(|(i, _)| i)
        .collect()
}

/// "device#zone@start" (setup.ts `runKey`).
pub fn run_key(zone: &SetupZone, start: u32) -> String {
    format!("{}#{}@{}", zone.device_id, zone.zone_index, start)
}

/// The next run at/after (zi, off) neither matched nor skipped (setup.ts `locateRun`).
pub fn locate_run(layout: &Layout, zones: &[SetupZone], skipped: &std::collections::HashSet<String>, zi: usize, off: u32) -> Option<Run> {
    for (i, zone) in zones.iter().enumerate().skip(zi) {
        let mut o = if i == zi { off } else { 0 };
        while o < zone.led_count {
            let count = run_len(zone.led_count, zone.doubtful, is_fixed_kind(zone.device_kind), zone.placements.len(), o);
            if count == 0 {
                break;
            }
            let done = skipped.contains(&run_key(zone, o))
                || placements_of_run(layout, zone, o, count).iter().any(|p| layout.zones[*p].part.as_ref().map(|pt| pt.mapped).unwrap_or(false));
            if !done {
                return Some(Run { zi: i, start: o, count, whole: o == 0 && count == zone.led_count });
            }
            o += count;
        }
    }
    None
}

/// Every default run of the zones (setup.ts `allRuns`).
pub fn all_runs(zones: &[SetupZone]) -> Vec<Run> {
    let mut out = Vec::new();
    for (zi, zone) in zones.iter().enumerate() {
        let mut off = 0;
        while off < zone.led_count {
            let count = run_len(zone.led_count, zone.doubtful, is_fixed_kind(zone.device_kind), zone.placements.len(), off);
            if count == 0 {
                break;
            }
            out.push(Run { zi, start: off, count, whole: off == 0 && count == zone.led_count });
            off += count;
        }
    }
    out
}

/// The guess the owner starts from (setup.ts `expandGuess`).
pub fn expand_guess(layout: &Layout, devices: Option<&[DeviceInfo]>) -> Layout {
    let mut cur = layout.clone();
    for zone in setup_zones(layout, devices) {
        if !zone.doubtful || is_fixed_kind(zone.device_kind) || zone.led_count == 0 {
            continue;
        }
        let whole: Vec<usize> = zone.placements.iter().filter(|i| {
            let z = &cur.zones[**i];
            z.leds.is_none() && !z.part.as_ref().map(|p| p.mapped).unwrap_or(false)
        }).cloned().collect();
        let idx = cur.zones.iter().position(|p| p.device_id.0 == zone.device_id && p.zone_index == zone.zone_index && p.leds.is_none() && !p.part.as_ref().map(|pt| pt.mapped).unwrap_or(false));
        let Some(idx) = idx else { continue };
        if whole.is_empty() {
            continue;
        }
        let template = cur.zones[idx].clone();
        let runs = all_runs(std::slice::from_ref(&zone));
        if runs.len() < 2 {
            continue;
        }
        let mut zones: Vec<canvas_proto::layout::PlacedZone> = cur.zones.clone();
        zones.remove(idx);
        cur.zones = zones;
        for r in runs {
            let box_ = place_for_part(&cur, PartKind::Fan, &template);
            cur.zones.push(canvas_proto::layout::PlacedZone {
                device_id: template.device_id.clone(),
                zone_index: template.zone_index,
                x: box_.0 as f32,
                y: box_.1 as f32,
                width: box_.2 as f32,
                height: box_.3 as f32,
                rotation_deg: 0.0,
                reversed: false,
                visible: true,
                shape: ZoneShape::Ring,
                label: None,
                leds: Some(LedSlice { start: r.start, count: r.count }),
                part: Some(Part { kind: PartKind::Fan, facing: Facing::Viewer, mapped: false }),
                locked: false,
            });
        }
    }
    cur
}

/// Where a new part goes (setup.ts `placeForPart`).
pub fn place_for_part(layout: &Layout, kind: PartKind, template: &canvas_proto::layout::PlacedZone) -> (f64, f64, f64, f64) {
    let case_box = layout.landmarks.iter().find(|l| l.kind == PartKind::Case && l.visible);
    if let Some(case_box) = case_box {
        let want = match kind {
            PartKind::Fan => "fan",
            PartKind::Ram => "ram",
            PartKind::Gpu => "gpu",
            PartKind::Strip => "edge",
            _ => "",
        };
        if !want.is_empty() {
            let slots = case_slots(case_box.x as f64, case_box.y as f64, case_box.width as f64, case_box.height as f64);
            for s in slots {
                if s.0 != want {
                    continue;
                }
                let taken = layout.zones.iter().any(|z| (z.x as f64) < s.1 + s.3 && s.1 < (z.x as f64) + (z.width as f64) && (z.y as f64) < s.2 + s.4 && s.2 < (z.y as f64) + (z.height as f64));
                if !taken {
                    return (s.1, s.2, s.3, s.4);
                }
            }
        }
    }
    let (w, h) = if kind == PartKind::Fan { (30.0, 30.0) } else { (template.width as f64, template.height as f64) };
    let x = ((template.x as f64) + (template.width as f64) + 3.0).clamp(0.0, (320.0 - w).max(0.0));
    let y = (template.y as f64).clamp(0.0, (200.0 - h).max(0.0));
    (x, y, w, h)
}

/// Case slots by kind (setup.ts via caseForms `slotsForBox`): ("fan"|"ram"|"gpu"|"edge", x,y,w,h).
fn case_slots(x: f64, y: f64, w: f64, h: f64) -> Vec<(&'static str, f64, f64, f64, f64)> {
    let mut out = Vec::new();
    for i in 0..3 {
        out.push(("fan", x + 10.0 + i as f64 * 32.0, y + 10.0, 30.0, 30.0));
    }
    out.push(("ram", x + w - 12.0, y + 8.0, 6.0, 34.0));
    out.push(("gpu", x + 12.0, y + h - 24.0, 70.0, 14.0));
    out.push(("edge", x, y, w, 5.0));
    out
}

/// The shape a part of that kind takes (setup.ts `shapeForKind`).
pub fn shape_for_kind_pub(kind: PartKind) -> ZoneShape {
    if kind == PartKind::Fan { ZoneShape::Ring } else { ZoneShape::Strip }
}

/// Three finish tiles (FinishStep.tsx `pickThree`).
pub fn pick_three(effects: &[canvas_proto::api::EffectSummary]) -> Vec<canvas_proto::api::EffectSummary> {
    const FIRST: [&str; 6] = ["aurora", "rainbow_wave", "spiral", "gradient", "fire", "solid"];
    let plain: Vec<&canvas_proto::api::EffectSummary> = effects.iter().filter(|e| !e.needs.screen && !e.needs.audio).collect();
    let mut out: Vec<canvas_proto::api::EffectSummary> = Vec::new();
    for id in FIRST {
        if let Some(hit) = plain.iter().find(|e| e.id == id) {
            if !out.iter().any(|e| e.id == hit.id) {
                out.push((*hit).clone());
            }
            if out.len() == 3 {
                return out;
            }
        }
    }
    for e in plain {
        if !out.iter().any(|x| x.id == e.id) {
            out.push(e.clone());
        }
        if out.len() == 3 {
            break;
        }
    }
    out
}

/// The two built-in effects the flow plays (onboardEffects.ts).
pub const HELLO_EFFECT: &str = "onboard-hello";
pub const SWEEP_EFFECT: &str = "onboard-sweep";
/// Sweep duration on arrival (FinishStep.tsx `SWEEP_MS`), and with motion off.
#[allow(dead_code)]
pub const SWEEP_MS: u64 = 3000;
#[allow(dead_code)]
pub const SWEEP_MS_REDUCED: u64 = 500;

/// The hello variant (HelloStep.tsx:35-44).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloMode {
    Gate,
    Waking,
    Ladder,
    Happy,
}

/// "SignalRGB has your lights" when a signal_rgb conflict is present.
pub fn hello_mode(has_conflict: bool, waking: bool, found: usize, ladder: bool) -> HelloMode {
    if has_conflict {
        HelloMode::Gate
    } else if waking {
        HelloMode::Waking
    } else if found == 0 || ladder {
        HelloMode::Ladder
    } else {
        HelloMode::Happy
    }
}

// ---------------------------------------------------------------------------
// The view state. Lives on LayoutView; the view is drawn over the stage.
// ---------------------------------------------------------------------------

/// The onboarding flow's state, owned by LayoutView.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct OnboardData {
    pub steps: Vec<StepKind>,
    pub at: usize,
    pub dir_forward: bool,
    pub scope: Option<Scope>,
    pub again: bool,
    pub decided: bool,
    /// Hello: "Nothing lit up?" was pressed.
    pub ladder: bool,
    /// Ladder/Parts Rescan busy flag.
    pub busy: bool,
    /// Ladder: a Rescan has finished.
    pub asked: bool,
    /// Ladder "More help" fold open.
    pub fold_open: bool,
    /// Hello wave played (once per mount).
    pub waved: bool,
    /// Match: the tapped placement index.
    pub tapped: Option<usize>,
    /// Match: resizing the tapped run.
    pub resizing: bool,
    /// Match: the resize count.
    pub count: u32,
    /// Match: the current run (None = done).
    pub run: Option<Run>,
    /// Match: the focus placement the drawing pulses.
    pub focus: Option<usize>,
    /// Finish: the sweep has crossed the PC.
    pub swept: bool,
    /// Finish: the effect the owner picked.
    pub picked: Option<String>,
    /// Parts: the "Every light already has a part" note.
    pub note: Option<String>,
    /// Parts guess applied once.
    pub guessed: bool,
    /// The x of the case guides' labels (Onboarding.tsx `labelX`).
    pub label_x: f64,
    /// Before-effect remembered for restore (onboardEffects.ts `before`).
    /// None = nothing remembered yet; Some(None) = nothing was running;
    /// Some(Some((id, props))) = a running effect to put back.
    pub before_effect: Option<Option<(String, canvas_proto::props::Props)>>,
}

impl OnboardData {
    pub fn new(again: bool, scope: Option<Scope>) -> Self {
        let steps = build_steps(scope.as_ref());
        let first = if again && scope.is_none() { 1 } else { 0 };
        OnboardData {
            steps,
            at: first,
            dir_forward: true,
            scope,
            again,
            decided: true,
            ladder: false,
            busy: false,
            asked: false,
            fold_open: false,
            waved: false,
            tapped: None,
            resizing: false,
            count: 0,
            run: None,
            focus: None,
            swept: false,
            picked: None,
            note: None,
            guessed: false,
            label_x: 0.0,
            before_effect: None,
        }
    }

    pub fn step(&self) -> StepKind {
        *self.steps.get(self.at).unwrap_or(self.steps.last().unwrap_or(&StepKind::Hello))
    }

    pub fn first(&self) -> usize {
        if self.again && self.scope.is_none() { 1 } else { 0 }
    }

    pub fn go(&mut self, to: usize) {
        self.dir_forward = to >= self.at;
        self.at = to.min(self.steps.len().saturating_sub(1));
    }

    pub fn next_step(&mut self) {
        self.go(self.at + 1);
    }

    pub fn back(&self) -> Option<usize> {
        if self.at > self.first() {
            Some(self.at - 1)
        } else {
            None
        }
    }
}

/// The stage the onboarding draws over (layout.rs render_page's desk, reused).
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub fn onboarding_stage(
    view: &mut LayoutView,
    _t: Theme,
    window: &mut Window,
    _cx: &mut Context<LayoutView>,
    pulse: &[Sel],
    spotlight: Option<&[Sel]>,
    guides: &[CaseGuide],
    unlock_badge: bool,
) -> gpui::AnyElement {
    let t = onboard_theme(view, StepKind::Hello);
    let colours = DeskTokens::for_theme(&t);
    let layout = view.layout().cloned();
    let devices = view.devices().to_vec();
    let Some(layout) = layout else {
        return div().flex_1().into_any_element();
    };
    let _ = (pulse, spotlight, guides, unlock_badge, window);
    let desk_input = DeskInput {
        zones: layout.zones.clone(),
        landmarks: layout.landmarks.clone(),
        case_form: layout.case_form,
        devices: devices.clone(),
        unit: view.unit,
        mode: view.mode,
        sampling: layout.sampling,
        frame: view.feed.frame(),
        selection: view.selection.clone(),
        view: view.view,
        overlay: view.overlay.clone(),
        unlock_badge: true,
    };
    let desk_origin = view.desk_origin.clone();
    desk_with_hint(&t, &colours, desk_input, desk_origin, true)
}

/// A case guide drawn on the canvas (Onboarding.tsx `guides`).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CaseGuide {
    pub id: String,
    pub name: String,
    pub size: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub label_x: f64,
}

/// The Case screen's guides, in canvas units.
#[allow(dead_code)]
pub fn case_guides_for(unit: LengthUnit, label_x: f64) -> Vec<CaseGuide> {
    case_guides()
        .iter()
        .map(|(id, name, cm_w, cm_h)| CaseGuide {
            id: id.to_string(),
            name: name.to_string(),
            size: format!("{} × {}", trim_cm(&format_length(cm_w * 10.0, unit)), trim_cm(&format_length(cm_h * 10.0, unit))),
            x: CASE_ANCHOR_X - (cm_w * 2.5) / 2.0,
            y: CASE_ANCHOR_Y - cm_h * 2.5,
            width: cm_w * 2.5,
            height: cm_h * 2.5,
            label_x,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// API helpers (existing client, existing routes only).
// ---------------------------------------------------------------------------

/// Activate an effect (onboardEffects.ts `play`/`restore`, choose).
pub fn activate_effect(id: &str, props: &canvas_proto::props::Props) {
    let id = id.to_string();
    let body = serde_json::json!({"props": props, "preset": null}).to_string();
    std::thread::spawn(move || {
        let client = DaemonClient::new();
        let _ = client.post_json(&format!("/api/v1/effects/{id}/activate"), &body);
    });
}

/// POST /api/v1/onboarding/session.
pub fn session(active: bool) {
    std::thread::spawn(move || {
        let client = DaemonClient::new();
        let body = serde_json::json!({"active": active}).to_string();
        let _ = client.post_json("/api/v1/onboarding/session", &body);
    });
}

/// POST /api/v1/devices/{id}/identify (MatchStep.tsx).
#[allow(dead_code)]
pub fn identify(device_id: &str, zone: u32, led: u32, count: Option<u32>, stop: bool) {
    let device_id = device_id.to_string();
    std::thread::spawn(move || {
        let client = DaemonClient::new();
        let body = if stop {
            serde_json::json!({"zone": zone, "led": 0, "stop": true}).to_string()
        } else if let Some(c) = count.filter(|c| *c > 1) {
            serde_json::json!({"zone": zone, "led": led, "count": c, "stop": false}).to_string()
        } else {
            serde_json::json!({"zone": zone, "led": led, "stop": false}).to_string()
        };
        let _ = client.post_json(&format!("/api/v1/devices/{device_id}/identify"), &body);
    });
}

/// GET /api/v1/devices?rescan=1.
pub fn rescan_devices() -> Result<Vec<DeviceInfo>, String> {
    let client = DaemonClient::new();
    let body = client.get("/api/v1/devices?rescan=1").map_err(|e| e.to_string())?;
    let parsed: canvas_proto::api::DevicesResponse = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(parsed.devices)
}

/// POST /api/v1/onboarding/complete.
pub fn complete_onboarding() {
    std::thread::spawn(|| {
        let client = DaemonClient::new();
        let _ = client.post_json("/api/v1/onboarding/complete", "");
    });
}

// ---------------------------------------------------------------------------
// UI.
// ---------------------------------------------------------------------------

/// The accent of the onboarding takeover.
///
/// React writes the running light's dominant hue to `--live-h`
/// (`app/web/src/lib/preview.ts`), so the chrome answers the light the owner
/// is being shown. The onboarding's own light is the brand violet (#6000fc,
/// hue 263), and the hello wave paints it: the accent is still the app's
/// blue on the two first screens, where the wave has not settled yet, and
/// violet from Your parts on (measured in the React captures: the logo mark
/// is #80aeef on the Hello and the Case, #ab80ef on the other three).
pub const ONBOARDING_HUE: u16 = 263;
/// The app's own accent (`--live-h: 215`), the hue the chrome has before the
/// onboarding's wave has settled.
pub const APP_HUE: u16 = 215;

/// The theme the takeover draws with: the app's theme on the accent the
/// screen is on.
fn onboard_theme(view: &LayoutView, step: StepKind) -> Theme {
    let hue = match step {
        StepKind::Hello | StepKind::Case => APP_HUE,
        StepKind::Parts | StepKind::Match | StepKind::Finish => ONBOARDING_HUE,
    };
    view.theme.with_live_hue(hue)
}

/// The word space of the UI font at a given weight, in em (the `hmtx` advance
/// of U+0020 in `assets/fonts/schibsted-grotesk-*.ttf`). CSS adds the
/// letter-spacing after the space too, so a tracked line needs it.
const UI_SPACE_EM_600: f32 = 0.22705;
/// The mono font's space: a full 0.6 em (`hmtx`, U+0020 in
/// `assets/fonts/jetbrains-mono-400.ttf`).
const MONO_SPACE_EM: f32 = 0.6;

fn kicker(t: Theme, text: &str) -> gpui::AnyElement {
    crate::components::panel::tracked_text_space(
        text,
        14.0,
        0.01,
        MONO_SPACE_EM,
        gpui::FontWeight::NORMAL,
        crate::fonts::MONO_FAMILY,
        Theme::color(t.mute),
        1.45,
    )
}

fn step_head(t: Theme, kicker: Option<String>, title: gpui::AnyElement, lead: Option<&str>) -> gpui::AnyElement {
    let mut col = div().flex().flex_col().gap(px(8.0));
    if let Some(k) = kicker {
        col = col.child(crate::views::onboarding::kicker(t, &k));
    }
    col = col.child(title);
    if let Some(l) = lead {
        col = col.child(
            div()
                .text_size(px(16.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child(l.to_string())
                .min_w(px(0.0)),
        );
    }
    col.into_any_element()
}

/// The step's title (React's `.title`): 28 px, weight 600, line-height 1.12
/// and `letter-spacing: -0.02em`. The tracking is what keeps "Put in what's
/// in your PC." on one line: without it the line is 14 px wider than the
/// panel and wraps.
fn title_text(t: Theme, text: &str, phone: bool) -> gpui::AnyElement {
    let size = if phone { 22.0 } else { 28.0 };
    crate::components::panel::tracked_text_space_nowrap(
        text,
        size,
        -0.02,
        UI_SPACE_EM_600,
        gpui::FontWeight::SEMIBOLD,
        crate::fonts::UI_FAMILY,
        Theme::color(t.ink),
        1.12,
    )
}

/// One LED dot of the waiting pulse (Gate/Waking): 8 px in --live.
fn waiting_dots(t: Theme) -> gpui::AnyElement {
    let mut row = div().flex().gap(px(6.0));
    for _ in 0..5 {
        row = row.child(div().size(px(8.0)).rounded_full().bg(t.live()).opacity(0.6));
    }
    row.into_any_element()
}

fn hint(t: Theme, text: &str) -> gpui::AnyElement {
    div()
        .text_sm()
        .line_height(gpui::relative(1.45))
        .font_family(crate::fonts::UI_FAMILY)
        .text_color(Theme::color(t.mute))
        .child(text.to_string())
        .min_w(px(0.0))
        .into_any_element()
}

/// The thin tailer helper: a full-height panel body with a pinned foot.
fn foot_primary(t: Theme, id: &str, label: &str, icon: Option<(&str, &str, f32)>, disabled: bool, cx: &mut Context<LayoutView>, on_click: impl Fn(&mut LayoutView, &mut Context<LayoutView>) + 'static) -> gpui::AnyElement {
    let entity = cx.entity();
    let label = label.to_string();
    let el = crate::components::button::button_cta(&t, id, &label, icon, disabled, move |_, _, app| {
        entity.update(app, |this: &mut LayoutView, cx| on_click(this, cx));
    });
    div().w_full().child(el).into_any_element()
}

fn foot_row_ghost(t: Theme, id: &str, label: &str, icon: Option<(&str, &str, f32)>, disabled: bool, cx: &mut Context<LayoutView>, on_click: impl Fn(&mut LayoutView, &mut Context<LayoutView>) + 'static) -> gpui::AnyElement {
    let entity = cx.entity();
    let label = label.to_string();
    match icon {
        Some((name, weight, px_size)) => button_icon(&t, id, &label, Variant::Ghost, disabled, true, name, weight, px_size, move |_, _, app| {
            entity.update(app, |this: &mut LayoutView, cx| on_click(this, cx));
        }).into_any_element(),
        None => crate::components::button::button_sm(&t, id, &label, Variant::Ghost, disabled, move |_, _, app| {
            entity.update(app, |this: &mut LayoutView, cx| on_click(this, cx));
        }).into_any_element(),
    }
}

/// The onboarding takeover body (Onboarding.tsx:405-445).
pub fn render_onboarding(view: &mut LayoutView, window: &mut Window, cx: &mut Context<LayoutView>) -> gpui::AnyElement {
    let phone = view.width <= 960.0;
    let data = view.onboarding.clone();
    let Some(data) = data else {
        return div().flex_1().into_any_element();
    };
    let step = data.step();
    let t = onboard_theme(view, step);
    let status = view.status.clone();
    let devices = view.devices.clone();
    let layout = view.layout().cloned();
    let Some(layout) = layout else {
        return div().flex_1().into_any_element();
    };
    let effects = view.effects.clone().map(|e| e.effects).unwrap_or_default();
    let save_error = view.save_error.clone();

    // Session heartbeat (Onboarding.tsx:127-137): black from after hello until finish.
    let session_on = step != StepKind::Hello && step != StepKind::Finish;
    if session_on && !view.capture_frozen {
        session(true);
    } else if !session_on && !view.capture_frozen {
        session(false);
    }

    // Parts guess on arrival (Onboarding.tsx:306-310).
    if step == StepKind::Parts && !data.guessed {
        let devices_for_guess = devices.clone();
        view.commit(|l| expand_guess(l, devices_for_guess.as_deref()));
        if let Some(d) = view.onboarding.as_mut() {
            d.guessed = true;
        }
    }

    // Case screen mounts: ensure + unlock the case (Onboarding.tsx:148-156).
    if step == StepKind::Case {
        view.ensure_case();
        view.set_case_locked(false);
        if !data.dir_forward {
            view.set_case_locked(false);
        }
    }

    // Hello wave (HelloStep.tsx:46-52): once per mount reaching happy.
    // In capture the wave is not played (frozen) so both editors stay still.
    let found = devices.as_ref().map(|d| d.iter().filter(|x| x.connected).count()).unwrap_or(0);
    let has_conflict = status.as_ref().is_some_and(|s| s.conflicts.iter().any(|c| matches!(c, canvas_proto::api::Conflict::SignalRgb { .. })));
    let waking = status.as_ref().is_some_and(|s| s.openrgb == canvas_proto::api::LinkState::Connecting);
    let mode = hello_mode(has_conflict, waking, found, data.ladder);
    if mode == HelloMode::Happy && !data.waved && !view.capture_frozen {
        if let Some(d) = view.onboarding.as_mut() {
            d.waved = true;
        }
        remember_before(view);
        activate_effect(HELLO_EFFECT, &canvas_proto::props::Props::default());
    }

    // Finish sweep on arrival (FinishStep.tsx:70-76).
    if step == StepKind::Finish && !data.swept && view.capture_frozen {
        // A capture is taken while the sweep is still crossing (React's
        // capture shoots the Finish screen about a second after it arrives,
        // and the sweep takes three), so the capture shows the sweeping
        // screen: "Watch your PC." with the band on the bar and the Done key
        // disabled. The swept screen ("Your PC is mapped." with the tiles)
        // is what the owner sees three seconds later.
        let _ = &data.swept;
    }

    let bar = header_bar(view, window, cx, step, &layout, devices.as_deref(), &data);
    let truth = hardware_line(view, &layout, devices.as_deref(), step, mode == HelloMode::Gate && step == StepKind::Hello, save_error.as_deref());

    let stage = onboarding_stage_for(view, window, cx, step, &data);
    let panel = panel_for(view, window, cx, step, &data, mode, &layout, devices.as_deref(), &effects, save_error.as_deref());

    let main = if phone {
        // The phone takeover (`onboarding.module.css:968`): the drawing sits
        // on its own and the panel is a bottom sheet pinned to the bottom of
        // the window (React's `position: sticky; bottom: 0`). The drawing
        // keeps its own height, so when the sheet is tall the two overlap
        // (React's sheet does the same: it covers the drawing's last rows).
        // The 16 px gap between them is React's own (`gap: var(--s4)`).
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_4()
            .p_4()
            .pb_0()
            .child(stage)
            .child(panel)
    } else {
        div().flex().flex_1().min_h_0().gap_6().p_6().child(stage).child(panel)
    };

    let mut root = div().flex().flex_col().flex_1().min_h_0().min_w_0().text_base().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.ink));
    root = root.child(bar);
    root = root.child(truth);
    root = root.child(main);
    root.into_any_element()
}

fn remember_before(view: &mut LayoutView) {
    let cur_effect = view.layout().and_then(|l| l.effect.clone()).map(|e| (e.id.clone(), e.props.clone()));
    if let Some(data) = view.onboarding.as_mut() {
        if data.before_effect.is_none() {
            data.before_effect = Some(cur_effect);
        }
    }
}

fn restore_effect(view: &mut LayoutView) {
    let before = view.onboarding.as_mut().and_then(|d| d.before_effect.take());
    if let Some(Some((id, props))) = before {
        activate_effect(&id, &props);
    }
}

/// The header: brand, progress (from Parts on), "Set up later".
fn header_bar(view: &mut LayoutView, _window: &mut Window, cx: &mut Context<LayoutView>, step: StepKind, layout: &Layout, devices: Option<&[DeviceInfo]>, _data: &OnboardData) -> gpui::AnyElement {
    let t = onboard_theme(view, step);
    let phone = view.width <= 960.0;
    let show_progress = step != StepKind::Hello && step != StepKind::Case;
    // BrandMark (BrandMark.tsx) + Wordmark (shell.rs): easel 18 px.
    let brand = div()
        .flex()
        .items_center()
        .gap_3()
        .text_color(Theme::color(t.ink))
        .child(
            div()
                .relative()
                .w(px(18.0))
                .h(px(21.0))
                .child(gpui::svg().path("icons/EaselLegs.svg").absolute().inset_0().w_full().h_full().text_color(Theme::color(t.ink)))
                .child(gpui::svg().path("icons/EaselCanvas.svg").absolute().inset_0().w_full().h_full().text_color(t.live())),
        )
        .child(
            div()
                .flex()
                .items_center()
                .text_size(px(15.0))
                .font_weight(gpui::FontWeight(650.0))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child("c")
                .child(div().relative().w(px(9.3)).h(px(10.9)).child(gpui::svg().path("icons/EaselLegs.svg").absolute().inset_0().w_full().h_full().text_color(Theme::color(t.ink))).child(gpui::svg().path("icons/EaselCanvas.svg").absolute().inset_0().w_full().h_full().text_color(t.live())))
                .child("nvas")
                .child(div().font_family(crate::fonts::MONO_FAMILY).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.live()).child("rgb")),
        );
    let later = if step != StepKind::Hello && step != StepKind::Finish {
        foot_row_ghost(t, "onb-later-head", "Set up later", Some(("X", "", 14.0)), false, cx, |this, cx| {
            restore_effect(this);
            this.later_onboarding(cx);
        })
    } else {
        div().into_any_element()
    };
    if phone {
        // React's phone bar (`onboarding.module.css:995`): two rows (the
        // brand and the way out, then the progress), 8 px of padding above
        // and below, an 8 px gap between the rows and one rule under the
        // whole bar. The row with the "Set up later" key is as tall as the
        // key (28 px), not the desktop's 56.
        let mut col = div().flex().flex_col().min_w_0().min_h(px(56.0)).bg(Theme::color(t.panel));
        let top = div()
            .flex()
            .items_center()
            .gap_3()
            .pt_2()
            .px_4()
            .child(div().flex_1().child(brand))
            .child(later);
        col = col.child(top);
        if show_progress {
            col = col.child(
                div()
                    .flex()
                    .mt_2()
                    .px_4()
                    .pb_2()
                    .child(progress_dots(t, layout, devices, step == StepKind::Finish)),
            );
        } else {
            col = col.child(div().h(px(8.0)).flex_none());
        }
        col = col.child(div().h(px(8.0)).flex_none());
        col = col.border_b_1().border_color(Theme::color(t.line));
        col.into_any_element()
    } else {
        // Desktop: one row, progress centred (grid: brand / progress / end).
        let mut row = div()
            .flex()
            .items_center()
            .gap_4()
            .min_h(px(56.0))
            .px_6()
            .border_b_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel));
        row = row.child(brand);
        row = row.child(div().flex_1().flex().justify_center().min_w_0().child(if show_progress {
            progress_dots(t, layout, devices, step == StepKind::Finish)
        } else {
            div().into_any_element()
        }));
        row = row.child(later);
        row.into_any_element()
    }
}

/// The hardware-truth line (HardwareLine.tsx).
fn hardware_line(view: &mut LayoutView, layout: &Layout, devices: Option<&[DeviceInfo]>, step: StepKind, gated: bool, save_error: Option<&str>) -> gpui::AnyElement {
    let t = onboard_theme(view, step);
    let status = view.status.clone();
    let phone = view.width <= 960.0;
    let devices_for_line = if step == StepKind::Hello || step == StepKind::Case { None } else { devices };
    let mut items: Vec<String> = Vec::new();
    if status.as_ref().is_some_and(|s| s.conflicts.iter().any(|c| matches!(c, canvas_proto::api::Conflict::SignalRgb { .. }))) && !gated {
        items.push("SignalRGB is running".to_string());
    }
    if status.as_ref().is_some_and(|s| s.openrgb == canvas_proto::api::LinkState::Down) {
        items.push("OpenRGB is not answering".to_string());
    }
    let missing = missing_parts(layout, devices_for_line);
    if missing > 0 && status.as_ref().is_some_and(|s| s.openrgb == canvas_proto::api::LinkState::Connected) {
        items.push(if missing == 1 { "1 light is missing".to_string() } else { format!("{missing} lights are missing") });
    }
    if let Some(e) = save_error {
        items.push(format!("Not saved: {e}"));
    }
    if items.is_empty() {
        return div().into_any_element();
    }
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .px(if phone { px(16.0) } else { px(24.0) })
        .py_2()
        .border_b_1()
        .border_color(Theme::color(t.line))
        .bg(Theme::color(t.warn_wash))
        .text_color(Theme::color(t.dim))
        .text_sm();
    for item in items {
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().size(px(8.0)).rounded_full().bg(Theme::color(t.warn)))
                .child(item),
        );
    }
    row.into_any_element()
}

fn progress_dots(t: Theme, layout: &Layout, devices: Option<&[DeviceInfo]>, celebrate: bool) -> gpui::AnyElement {
    let devs = devices.unwrap_or(&[]);
    let parts = progress_parts(layout, devs);
    let mapped = parts.iter().filter(|p| p.mapped).count();
    let mut dots = div().flex().flex_wrap().items_center().gap(px(6.0)).min_w_0();
    for p in &parts {
        let bg = if p.mapped { Theme::color(t.a3) } else { Theme::color(t.line_2) };
        dots = dots.child(div().size(px(9.0)).rounded_full().bg(bg));
    }
    let _ = celebrate;
    // React's count (`ProgressDots.tsx:47-49`): the mapped number sits in
    // its own cell, so "0" stands where a "9" would; the total is padded to
    // two digits. The mono font's space and its "0" are both a 0.6 em wide
    // (`hmtx`), so the cells below are exact.
    const MONO_EM: f32 = 0.6;
    let size = 14.0;
    let count = div()
        .flex()
        .items_center()
        .flex_none()
        .text_size(px(size))
        .font_family(crate::fonts::MONO_FAMILY)
        .text_color(Theme::color(t.dim))
        .whitespace_nowrap()
        .child(div().flex_none().child(format!("{mapped}")))
        .child(div().w(px(size * MONO_EM)).flex_none())
        .child("/")
        .child(div().w(px(size * MONO_EM)).flex_none())
        .child(format!("{:02}", parts.len()));
    div()
        .flex()
        .items_center()
        .gap_4()
        .min_w_0()
        .child(dots)
        .child(count)
        .into_any_element()
}

/// The stage column: the desk, in its frame, and (desktop only) its toolbar.
fn onboarding_stage_for(view: &mut LayoutView, window: &mut Window, cx: &mut Context<LayoutView>, step: StepKind, _data: &OnboardData) -> gpui::AnyElement {
    let t = onboard_theme(view, step);
    let phone = view.width <= 960.0;
    let colours = DeskTokens::for_theme(&t);
    let layout = view.layout().cloned();
    let devices = view.devices().to_vec();
    let Some(layout) = layout else {
        return div().flex_1().into_any_element();
    };
    let _ = window;
    let desk_input = DeskInput {
        zones: layout.zones.clone(),
        landmarks: layout.landmarks.clone(),
        case_form: layout.case_form,
        devices: devices.clone(),
        unit: view.unit,
        mode: view.mode,
        sampling: layout.sampling,
        frame: view.feed.frame(),
        selection: view.selection.clone(),
        view: view.view,
        overlay: view.overlay.clone(),
        unlock_badge: true,
    };
    let desk_origin = view.desk_origin.clone();
    let desk = desk_with_hint(&t, &colours, desk_input, desk_origin, true);
    let desk = desk_handlers(desk, cx.entity());
    // React's desk is the stage itself (`desk.module.css` `.desk.canvas`): a
    // bordered, rounded panel with the drawing in it. On the desktop the
    // toolbar is its own row under it (not inside a frame, as on the Layout
    // page); on a phone the toolbar is not drawn at all.
    let frame = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .r_lg()
        .bg(Theme::color(t.sunk))
        .overflow_hidden()
        .child(desk);
    if phone {
        // The drawing fills what the sheet leaves (onboarding.module.css
        // :1027-1033): React's `.stage` is `flex: 1 1 auto` and the desk canvas
        // inside it is `flex: 1 1 0; height: 100%`, so the desk takes the whole
        // stage - it does not keep its 320:200 proportions when the sheet is
        // short. Measured in the React capture at 390x844: the desk runs from
        // y 72 to y 396.52 on the Case screen (the sheet's top edge is at
        // 428.52), and from y 97.3 to y 321.05 on the Parts screen (the sheet
        // is taller there and covers the drawing's last rows).
        //
        // The desk's own 320:200 height is the floor: the sheet's height is
        // what it is, and when it is taller than the window the drawing keeps
        // its proportions and the sheet covers it (React's sticky sheet).
        let aspect = ((view.width - 32.0) * 0.625).max(160.0);
        div()
            .flex()
            .flex_col()
            .flex_auto()
            .flex_basis(px(aspect + 16.0))
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(aspect + 16.0))
                    .child(
                        // The stage's own 16 px of padding under the drawing
                        // (onboarding.module.css:1032).
                        div().flex().flex_col().flex_1().min_h_0().pb_4().child(frame),
                    ),
            )
            .into_any_element()
    } else {
        // The desk's own box, in css pixels: the stage column (the window
        // less the page's padding, the gap and the 360 px panel), less the
        // desk's border. The zoom is read from it, as React reads it from
        // the desk's own client box.
        let desk_box = (view.width - 434.0, view.height - 160.0);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_3()
            .child(frame)
            .child(stage_toolbar_onboarding(view, &t, desk_box, cx))
            .into_any_element()
    }
}

// The desk's pointer handlers, reused from layout.rs (desk_handlers is private
// there, so the handlers are attached by LayoutView's own helper below).
fn desk_handlers(el: gpui::AnyElement, entity: gpui::Entity<LayoutView>) -> gpui::AnyElement {
    crate::views::layout::desk_handlers_pub(el, entity)
}

fn stage_toolbar_onboarding(view: &mut LayoutView, t: &Theme, desk: (f32, f32), cx: &mut Context<LayoutView>) -> gpui::AnyElement {
    crate::views::layout::stage_toolbar_onboarding_pub(view, t, desk, cx)
}

/// The panel: one step's body (Onboarding.tsx:438-441).
#[allow(clippy::too_many_arguments)]
fn panel_for(
    view: &mut LayoutView,
    window: &mut Window,
    cx: &mut Context<LayoutView>,
    step: StepKind,
    data: &OnboardData,
    mode: HelloMode,
    layout: &Layout,
    devices: Option<&[DeviceInfo]>,
    effects: &[canvas_proto::api::EffectSummary],
    _save_error: Option<&str>,
) -> gpui::AnyElement {
    let t = onboard_theme(view, step);
    let phone = view.width <= 960.0;
    let body = match step {
        StepKind::Hello => hello_body(view, cx, mode, devices),
        StepKind::Case => case_body(view, cx, layout, phone),
        StepKind::Parts => parts_body(view, cx, layout, devices, phone),
        StepKind::Match => match_body(view, cx, layout, devices, phone),
        StepKind::Finish => finish_body(view, cx, layout, devices, effects, phone),
    };
    let screen = format!("Screen {} of {}", data.at + 1, data.steps.len());
    let _ = screen;
    let panel = div()
        .flex()
        .flex_col()
        .w(if phone { gpui::px(0.0) } else { gpui::px(360.0) })
        .min_w_0()
        .min_h_0()
        .r_md()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .bg(Theme::color(t.panel))
        .overflow_hidden()
        .child(body);
    let panel = if phone {
        // React's sheet (the 960 px rule): a top border and the top corners
        // only, the full width of the window and no more than 66 vh tall.
        // It sits at the bottom of the window: the drawing above it takes
        // what is left (`flex: 1 1 auto`), and when the sheet is taller than
        // the window the drawing gives way instead (React's sticky sheet).
        let mut panel = panel
            .flex()
            .flex_col()
            .flex_none()
            // The sheet is the width of the window: React's negative side
            // margins widen it past the page's own padding.
            .w(px(view.width))
            .max_h(px(view.height * 0.66))
            .min_h_0()
            .ml(px(-16.0))
            .mr(px(-16.0))
            .border_0()
            .border_t_1()
            .border_color(Theme::color(t.line_2));
        let corner = gpui::AbsoluteLength::Pixels(gpui::px(10.0));
        panel.style().corner_radii.top_left = Some(corner);
        panel.style().corner_radii.top_right = Some(corner);
        panel.style().corner_radii.bottom_right = None;
        panel.style().corner_radii.bottom_left = None;
        panel
    } else {
        panel.flex_none()
    };
    let _ = window;
    panel.into_any_element()
}

/// One screen of the flow: the head, the body and the foot. React's `.step`
/// is a column with a 20 px gap and 24 px of padding, and the foot is pinned
/// to the bottom of the panel (`margin-top: auto`), so the primary action is
/// in the same place on every screen.
///
/// The free space is shared with auto margins, as React does it: the centred
/// block's `margin-block: auto` and the foot's `margin-top: auto` split it
/// three ways. A flexible spacer would not do: it would grow the screen's own
/// height to whatever the window had left, and the sheet would then be taller
/// than its content (React's sheet is exactly as tall as its content).
fn step_frame(_t: Theme, phone: bool, center: bool, head: gpui::AnyElement, body: Option<gpui::AnyElement>, foot: gpui::AnyElement) -> gpui::AnyElement {
    let pad_v = if phone { px(20.0) } else { px(24.0) };
    let pad_h = if phone { px(16.0) } else { px(24.0) };
    // React's `.step` gap is `--s4` on a phone (the 960 px rule) and `--s5`
    // on the desktop.
    let gap = if phone { px(16.0) } else { px(20.0) };
    // On a phone the sheet is pinned to the bottom of the window and takes
    // the height of its own content (React's sheet is `position: sticky`,
    // so it is as tall as it needs to be); on the desktop the panel has a
    // height of its own and the screen fills it.
    let mut col = if phone {
        div().flex().flex_col().flex_none().px(pad_h).py(pad_v)
    } else {
        div().flex().flex_col().flex_1().min_h_0().px(pad_h).py(pad_v)
    };
    let mut inner = div().flex().flex_col().gap(gap);
    inner = inner.child(head);
    if let Some(b) = body {
        inner = inner.child(b);
    }
    col = col.gap(gap);
    if phone {
        // The sheet is as tall as its content (React's sticky sheet): the
        // block sits at the top of the sheet and the foot under it.
        col = col.child(inner);
        col = col.child(div().flex().flex_col().child(foot));
    } else if center {
        // React's `.centerBlock` has `margin-block: auto` and the foot has
        // `margin-top: auto`: the free space of the panel is split three
        // ways, so the block sits a third of the way down the panel and the
        // foot a third of the way up from its own bottom. The 20 px gap
        // between the block and the foot is a spacer of its own, so the
        // three shares stay equal.
        col = col.gap_0();
        col = col.child(div().flex_1());
        col = col.child(inner);
        col = col.child(div().flex_1());
        col = col.child(div().flex_none().h(gap));
        col = col.child(div().flex_1());
        col = col.child(foot);
    } else {
        col = col.child(inner);
        // The foot's own margin-top: auto: it sits on the panel's bottom edge
        // (React's `.foot`).
        col = col.child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .flex_col()
                .justify_end()
                .child(foot),
        );
    }
    col.into_any_element()
}

fn count_row(t: Theme, value: Option<usize>) -> gpui::AnyElement {
    div()
        .flex()
        .items_baseline()
        .gap_3()
        .font_family(crate::fonts::MONO_FAMILY)
        .child(
            div()
                .min_w(px(24.0))
                .text_size(px(40.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .line_height(gpui::relative(1.0))
                .text_color(if value.is_some() { t.live() } else { Theme::color(t.mute) })
                .child(value.map(|v| v.to_string()).unwrap_or("—".to_string())),
        )
        .child(
            div()
                .text_size(px(16.0))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child(if value == Some(1) { "light found".to_string() } else { "lights found".to_string() }),
        )
        .into_any_element()
}

fn hello_body(view: &mut LayoutView, cx: &mut Context<LayoutView>, mode: HelloMode, devices: Option<&[DeviceInfo]>) -> gpui::AnyElement {
    let t = onboard_theme(view, StepKind::Hello);
    let phone = view.width <= 960.0;
    let found = devices.map(|d| d.iter().filter(|x| x.connected).count()).unwrap_or(0);
    let data = view.onboarding.clone().unwrap();
    match mode {
        HelloMode::Gate => {
            let head = step_head(
                t,
                Some("SignalRGB".to_string()),
                title_text(t, "SignalRGB has your lights.", phone),
                Some("Quit it from the tray."),
            );
            let watching = div().flex().items_center().gap_3().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.dim)).child(waiting_dots(t)).child("Watching for it to quit").into_any_element();
            let foot = div()
                .flex()
                .flex_col()
                .gap_2()
                .pt(px(16.0))
                .child(foot_primary(t, "onb-gate-check", "Check again", None, false, cx, |this, cx| {
                    this.poll_status_effects(cx);
                    cx.notify();
                }))
                .child(
                    div().flex().justify_between().items_center().min_h(px(32.0)).child(foot_row_ghost(t, "onb-gate-cont", "Continue without lights", None, false, cx, |this, cx| {
                        this.goto_next(cx);
                    })).child(foot_row_ghost(t, "onb-gate-later", "Set up later", None, false, cx, |this, cx| {
                            restore_effect(this);
                            this.later_onboarding(cx);
                        })),
                )
                .into_any_element();
            step_frame(t, phone, false, head, Some(watching), foot)
        }
        HelloMode::Waking => {
            let greeting = greeting_el(t, phone);
            let head = step_head(t, None, greeting, Some("This takes a moment."));
            let body = div().flex().flex_col().gap_4().child(waiting_dots(t)).child(count_row(t, None)).into_any_element();
            let title = head;
            let foot = div()
                .flex()
                .flex_col()
                .gap_2()
                .pt(px(16.0))
                .child(foot_primary(t, "onb-wake", "Map my PC", Some(("ArrowRight", "bold", 16.0)), true, cx, |_, _| {}))
                .child(
                    div().flex().justify_between().items_center().min_h(px(32.0)).child(div().into_any_element()).child(foot_row_ghost(t, "onb-wake-later", "Set up later", None, false, cx, |this, cx| {
                        restore_effect(this);
                        this.later_onboarding(cx);
                    })),
                )
                .into_any_element();
            step_frame(t, phone, true, title, Some(body), foot)
        }
        HelloMode::Ladder => {
            let volunteer = found > 0;
            let has_back = volunteer;
            let title = if volunteer { "Nothing lit up?" } else { "No lights found." };
            let head = step_head(t, None, title_text(t, title, phone), Some("Try these, top to bottom."));
            let no_other = view.status.as_ref().map(|s| s.conflicts.is_empty()).unwrap_or(true);
            let mut ladder = div().flex().flex_col().border_1().border_color(Theme::color(t.line)).r_sm().overflow_hidden();
            for (text, tag) in [
                ("Quit other RGB apps.", if no_other { Some("SignalRGB not seen".to_string()) } else { None }),
                ("Replug it, or try another port.", None),
                ("Turn off Fast Startup.", None),
                ("Rescan.", None),
            ] {
                let done = text == "Quit other RGB apps." && no_other;
                let mut rung = div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_3()
                    .border_b_1()
                    .border_color(Theme::color(t.line))
                    .text_size(px(15.0))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(if done { Theme::color(t.dim) } else { Theme::color(t.ink) })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(20.0))
                            .rounded_full()
                            .border_1()
                            .border_color(Theme::color(t.line_2))
                            .bg(if done { Theme::color(t.ok) } else { Theme::color(t.panel) })
                            .text_color(Theme::color(t.on_accent))
                            .text_size(px(12.0))
                            .font_family(crate::fonts::UI_FAMILY)
                            .child(if done { "✓".to_string() } else { "•".to_string() }),
                    )
                    .child(text.to_string());
                if let Some(tag) = tag {
                    rung = rung.child(div().font_family(crate::fonts::MONO_FAMILY).text_sm().text_color(Theme::color(t.mute)).child(tag));
                }
                ladder = ladder.child(rung);
            }
            let mut body = div().flex().flex_col().gap_4().child(ladder);
            if data.asked && !data.busy && !volunteer {
                body = body.child(hint(t, "Still no lights."));
            }
            let fold = {
                let fold = div().flex().flex_col();
                let mut f = fold.child(
                    div().flex().items_center().gap(px(6.0)).py_1().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.dim)).child(if data.fold_open { "▾".to_string() } else { "▸".to_string() }).child("More help"),
                );
                if data.fold_open {
                    f = f.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .px_3()
                            .py_3()
                            .border_l_1()
                            .border_color(Theme::color(t.line))
                            .ml(px(5.0))
                            .child(hint(t, "Close iCUE, Armoury Crate and Razer Synapse."))
                            .child(hint(t, "Plug USB lights straight into the PC."))
                            .child(hint(t, "Restart Windows, not just shut down.")),
                    );
                }
                f.into_any_element()
            };
            body = body.child(fold);
            // The fold toggle is the header above (clickable via foot_row? keep a ghost button below for now)
            let foot = div()
                .flex()
                .flex_col()
                .gap_2()
                .pt(px(16.0))
                .child(foot_primary(t, "onb-ladder-rescan", if data.busy { "Looking…" } else { "Rescan" }, Some(("ArrowsClockwise", "", 16.0)), data.busy, cx, |this, cx| {
                    this.start_rescan(cx);
                }))
                .child(
                    div().flex().justify_between().items_center().min_h(px(32.0)).child(if has_back {
                        foot_row_ghost(t, "onb-ladder-back", "Back", Some(("ArrowLeft", "", 14.0)), false, cx, |this, cx| {
                            if let Some(d) = this.onboarding.as_mut() { d.ladder = false; }
                            cx.notify();
                        })
                    } else { div().into_any_element() }).child(foot_row_ghost(t, "onb-ladder-cont", "Continue without lights", None, false, cx, |this, cx| {
                        this.goto_next(cx);
                    })),
                )
                .into_any_element();
            step_frame(t, phone, false, head, Some(body.into_any_element()), foot)
        }
        HelloMode::Happy => {
            let greeting = greeting_el(t, phone);
            let head = div().flex().flex_col().child(greeting).into_any_element();
            let body = div().flex().flex_col().gap_4().child(count_row(t, Some(found))).into_any_element();
            let foot = div()
                .flex()
                .flex_col()
                .gap_2()
                .pt(px(16.0))
                .child(foot_primary(t, "onb-hello-start", "Map my PC", Some(("ArrowRight", "bold", 16.0)), false, cx, |this, cx| {
                    // start(): rebuild steps and go to Case (setAt(1)).
                    if let Some(d) = this.onboarding.as_mut() {
                        if d.scope.is_none() {
                            let devices = this.devices.clone();
                            let _ = devices;
                            d.steps = build_steps(d.scope.as_ref());
                        }
                        if d.steps.len() > 1 { d.go(1); } else { d.go(0); }
                        d.dir_forward = true;
                    }
                    this.sync_onboarding_match_run();
                    this.ensure_case();
                    cx.notify();
                }))
                .child(
                    div().flex().justify_between().items_center().min_h(px(32.0)).child(foot_row_ghost(t, "onb-hello-nothing", "Nothing lit up?", None, false, cx, |this, cx| {
                        if let Some(d) = this.onboarding.as_mut() { d.ladder = true; }
                        cx.notify();
                    })).child(foot_row_ghost(t, "onb-hello-later", "Set up later", None, false, cx, |this, cx| {
                        restore_effect(this);
                        this.later_onboarding(cx);
                    })),
                )
                .into_any_element();
            step_frame(t, phone, true, head, Some(body), foot)
        }
    }
}

fn greeting_el(t: Theme, phone: bool) -> gpui::AnyElement {
    let word = hello_for("en");
    let size = if phone { 52.0 } else { 64.0 };
    div()
        .flex()
        .text_size(px(size))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .line_height(gpui::relative(1.0))
        .font_family(crate::fonts::UI_FAMILY)
        .text_color(Theme::color(t.ink))
        .child(word.to_string())
        .child(div().text_color(t.live()).child("."))
        .into_any_element()
}

fn case_body(view: &mut LayoutView, cx: &mut Context<LayoutView>, layout: &Layout, phone: bool) -> gpui::AnyElement {
    let t = onboard_theme(view, StepKind::Case);
    let data = view.onboarding.clone().unwrap();
    let case_box = layout.landmarks.iter().find(|l| l.kind == PartKind::Case).cloned();
    let at_default = case_box.as_ref().is_some_and(|c| {
        (c.width - 120.0).abs() < 0.01 && (c.height - 122.0).abs() < 0.01 && (c.x + c.width / 2.0 - CASE_ANCHOR_X as f32).abs() < 0.5 && (c.y + c.height - CASE_ANCHOR_Y as f32).abs() < 0.5
    });
    let _ = at_default;
    let head = step_head(t, Some("Case".to_string()), title_text(t, "Is this your case?", phone), Some("A typical mid tower. Adjust only if yours is clearly bigger or smaller."));
    let size_row = div()
        .flex()
        .items_baseline()
        .justify_between()
        .gap_3()
        .px_4()
        .py_4()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .r_sm()
        .bg(Theme::color(t.sunk))
        .child(
            div().flex_1().font_family(crate::fonts::MONO_FAMILY).text_size(px(28.0)).text_color(Theme::color(t.ink)).child(case_box.as_ref().map(|c| case_text(f64::from(c.width), view.unit)).unwrap_or("—".to_string())),
        )
        .child(div().font_family(crate::fonts::MONO_FAMILY).text_color(Theme::color(t.mute)).child("×"))
        .child(
            div().flex_1().text_right().font_family(crate::fonts::MONO_FAMILY).text_size(px(28.0)).text_color(Theme::color(t.ink)).child(case_box.as_ref().map(|c| case_text(f64::from(c.height), view.unit)).unwrap_or("—".to_string())),
        )
        .into_any_element();
    let unit_row = {
        let entity = cx.entity();
        let unit = view.unit;
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            // The hint takes what the unit switch leaves (React's `.unitRow`
            // is a space-between row and the hint wraps in what is left).
            .child(div().flex_1().min_w(px(0.0)).child(hint(t, "The dashed outlines are other tower sizes, to compare.")))
            // React's `.unitSwitch` is `flex: none`: the switch keeps its
            // width and the hint wraps in what is left.
            .child(div().flex_none().child(segmented(&t, "onb-unit", &[("cm", "cm"), ("in", "in")], match unit { LengthUnit::Cm => "cm", LengthUnit::In => "in" }, move |value, _w, cx| {
                entity.update(cx, |this: &mut LayoutView, cx| {
                    this.unit = if value == "in" { LengthUnit::In } else { LengthUnit::Cm };
                    cx.notify();
                });
            })))
            .into_any_element()
    };
    let body = div().flex().flex_col().gap_4().child(size_row).child(unit_row).into_any_element();
    let has_back = data.back().is_some();
    let foot = div()
        .flex()
        .flex_col()
        .gap_2()
        .pt(px(16.0))
        .child(foot_primary(t, "onb-case-ok", "Looks right", Some(("ArrowRight", "bold", 16.0)), false, cx, |this, cx| {
            this.set_case_locked(true);
            if let Some(d) = this.onboarding.as_mut() { d.next_step(); }
            this.sync_onboarding_match_run();
            cx.notify();
        }))
        .child(
            div().flex().justify_between().items_center().min_h(px(32.0)).child(if has_back {
                foot_row_ghost(t, "onb-case-back", "Back", Some(("ArrowLeft", "", 14.0)), false, cx, |this, cx| {
                    if let Some(b) = this.onboarding.as_ref().and_then(|d| d.back()) { if let Some(d) = this.onboarding.as_mut() { d.go(b); } }
                    cx.notify();
                })
            } else { div().into_any_element() }).child(foot_row_ghost(t, "onb-case-reset", "Reset", Some(("ArrowCounterClockwise", "", 14.0)), false, cx, |this, cx| {
                this.set_case_box(Some(120.0), Some(122.0), None);
                cx.notify();
            })),
        )
        .into_any_element();
    step_frame(t, phone, true, head, Some(body), foot)
}

fn parts_body(view: &mut LayoutView, cx: &mut Context<LayoutView>, layout: &Layout, devices: Option<&[DeviceInfo]>, phone: bool) -> gpui::AnyElement {
    let t = onboard_theme(view, StepKind::Parts);
    let data = view.onboarding.clone().unwrap();
    let devs = devices.unwrap_or(&[]);
    let missing = missing_parts(layout, devices);
    let lit = layout.zones.len();
    let dark = layout.landmarks.iter().filter(|l| l.kind != PartKind::Case).count();
    let removable = view.selection.iter().filter(|s| matches!(s, Sel::Zone(_)) || matches!(s, Sel::Landmark(i) if layout.landmarks.get(*i).map(|l| l.kind) != Some(PartKind::Case))).count();
    let head = step_head(t, Some("Your parts".to_string()), title_text(t, "Put in what's in your PC.", phone), Some("Start from this guess: drag a part where it really sits, remove what you don't have, add what's missing."));
    let showing = div()
        .relative()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .px_4()
        .py_3()
        .border_1()
        // React's `.showing` has a 3 px left border in the live colour.
        .border_l_3()
        .border_color(Theme::color(t.line_2))
        .r_sm()
        .bg(Theme::color(t.sunk))
        .child(
            div()
                .absolute()
                .left(px(-3.0))
                .top_0()
                .bottom_0()
                .w(px(3.0))
                .bg(t.live()),
        )
        .child(div().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.mute)).child("On the drawing"))
        .child(
            div()
                .flex()
                .items_baseline()
                .text_size(px(20.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                // The count is in the mono font (React's `.fig`).
                .child(div().font_family(crate::fonts::MONO_FAMILY).child(format!("{}", lit + dark)))
                .child(" parts"),
        )
        .into_any_element();
    let mut body = div().flex().flex_col().gap_4().child(showing);
    if missing > 0 {
        let missing_text = if missing == 1 { "1 light was not found.".to_string() } else { format!("{missing} lights were not found.") };
        body = body.child(hint(t, &missing_text));
    }
    body = body.child(hint(t, &format!("Parts snap to edges. Sizes show in {} as you drag.", if view.unit == LengthUnit::In { "inches" } else { "cm" })));
    let add_row = {
        let mut row = div().flex().flex_wrap().gap_2();
        let entity = cx.entity();
        // React's `PartsStep.tsx`: the four parts that take a run of lights,
        // then the two that do not, each a chip of its icon, its name and a
        // bold plus.
        for (kind, label, icon) in [
            (PartKind::Fan, "Fan", "Fan"),
            (PartKind::Strip, "Strip", "Minus"),
            (PartKind::Pump, "Pump", "Stack"),
            (PartKind::Ram, "RAM", "Package"),
        ] {
            let e = entity.clone();
            row = row.child(crate::components::chip::chip_add_part(&t, &format!("onb-add-{label}"), label, (icon, "", 14.0), move |_, _, cx| {
                e.update(cx, |this: &mut LayoutView, cx| {
                    this.add_lit_part(kind, cx);
                });
            }));
        }
        for (kind, label, icon) in [(PartKind::Gpu, "GPU", "Stack"), (PartKind::Cpu, "CPU", "Cpu")] {
            let e = entity.clone();
            row = row.child(crate::components::chip::chip_add_part(&t, &format!("onb-add-{label}"), label, (icon, "", 14.0), move |_, _, cx| {
                e.update(cx, |this: &mut LayoutView, cx| {
                    this.add_landmark(kind);
                    cx.notify();
                });
            }));
        }
        row
    };
    let miss_row = div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .px_3()
        .py_1()
        .border_1()
        .border_color(Theme::color(t.warn))
        .r_sm()
        .bg(Theme::color(t.warn_wash))
        .text_sm()
        .font_family(crate::fonts::UI_FAMILY)
        .text_color(Theme::color(t.dim))
        .child(foot_row_ghost(t, "onb-remove", &format!("Remove {}", if removable > 1 { "parts" } else { "part" }), Some(("Trash", "", 14.0)), removable == 0, cx, |this, cx| {
            this.remove_selected_parts();
            cx.notify();
        }))
        .child(if missing > 0 {
            foot_row_ghost(t, "onb-parts-rescan", "Rescan", Some(("ArrowsClockwise", "", 14.0)), data.busy, cx, |this, cx| {
                this.start_rescan(cx);
            })
        } else { div().into_any_element() })
        .into_any_element();
    body = body.child(div().flex().flex_col().gap_3().child(add_row).child(miss_row));
    if let Some(note) = &data.note {
        body = body.child(hint(t, note));
    }
    let has_back = data.back().is_some();
    let foot = div()
        .flex()
        .flex_col()
        .gap_2()
        .pt(px(16.0))
        .child(foot_primary(t, "onb-parts-next", "Match the lights", Some(("ArrowRight", "bold", 16.0)), false, cx, |this, cx| {
            if let Some(d) = this.onboarding.as_mut() { d.next_step(); }
            this.sync_onboarding_match_run();
            cx.notify();
        }))
        .child(
            div().flex().justify_between().items_center().min_h(px(32.0)).child(if has_back {
                foot_row_ghost(t, "onb-parts-back", "Back", Some(("ArrowLeft", "", 14.0)), false, cx, |this, cx| {
                    if let Some(b) = this.onboarding.as_ref().and_then(|d| d.back()) { if let Some(d) = this.onboarding.as_mut() { d.go(b); } }
                    cx.notify();
                })
            } else { div().into_any_element() }).child(div().into_any_element()),
        )
        .into_any_element();
    let _ = devs;
    step_frame(t, phone, false, head, Some(body.into_any_element()), foot)
}

fn match_body(view: &mut LayoutView, cx: &mut Context<LayoutView>, layout: &Layout, _devices: Option<&[DeviceInfo]>, phone: bool) -> gpui::AnyElement {
    let t = onboard_theme(view, StepKind::Match);
    let data = view.onboarding.clone().unwrap();
    let zones = view.onboarding_match_zones();
    let total = all_runs(&zones).len();
    let done_at = data.run.as_ref().map(|r| all_runs(&zones).iter().position(|x| x.zi == r.zi && x.start >= r.start).unwrap_or(total)).unwrap_or(total);
    let kicker = if data.run.is_some() { format!("Match the lights · {} of {}", done_at + 1, total) } else { "Match the lights".to_string() };
    let has_back = data.back().is_some();
    if data.run.is_none() {
        let head = step_head(t, Some(kicker), title_text(t, "Every light is matched.", phone), Some("Each part now knows which lights are its own."));
        let foot = div()
            .flex()
            .flex_col()
            .gap_2()
            .pt(px(16.0))
            .child(foot_primary(t, "onb-match-cont", "Continue", Some(("ArrowRight", "bold", 16.0)), false, cx, |this, cx| {
                if let Some(d) = this.onboarding.as_mut() { d.next_step(); }
                this.sync_onboarding_match_run();
                cx.notify();
            }))
            .child(div().flex().justify_between().items_center().min_h(px(32.0)).child(if has_back {
                foot_row_ghost(t, "onb-match-back", "Back", Some(("ArrowLeft", "", 14.0)), false, cx, |this, cx| {
                    if let Some(b) = this.onboarding.as_ref().and_then(|d| d.back()) { if let Some(d) = this.onboarding.as_mut() { d.go(b); } }
                    cx.notify();
                })
            } else { div().into_any_element() }).child(div().into_any_element()))
            .into_any_element();
        return step_frame(t, phone, false, head, None, foot);
    }
    let run = data.run.clone().unwrap();
    if data.tapped.is_some() {
        let can = zones.get(run.zi).map(|z| z.resizable).unwrap_or(false);
        let head = step_head(
            t,
            Some(kicker),
            title_text(t, "Is it lit all the way around?", phone),
            Some(if data.resizing { "Change how many lights it has until the whole part is lit." } else { "Look at the whole part, end to end." }),
        );
        let body = if data.resizing {
            let resizable_zone = zones.get(run.zi).cloned();
            let max = resizable_zone.as_ref().map(|z| z.max_leds).unwrap_or(1);
            let count = data.count;
            let entity = cx.entity();
            let fewer_entity = entity.clone();
            let more_entity = entity.clone();
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_4()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .r_sm()
                    .bg(Theme::color(t.sunk))
                    .child(icon_button(&t, "onb-fewer", "Minus", "", false, false, count <= 1, move |_, _, app| {
                        fewer_entity.update(app, |this: &mut LayoutView, cx| { this.resize_match_count(count.saturating_sub(1)); cx.notify(); });
                    }))
                    .child(div().flex_1().text_center().font_family(crate::fonts::MONO_FAMILY).text_size(px(28.0)).text_color(Theme::color(t.ink)).child(format!("{count}")))
                    .child(icon_button(&t, "onb-more", "Plus", "", false, false, count >= max, move |_, _, app| {
                        more_entity.update(app, |this: &mut LayoutView, cx| { this.resize_match_count(count + 1); cx.notify(); });
                    }))
                    .into_any_element(),
            )
        } else { None };
        let secondary = if data.resizing { None } else { Some(("No", "onb-match-no")) };
        let foot = div()
            .flex()
            .flex_col()
            .gap_2()
            .pt(px(16.0))
            .child(foot_primary(t, "onb-match-yes", if data.resizing { "That's all of it" } else { "Yes" }, Some(("Check", "bold", 16.0)), false, cx, |this, cx| {
                this.advance_match(cx);
            }))
            .child(
                div().flex().justify_between().items_center().min_h(px(32.0)).child(if has_back {
                    foot_row_ghost(t, "onb-match-back2", "Back", Some(("ArrowLeft", "", 14.0)), false, cx, |this, cx| {
                        if let Some(b) = this.onboarding.as_ref().and_then(|d| d.back()) { if let Some(d) = this.onboarding.as_mut() { d.go(b); } }
                        cx.notify();
                    })
                } else { div().into_any_element() }).child(match secondary {
                    Some((label, id)) => foot_row_ghost(t, id, label, None, false, cx, move |this, cx| {
                        if can {
                            if let Some(d) = this.onboarding.as_mut() { d.resizing = true; }
                        } else {
                            this.undo_match_tap();
                        }
                        cx.notify();
                    }),
                    None => div().into_any_element(),
                }),
            )
            .into_any_element();
        return step_frame(t, phone, false, head, body, foot);
    }
    let head = step_head(t, Some(kicker), title_text(t, "Which part is lit?", phone), Some("Everything else in your PC is off. Tap the lit part on the drawing."));
    let body = div().flex().flex_col().gap_4().child(hint(t, "If you can't see it, it may be behind something. Skip it and the next one lights.")).into_any_element();
    let foot = div()
        .flex()
        .flex_col()
        .gap_2()
        .pt(px(16.0))
        .child(foot_primary(t, "onb-match-nothing", "Nothing's lit", None, false, cx, |this, cx| {
            this.skip_match_run(cx);
        }))
        .child(div().flex().justify_between().items_center().min_h(px(32.0)).child(if has_back {
            foot_row_ghost(t, "onb-match-back3", "Back", Some(("ArrowLeft", "", 14.0)), false, cx, |this, cx| {
                if let Some(b) = this.onboarding.as_ref().and_then(|d| d.back()) { if let Some(d) = this.onboarding.as_mut() { d.go(b); } }
                cx.notify();
            })
        } else { div().into_any_element() }).child(div().into_any_element()))
        .into_any_element();
    let _ = layout;
    step_frame(t, phone, false, head, Some(body), foot)
}

fn finish_body(view: &mut LayoutView, cx: &mut Context<LayoutView>, layout: &Layout, devices: Option<&[DeviceInfo]>, effects: &[canvas_proto::api::EffectSummary], phone: bool) -> gpui::AnyElement {
    let t = onboard_theme(view, StepKind::Finish);
    let data = view.onboarding.clone().unwrap();
    let scoped = data.scope.is_some();
    if !data.swept {
        let head = step_head(t, Some("Finish".to_string()), title_text(t, "Watch your PC.", phone), Some("A band of light crosses it."));
        let body = div().flex().items_center().gap_3().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.dim)).child(div().w_full().h(px(6.0)).rounded_full().bg(Theme::color(t.line_2))).into_any_element();
        let foot = div().flex().flex_col().gap_2().pt(px(16.0)).child(foot_primary(t, "onb-finish-done-w", "Done", Some(("Check", "bold", 16.0)), true, cx, |_, _| {})).into_any_element();
        return step_frame(t, phone, false, head, Some(body), foot);
    }
    let devs = devices.unwrap_or(&[]);
    let parts = progress_parts(layout, devs);
    let fans = parts.iter().filter(|p| layout.zones[p.index].part.map(|pt| pt.kind == PartKind::Fan).unwrap_or(false)).count();
    let mut kicker = format!("{} {}", parts.len(), if parts.len() == 1 { "part" } else { "parts" });
    if fans > 0 {
        kicker = format!("{} · {} {}", kicker, fans, if fans == 1 { "fan" } else { "fans" });
    }
    let head = step_head(t, Some(kicker), title_text(t, if scoped { "That part is mapped." } else { "Your PC is mapped." }, phone), if scoped { None } else { Some("Pick a look to try.") });
    let tiles: Vec<canvas_proto::api::EffectSummary> = if scoped { Vec::new() } else { pick_three(effects).to_vec() };
    let mut body = div().flex().flex_col().gap_2();
    if !scoped && !tiles.is_empty() {
        let mut col = div().flex().flex_col().gap_2();
        for e in tiles {
            let id = e.id.clone();
            let pressed = data.picked.as_deref() == Some(e.id.as_str());
            let entity = cx.entity();
            let tile = div()
                .id(gpui::SharedString::from(format!("onb-tile-{}", e.id)))
                .flex()
                .items_center()
                .gap_3()
                .min_w_0()
                .p(px(6.0))
                .border_1()
                .border_color(if pressed { t.live() } else { Theme::color(t.line_2) })
                .r_sm()
                .bg(if pressed { t.live_soft() } else { Theme::color(t.raised) })
                .text_color(if pressed { Theme::color(t.ink) } else { Theme::color(t.dim) })
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    let id = id.clone();
                    entity.update(cx, |this: &mut LayoutView, cx| {
                        this.choose_effect(&id);
                        cx.notify();
                    });
                })
                .child(crate::components::thumb::thumb(&t, &e.id, 48.0, 30.0, crate::components::thumb::cached(&e.id), true))
                .child(
                    div().flex().flex_col().gap(px(2.0)).flex_1().min_w_0()
                        .child(div().text_size(px(16.0)).font_weight(gpui::FontWeight::MEDIUM).font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.ink)).whitespace_nowrap().child(e.name.clone()))
                        .child(div().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.mute)).whitespace_nowrap().child(if pressed { "Playing on your PC".to_string() } else { "Press to play".to_string() })),
                );
            col = col.child(tile);
        }
        body = body.child(col);
    }
    if let Some(err) = view.save_error.clone() {
        body = body.child(div().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.down)).child(format!("Not saved: {err}")));
    }
    let foot = div()
        .flex()
        .flex_col()
        .gap_2()
        .pt(px(16.0))
        .child(foot_primary(t, "onb-finish-done", "Done", Some(("Check", "bold", 16.0)), false, cx, |this, cx| {
            this.finish_onboarding(cx);
        }))
        .child(
            div().flex().justify_between().items_center().min_h(px(32.0)).child(div().into_any_element()).child(foot_row_ghost(t, "onb-finish-sweep", "Sweep again", Some(("ArrowCounterClockwise", "", 14.0)), false, cx, |this, cx| {
                if let Some(d) = this.onboarding.as_mut() { d.swept = false; }
                activate_effect(SWEEP_EFFECT, &canvas_proto::props::Props::default());
                cx.notify();
            })),
        )
        .into_any_element();
    step_frame(t, phone, false, head, Some(body.into_any_element()), foot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_screens_full_run_two_for_a_redo() {
        assert_eq!(build_steps(None), vec![StepKind::Hello, StepKind::Case, StepKind::Parts, StepKind::Match, StepKind::Finish]);
        assert_eq!(build_steps(Some(&Scope { device_id: "d".into(), zone_index: 0 })), vec![StepKind::Match, StepKind::Finish]);
    }

    #[test]
    fn opens_while_not_complete_and_not_put_off() {
        assert!(should_open(false, false));
        assert!(!should_open(true, false));
        assert!(!should_open(false, true));
    }

    #[test]
    fn hello_variants_pick_in_order() {
        assert_eq!(hello_mode(true, true, 0, false), HelloMode::Gate);
        assert_eq!(hello_mode(false, true, 0, false), HelloMode::Waking);
        assert_eq!(hello_mode(false, false, 0, false), HelloMode::Ladder);
        assert_eq!(hello_mode(false, false, 3, true), HelloMode::Ladder);
        assert_eq!(hello_mode(false, false, 3, false), HelloMode::Happy);
    }

    #[test]
    fn case_text_cm_rounds_inches_tenth() {
        assert_eq!(case_text(120.0, LengthUnit::Cm), "48 cm");
        assert_eq!(case_text(122.0, LengthUnit::Cm), "49 cm");
        assert_eq!(case_text(120.0, LengthUnit::In), "18.9 in");
        assert_eq!(trim_cm("37.0 cm"), "37 cm");
    }

    #[test]
    fn greetings_cover_thirty_languages() {
        assert_eq!(hello_for("en"), "Hello");
        assert_eq!(hello_for("pt-BR"), "Olá");
        assert_eq!(hello_for("es"), "Hola");
        assert_eq!(hello_for("xx"), "Hello");
        assert_eq!(hello_for(""), "Hello");
        assert_eq!(hello_for("ja"), "こんにちは");
    }

    #[test]
    fn run_len_twelve_and_tail_joins() {
        // size = max(12, ceil(36/3)) = 12; a tiny tail joins the run before it.
        assert_eq!(run_len(36, true, false, 3, 0), 12);
        assert_eq!(run_len(36, true, false, 3, 12), 12);
        assert_eq!(run_len(36, true, false, 3, 24), 12);
        assert_eq!(run_len(14, true, false, 1, 0), 14);
        assert_eq!(run_len(8, false, true, 1, 0), 8);
        // One placement owns the whole zone: ceil(36/1)=36, so the run is whole.
        assert_eq!(run_len(36, true, false, 1, 0), 36);
    }

    #[test]
    fn fan_leads_locate_run_skips_mapped_and_skipped() {
        use canvas_proto::device::{DeviceId, DeviceInfo as ProtoInfo, DeviceKind, ZoneInfo, ZoneKind};
        let dev = ProtoInfo {
            id: DeviceId("d".into()),
            name: "Board".into(),
            vendor: "fake".into(),
            kind: DeviceKind::Motherboard,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones: vec![ZoneInfo { index: 0, name: "JARGB1".into(), kind: ZoneKind::Linear, led_count: 24, min_leds: 0, max_leds: 24, resizable: false }],
            connected: true,
        };
        let layout: Layout = serde_json::from_str(r#"{"id":"d","name":"Default","render_scale":50,"sampling":"area_average","zones":[],"effect":null,"case_form":"none","landmarks":[]}"#).unwrap();
        let zones = setup_zones(&layout, Some(std::slice::from_ref(&dev)));
        assert_eq!(zones.len(), 1);
        let skipped = std::collections::HashSet::new();
        // No placements: size = ceil(24/1) = 24, so the whole zone is one run.
        let first = locate_run(&layout, &zones, &skipped, 0, 0).unwrap();
        assert_eq!((first.start, first.count), (0, 24));
        assert!(first.whole);
    }

    #[test]
    fn progress_counts_mapped_parts_only() {
        let layout: Layout = serde_json::from_str(r#"{"id":"d","name":"Default","render_scale":50,"sampling":"area_average","zones":[{"device_id":"d","zone_index":0,"x":0,"y":0,"width":10,"height":10,"rotation_deg":0,"reversed":false,"visible":true,"shape":{"type":"strip"},"leds":null,"part":{"kind":"fan","facing":"viewer","mapped":true},"locked":false}],"effect":null,"case_form":"none","landmarks":[]}"#).unwrap();
        use canvas_proto::device::{DeviceId, DeviceInfo as ProtoInfo, DeviceKind, ZoneInfo, ZoneKind};
        let dev = ProtoInfo {
            id: DeviceId("d".into()),
            name: "Board".into(),
            vendor: "fake".into(),
            kind: DeviceKind::Motherboard,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones: vec![ZoneInfo { index: 0, name: "JARGB1".into(), kind: ZoneKind::Linear, led_count: 24, min_leds: 0, max_leds: 24, resizable: false }],
            connected: true,
        };
        assert_eq!(progress(&layout, std::slice::from_ref(&dev)), (1, 1));
        assert_eq!(progress_parts(&layout, std::slice::from_ref(&dev)).len(), 1);
    }

    #[test]
    fn pick_three_prefers_the_first_named() {
        fn eff(id: &str) -> canvas_proto::api::EffectSummary {
            canvas_proto::api::EffectSummary { id: id.into(), name: id.into(), description: String::new(), needs: canvas_proto::props::InputNeeds::default(), favorite: false }
        }
        let list = vec![eff("other"), eff("solid"), eff("fire"), eff("gradient"), eff("aurora")];
        let got: Vec<String> = pick_three(&list).iter().map(|e| e.id.clone()).collect();
        assert_eq!(got, vec!["aurora".to_string(), "gradient".to_string(), "fire".to_string()]);
    }

    #[test]
    fn onboarding_data_starts_on_case_for_a_redo() {
        let d = OnboardData::new(true, None);
        assert_eq!(d.step(), StepKind::Case);
        assert_eq!(d.back(), None);
        let d = OnboardData::new(true, Some(Scope { device_id: "d".into(), zone_index: 0 }));
        assert_eq!(d.step(), StepKind::Match);
        assert_eq!(build_steps(d.scope.as_ref()).len(), 2);
    }

    #[test]
    fn swipe_constants_match_react() {
        assert_eq!(SWEEP_MS, 3000);
        assert_eq!(SWEEP_MS_REDUCED, 500);
        assert_eq!(HELLO_EFFECT, "onboard-hello");
        assert_eq!(SWEEP_EFFECT, "onboard-sweep");
    }

    #[test]
    fn case_guides_stand_on_the_floor_line() {
        let guides = case_guides_for(LengthUnit::Cm, 0.0);
        assert_eq!(guides.len(), 3);
        for g in &guides {
            assert!((g.y + g.height - CASE_ANCHOR_Y).abs() < 0.01, "{g:?}");
            assert!((g.x + g.width / 2.0 - CASE_ANCHOR_X).abs() < 0.01, "{g:?}");
            assert!(!g.size.is_empty());
        }
    }
}
