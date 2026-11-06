//! `FansView`, after `app/web/src/views/FansView.tsx`, `FanBulk.tsx` and
//! `FanCurveEditor.tsx`.
//!
//! The port keeps the React view's rules (the checklist is
//! `docs/gpui-parity/fans.md`):
//! - the profile strip, the temperatures panel, the headers table (or the
//!   phone cards), the bulk bar over the list, and the header editor beside
//!   it;
//! - the editor's three modes (BIOS, Fixed %, Curve), the curve chart, the
//!   "don't exceed" line, the availability override, and the safety texts;
//! - the load states: loading placeholder, the service-down panel with the
//!   last reading, the stale notice over live readings.
//!
//! The React demo-mode states (`?state=...`) are demo-backend behavior and
//! have no GPUI counterpart: the GPUI editor always talks to a real (here,
//! fake) daemon over HTTP.
//!
//! Two deviations from the React page, both forced by the shell: the page's
//! title and its profile actions live on the page here (the React editor puts
//! them in the status strip, and the strip is the other branch's), and the
//! numbers do not count up (the Motion slice is not ported).

use std::collections::HashMap;
use std::time::Duration;

use canvas_proto::fans::{
    CurvePoint, FanHeaderStatus, FanMode, FansStatus, HotKind, HotLimits, TempReading, TempSource,
};
use gpui::{div, prelude::*, px, Context, IntoElement, Rgba, Window};

use crate::client::DaemonClient;
use crate::components::fan_bits::{
    chip, fans_button, fans_icon_button, fans_number_field, live, live_soft, meter, mix_srgb,
    sparkline, tick_box, tint, BtnVariant,
};
use crate::components::fan_bulk::{
    action_words, applied_detail, can_drive, decide, fan_bulk_bar, is_handed_back, is_unavailable,
    plain_label, BulkAction, BulkDecision, BulkOutcome, BulkState,
};
use crate::components::fan_curve::{
    base_floor, bounds_for, curve_geometry, default_curve, hot_row, pct_of_y, sorted_points,
    temp_of_x, ChartBox, HOT_MIN,
};
use crate::components::panel::panel;
use crate::components::segmented::segmented;
use crate::components::text_input::{format_value, TextInput};
use crate::theme::Theme;

mod editor;

// ---------------------------------------------------------------------------
// Logic, ported from FansView.tsx and covered by the tests below.

/// The page's width: the window less the rail and the page's own padding.
///
/// React measures the `.content` box (the size container `page`) with a
/// `ResizeObserver`; GPUI 0.2.2 has no such call, so the port works the width
/// out from the window and the same rail rule the shell uses.
pub fn page_width(window_width: f32) -> f32 {
    let rail = if window_width >= 1280.0 {
        200.0
    } else if window_width > 720.0 {
        64.0
    } else {
        0.0
    };
    (window_width - rail - 48.0).max(0.0)
}

/// An empty fan status: the placeholder's shape.
pub fn empty_fans() -> FansStatus {
    FansStatus {
        chip: None,
        hardware_watchdog: None,
        headers: Vec::new(),
        temps: Vec::new(),
        active_profile: String::new(),
        profiles: Vec::new(),
        floor_percent: canvas_proto::fans::DEFAULT_FLOOR_PERCENT,
        hot: Vec::new(),
    }
}

/// The mode a header is asked for, as a word.
pub fn mode_label(mode: &FanMode) -> String {
    match mode {
        FanMode::Bios => "BIOS".to_string(),
        FanMode::Fixed { percent } => format!("Fixed {percent}%"),
        FanMode::Curve { .. } => "Curve".to_string(),
    }
}

/// The temperature a curve header reads, if its source reports one.
pub fn temp_for(mode: &FanMode, temps: &[TempReading]) -> Option<f32> {
    let FanMode::Curve { source, .. } = mode else {
        return None;
    };
    let key = source_key(source);
    temps
        .iter()
        .find(|t| source_key(&t.source) == key)
        .and_then(|t| t.celsius)
}

/// A source's key, as the pickers work on keys.
pub fn source_key(source: &TempSource) -> String {
    match source {
        TempSource::Board(name) => format!("board:{name}"),
        TempSource::Cpu => "cpu".to_string(),
        TempSource::Gpu => "gpu".to_string(),
    }
}

/// A key back to a source.
pub fn source_from_key(key: &str) -> TempSource {
    match key.strip_prefix("board:") {
        Some(name) => TempSource::Board(name.to_string()),
        None => {
            if key == "gpu" {
                TempSource::Gpu
            } else {
                TempSource::Cpu
            }
        }
    }
}

/// A temperature bar reads cool, warm or hot, and says so in colour only.
pub fn temp_tone(t: &Theme, celsius: f32) -> gpui::Rgba {
    if celsius < 50.0 {
        Theme::color(t.a3)
    } else if celsius < 72.0 {
        Theme::color(t.warn)
    } else {
        Theme::color(t.down)
    }
}

/// The name of the next free copy of a profile.
pub fn next_profile_name(base: &str, profiles: &[String]) -> String {
    let mut name = format!("{base} copy");
    let mut n = 2;
    while profiles.iter().any(|p| p == &name) {
        name = format!("{base} copy {n}");
        n += 1;
    }
    name
}

/// The one sentence a screen reader hears for a header's reason: it names the
/// KIND, never the live value.
pub fn reason_kind_of(header: &FanHeaderStatus) -> String {
    let Some(_reason) = &header.override_reason else {
        return String::new();
    };
    if is_handed_back(header) {
        format!(
            "{}: canvas-rgb handed this header back to BIOS",
            header.label
        )
    } else {
        format!("canvas-rgb is overriding {}", header.label)
    }
}

/// What the daemon says about this header, in words that name it.
pub fn reason_text(header: &FanHeaderStatus) -> Option<String> {
    let reason = header.override_reason.as_ref()?;
    if is_handed_back(header) {
        Some(format!(
            "canvas-rgb handed {} back to BIOS: {reason}",
            header.label
        ))
    } else {
        Some(format!("canvas-rgb overrode {}: {reason}", header.label))
    }
}

/// Who is in charge, when the daemon has nothing to say.
pub fn quiet_line(mode: &FanMode) -> &'static str {
    match mode {
        FanMode::Bios => "The BIOS is in charge of this fan.",
        FanMode::Fixed { .. } => "Fixed duty is in charge; nothing is overriding it.",
        FanMode::Curve { .. } => "The curve is in charge; nothing is overriding it.",
    }
}

/// The handoff line of a row: the daemon took the header back.
pub fn handoff_text(header: &FanHeaderStatus, ticked: bool) -> Option<&'static str> {
    if !is_handed_back(header) {
        return None;
    }
    Some(if ticked {
        "Handed back to BIOS, skipped when applied"
    } else {
        "Handed back to BIOS"
    })
}

/// The body of the editor for a header the board does not route.
pub fn unavailable_body(header: &FanHeaderStatus) -> &'static str {
    if header.override_available == Some(false) {
        "You marked this header as not on your board, so there is no mode to set."
    } else {
        "Not on this board: this header is not fitted on your board, so there is no mode to set."
    }
}

/// The bulk count: the ticked headers, in list order.
pub fn checked_headers(fans: &FansStatus, ticked: &[String]) -> Vec<FanHeaderStatus> {
    fans.headers
        .iter()
        .filter(|h| can_drive(h) && ticked.iter().any(|id| id == &h.id))
        .cloned()
        .collect()
}

/// The sparkline's peak: the largest of 2000 and the samples.
pub fn sparkline_peak(history: &[u32]) -> u32 {
    history.iter().copied().max().unwrap_or(0).max(2000)
}

/// The header a row's checkbox names.
///
/// The GPUI editor has no accessible-name layer yet, so the name and the
/// reason a header cannot be ticked are kept here (and tested) but not drawn.
#[allow(dead_code)]
pub fn row_checkbox_label(header: &FanHeaderStatus) -> String {
    if is_unavailable(header) {
        format!("{}: not on this board, cannot be selected", header.label)
    } else if !can_drive(header) {
        format!("{}: tach only, cannot be selected", header.label)
    } else {
        format!("Select {}", header.label)
    }
}

/// Why a row's checkbox is blocked.
///
/// The GPUI editor has no tooltip layer yet, so the reason is kept here (and
/// tested) but not drawn.
#[allow(dead_code)]
pub fn row_checkbox_title(header: &FanHeaderStatus) -> Option<&'static str> {
    if is_unavailable(header) {
        Some(crate::components::fan_bulk::UNAVAILABLE_WHY)
    } else if !can_drive(header) {
        Some(crate::components::fan_bulk::TACH_ONLY_WHY)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// The view.

/// What a write to a header became.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteResult {
    Sent,
    Superseded,
}

/// The callback a finished write reports to.
type Done = Box<dyn FnOnce(WriteResult, Option<String>, &mut gpui::App) + 'static>;

/// A write waiting for its turn.
struct Pending {
    mode: FanMode,
    on_done: Done,
}

/// One header's write slot: one request at a time, and while one is out only
/// the newest waits (`app/web/src/lib/fanWrites.ts`).
struct Slot {
    running: bool,
    queued: Option<Pending>,
}

/// A bulk run in progress.
struct BulkRun {
    words: String,
    action: BulkAction,
    targets: Vec<FanHeaderStatus>,
    index: usize,
    outcomes: Vec<BulkOutcome>,
}

/// How wide a text field is (React's `.num`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldWidth {
    /// The field takes the room its row has left, but never less than the
    /// three digits of "100" and never more than 92 px.
    Fill,
    /// The field is a fixed width.
    Fixed(f32),
}

/// The table's columns, as React's auto table layout sizes them.
///
/// Every column takes the width its widest cell asks for, and the room that
/// is left over is shared out equally between the columns that can grow.
/// The tick column has a width of its own, so it takes no share.
#[derive(Debug, Clone, Copy)]
pub struct TableColumns {
    pub tick: f32,
    pub name: f32,
    pub rpm: f32,
    pub duty: f32,
    pub mode: f32,
    pub history: f32,
    /// The cell's own padding (`--s2` at 1000 px and up, `--s3` below).
    pub padding_x: f32,
    /// The duty meter's own width.
    pub duty_meter: f32,
}

/// The figures face's advance at 14 px and at 20 px (0.6 em).
const MONO_14: f32 = 8.4;
const MONO_20: f32 = 12.0;

/// The table's columns for the given page width.
pub fn table_columns(fans: &FansStatus, page: f32, width: f32) -> TableColumns {
    let wide = page >= 1000.0;
    let narrow_table = page <= 999.0;
    let with_word = page > 1499.0;
    let padding_x = if wide { 8.0 } else { 12.0 };
    let duty_meter = if wide { 52.0 } else { 44.0 };

    // The widest cell in each column.
    let name = fans
        .headers
        .iter()
        .map(|h| plain_label(&h.label).chars().count() as f32 * MONO_14)
        .fold(0.0, f32::max);
    let rpm = fans
        .headers
        .iter()
        .map(|h| h.rpm.unwrap_or(0).to_string().len() as f32 * MONO_20)
        .fold(0.0, f32::max)
        .max(MONO_20 * 4.0);
    let duty = duty_meter + 8.0 + 4.0 * MONO_14;
    // The mode's dot, and its word where the word fits.
    let mode = if with_word { 50.0 + 15.0 } else { 15.0 };
    let history = if narrow_table { 0.0 } else { 56.0 };
    let tick = 36.0;

    let auto = if narrow_table { 4.0 } else { 5.0 };
    let used = tick + name + rpm + duty + mode + history;
    let extra = (width - used).max(0.0) / auto;
    TableColumns {
        tick,
        name: name + extra,
        rpm: rpm + extra,
        duty: duty + extra,
        mode: mode + extra,
        history: history + extra,
        padding_x,
        duty_meter,
    }
}

pub struct FansView {
    pub theme: Theme,
    pub fans: Option<FansStatus>,
    pub fans_error: Option<String>,
    pub loading: bool,
    pub retrying: bool,
    /// The header the editor is on. `None` until one is picked.
    pub selected_id: Option<String>,
    pub ticked: Vec<String>,
    pub select_mode: bool,
    pub last_tick: Option<String>,
    /// The last readings, per header, for the sparklines.
    pub history: HashMap<String, Vec<u32>>,
    /// The editor's local state, rebuilt when the daemon changes the header,
    /// its mode or the active profile (the React remount rule).
    pub editor_key: String,
    pub editor_percent: f32,
    pub editor_points: Vec<CurvePoint>,
    pub editor_source: TempSource,
    pub editor_error: Option<String>,
    pub editor_busy: bool,
    pub hot_error: Option<String>,
    pub hot_busy: bool,
    pub hot_drag: Option<f32>,
    /// The profile strip.
    pub profile_error: Option<String>,
    pub confirming: Option<String>,
    pub more_open: bool,
    pub notice: Option<String>,
    /// The bulk bar.
    pub bulk_percent: f32,
    pub bulk_picked_from: Option<String>,
    pub bulk_busy: bool,
    pub bulk_result: Option<(String, Vec<BulkOutcome>)>,
    pub bulk_details_open: bool,
    bulk_run: Option<BulkRun>,
    // The write slots, one per header.
    slots: HashMap<String, Slot>,
    // The curve's debounce generation: a newer edit replaces the pending one.
    curve_generation: u64,
    /// The chart's measured box.
    pub chart_box: ChartBox,
    // The point a drag is holding.
    drag_point: Option<usize>,
    // The points as they were when the drag began.
    drag_start: Vec<CurvePoint>,
    /// The temperature source's list is open.
    pub source_select_open: bool,
    /// The reason box is open.
    pub reason_open: bool,
    /// The text fields (the point fields, the "don't exceed" field), one
    /// entity each.
    text_inputs: HashMap<String, gpui::Entity<TextInput>>,
    /// The window width, for the page's breakpoints.
    pub window_width: f32,
    /// The window height, for the page's height.
    window_height: f32,
}

impl FansView {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        let mut view = FansView {
            theme,
            fans: None,
            fans_error: None,
            loading: true,
            retrying: false,
            selected_id: None,
            ticked: Vec::new(),
            select_mode: false,
            last_tick: None,
            history: HashMap::new(),
            editor_key: String::new(),
            editor_percent: 50.0,
            editor_points: Vec::new(),
            editor_source: TempSource::Cpu,
            editor_error: None,
            editor_busy: false,
            hot_error: None,
            hot_busy: false,
            hot_drag: None,
            profile_error: None,
            confirming: None,
            more_open: false,
            notice: None,
            bulk_percent: 50.0,
            bulk_picked_from: None,
            bulk_busy: false,
            bulk_result: None,
            bulk_details_open: false,
            bulk_run: None,
            slots: HashMap::new(),
            curve_generation: 0,
            chart_box: ChartBox::new(),
            drag_point: None,
            drag_start: Vec::new(),
            source_select_open: false,
            reason_open: false,
            text_inputs: HashMap::new(),
            window_width: 1280.0,
            window_height: 800.0,
        };
        view.load(cx);
        view
    }

    fn client() -> DaemonClient {
        DaemonClient::new()
    }

    /// `GET /api/v1/fans`, then the next read a second later.
    fn load(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.get("/api/v1/fans")
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.retrying = false;
                match result {
                    Ok(body) => {
                        let parsed = serde_json::from_str::<FansStatus>(&body).ok();
                        if let Some(fans) = parsed {
                            this.apply_fans(fans);
                        }
                    }
                    Err(e) => {
                        this.fans_error = Some(e.to_string());
                    }
                }
                this.loading = false;
                cx.notify();
            });
            // The next poll, a second later (React polls `fans` every second).
            let timer = cx.background_executor().timer(Duration::from_secs(1));
            timer.await;
            let _ = view.update(cx, |this, cx| {
                this.load(cx);
            });
        })
        .detach();
    }

    /// A fresh read: the history grows by one sample per header, the ticks
    /// lose the headers that left, and the editor follows the daemon.
    fn apply_fans(&mut self, fans: FansStatus) {
        for header in &fans.headers {
            let sample = header.rpm.unwrap_or(0);
            let entry = self.history.entry(header.id.clone()).or_default();
            entry.push(sample);
            // The store keeps the last sixty samples.
            if entry.len() > 60 {
                let excess = entry.len() - 60;
                entry.drain(0..excess);
            }
        }
        let live: Vec<String> = fans.headers.iter().map(|h| h.id.clone()).collect();
        self.ticked.retain(|id| live.contains(id));
        self.fans = Some(fans);
        self.fans_error = None;
        self.sync_editor();
    }

    /// The header the editor is on: the picked one, or the first one on a
    /// desktop (nothing is picked on a phone).
    fn selected(&self, narrow: bool) -> Option<FanHeaderStatus> {
        let fans = self.fans.as_ref()?;
        let id = self.selected_id.clone().or(if narrow {
            None
        } else {
            fans.headers.first().map(|h| h.id.clone())
        });
        id.and_then(|id| fans.headers.iter().find(|h| h.id == id).cloned())
    }

    /// The editor's local state follows the daemon: a change to the header,
    /// its mode or the active profile rebuilds it (the React remount rule).
    fn sync_editor(&mut self) {
        let narrow = self.page_width() <= 640.0;
        let Some(header) = self.selected(narrow) else {
            return;
        };
        let Some(fans) = self.fans.as_ref() else {
            return;
        };
        let key = format!(
            "{}|{}|{}",
            header.id,
            mode_word(&header.mode),
            fans.active_profile
        );
        if key == self.editor_key {
            return;
        }
        self.editor_key = key;
        let base = base_floor(&header.id, fans.floor_percent);
        self.editor_percent = match &header.mode {
            FanMode::Fixed { percent } => f32::from(*percent),
            _ => f32::from(base.max(50)),
        };
        self.editor_points = match &header.mode {
            FanMode::Curve { points, .. } => points.clone(),
            _ => default_curve(base),
        };
        self.editor_source = match &header.mode {
            FanMode::Curve { source, .. } => source.clone(),
            _ => TempSource::Cpu,
        };
        self.editor_error = None;
    }

    fn page_width(&self) -> f32 {
        page_width(self.window_width)
    }

    /// The page's height: the window less the status strip and the content
    /// box's own padding.
    ///
    /// React's `.rd-page { flex: 1 1 auto; min-height: 0; }` fills the
    /// content box, and the two columns fill it with it. GPUI 0.2.2 does not
    /// resolve a percentage height against a flex parent's height, so the
    /// page's height is worked out from the window instead.
    fn page_height(&self) -> f32 {
        let phone = self.window_width <= 720.0;
        let strip = if phone { 48.0 } else { 54.0 };
        let padding = if phone { 16.0 + 120.0 } else { 16.0 + 24.0 };
        (self.window_height - strip - padding).max(0.0)
    }

    /// One text field (React's `NumberField`): a plain typed number, with
    /// the field's unit beside it.
    ///
    /// The field is an entity of its own, so it keeps its own draft, its
    /// caret and its focus. It is made once and kept; the model only tells
    /// it what to show.
    pub(crate) fn text_field(
        &mut self,
        id: &str,
        value: f32,
        min: f32,
        max: f32,
        decimals: usize,
        unit: Option<&str>,
        width: FieldWidth,
        disabled: bool,
        on_commit: impl Fn(f32, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let shown = format_value(value, decimals);
        let entity = cx.entity();
        let handler = std::rc::Rc::new(move |v: f32, _window: &mut Window, app: &mut gpui::App| {
            entity.update(app, |_view, cx| on_commit(v, cx));
        });
        let input = match self.text_inputs.get(id) {
            Some(existing) => {
                let existing = existing.clone();
                existing.update(cx, |input, cx| {
                    input.set_value(&shown, cx);
                    input.set_disabled(disabled, cx);
                });
                existing
            }
            None => {
                let input = cx.new(|cx| {
                    TextInput::new(
                        &shown,
                        true,
                        min,
                        max,
                        decimals,
                        self.theme,
                        handler.clone(),
                        cx,
                    )
                });
                input.update(cx, |input, cx| input.set_disabled(disabled, cx));
                self.text_inputs.insert(id.to_string(), input.clone());
                input
            }
        };
        let t = self.theme;
        let well = match width {
            FieldWidth::Fill => div().flex_1().min_w(px(52.0)).max_w(px(92.0)),
            FieldWidth::Fixed(px_wide) => div().w(px(px_wide)).flex_shrink_0(),
        };
        let mut field = div().flex().items_center().gap(px(6.0)).min_w_0();
        if matches!(width, FieldWidth::Fill) {
            // A field that takes the room its row has left (React's `.num` in
            // the curve rows); a fixed one keeps its own width.
            field = field.flex_1();
        }
        field = field.child(well.child(input));
        if let Some(unit) = unit {
            field.child(
                div()
                    .text_size(px(14.0))
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(unit.to_string()),
            )
        } else {
            field
        }
        .into_any_element()
    }

    // -- writes ------------------------------------------------------------

    /// One write to a header at a time: a new write waits for the one in
    /// flight, and a write still waiting when a newer one arrives is dropped.
    fn enqueue_write(
        &mut self,
        header: String,
        mode: FanMode,
        on_done: Done,
        cx: &mut Context<Self>,
    ) {
        let superseded = self
            .slots
            .entry(header.clone())
            .or_insert_with(|| Slot {
                running: false,
                queued: None,
            })
            .queued
            .take();
        if let Some(pending) = superseded {
            (pending.on_done)(WriteResult::Superseded, None, cx);
        }
        let start = {
            let slot = self.slots.entry(header.clone()).or_insert_with(|| Slot {
                running: false,
                queued: None,
            });
            slot.queued = Some(Pending { mode, on_done });
            if slot.running {
                false
            } else {
                slot.running = true;
                true
            }
        };
        if start {
            self.pump_write(header, cx);
        }
    }

    fn pump_write(&mut self, header: String, cx: &mut Context<Self>) {
        let pending = {
            let Some(slot) = self.slots.get_mut(&header) else {
                return;
            };
            match slot.queued.take() {
                Some(pending) => pending,
                None => {
                    slot.running = false;
                    return;
                }
            }
        };
        let Pending { mode, on_done } = pending;
        let path = format!("/api/v1/fans/headers/{header}");
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let body = serde_json::to_string(&mode).unwrap_or_default();
            client.patch_json(&path, &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok(_) => on_done(WriteResult::Sent, None, cx),
                    Err(e) => on_done(WriteResult::Sent, Some(e.to_string()), cx),
                }
                view.pump_write(header, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// The editor sends a mode to its header.
    fn apply_mode(&mut self, header: String, mode: FanMode, cx: &mut Context<Self>) {
        self.editor_error = None;
        self.editor_busy = true;
        let entity = cx.entity();
        self.enqueue_write(
            header,
            mode,
            Box::new(move |result, error, cx| {
                entity.update(cx, |view, cx| {
                    view.editor_busy = false;
                    match result {
                        WriteResult::Superseded => {}
                        WriteResult::Sent => match error {
                            Some(message) => view.editor_error = Some(message),
                            None => view.load(cx),
                        },
                    }
                    cx.notify();
                });
            }),
            cx,
        );
    }

    // -- the editor --------------------------------------------------------

    /// A curve edit that has not been sent yet is replaced by whatever the
    /// user chooses next.
    fn cancel_curve(&mut self) {
        self.curve_generation += 1;
    }

    fn commit_curve(&mut self, points: Vec<CurvePoint>, cx: &mut Context<Self>) {
        self.editor_points = points;
        self.cancel_curve();
        let generation = self.curve_generation;
        let timer = cx.background_executor().timer(Duration::from_millis(250));
        cx.spawn(async move |view, cx| {
            timer.await;
            let _ = view.update(cx, |view, cx| {
                if view.curve_generation != generation {
                    return;
                }
                let Some(header) = view.selected(view.page_width() <= 640.0) else {
                    return;
                };
                let mode = FanMode::Curve {
                    source: view.editor_source.clone(),
                    points: view.editor_points.clone(),
                };
                view.apply_mode(header.id.clone(), mode, cx);
            });
        })
        .detach();
    }

    /// The hot line: the whole set of limits goes out, with one kind changed.
    fn commit_hot(&mut self, value: Option<f32>, cx: &mut Context<Self>) {
        let Some(fans) = self.fans.clone() else {
            return;
        };
        let kind = self.hot_kind();
        let mut limits = HotLimits::default();
        for row in &fans.hot {
            let v = if row.user_set {
                Some(row.limit_c)
            } else {
                None
            };
            match row.kind {
                HotKind::Cpu => limits.cpu_c = v,
                HotKind::Gpu => limits.gpu_c = v,
                HotKind::Board => limits.board_c = v,
            }
        }
        match kind {
            HotKind::Cpu => limits.cpu_c = value,
            HotKind::Gpu => limits.gpu_c = value,
            HotKind::Board => limits.board_c = value,
        }
        self.hot_busy = true;
        self.hot_error = None;
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let body = serde_json::to_string(&limits).unwrap_or_default();
            client.put_json("/api/v1/fans/hot", &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |view, cx| {
                view.hot_busy = false;
                match result {
                    Ok(_) => {
                        view.hot_drag = None;
                        view.load(cx);
                    }
                    Err(e) => {
                        view.hot_error = Some(e.to_string());
                        // A refused PUT snaps the line back to the status.
                        view.hot_drag = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The kind of source a header's hot line is for: a Fixed header has no
    /// curve source, so it reads the CPU.
    fn hot_kind(&self) -> HotKind {
        let narrow = self.page_width() <= 640.0;
        let Some(header) = self.selected(narrow) else {
            return HotKind::Cpu;
        };
        match &header.mode {
            FanMode::Curve { source, .. } => source.hot_kind(),
            _ => HotKind::Cpu,
        }
    }

    /// The owner's word on whether a header is on this board (issue #146).
    fn set_availability(
        &mut self,
        header: String,
        available: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        self.editor_error = None;
        self.editor_busy = true;
        let body = serde_json::json!({ "available": available }).to_string();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.put_json(
                &format!("/api/v1/fans/headers/{header}/availability"),
                &body,
            )
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |view, cx| {
                view.editor_busy = false;
                match result {
                    Ok(_) => view.load(cx),
                    Err(e) => view.editor_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // -- the profile strip -------------------------------------------------

    /// A profile action: the page refreshes after it, and only a message the
    /// page cannot show is said.
    fn profile_action(
        &mut self,
        call: impl Fn(&DaemonClient) -> Result<String, crate::client::ClientError> + Send + 'static,
        done: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        self.profile_error = None;
        let notice = done.map(|d| d.to_string());
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            call(&client)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok(_) => {
                        view.load(cx);
                        if let Some(notice) = notice {
                            view.notice = Some(notice);
                        }
                    }
                    Err(e) => view.profile_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The headers a profile holds right now: every header's current mode,
    /// BIOS included.
    fn current_profile(&self) -> Vec<(String, FanMode)> {
        self.fans
            .as_ref()
            .map(|fans| {
                fans.headers
                    .iter()
                    .map(|h| (h.id.clone(), h.mode.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn profile_body(&self, name: &str) -> String {
        let headers: serde_json::Map<String, serde_json::Value> = self
            .current_profile()
            .into_iter()
            .map(|(id, mode)| {
                (
                    id,
                    serde_json::to_value(mode).unwrap_or(serde_json::Value::Null),
                )
            })
            .collect();
        serde_json::json!({ "name": name, "headers": headers }).to_string()
    }

    fn switch_profile(&mut self, name: String, cx: &mut Context<Self>) {
        let body = serde_json::json!({ "name": name }).to_string();
        self.profile_action(
            move |client| client.patch_json("/api/v1/fans/profile", &body),
            None,
            cx,
        );
    }

    fn save_profile(&mut self, cx: &mut Context<Self>) {
        let Some(fans) = self.fans.clone() else {
            return;
        };
        let name = fans.active_profile.clone();
        let body = self.profile_body(&name);
        let notice = format!("{name} saved");
        self.profile_action(
            move |client| {
                client.put_json(&format!("/api/v1/fans/profiles/{}", encode(&name)), &body)
            },
            Some(Box::leak(notice.into_boxed_str())),
            cx,
        );
    }

    fn duplicate_profile(&mut self, cx: &mut Context<Self>) {
        let Some(fans) = self.fans.clone() else {
            return;
        };
        let name = next_profile_name(&fans.active_profile, &fans.profiles);
        let body = self.profile_body(&name);
        self.profile_action(
            move |client| {
                client.put_json(&format!("/api/v1/fans/profiles/{}", encode(&name)), &body)
            },
            None,
            cx,
        );
    }

    /// Rename a profile: the new name is put in place, the page is switched
    /// to it and the old one is deleted.
    ///
    /// The React field is a text input; GPUI 0.2.2 has no text input element,
    /// so nothing calls this yet (the rule is kept and tested).
    #[allow(dead_code)]
    fn rename_profile(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(fans) = self.fans.clone() else {
            return;
        };
        let old = fans.active_profile.clone();
        if name.trim().is_empty() {
            return;
        }
        let body = self.profile_body(&name);
        let switch = serde_json::json!({ "name": name }).to_string();
        self.profile_action(
            move |client| {
                client.put_json(&format!("/api/v1/fans/profiles/{}", encode(&name)), &body)?;
                client.patch_json("/api/v1/fans/profile", &switch)?;
                if old != name {
                    client.delete(&format!("/api/v1/fans/profiles/{}", encode(&old)))?;
                }
                Ok(String::new())
            },
            None,
            cx,
        );
    }

    fn delete_profile(&mut self, name: String, cx: &mut Context<Self>) {
        self.confirming = None;
        self.profile_action(
            move |client| client.delete(&format!("/api/v1/fans/profiles/{}", encode(&name))),
            None,
            cx,
        );
    }

    // -- the bulk bar ------------------------------------------------------

    fn bulk_start(&mut self, action: BulkAction, cx: &mut Context<Self>) {
        let Some(fans) = self.fans.clone() else {
            return;
        };
        let checked = checked_headers(&fans, &self.ticked);
        if checked.is_empty() || self.bulk_busy {
            return;
        }
        let from = match &action {
            BulkAction::Curve { from } => fans
                .headers
                .iter()
                .find(|h| &h.id == from)
                .map(|h| h.label.clone()),
            _ => None,
        };
        let words = action_words(&action, from.as_deref());
        self.bulk_result = None;
        self.bulk_details_open = false;
        self.bulk_busy = true;
        self.bulk_run = Some(BulkRun {
            words,
            action,
            targets: checked,
            index: 0,
            outcomes: Vec::new(),
        });
        self.bulk_step(cx);
    }

    /// One header of the run, then the next.
    fn bulk_step(&mut self, cx: &mut Context<Self>) {
        let Some(fans) = self.fans.clone() else {
            return;
        };
        let (target, action) = {
            let Some(run) = self.bulk_run.as_mut() else {
                return;
            };
            if run.index >= run.targets.len() {
                (None, None)
            } else {
                (
                    Some(run.targets[run.index].clone()),
                    Some(run.action.clone()),
                )
            }
        };
        let (Some(target), Some(action)) = (target, action) else {
            // The run is over.
            let (words, outcomes) = match self.bulk_run.take() {
                Some(run) => (run.words, run.outcomes),
                None => return,
            };
            self.bulk_busy = false;
            self.bulk_result = Some((words, outcomes));
            self.load(cx);
            return;
        };
        match decide(&action, &target, &fans) {
            BulkDecision::Skip { detail } | BulkDecision::Fail { detail } => {
                let state = BulkState::Skipped;
                if let Some(run) = self.bulk_run.as_mut() {
                    run.outcomes.push(BulkOutcome {
                        id: target.id.clone(),
                        label: target.label.clone(),
                        state,
                        detail,
                    });
                    run.index += 1;
                }
                self.bulk_step(cx);
            }
            BulkDecision::Send(mode) => {
                let entity = cx.entity();
                let header = target.id.clone();
                let label = target.label.clone();
                let mode_for_detail = mode.clone();
                self.enqueue_write(
                    header,
                    mode,
                    Box::new(move |result, error, cx| {
                        entity.update(cx, |view, cx| {
                            let (state, detail) = match (result, error) {
                                (WriteResult::Superseded, _) => (
                                    BulkState::Skipped,
                                    "Superseded by your edit of this header: that edit went out instead."
                                        .to_string(),
                                ),
                                (WriteResult::Sent, Some(message)) => (BulkState::Failed, message),
                                (WriteResult::Sent, None) => (
                                    BulkState::Applied,
                                    applied_detail(&action, &mode_for_detail),
                                ),
                            };
                            if let Some(run) = view.bulk_run.as_mut() {
                                run.outcomes.push(BulkOutcome {
                                    id: target.id.clone(),
                                    label: label.clone(),
                                    state,
                                    detail,
                                });
                                run.index += 1;
                            }
                            view.bulk_step(cx);
                            cx.notify();
                        });
                    }),
                    cx,
                );
            }
        }
    }

    // -- the editor's actions ----------------------------------------------

    /// The mode control: BIOS, Fixed % or Curve.
    fn set_mode(&mut self, header_id: &str, mode: &str, cx: &mut Context<Self>) {
        let Some(fans) = self.fans.clone() else {
            return;
        };
        let Some(header) = fans.headers.iter().find(|h| h.id == header_id) else {
            return;
        };
        let base = base_floor(&header.id, fans.floor_percent);
        // A curve edit that has not been sent yet is dropped by a mode change.
        self.cancel_curve();
        match mode {
            "bios" => self.apply_mode(header_id.to_string(), FanMode::Bios, cx),
            "fixed" => {
                let percent = self.editor_percent.clamp(f32::from(base), 100.0);
                self.editor_percent = percent;
                self.apply_mode(
                    header_id.to_string(),
                    FanMode::Fixed {
                        percent: percent.round() as u8,
                    },
                    cx,
                );
            }
            "curve" => {
                let points = if self.editor_points.len() >= editor::MIN_POINTS {
                    self.editor_points.clone()
                } else {
                    default_curve(base)
                };
                self.editor_points = points.clone();
                self.apply_mode(
                    header_id.to_string(),
                    FanMode::Curve {
                        source: self.editor_source.clone(),
                        points,
                    },
                    cx,
                );
            }
            _ => {}
        }
    }

    /// The fixed duty, from the slider or the number field.
    fn commit_percent(&mut self, header_id: &str, value: f32, cx: &mut Context<Self>) {
        let Some(fans) = self.fans.clone() else {
            return;
        };
        let base = base_floor(header_id, fans.floor_percent);
        let percent = value.round().clamp(f32::from(base), 100.0);
        self.editor_percent = percent;
        self.cancel_curve();
        self.apply_mode(
            header_id.to_string(),
            FanMode::Fixed {
                percent: percent as u8,
            },
            cx,
        );
    }

    /// The temperature source changed: the curve goes out with the new source
    /// and the current points at once.
    fn set_source(&mut self, header_id: &str, key: &str, cx: &mut Context<Self>) {
        let source = source_from_key(key);
        self.editor_source = source.clone();
        self.source_select_open = false;
        self.cancel_curve();
        let mode = FanMode::Curve {
            source,
            points: self.editor_points.clone(),
        };
        self.apply_mode(header_id.to_string(), mode, cx);
    }

    /// The floor of the header the editor is on.
    fn base_floor_for(&self) -> u8 {
        let Some(fans) = self.fans.as_ref() else {
            return 20;
        };
        let narrow = self.page_width() <= 640.0;
        let Some(header) = self.selected(narrow) else {
            return 20;
        };
        base_floor(&header.id, fans.floor_percent)
    }

    /// The highest value the hot line may take.
    fn hot_max(&self) -> f32 {
        let Some(fans) = self.fans.as_ref() else {
            return HOT_MIN;
        };
        let kind = self.hot_kind();
        hot_row(&fans.hot, kind)
            .map(|h| h.max_settable_c)
            .unwrap_or(HOT_MIN)
    }

    /// The hot line's value: the drag value, or the one in force.
    fn hot_value(&self) -> Option<f32> {
        if let Some(value) = self.hot_drag {
            return Some(value);
        }
        let fans = self.fans.as_ref()?;
        let kind = self.hot_kind();
        hot_row(&fans.hot, kind).map(|h| h.limit_c)
    }

    /// A dragged point follows the pointer, between its neighbours.
    fn move_dragged_point(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        let index = match self.drag_point {
            Some(index) => index,
            None => return,
        };
        let geom = curve_geometry(self.chart_box.width().max(1.0));
        let points = self.editor_points.clone();
        let (lo, hi) = bounds_for(&points, index);
        let floor = self.base_floor_for();
        let temp = temp_of_x(&geom, x).round().clamp(lo, hi.max(lo));
        let percent = (pct_of_y(&geom, y).round())
            .clamp(f32::from(floor), 100.0)
            .max(0.0) as u8;
        if let Some(point) = self.editor_points.get_mut(index) {
            point.temp_c = temp;
            point.percent = percent;
        }
        cx.notify();
    }

    /// The drag ends: the list is sorted and committed.
    fn drop_dragged_point(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        if self.drag_point.is_none() {
            return;
        }
        // The point moves to where the pointer let go, then the list is sorted
        // and the write goes out.
        self.move_dragged_point(x, y, cx);
        self.drag_point = None;
        let points = sorted_points(&self.editor_points);
        self.commit_curve(points, cx);
    }

    /// The hot line follows the pointer.
    fn move_hot(&mut self, x: f32, cx: &mut Context<Self>) {
        let geom = curve_geometry(self.chart_box.width().max(1.0));
        let max = self.hot_max();
        self.hot_drag = Some(temp_of_x(&geom, x).round().clamp(HOT_MIN, max));
        cx.notify();
    }

    /// The hot line's drag ends: the value goes out.
    fn drop_hot(&mut self, x: f32, cx: &mut Context<Self>) {
        let geom = curve_geometry(self.chart_box.width().max(1.0));
        let max = self.hot_max();
        let value = temp_of_x(&geom, x).round().clamp(HOT_MIN, max);
        self.hot_drag = Some(value);
        self.commit_hot(Some(value), cx);
    }

    /// A point's temperature: the list is sorted and the write waits for the
    /// debounce.
    fn commit_point_temp(&mut self, index: usize, value: f32, cx: &mut Context<Self>) {
        let mut points = self.editor_points.clone();
        if let Some(point) = points.get_mut(index) {
            point.temp_c = value;
        }
        let mut sorted = points;
        sorted.sort_by(|a, b| {
            a.temp_c
                .partial_cmp(&b.temp_c)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        self.commit_curve(sorted, cx);
    }

    /// A point's percent, clamped to the floor.
    fn commit_point_percent(&mut self, index: usize, value: f32, cx: &mut Context<Self>) {
        let floor = f32::from(self.base_floor_for());
        let mut points = self.editor_points.clone();
        if let Some(point) = points.get_mut(index) {
            point.percent = value.round().clamp(floor, 100.0).max(0.0) as u8;
        }
        self.commit_curve(points, cx);
    }

    /// A point is removed (never the last two).
    fn remove_point(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.editor_points.len() <= editor::MIN_POINTS {
            return;
        }
        let mut points = self.editor_points.clone();
        if index >= points.len() {
            return;
        }
        points.remove(index);
        self.commit_curve(points, cx);
    }

    /// A point is added: ten degrees after the last one.
    fn add_point(&mut self, cx: &mut Context<Self>) {
        if self.editor_points.len() >= editor::MAX_POINTS {
            return;
        }
        let base = self.base_floor_for();
        let mut points = self.editor_points.clone();
        let temp = match points.last() {
            Some(last) => (last.temp_c + 10.0).min(100.0),
            None => 50.0,
        };
        points.push(CurvePoint {
            temp_c: temp,
            percent: base.max(100),
        });
        let mut sorted = points;
        sorted.sort_by(|a, b| {
            a.temp_c
                .partial_cmp(&b.temp_c)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        self.commit_curve(sorted, cx);
    }

    /// -- the list ----------------------------------------------------------
    /// Ticks or unticks a header; with shift, every header between the last
    /// tick and this one.
    fn toggle_tick(&mut self, id: &str, shift: bool, cx: &mut Context<Self>) {
        let Some(fans) = self.fans.clone() else {
            return;
        };
        let Some(header) = fans.headers.iter().find(|h| h.id == id) else {
            return;
        };
        if !can_drive(header) {
            return;
        }
        let drivable: Vec<String> = fans
            .headers
            .iter()
            .filter(|h| can_drive(h))
            .map(|h| h.id.clone())
            .collect();
        let on = !self.ticked.iter().any(|t| t == id);
        let mut ids = vec![id.to_string()];
        if shift {
            if let Some(last) = self.last_tick.clone() {
                let a = drivable.iter().position(|x| *x == last);
                let b = drivable.iter().position(|x| x == id);
                if let (Some(a), Some(b)) = (a, b) {
                    let (lo, hi) = (a.min(b), a.max(b));
                    ids = drivable[lo..=hi].to_vec();
                }
            }
        }
        self.last_tick = Some(id.to_string());
        if on {
            for id in ids {
                if !self.ticked.contains(&id) {
                    self.ticked.push(id);
                }
            }
        } else {
            self.ticked.retain(|t| !ids.contains(t));
        }
        cx.notify();
    }

    fn clear_ticks(&mut self, cx: &mut Context<Self>) {
        self.ticked.clear();
        self.last_tick = None;
        cx.notify();
    }
}

/// The mode's word, as the remount key uses it.
fn mode_word(mode: &FanMode) -> &'static str {
    match mode {
        FanMode::Bios => "bios",
        FanMode::Fixed { .. } => "fixed",
        FanMode::Curve { .. } => "curve",
    }
}

/// `encodeURIComponent` for a profile name.
fn encode(name: &str) -> String {
    let mut out = String::new();
    for byte in name.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The drawing.

/// The mode state: a dot and a word, no border, so it never reads as a
/// button.
impl FansView {
    /// The strip's title slot: the page's name and its one-line context.
    ///
    /// React draws the page's title in the status strip (`PageTitle`), with the
    /// page's context beside it. The context is hidden while the read has
    /// failed or has no data yet.
    pub fn strip_title(&self) -> (String, Option<String>) {
        let mut context = None;
        if let Some(fans) = &self.fans {
            if self.fans_error.is_none() {
                let chip = fans.chip.clone().unwrap_or_else(|| "No chip".to_string());
                context = Some(format!(
                    "{chip} \u{00B7} floor {}%",
                    fans.floor_percent
                ));
            }
        }
        ("Fans".to_string(), context)
    }

    /// The strip's actions slot: the profile picker, the save key and the menu
    /// that holds the rest (React `PageActions`).
    pub fn strip_actions(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        self.profile_strip(cx).into_any_element()
    }

    fn mode_state(&self, mode: &FanMode, with_word: bool) -> impl IntoElement {
        let t = &self.theme;
        let (dot, word_color) = match mode {
            FanMode::Curve { .. } => (live(t), Theme::color(t.ink)),
            FanMode::Fixed { .. } => (Theme::color(t.warn), Theme::color(t.dim)),
            FanMode::Bios => (Theme::color(t.mute), Theme::color(t.dim)),
        };
        let mut row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().size(px(7.0)).rounded_full().bg(dot));
        if with_word {
            row = row.child(
                div()
                    .text_size(px(14.0))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(word_color)
                    .child(mode_label(mode)),
            );
        }
        row
    }

    /// The profile strip: the profile that is on, the key that saves it and
    /// the menu that holds the rest.
    fn profile_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let Some(fans) = self.fans.clone() else {
            return div();
        };
        let options: Vec<(&str, &str)> = fans
            .profiles
            .iter()
            .map(|p| (p.as_str(), p.as_str()))
            .collect();
        let entity = cx.entity();
        let selected_profile = fans.active_profile.clone();
        let seg = segmented(
            t,
            "fans-profile-segmented",
            &options,
            &selected_profile,
            move |value, _, cx| {
                entity.update(cx, |view, cx| view.switch_profile(value, cx));
            },
        );
        let entity = cx.entity();
        let save_btn = fans_button(
            t,
            "fans-profile-save",
            &format!("Save {}", fans.active_profile),
            BtnVariant::Primary,
            true,
            false,
            Some("FloppyDisk"),
            move |_, _, cx| {
                entity.update(cx, |view, cx| view.save_profile(cx));
            },
        );
        let entity = cx.entity();
        let more_btn = fans_icon_button(
            t,
            "fans-profile-menu",
            "The fan profiles",
            "DotsThreeOutline",
            28.0,
            16.0,
            false,
            move |_, _, cx| {
                entity.update(cx, |view, cx| {
                    view.more_open = !view.more_open;
                    cx.notify();
                });
            },
        );
        // The menu hangs under its own key (React's `Popover` anchors it to
        // the key).
        let mut menu = div().relative().child(more_btn);
        if self.more_open {
            menu = menu.child(self.profile_menu(cx));
        }
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(seg)
            .child(save_btn)
            .child(menu)
    }

    /// The popover: duplicate, rename and the delete rows.
    fn profile_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let Some(fans) = self.fans.clone() else {
            return div().id(gpui::SharedString::from("fans-profile-menu-body"));
        };
        let entity = cx.entity();
        let duplicate = fans_button(
            t,
            "fans-profile-duplicate",
            "Duplicate",
            BtnVariant::Default,
            true,
            false,
            Some("CopySimple"),
            move |_, _, cx| {
                entity.update(cx, |view, cx| view.duplicate_profile(cx));
            },
        );
        let entity = cx.entity();
        let rename = fans_number_field(
            t,
            "fans-profile-rename",
            0.0,
            0.0,
            9999.0,
            1.0,
            None,
            false,
            move |_, _, _| {
                // The React field commits on Enter or blur; the GPUI field
                // has no text entry yet, so the rename stays unticked.
                let _ = &entity;
            },
        );
        let mut body = div().flex().flex_col().gap_3().child(duplicate).child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child("Rename"),
                )
                .child(rename),
        );
        match &self.confirming {
            Some(name) => {
                let entity = cx.entity();
                let name = name.clone();
                let delete_name = name.clone();
                let delete = fans_button(
                    t,
                    "fans-profile-delete-confirm",
                    "Delete",
                    BtnVariant::Danger,
                    true,
                    false,
                    None,
                    move |_, _, cx| {
                        entity.update(cx, |view, cx| view.delete_profile(delete_name.clone(), cx));
                    },
                );
                let entity = cx.entity();
                let cancel = fans_button(
                    t,
                    "fans-profile-delete-cancel",
                    "Cancel",
                    BtnVariant::Ghost,
                    true,
                    false,
                    None,
                    move |_, _, cx| {
                        entity.update(cx, |view, cx| {
                            view.confirming = None;
                            cx.notify();
                        });
                    },
                );
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .rounded_md()
                        .border_1()
                        .border_color(Theme::color(t.down))
                        .p_2()
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.dim))
                                .child(format!("Delete {name}? Its saved headers are removed.")),
                        )
                        .child(div().flex().gap_2().child(delete).child(cancel)),
                );
            }
            None => {
                for name in fans.profiles.iter().filter(|p| **p != fans.active_profile) {
                    let entity = cx.entity();
                    let name = name.clone();
                    let picked = name.clone();
                    let delete = fans_button(
                        t,
                        &format!("fans-profile-delete-{name}"),
                        "Delete",
                        BtnVariant::Ghost,
                        true,
                        false,
                        Some("Trash"),
                        move |_, _, cx| {
                            entity.update(cx, |view, cx| {
                                view.confirming = Some(picked.clone());
                                cx.notify();
                            });
                        },
                    );
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .font_family(crate::fonts::UI_FAMILY)
                                    .text_color(Theme::color(t.ink))
                                    .child(name.clone()),
                            )
                            .child(delete),
                    );
                }
            }
        }
        if let Some(error) = &self.profile_error {
            body = body.child(
                div()
                    .text_size(px(15.0))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.down))
                    .child(error.clone()),
            );
        }
        div()
            .id(gpui::SharedString::from("fans-profile-menu-body"))
            .absolute()
            .top(px(30.0))
            .right_0()
            .w(px(260.0))
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.panel))
            .shadow(vec![gpui::BoxShadow {
                color: gpui::Hsla::from(gpui::Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.5,
                }),
                offset: gpui::point(px(0.0), px(12.0)),
                blur_radius: px(32.0),
                spread_radius: px(0.0),
            }])
            .child(body)
    }

    /// The temperatures panel.
    fn temperatures(&self, page: f32) -> gpui::AnyElement {
        let t = &self.theme;
        let Some(fans) = self.fans.as_ref() else {
            return div().into_any_element();
        };
        let mut body = div().flex().flex_col().gap_4();
        if fans.temps.is_empty() {
            body = body.child(
                div()
                    .text_size(px(15.0))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("No temperature sources reported."),
            );
        } else {
            let mut row = div().flex().gap_4();
            for temp in &fans.temps {
                row = row.child(self.temp_cell(temp, page));
            }
            body = body.child(row);
        }
        panel(t, "fans-temps", "Temperatures", div(), body).into_any_element()
    }

    /// One temperature: its label, its meter and its reading.
    fn temp_cell(&self, temp: &TempReading, page: f32) -> impl IntoElement {
        let t = &self.theme;
        let celsius = temp.celsius.unwrap_or(0.0);
        let tone = temp_tone(t, celsius);
        let fill = ((celsius - 20.0) / 80.0).clamp(0.0, 1.0);
        let phone = page <= 640.0;
        let label = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .text_size(px(14.0))
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.dim))
            .child(temp.label.clone());
        let value = div()
            .text_size(px(14.0))
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.ink))
            .child(format!("{celsius:.1} {}", "\u{00B0}C"));
        let bar = meter(t, &format!("fans-temp-{}", temp.label), fill, tone, None);
        let mut cell = div().flex_1().flex_col().gap_1().min_w_0();
        if phone {
            // The phone cell: the label, the value and the bar, one over the
            // other.
            cell = cell.child(label).child(value).child(bar);
        } else if page <= 1499.0 {
            // The label stands over the value and the bar.
            cell = cell.child(label).child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(value)
                    .child(div().flex_1().child(bar)),
            );
        } else {
            // The label and the value share a line, the bar under them.
            cell = cell
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(label)
                        .child(value),
                )
                .child(bar);
        }
        cell
    }
}

// ---------------------------------------------------------------------------
// The headers table and the phone cards.

impl FansView {
    /// The Headers panel: the bulk bar over the list, then the table (or the
    /// phone cards).
    fn headers_panel(&self, page: f32, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = &self.theme;
        let narrow = page <= 640.0;
        let Some(fans) = self.fans.as_ref() else {
            return div().id(gpui::SharedString::from("fans-headers"));
        };
        let checked = checked_headers(fans, &self.ticked);
        // On a desktop the list scrolls inside the panel (React
        // `.table-scroll`); on a phone the page scrolls and the panel is as
        // tall as its rows.
        let mut body = div().flex().flex_col();
        if !narrow {
            body = body.flex_1().min_h_0();
        }
        // The Select/Done key, on a phone only.
        let actions = if narrow {
            let entity = cx.entity();
            div().child(fans_button(
                t,
                "fans-select-key",
                if self.select_mode { "Done" } else { "Select" },
                if self.select_mode {
                    BtnVariant::Primary
                } else {
                    BtnVariant::Default
                },
                true,
                self.bulk_busy,
                Some("ListChecks"),
                move |_, _, cx| {
                    entity.update(cx, |view, cx| {
                        if view.select_mode {
                            view.clear_ticks(cx);
                        }
                        view.select_mode = !view.select_mode;
                        cx.notify();
                    });
                },
            ))
        } else {
            div()
        };
        if !narrow || self.select_mode {
            body = body.child(self.bulk_bar(fans, &checked, cx));
        }
        if narrow {
            body = body.child(self.phone_cards(fans, page, cx));
        } else {
            let width = self.page_width() * 1.1 / 2.1 - 32.0;
            body = body.child(self.headers_table(fans, page, width, cx));
        }
        let panel = panel(t, "fans-headers", "Headers", actions, body);
        if narrow {
            panel
        } else {
            // React `.rd-headers { flex: 1 1 auto; display: flex;
            // flex-direction: column; min-height: 0; }`: the panel takes the
            // room the column has left and the list scrolls inside it.
            panel.flex_1().min_h_0()
        }
    }

    /// The bulk bar over the list.
    fn bulk_bar(
        &self,
        fans: &FansStatus,
        checked: &[FanHeaderStatus],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let entity = cx.entity();
        let entity2 = cx.entity();
        let entity3 = cx.entity();
        let entity4 = cx.entity();
        let entity5 = cx.entity();
        fan_bulk_bar(
            t,
            "fans-bulk",
            fans,
            checked,
            self.bulk_percent,
            self.bulk_picked_from.as_deref(),
            self.bulk_busy,
            self.bulk_result.clone(),
            self.bulk_details_open,
            move |_, cx| {
                entity.update(cx, |view, cx| {
                    view.bulk_result = None;
                    view.bulk_details_open = false;
                    view.clear_ticks(cx);
                });
            },
            move |_, cx| {
                entity2.update(cx, |view, cx| view.bulk_start(BulkAction::Bios, cx));
            },
            move |value, _, cx| {
                entity3.update(cx, |view, cx| {
                    view.bulk_percent = value;
                    view.bulk_start(
                        BulkAction::Fixed {
                            percent: value.round().clamp(0.0, 100.0) as u8,
                        },
                        cx,
                    );
                });
            },
            move |from, _, cx| {
                entity4.update(cx, |view, cx| {
                    view.bulk_picked_from = Some(from.to_string());
                    view.bulk_start(
                        BulkAction::Curve {
                            from: from.to_string(),
                        },
                        cx,
                    );
                });
            },
            move |_, cx| {
                entity5.update(cx, |view, cx| {
                    view.bulk_details_open = !view.bulk_details_open;
                    cx.notify();
                });
            },
        )
    }

    /// The headers table: the tick box, the name, the speed, the duty, the
    /// mode and the last minute.
    fn headers_table(
        &self,
        fans: &FansStatus,
        page: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let narrow_table = page <= 999.0;
        let with_word = page > 1499.0;
        let cols = table_columns(fans, page, width);
        let padding_x = cols.padding_x;

        // The head.
        let mut head = div()
            .flex()
            .items_center()
            .py(px(4.0))
            .border_b_1()
            .border_color(Theme::color(t.line))
            .text_size(px(14.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.mute));
        let drivable: Vec<String> = fans
            .headers
            .iter()
            .filter(|h| can_drive(h))
            .map(|h| h.id.clone())
            .collect();
        let all_ticked =
            !drivable.is_empty() && checked_headers(fans, &self.ticked).len() == drivable.len();
        let some_ticked = !checked_headers(fans, &self.ticked).is_empty();
        let entity = cx.entity();
        let select_all = tick_box(
            t,
            "fans-select-all",
            all_ticked,
            some_ticked && !all_ticked,
            false,
            move |_, _, cx| {
                entity.update(cx, |view, cx| {
                    if all_ticked {
                        view.clear_ticks(cx);
                    } else {
                        view.ticked = drivable.clone();
                        cx.notify();
                    }
                });
            },
        );
        head = head
            .child(
                div()
                    .w(px(cols.tick))
                    .px(px(padding_x))
                    .pr_0()
                    // React's `.tick` is a 24 px target around the 18 px box.
                    .child(
                        div()
                            .size(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(select_all),
                    ),
            )
            .child(
                div()
                    .w(px(cols.name))
                    .px(px(padding_x))
                    .min_w_0()
                    .child("Header"),
            )
            .child(
                div()
                    .w(px(cols.rpm))
                    .px(px(padding_x))
                    .text_right()
                    .whitespace_nowrap()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .child("RPM"),
            )
            .child(
                div()
                    .w(px(cols.duty))
                    .px(px(padding_x))
                    .child("Duty"),
            )
            .child(
                div()
                    .w(px(cols.mode))
                    .px(px(padding_x))
                    .child("Mode"),
            );
        if !narrow_table {
            head = head.child(
                div()
                    .w(px(cols.history))
                    .px(px(padding_x))
                    .whitespace_nowrap()
                    .child("Last 60 s"),
            );
        }

        // The rows.
        let mut body = div().flex().flex_col();
        for header in &fans.headers {
            body = body.child(self.header_row(header, page, with_word, &cols, cx));
        }
        div()
            .id(gpui::SharedString::from("fans-table"))
            .flex()
            .flex_col()
            // React `.table-scroll { flex: 1 1 auto; min-height: 0;
            // overflow: auto; }`: the list scrolls inside the panel.
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(head)
            .child(body)
    }

    /// One row of the table.
    fn header_row(
        &self,
        header: &FanHeaderStatus,
        page: f32,
        with_word: bool,
        cols: &TableColumns,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let narrow_table = cols.history == 0.0;
        let unavailable = is_unavailable(header);
        let tach = !unavailable && !can_drive(header);
        let blocked = !can_drive(header);
        let checked = self.ticked.iter().any(|id| id == &header.id);
        let selected = self
            .selected(page <= 640.0)
            .is_some_and(|s| s.id == header.id);
        let history = self.history.get(&header.id).cloned().unwrap_or_default();
        let peak = sparkline_peak(&history);
        let duty = header.pwm_percent.unwrap_or(0) as f32;

        let entity = cx.entity();
        let id = header.id.clone();
        let box_entity = cx.entity();
        let box_id = header.id.clone();
        let tick = tick_box(
            t,
            &format!("fans-tick-{id}"),
            checked,
            false,
            blocked,
            move |_, _, cx| {
                box_entity.update(cx, |view, cx| view.toggle_tick(&box_id, false, cx));
            },
        );
        let _ = entity;

        // The name and its chips.
        let name_color = if unavailable {
            Theme::color(t.mute)
        } else {
            Theme::color(t.ink)
        };
        let mut name = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .gap_x_2()
            .max_w(px(34.0 * 8.0))
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(name_color)
                    .child(plain_label(&header.label)),
            );
        if unavailable {
            name = name.child(chip(t, &format!("fans-chip-{id}"), "Not on this board"));
        } else if tach {
            name = name.child(chip(t, &format!("fans-chip-{id}"), "tach only"));
        }
        if let Some(handoff) = handoff_text(header, checked) {
            name = name.child(
                div()
                    .text_size(px(14.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.warn))
                    .child(handoff),
            );
        }
        if let Some(reason) = &header.override_reason {
            name = name.child(
                div()
                    .flex_1()
                    .w_full()
                    .text_size(px(14.0))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(reason.clone()),
            );
        }

        let rpm_cell = div()
            .w(px(cols.rpm))
            .px(px(cols.padding_x))
            .text_right()
            .text_size(px(20.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(if unavailable {
                Theme::color(t.mute)
            } else {
                Theme::color(t.ink)
            })
            .child(format!("{}", header.rpm.unwrap_or(0)));
        let duty_cell = div()
            .w(px(cols.duty))
            .px(px(cols.padding_x))
            .flex()
            .items_center()
            .justify_end()
            .gap_2()
            .child(meter(
                t,
                &format!("fans-duty-meter-{id}"),
                duty / 100.0,
                Theme::color(t.a3),
                Some(cols.duty_meter),
            ))
            .child(
                div()
                    .text_size(px(14.0))
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(if unavailable {
                        Theme::color(t.mute)
                    } else {
                        Theme::color(t.ink)
                    })
                    .child(match header.pwm_percent {
                        None => "\u{2014}".to_string(),
                        Some(pwm) => format!("{pwm}%"),
                    }),
            );
        let mode_cell = div()
            .w(px(cols.mode))
            .px(px(cols.padding_x))
            .child(self.mode_state(&header.mode, with_word));

        let mut row = div()
            .id(gpui::SharedString::from(format!("fans-row-{id}")))
            .relative()
            .flex()
            .items_center()
            .min_h(px(40.0))
            .py(px(4.0))
            .border_b_1()
            .border_color(Theme::color(t.line))
            .cursor_pointer()
            .child(
                div()
                    .w(px(cols.tick))
                    .px(px(cols.padding_x))
                    .pr_0()
                    // React's `.tick` is a 24 px target around the 18 px box.
                    .child(
                        div()
                            .size(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(tick),
                    ),
            )
            .child(div().w(px(cols.name)).px(px(cols.padding_x)).min_w_0().child(name))
            .child(rpm_cell)
            .child(duty_cell)
            .child(mode_cell);
        if !narrow_table {
            row = row.child(
                div()
                    .w(px(cols.history))
                    .px(px(cols.padding_x))
                    .child(sparkline(
                        t,
                        &format!("fans-spark-{id}"),
                        &history,
                        peak,
                        56.0,
                        16.0,
                    )),
            );
        }
        if selected {
            row = row.bg(live_soft(t)).child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(3.0))
                    .bg(live(t)),
            );
        }
        if unavailable {
            row = row.text_color(Theme::color(t.mute));
        }
        let entity = cx.entity();
        let select_id = header.id.clone();
        row.on_click(move |_, _, cx| {
            entity.update(cx, |view, cx| {
                view.selected_id = Some(select_id.clone());
                cx.notify();
            });
        })
    }

    /// The phone cards: the headers as compact rows.
    fn phone_cards(
        &self,
        fans: &FansStatus,
        page: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = &self.theme;
        let mut list = div().flex().flex_col();
        for header in &fans.headers {
            let id = header.id.clone();
            let unavailable = is_unavailable(header);
            let tach = !unavailable && !can_drive(header);
            let blocked = !can_drive(header);
            let checked = self.ticked.iter().any(|x| x == &id);
            let selected = self.selected(page <= 640.0).is_some_and(|s| s.id == id);
            let history = self.history.get(&id).cloned().unwrap_or_default();
            let peak = sparkline_peak(&history);
            let duty = header.pwm_percent.unwrap_or(0) as f32;

            let mut head = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font_family(crate::fonts::MONO_FAMILY)
                        .text_color(if unavailable {
                            Theme::color(t.mute)
                        } else {
                            Theme::color(t.ink)
                        })
                        .child(header.label.clone()),
                )
                .child(self.mode_state(&header.mode, false));
            if selected && !self.select_mode {
                head = head.child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(live(t))
                        .child("Editing"),
                );
            }
            if unavailable {
                head = head.child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(live(t))
                        .child(if self.select_mode {
                            "Not on this board, cannot be set"
                        } else {
                            "Not on this board"
                        }),
                );
            }
            if self.select_mode && tach {
                head = head.child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(live(t))
                        .child("Tach only, cannot be set"),
                );
            }
            if let Some(handoff) = handoff_text(header, self.select_mode && checked) {
                head = head.child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.warn))
                        .child(handoff),
                );
            }
            let rpm_readout = div()
                .flex()
                .items_baseline()
                .gap_1()
                .text_size(px(20.0))
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(if unavailable {
                    Theme::color(t.mute)
                } else {
                    Theme::color(t.ink)
                })
                .child(format!("{}", header.rpm.unwrap_or(0)))
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child("RPM"),
                );
            // On a phone the card is one line: the name, its mode and its
            // speed. The meter, the percent and the sparkline are hidden there.
            let readouts = if page <= 640.0 {
                div().flex().items_center().gap_3().child(rpm_readout)
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(rpm_readout)
                    .child(meter(
                        t,
                        &format!("fans-card-meter-{id}"),
                        duty / 100.0,
                        Theme::color(t.a3),
                        Some(52.0),
                    ))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(match header.pwm_percent {
                                None => "\u{2014}".to_string(),
                                Some(pwm) => format!("{pwm}%"),
                            }),
                    )
                    .child(sparkline(
                        t,
                        &format!("fans-card-spark-{id}"),
                        &history,
                        peak,
                        180.0,
                        44.0,
                    ))
            };

            let mut card = div()
                .id(gpui::SharedString::from(format!("fans-card-{id}")))
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .py(px(8.0))
                .px(px(12.0))
                .border_b_1()
                .border_color(Theme::color(t.line))
                .cursor_pointer();
            if self.select_mode {
                let entity = cx.entity();
                let box_id = id.clone();
                card = card.child(
                    div()
                        .id(gpui::SharedString::from(format!("fans-card-tick-{id}")))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(if checked {
                            Rgba { a: 0.0, ..live(t) }
                        } else {
                            Theme::color(t.line_2)
                        })
                        .bg(if checked {
                            live(t)
                        } else if blocked {
                            Theme::color(t.raised)
                        } else {
                            Theme::color(t.sunk)
                        })
                        .child(
                            gpui::svg()
                                .path(if checked {
                                    "icons/Check-bold.svg"
                                } else {
                                    "icons/Minus-bold.svg"
                                })
                                .size_3()
                                .text_color(if checked {
                                    Theme::color(t.on_accent)
                                } else {
                                    Theme::color(t.mute)
                                }),
                        )
                        .on_click(move |_, _, cx| {
                            if blocked {
                                return;
                            }
                            entity.update(cx, |view, cx| view.toggle_tick(&box_id, false, cx));
                        }),
                );
            }
            card = card
                .child(div().flex_1().min_w_0().child(head))
                .child(readouts.flex_none());
            if selected && !self.select_mode {
                card = card.bg(live_soft(t));
            }
            let entity = cx.entity();
            let select_id = id.clone();
            card = card.on_click(move |_, _, cx| {
                entity.update(cx, |view, cx| {
                    if view.select_mode {
                        if !blocked {
                            view.toggle_tick(&select_id, false, cx);
                        }
                    } else {
                        view.selected_id = Some(select_id.clone());
                        cx.notify();
                    }
                });
            });
            list = list.child(card);
        }
        div()
            .id(gpui::SharedString::from("fans-cards"))
            .flex()
            .flex_col()
            .child(list)
    }
}

// ---------------------------------------------------------------------------
// The page.

impl Render for FansView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.window_width = f32::from(window.window_bounds().get_bounds().size.width);
        self.window_height = f32::from(window.window_bounds().get_bounds().size.height);
        let page = self.page_width();
        let narrow = page <= 640.0;
        let t = self.theme;
        // React `.rd-page`: a column that fills the content box, with its own
        // gap. The shell's content box already carries the page's padding, so
        // the page adds none of its own, and it must be allowed to be shorter
        // than its content (React `min-height: 0`), or the strip above it
        // shrinks.
        let mut root = div()
            .id(gpui::SharedString::from("fans-page"))
            .relative()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_h_0()
            .min_w_0();
        if let Some(notice) = &self.notice {
            root = root.child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(Theme::color(t.ok))
                    .bg(mix_srgb(Theme::color(t.ok), Theme::color(t.panel), 0.10))
                    .text_size(px(15.0))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child(notice.clone()),
            );
        }
        // The profile picker, the save key and the menu are in the strip
        // (React `PageActions`); the page keeps only its own body.
        // A read that failed after the readings came keeps the page and says
        // the readings may be old.
        if self.fans.is_some() {
            if let Some(error) = &self.fans_error {
                root = root.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .px_3()
                        .rounded_md()
                        .border_1()
                        .border_color(tint(Theme::color(t.warn), 0.40))
                        .bg(mix_srgb(Theme::color(t.warn), Theme::color(t.sunk), 0.08))
                        .text_size(px(14.0))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child("The fan readings may be out of date.")
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_family(crate::fonts::MONO_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child(error.clone()),
                        ),
                );
            }
        }
        match (self.fans.is_some(), self.fans_error.clone()) {
            (false, Some(error)) => {
                root = root.child(self.service_down(&error, cx));
            }
            (false, None) => {
                root = root.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_4()
                        .rounded_lg()
                        .border_1()
                        .border_color(Theme::color(t.line))
                        .bg(Theme::color(t.panel))
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child("Reading the fans"),
                        ),
                );
            }
            (true, _) => {
                if narrow {
                    // One column on a phone: the readings over the list, and
                    // the editor is a sheet over them.
                    let left = div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .child(div().min_w_0().child(self.temperatures(page)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .min_h_0()
                                .flex_1()
                                .child(self.headers_panel(page, cx)),
                        );
                    root = root.child(left);
                    if let Some(header) = self.selected(narrow) {
                        let editor = self.editor(&header, page, cx);
                        root = root.child(
                            div()
                                .id(gpui::SharedString::from("fans-editor-sheet"))
                                .absolute()
                                .top(px(-16.0))
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .p_3()
                                .bg(Theme::color(t.g1))
                                .overflow_y_scroll()
                                .child(editor),
                        );
                    }
                } else {
                    // React `.rd-fans-grid`: two columns, `minmax(0, 1.1fr)`
                    // and `minmax(0, 1fr)`, with a 16 px gap between them.
                    let gap = 16.0;
                    let avail = (page - gap).max(0.0);
                    let left_w = avail * 1.1 / 2.1;
                    let right_w = avail / 2.1;
                    let page_h = self.page_height();
                    let left = div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .w(px(left_w))
                        .min_w_0()
                        .min_h_0()
                        .h(px(page_h))
                        .child(div().min_w_0().child(self.temperatures(page)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .min_h_0()
                                .flex_1()
                                .child(self.headers_panel(page, cx)),
                        );
                    let mut grid = div()
                        .flex()
                        .gap_4()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_hidden();
                    grid = grid.child(left);
                    if let Some(header) = self.selected(narrow) {
                        let editor = self.editor(&header, page, cx);
                        grid = grid.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_4()
                                .w(px(right_w))
                                .min_w_0()
                                .min_h_0()
                                .h(px(page_h))
                                .child(editor),
                        );
                    }
                    root = root.child(grid);
                }
            }
        }
        root
    }
}

/// The service-down page: the fan service is not answering, and the last
/// reading stands under it.
impl FansView {
    fn service_down(&self, error: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let entity = cx.entity();
        let retry = fans_button(
            t,
            "fans-try-again",
            "Try again",
            BtnVariant::Primary,
            false,
            self.retrying,
            Some("ArrowClockwise"),
            move |_, _, cx| {
                entity.update(cx, |view, cx| {
                    view.retrying = true;
                    view.loading = true;
                    view.load(cx);
                });
            },
        );
        let body = div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .p_8()
            .child(
                gpui::svg()
                    .path("icons/Fan.svg")
                    .size(px(24.0))
                    .text_color(Theme::color(t.warn)),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child("The fan service is not answering"),
            )
            .child(
                div()
                    .text_size(px(15.0))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(
                        "No answer for a moment. The fans stay under BIOS control; the lights are unaffected.",
                    ),
            )
            .child(retry)
            .child(
                div()
                    .text_size(px(15.0))
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(error.to_string()),
            );
        div().flex().flex_col().gap_4().child(
            div()
                .rounded_lg()
                .border_1()
                .border_color(Theme::color(t.line))
                .bg(Theme::color(t.panel))
                .child(body),
        )
    }
}

// ---------------------------------------------------------------------------
// The tests.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::fan_bits::warn_wash;
    use crate::components::fan_curve::{duty_for, hot_floor, x_of, y_of};
    use canvas_proto::fans::{CurvePoint, HotLimitStatus};

    fn header(id: &str, label: &str, pwm: Option<u8>, mode: FanMode) -> FanHeaderStatus {
        FanHeaderStatus {
            id: id.to_string(),
            label: label.to_string(),
            rpm: Some(1200),
            pwm_percent: pwm,
            mode,
            on_bios: false,
            override_reason: None,
            available: true,
            override_available: None,
        }
    }

    fn fans(headers: Vec<FanHeaderStatus>) -> FansStatus {
        FansStatus {
            chip: Some("NCT6687D-R @ 0xA20".into()),
            hardware_watchdog: Some(false),
            headers,
            temps: vec![
                TempReading {
                    source: TempSource::Cpu,
                    label: "CPU (Tctl)".into(),
                    celsius: Some(42.5),
                },
                TempReading {
                    source: TempSource::Gpu,
                    label: "GPU (NVML)".into(),
                    celsius: Some(55.0),
                },
            ],
            active_profile: "Default".into(),
            profiles: vec!["Default".into(), "Quiet".into()],
            floor_percent: 20,
            hot: Vec::new(),
        }
    }

    #[test]
    fn the_page_width_follows_the_rail() {
        // A wide window: the 200 px rail.
        assert_eq!(page_width(1920.0), 1920.0 - 200.0 - 48.0);
        // An icon rail.
        assert_eq!(page_width(1024.0), 1024.0 - 64.0 - 48.0);
        // No rail on a phone.
        assert_eq!(page_width(390.0), 390.0 - 48.0);
    }

    #[test]
    fn the_mode_words_are_the_react_ones() {
        assert_eq!(mode_label(&FanMode::Bios), "BIOS");
        assert_eq!(mode_label(&FanMode::Fixed { percent: 40 }), "Fixed 40%");
        assert_eq!(
            mode_label(&FanMode::Curve {
                source: TempSource::Cpu,
                points: Vec::new(),
            }),
            "Curve"
        );
    }

    #[test]
    fn the_temperature_tone_follows_the_thresholds() {
        let t = Theme::dark();
        assert_eq!(temp_tone(&t, 49.9), Theme::color(t.a3));
        assert_eq!(temp_tone(&t, 50.0), Theme::color(t.warn));
        assert_eq!(temp_tone(&t, 71.9), Theme::color(t.warn));
        assert_eq!(temp_tone(&t, 72.0), Theme::color(t.down));
    }

    #[test]
    fn the_copy_name_takes_the_first_free_one() {
        let profiles = vec!["Default".to_string(), "Quiet".to_string()];
        assert_eq!(next_profile_name("Default", &profiles), "Default copy");
        let profiles = vec![
            "Default".to_string(),
            "Default copy".to_string(),
            "Default copy 2".to_string(),
        ];
        assert_eq!(next_profile_name("Default", &profiles), "Default copy 3");
    }

    #[test]
    fn the_reason_words_name_the_kind() {
        let mut h = header(
            "cpu_fan",
            "CPU fan",
            Some(40),
            FanMode::Fixed { percent: 50 },
        );
        assert_eq!(reason_kind_of(&h), "");
        h.override_reason = Some("at the safety ceiling".into());
        assert_eq!(reason_kind_of(&h), "canvas-rgb is overriding CPU fan");
        h.on_bios = true;
        assert_eq!(
            reason_kind_of(&h),
            "CPU fan: canvas-rgb handed this header back to BIOS"
        );
        assert_eq!(
            reason_text(&h).unwrap(),
            "canvas-rgb handed CPU fan back to BIOS: at the safety ceiling"
        );
    }

    #[test]
    fn the_quiet_line_says_who_is_in_charge() {
        assert_eq!(
            quiet_line(&FanMode::Bios),
            "The BIOS is in charge of this fan."
        );
        assert_eq!(
            quiet_line(&FanMode::Fixed { percent: 50 }),
            "Fixed duty is in charge; nothing is overriding it."
        );
        assert_eq!(
            quiet_line(&FanMode::Curve {
                source: TempSource::Cpu,
                points: Vec::new()
            }),
            "The curve is in charge; nothing is overriding it."
        );
    }

    #[test]
    fn the_handoff_line_says_when_it_was_skipped() {
        let mut h = header(
            "cpu_fan",
            "CPU fan",
            Some(40),
            FanMode::Fixed { percent: 50 },
        );
        assert_eq!(handoff_text(&h, false), None);
        h.on_bios = true;
        assert_eq!(handoff_text(&h, false), Some("Handed back to BIOS"));
        assert_eq!(
            handoff_text(&h, true),
            Some("Handed back to BIOS, skipped when applied")
        );
    }

    #[test]
    fn the_unavailable_body_names_the_owner() {
        let mut h = header("sys_fan1", "System fan 1", Some(40), FanMode::Bios);
        h.available = false;
        assert_eq!(
            unavailable_body(&h),
            "Not on this board: this header is not fitted on your board, so there is no mode to set."
        );
        h.override_available = Some(false);
        assert_eq!(
            unavailable_body(&h),
            "You marked this header as not on your board, so there is no mode to set."
        );
    }

    #[test]
    fn the_sparkline_peak_is_never_under_2000() {
        assert_eq!(sparkline_peak(&[]), 2000);
        assert_eq!(sparkline_peak(&[100, 200]), 2000);
        assert_eq!(sparkline_peak(&[100, 3000]), 3000);
    }

    #[test]
    fn the_checkbox_label_says_why_a_header_is_blocked() {
        let tach = header("sys_fan2", "System fan 2 (tach only)", None, FanMode::Bios);
        assert_eq!(
            row_checkbox_label(&tach),
            "System fan 2 (tach only): tach only, cannot be selected"
        );
        assert_eq!(
            row_checkbox_title(&tach),
            Some(crate::components::fan_bulk::TACH_ONLY_WHY)
        );
        let mut missing = header("sys_fan1", "System fan 1", Some(40), FanMode::Bios);
        missing.available = false;
        assert_eq!(
            row_checkbox_label(&missing),
            "System fan 1: not on this board, cannot be selected"
        );
        let plain = header("sys_fan3", "System fan 3", Some(40), FanMode::Bios);
        assert_eq!(row_checkbox_label(&plain), "Select System fan 3");
        assert_eq!(row_checkbox_title(&plain), None);
    }

    #[test]
    fn the_source_keys_round_trip() {
        assert_eq!(source_key(&TempSource::Cpu), "cpu");
        assert_eq!(source_key(&TempSource::Gpu), "gpu");
        assert_eq!(
            source_key(&TempSource::Board("temp0".into())),
            "board:temp0"
        );
        assert_eq!(source_from_key("cpu"), TempSource::Cpu);
        assert_eq!(source_from_key("gpu"), TempSource::Gpu);
        assert_eq!(
            source_from_key("board:temp0"),
            TempSource::Board("temp0".into())
        );
    }

    #[test]
    fn the_curve_reads_the_source_that_reports() {
        let mode = FanMode::Curve {
            source: TempSource::Cpu,
            points: vec![CurvePoint {
                temp_c: 30.0,
                percent: 30,
            }],
        };
        let temps = vec![
            TempReading {
                source: TempSource::Cpu,
                label: "CPU (Tctl)".into(),
                celsius: Some(42.5),
            },
            TempReading {
                source: TempSource::Gpu,
                label: "GPU (NVML)".into(),
                celsius: Some(55.0),
            },
        ];
        assert_eq!(temp_for(&mode, &temps), Some(42.5));
        // A source with no reading reads as none.
        let missing = FanMode::Curve {
            source: TempSource::Board("temp9".into()),
            points: Vec::new(),
        };
        assert_eq!(temp_for(&missing, &temps), None);
        // A fixed header has no curve temperature.
        assert_eq!(temp_for(&FanMode::Fixed { percent: 50 }, &temps), None);
    }

    #[test]
    fn the_checked_headers_are_the_drivable_ticked_ones() {
        let status = fans(vec![
            header("cpu_fan", "CPU fan", Some(40), FanMode::Bios),
            header("sys_fan1", "System fan 1", Some(40), FanMode::Bios),
            header("sys_fan2", "System fan 2 (tach only)", None, FanMode::Bios),
        ]);
        let ticked = vec!["cpu_fan".to_string(), "sys_fan2".to_string()];
        let checked = checked_headers(&status, &ticked);
        assert_eq!(checked.len(), 1);
        assert_eq!(checked[0].id, "cpu_fan");
    }

    #[test]
    fn the_profile_name_is_encoded() {
        assert_eq!(encode("Default"), "Default");
        assert_eq!(encode("Quiet copy 2"), "Quiet%20copy%202");
    }

    #[test]
    fn the_curve_geometry_is_the_react_one() {
        let geom = curve_geometry(500.0);
        assert_eq!(geom.height, 310.0);
        assert_eq!(x_of(&geom, 60.0), 48.0 + 0.5 * geom.plot_w);
        assert_eq!(y_of(&geom, 50.0), 14.0 + 0.5 * geom.plot_h);
    }

    #[test]
    fn the_hot_floor_follows_the_header() {
        assert_eq!(hot_floor("cpu_fan", 20), 50);
        assert_eq!(hot_floor("sys_fan1", 20), 35);
    }

    #[test]
    fn the_bounds_keep_a_point_between_its_neighbours() {
        let points = vec![
            CurvePoint {
                temp_c: 30.0,
                percent: 30,
            },
            CurvePoint {
                temp_c: 50.0,
                percent: 50,
            },
        ];
        let (lo, hi) = bounds_for(&points, 0);
        assert_eq!(lo, 20.0);
        assert_eq!(hi, 49.0);
    }

    #[test]
    fn the_duty_of_a_curve_is_the_daemons() {
        let points = vec![
            CurvePoint {
                temp_c: 30.0,
                percent: 30,
            },
            CurvePoint {
                temp_c: 70.0,
                percent: 100,
            },
        ];
        assert_eq!(duty_for(&points, Some(50.0)), Some(65.0));
    }

    #[test]
    fn the_hot_row_is_the_sources() {
        let rows = vec![HotLimitStatus {
            kind: HotKind::Cpu,
            limit_c: 74.0,
            user_set: false,
            auto_c: 74.0,
            rated_max_c: Some(89.0),
            ceiling_c: 84.0,
            max_settable_c: 82.0,
        }];
        assert!(hot_row(&rows, HotKind::Cpu).is_some());
        assert!(hot_row(&rows, HotKind::Gpu).is_none());
    }

    #[test]
    fn the_sorted_points_keep_the_temperature_order() {
        let points = vec![
            CurvePoint {
                temp_c: 70.0,
                percent: 100,
            },
            CurvePoint {
                temp_c: 30.0,
                percent: 30,
            },
        ];
        let sorted = sorted_points(&points);
        assert_eq!(sorted[0].temp_c, 30.0);
        assert_eq!(sorted[1].temp_c, 70.0);
    }

    #[test]
    fn the_temperature_reads_the_x_back() {
        let geom = curve_geometry(400.0);
        let x = 100.0;
        let temp = temp_of_x(&geom, x);
        assert!((temp - 20.0 - ((x - 48.0) / geom.plot_w) * 80.0).abs() < 0.001);
    }

    #[test]
    fn the_warn_wash_follows_the_theme() {
        let dark = Theme::dark();
        let light = Theme::light();
        assert_ne!(warn_wash(&dark), warn_wash(&light));
        // The rule's own tint is a mix with the sunk colour.
        let mixed = mix_srgb(Theme::color(dark.a2), Theme::color(dark.sunk), 0.08);
        assert!(mixed.a > 0.99);
        assert_eq!(tint(Theme::color(dark.down), 0.18).a, 0.18);
    }
}
