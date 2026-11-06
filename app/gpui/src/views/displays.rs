//! `DisplaysView`, after `app/web/src/views/DisplaysView.tsx`.
//!
//! The port keeps the React view's rules (the checklist is
//! `docs/gpui-parity/displays.md`):
//! - one card per display, keyed by id and the saved config (a new saved config
//!   replaces the draft, dropping unsaved edits — the remount rule);
//! - the source picker, fixed value, unit/decimals segmented controls, blink
//!   switch and threshold, with Save (`PUT`) and Stop (`DELETE`);
//! - load states: loading placeholder, empty, unsupported (404), error with
//!   Try again, stale notice over live cards;
//! - the big digit readout, the "Now" line and the digit-count line.
//!
//! The React demo-mode states (`?state=...`) are demo-backend behavior and
//! have no GPUI counterpart: the GPUI editor always talks to a real (here,
//! fake) daemon over HTTP.

use std::collections::HashMap;

use canvas_proto::api::DisplayStatus;
use canvas_proto::display::{DisplayConfig, DisplaySource, DisplayUnit};
use canvas_proto::fans::FansStatus;
use gpui::{Context, IntoElement, Window, div, prelude::*, px};

use crate::client::DaemonClient;
use crate::components::button::{Variant, button};
use crate::components::empty_state::empty_state;
use crate::components::number_field::number_field;
use crate::components::panel::{field, panel, state_dot};
use crate::components::select::select;
use crate::components::segmented::segmented;
use crate::components::toggle::toggle;
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Logic, ported from DisplaysView.tsx and covered by the tests below.

/// The source as one string key, as the picker works on keys.
pub fn source_key(source: &DisplaySource) -> String {
    match source {
        DisplaySource::CpuTemp => "cpu_temp".into(),
        DisplaySource::GpuTemp => "gpu_temp".into(),
        DisplaySource::BoardTemp { sensor } => format!("board:{sensor}"),
        DisplaySource::FanRpm { header } => format!("fan:{header}"),
        DisplaySource::Fixed { .. } => "fixed".into(),
        DisplaySource::Api => "api".into(),
    }
}

/// A key back to a source. `fixed` keeps the current value if there is one,
/// else 0 (React `sourceFromKey`).
pub fn source_from_key(key: &str, current: &DisplaySource) -> DisplaySource {
    if key == "cpu_temp" {
        DisplaySource::CpuTemp
    } else if key == "gpu_temp" {
        DisplaySource::GpuTemp
    } else if let Some(sensor) = key.strip_prefix("board:") {
        DisplaySource::BoardTemp { sensor: sensor.into() }
    } else if let Some(header) = key.strip_prefix("fan:") {
        DisplaySource::FanRpm { header: header.into() }
    } else if key == "fixed" {
        let value = match current {
            DisplaySource::Fixed { value } => *value,
            _ => 0.0,
        };
        DisplaySource::Fixed { value }
    } else {
        DisplaySource::Api
    }
}

/// A fan RPM is a whole number: the daemon refuses decimals with one.
pub fn is_rpm(source: &DisplaySource) -> bool {
    matches!(source, DisplaySource::FanRpm { .. })
}

/// The digits the display shows, the way the adapter writes them.
pub fn display_digits(
    value: Option<f32>,
    digits: u8,
    decimals: u8,
    temperature: bool,
) -> (Vec<Option<char>>, bool, bool) {
    let Some(value) = value else {
        return (Vec::new(), false, true);
    };
    if value < 0.0 {
        return (Vec::new(), false, false);
    }
    let scaled = if temperature {
        if decimals == 0 {
            value.round() as i64 * 10
        } else {
            (value * 10.0).round() as i64
        }
    } else {
        value.round() as i64
    };
    if scaled > 10_i64.pow(digits as u32) - 1 {
        return (Vec::new(), false, false);
    }
    let text = format!("{:0>width$}", scaled, width = digits as usize);
    let cells = text
        .chars()
        .enumerate()
        .map(|(i, ch)| if i == 0 && ch == '0' { None } else { Some(ch) })
        .collect();
    (cells, temperature, true)
}

/// What a display would show with this source, read from the fan service's
/// temperatures and header speeds. Fahrenheit converts from Celsius.
pub fn preview_value(
    source: &DisplaySource,
    unit: DisplayUnit,
    fans: Option<&FansStatus>,
    api_value: Option<f32>,
) -> Option<f32> {
    let mut value = match source {
        DisplaySource::Fixed { value } => Some(*value),
        DisplaySource::Api => api_value,
        DisplaySource::CpuTemp => fans
            .and_then(|f| {
                f.temps
                    .iter()
                    .find(|t| t.source == canvas_proto::fans::TempSource::Cpu)
            })
            .and_then(|t| t.celsius),
        DisplaySource::GpuTemp => fans
            .and_then(|f| {
                f.temps
                    .iter()
                    .find(|t| t.source == canvas_proto::fans::TempSource::Gpu)
            })
            .and_then(|t| t.celsius),
        DisplaySource::BoardTemp { sensor } => fans
            .and_then(|f| {
                f.temps.iter().find(|t| {
                    t.source == canvas_proto::fans::TempSource::Board(sensor.clone())
                })
            })
            .and_then(|t| t.celsius),
        DisplaySource::FanRpm { header } => fans
            .and_then(|f| f.headers.iter().find(|h| &h.id == header))
            .and_then(|h| h.rpm.map(|r| r as f32)),
    };
    if let Some(v) = value {
        if unit == DisplayUnit::Fahrenheit {
            value = Some(v * 9.0 / 5.0 + 32.0);
        }
    }
    value
}



// ---------------------------------------------------------------------------
// The view.

pub enum DisplaysState {
    Loading,
    Ready,
    Empty,
    Unsupported,
    Error(String),
}

pub struct CardState {
    pub draft: DisplayConfig,
    pub saved_key: String,
    pub busy: bool,
    pub error: Option<String>,
    pub select_open: bool,
}

pub struct DisplaysView {
    pub theme: Theme,
    pub state: DisplaysState,
    pub rows: Vec<DisplayStatus>,
    pub stale_error: Option<String>,
    pub retrying: bool,
    pub fans: Option<FansStatus>,
    pub cards: HashMap<String, CardState>,
}

impl DisplaysView {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        let mut view = DisplaysView {
            theme,
            state: DisplaysState::Loading,
            rows: Vec::new(),
            stale_error: None,
            retrying: false,
            fans: None,
            cards: HashMap::new(),
        };
        view.load(cx);
        view
    }

    fn client() -> DaemonClient {
        DaemonClient::new()
    }

    /// `GET /api/v1/displays` (and `/api/v1/fans` for the source options).
    /// Runs off the UI thread; the result is applied on the next render.
    fn load(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let displays = client.get("/api/v1/displays");
            let fans = client.get("/api/v1/fans");
            (displays, fans)
        });
        cx.spawn(async move |view, cx| {
            let (displays, fans) = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.retrying = false;
                if let Ok(fans) = fans {
                    if let Ok(parsed) = serde_json::from_str::<FansStatus>(&fans) {
                        this.fans = Some(parsed);
                    }
                }
                match displays {
                    Ok(body) => {
                        let rows: Vec<DisplayStatus> = serde_json::from_str(&body).unwrap_or_default();
                        if rows.is_empty() {
                            this.state = DisplaysState::Empty;
                        } else {
                            this.state = DisplaysState::Ready;
                        }
                        this.rows = rows;
                        this.stale_error = None;
                        this.sync_cards();
                    }
                    Err(crate::client::ClientError::Daemon { status: 404, .. }) => {
                        this.state = DisplaysState::Unsupported;
                        this.rows.clear();
                    }
                    Err(e) => {
                        if this.rows.is_empty() {
                            this.state = DisplaysState::Error(e.to_string());
                        } else {
                            this.stale_error = Some(e.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The draft starts from the daemon's config and follows it when the
    /// saved config changes (React remount rule).
    fn sync_cards(&mut self) {
        let rows: Vec<DisplayStatus> = self.rows.clone();
        for row in &rows {
            let key = serde_json::to_string(&row.config).unwrap_or_default();
            let saved_key = key.clone();
            self.cards.entry(row.info.id.to_string()).or_insert_with(|| CardState {
                draft: row.config.clone(),
                saved_key,
                busy: false,
                error: None,
                select_open: false,
            });
            if let Some(card) = self.cards.get_mut(&row.info.id.to_string()) {
                if card.saved_key != key {
                    card.draft = row.config.clone();
                    card.saved_key = key;
                    card.error = None;
                }
            }
        }
        let ids: Vec<String> = rows.iter().map(|r| r.info.id.to_string()).collect();
        self.cards.retain(|id, _| ids.contains(id));
    }

    fn save(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(card) = self.cards.get(id) else { return };
        let draft = card.draft.clone();
        if let Some(card) = self.cards.get_mut(id) {
            card.busy = true;
            card.error = None;
        }
        let id_owned = id.to_string();
        let bg_id = id_owned.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let body = serde_json::to_string(&draft).unwrap_or_default();
            client.put_json(&format!("/api/v1/displays/{bg_id}"), &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                if let Some(card) = this.cards.get_mut(&id_owned) {
                    card.busy = false;
                }
                match result {
                    Ok(_) => this.load(cx),
                    Err(e) => {
                        if let Some(card) = this.cards.get_mut(&id_owned) {
                            card.error = Some(e.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn stop(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(card) = self.cards.get_mut(id) {
            card.busy = true;
            card.error = None;
        }
        let id_owned = id.to_string();
        let bg_id = id_owned.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.delete(&format!("/api/v1/displays/{bg_id}"))
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                if let Some(card) = this.cards.get_mut(&id_owned) {
                    card.busy = false;
                }
                match result {
                    Ok(_) => this.load(cx),
                    Err(e) => {
                        if let Some(card) = this.cards.get_mut(&id_owned) {
                            card.error = Some(e.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn connected_count(rows: &[DisplayStatus]) -> usize {
        rows.iter().filter(|r| r.info.connected).count()
    }

    fn card(&self, row: &DisplayStatus, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let card = self.cards.get(&row.info.id.to_string());
        let draft = card.map(|c| c.draft.clone()).unwrap_or_else(|| row.config.clone());
        let busy = card.map(|c| c.busy).unwrap_or(false);
        let error = card.and_then(|c| c.error.clone());
        let select_open = card.map(|c| c.select_open).unwrap_or(false);
        let saved_key = serde_json::to_string(&row.config).unwrap_or_default();
        let draft_key = serde_json::to_string(&draft).unwrap_or_default();
        let driven = row.shown.is_some();
        let dirty = draft_key != saved_key || !driven;
        let rpm = is_rpm(&draft.source);
        let temperature = !rpm;
        let preview = preview_value(
            &draft.source,
            draft.unit,
            self.fans.as_ref(),
            row.shown.as_ref().and_then(|s| s.value),
        );
        let blocked_by = row.info.blocked_by.clone();

        let options: Vec<(String, String)> = {
            let mut opts = vec![
                ("cpu_temp".to_string(), "CPU temperature".to_string()),
                ("gpu_temp".to_string(), "GPU temperature".to_string()),
            ];
            if let Some(fans) = &self.fans {
                for temp in fans
                    .temps
                    .iter()
                    .filter(|t| matches!(t.source, canvas_proto::fans::TempSource::Board(_)))
                {
                    let name = match &temp.source {
                        canvas_proto::fans::TempSource::Board(name) => name.clone(),
                        _ => String::new(),
                    };
                    opts.push((format!("board:{name}"), format!("{} (board sensor)", temp.label)));
                }
                for header in &fans.headers {
                    opts.push((
                        format!("fan:{}", header.id),
                        format!("{} (fan RPM)", header.label),
                    ));
                }
            }
            opts.push(("fixed".into(), "A fixed number".into()));
            opts.push(("api".into(), "Set by API".into()));
            opts
        };
        let option_refs: Vec<(&str, &str)> = options
            .iter()
            .map(|(v, l)| (v.as_str(), l.as_str()))
            .collect();

        let id: String = row.info.id.to_string();
        let view_entity = cx.entity();

        // Save / Stop in the card head.
        let save_entity = view_entity.clone();
        let save_id = id.clone();
        let save_btn = button(
            t,
            &format!("{id}-save"),
            "Save",
            Variant::Primary,
            !dirty || busy,
            move |_, _, cx| {
                save_entity.update(cx, |this, cx| this.save(&save_id, cx));
            },
        );
        let stop_entity = view_entity.clone();
        let stop_id = id.clone();
        let stop_btn = button(
            t,
            &format!("{id}-stop"),
            "Stop",
            Variant::Ghost,
            busy,
            move |_, _, cx| {
                stop_entity.update(cx, |this, cx| this.stop(&stop_id, cx));
            },
        );
        let dot_words = if let Some(by) = &blocked_by {
            format!("waiting for {by}")
        } else if row.info.connected {
            "connected".to_string()
        } else {
            "not connected".to_string()
        };
        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(state_dot(
                t,
                if blocked_by.is_some() { "warn" } else if row.info.connected { "ok" } else { "down" },
                &dot_words,
            ))
            .child(save_btn)
            .child(stop_btn);
        let actions = if dirty {
            actions.child(
                div()
                    .text_sm()
                    .font(crate::fonts::font_ui())
                    .text_color(Theme::color(t.mute))
                    .child("Unsaved"),
            )
        } else {
            actions
        };

        // The digit readout.
        let (cells, dot, fits) = display_digits(preview, row.info.digits, draft.decimals, temperature);
        let mark = if temperature {
            if draft.unit == DisplayUnit::Celsius { "°C" } else { "°F" }
        } else {
            "RPM"
        };
        let readout = if preview.is_none() {
            div()
                .flex()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .font(crate::fonts::font_ui())
                        .text_color(Theme::color(t.mute))
                        .child("No reading yet"),
                )
        } else if !fits {
            div()
                .flex()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .font(crate::fonts::font_ui())
                        .text_color(Theme::color(t.mute))
                        .child("The number does not fit"),
                )
        } else {
            let mut row_cells = div()
                .flex()
                .items_baseline()
                .gap_1()
                .justify_center();
            for (i, cell) in cells.iter().enumerate() {
                row_cells = row_cells.child(
                    div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::BOLD)
                        .font(crate::fonts::font_mono())
                        .text_color(Theme::color(t.ink))
                        .child(cell.unwrap_or(' ').to_string()),
                );
                if dot && i == cells.len().saturating_sub(2) {
                    row_cells = row_cells.child(
                        div()
                            .size(px(4.0))
                            .rounded_full()
                            .bg(Theme::color(t.ink)),
                    );
                }
            }
            row_cells.child(
                div()
                    .text_base()
                    .font(crate::fonts::font_mono())
                    .text_color(Theme::color(t.mute))
                    .child(mark.to_string()),
            )
        };

        // The "Now" line.
        let now = match &row.shown {
            Some(shown) => {
                let value = match shown.value {
                    Some(v) => format!("{v}"),
                    None => "—".to_string(),
                };
                let unit = if shown.mode == canvas_proto::display::DisplayMode::Temperature {
                    if shown.unit == DisplayUnit::Celsius { "°C" } else { "°F" }
                } else {
                    "RPM"
                };
                let blink = if shown.blink { " · blinking" } else { "" };
                format!("Now {value} {unit}{blink}")
            }
            None => "Now not driven".to_string(),
        };
        let note = format!(
            "{} digits · {} decimal{}",
            row.info.digits,
            row.info.max_decimals,
            if row.info.max_decimals == 1 { "" } else { "s" }
        );

        // The controls: source, fixed value, unit, decimals, blink.
        let sel_entity = view_entity.clone();
        let sel_id = id.clone();
        let source_select = select(
            t,
            &format!("{id}-source"),
            &format!("{} source", row.info.name),
            &option_refs,
            &source_key(&draft.source),
            select_open,
            {
                let entity = sel_entity.clone();
                let id = sel_id.clone();
                move |_, cx| {
                    entity
                        .update(cx, |this, cx| {
                            if let Some(card) = this.cards.get_mut(&id) {
                                card.select_open = !card.select_open;
                            }
                            cx.notify();
                        });
                }
            },
            {
                let entity = view_entity.clone();
                let id = id.clone();
                move |key, _, cx| {
                    entity
                        .update(cx, |this, cx| {
                            if let Some(card) = this.cards.get_mut(&id) {
                                let source = source_from_key(&key, &card.draft.source);
                                let decimals = if is_rpm(&source) { 0 } else { card.draft.decimals };
                                card.draft.source = source;
                                card.draft.decimals = decimals;
                                card.select_open = false;
                                card.error = None;
                            }
                            cx.notify();
                        });
                }
            },
        );

        let controls_head = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(field(t, &format!("{id}-source-field"), "Source", source_select));

        let controls = if matches!(draft.source, DisplaySource::Fixed { .. }) {
            let fixed_value = match draft.source {
                DisplaySource::Fixed { value } => value,
                _ => 0.0,
            };
            let entity = view_entity.clone();
            let id = id.clone();
            controls_head.child(field(
                t,
                &format!("{id}-value-field"),
                "Value",
                number_field(
                    t,
                    &format!("{id}-fixed"),
                    fixed_value,
                    0.0,
                    9999.0,
                    1.0,
                    None,
                    move |v, _, cx| {
                        entity
                            .update(cx, |this, cx| {
                                if let Some(card) = this.cards.get_mut(&id) {
                                    card.draft.source = DisplaySource::Fixed { value: v };
                                    card.error = None;
                                }
                                cx.notify();
                            });
                    },
                ),
            ))
        } else {
            controls_head
        };

        let unit_entity = view_entity.clone();
        let unit_id = id.clone();
        let unit = field(
            t,
            &format!("{id}-unit-field"),
            "Unit",
            segmented(
                t,
                &format!("{id}-unit"),
                &[("celsius", "°C"), ("fahrenheit", "°F")],
                if draft.unit == DisplayUnit::Celsius { "celsius" } else { "fahrenheit" },
                move |v, _, cx| {
                    unit_entity
                        .update(cx, |this, cx| {
                            if let Some(card) = this.cards.get_mut(&unit_id) {
                                card.draft.unit = if v == "celsius" {
                                    DisplayUnit::Celsius
                                } else {
                                    DisplayUnit::Fahrenheit
                                };
                                card.error = None;
                            }
                            cx.notify();
                        });
                },
            ),
        );

        let dec_entity = view_entity.clone();
        let dec_id = id.clone();
        let decimals = field(
            t,
            &format!("{id}-decimals-field"),
            "Decimals",
            segmented(
                t,
                &format!("{id}-decimals"),
                &[("0", "0"), ("1", "1")],
                if draft.decimals == 0 { "0" } else { "1" },
                move |v, _, cx| {
                    dec_entity
                        .update(cx, |this, cx| {
                            if let Some(card) = this.cards.get_mut(&dec_id) {
                                card.draft.decimals = v.parse().unwrap_or(0);
                                card.error = None;
                            }
                            cx.notify();
                        });
                },
            ),
        );

        let blink_entity = view_entity.clone();
        let blink_id = id.clone();
        let blink_on = draft.blink_above.is_some();
        let blink = field(
            t,
            &format!("{id}-blink-field"),
            "Blink at",
            toggle(t, &format!("{id}-blink"), blink_on, move |on, _, cx| {
                blink_entity
                    .update(cx, |this, cx| {
                        if let Some(card) = this.cards.get_mut(&blink_id) {
                            card.draft.blink_above = if on { Some(60.0) } else { None };
                            card.error = None;
                        }
                        cx.notify();
                    });
            }),
        );
        let controls = controls.child(unit).child(decimals).child(blink);
        let controls = if blink_on {
            let entity = view_entity.clone();
            let id = id.clone();
            let threshold = draft.blink_above.unwrap_or(60.0);
            controls.child(
                div()
                    .w(px(160.0))
                    .child(number_field(
                        t,
                        &format!("{id}-threshold"),
                        threshold,
                        0.0,
                        9999.0,
                        1.0,
                        Some("°"),
                        move |v, _, cx| {
                            entity
                                .update(cx, |this, cx| {
                                    if let Some(card) = this.cards.get_mut(&id) {
                                        card.draft.blink_above = Some(v);
                                        card.error = None;
                                    }
                                    cx.notify();
                                });
                        },
                    )),
            )
        } else {
            controls
        };
        let controls = match error {
            Some(message) => controls.child(
                div()
                    .text_sm()
                    .font(crate::fonts::font_ui())
                    .text_color(Theme::color(t.down))
                    .child(message),
            ),
            None => controls,
        };

        panel(
            t,
            &format!("display-{id}"),
            &row.info.name,
            actions,
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(if blocked_by.is_some() {
                    // The slot keeps the notice's height in every state (the
                    // notice is the full-width warn box when blocked).
                    let by = blocked_by.clone().unwrap();
                    div()
                        .flex()
                        .gap_2()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(Theme::color(t.warn))
                        .bg(Theme::color(t.warn_wash))
                        .text_sm()
                        .font(crate::fonts::font_ui())
                        .text_color(Theme::color(t.ink))
                        .child(
                            gpui::svg()
                                .path("icons/WarningOctagon.svg")
                                .size_4()
                                .text_color(Theme::color(t.warn)),
                        )
                        .child(format!(
                            "{by} is running, so canvas-rgb leaves the display alone until it closes."
                        ))
                } else {
                    div().w_full().min_h(px(32.0))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .font(crate::fonts::font_ui())
                                        .text_color(Theme::color(t.mute))
                                        .child("Preview"),
                                )
                                .child(readout)
                                .child(
                                    div()
                                        .text_sm()
                                        .font(crate::fonts::font_ui())
                                        .text_color(Theme::color(t.dim))
                                        .child(now),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font(crate::fonts::font_mono())
                                        .text_color(Theme::color(t.mute))
                                        .child(note),
                                ),
                        )
                )
                .child(controls),
        )
    }
}

impl Render for DisplaysView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let title = div()
            .flex()
            .items_baseline()
            .gap_3()
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font(crate::fonts::font_ui())
                    .text_color(Theme::color(t.ink))
                    .child("Displays"),
            )
            .child(
                div()
                    .text_sm()
                    .font(crate::fonts::font_ui())
                    .text_color(Theme::color(t.mute))
                    .child(format!("{} connected", Self::connected_count(&self.rows))),
            );
        let mut page = div()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .size_full()
            .bg(Theme::color(t.g1))
            .child(title);
        if let Some(stale) = &self.stale_error {
            page = page.child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(Theme::color(t.warn))
                    .bg(Theme::color(t.warn_wash))
                    .text_sm()
                    .font(crate::fonts::font_ui())
                    .text_color(Theme::color(t.ink))
                    .child("The displays may be out of date.")
                    .child(
                        div()
                            .text_sm()
                            .font(crate::fonts::font_mono())
                            .text_color(Theme::color(t.mute))
                            .child(stale.clone()),
                    ),
            );
        }
        match &self.state {
            DisplaysState::Loading => {
                page = page.child(
                    div()
                        .p_4()
                        .rounded_lg()
                        .border_1()
                        .border_color(Theme::color(t.line))
                        .bg(Theme::color(t.panel))
                        .child(
                            div()
                                .text_sm()
                                .font(crate::fonts::font_ui())
                                .text_color(Theme::color(t.mute))
                                .child("Reading the displays"),
                        ),
                );
            }
            DisplaysState::Empty => {
                page = page.child(empty_state(
                    t,
                    "displays-empty",
                    "ThermometerSimple",
                    "No display has a source yet",
                    "Nothing is sent to a display until it has one.",
                    None::<gpui::Div>,
                ));
            }
            DisplaysState::Unsupported => {
                let entity = cx.entity();
                page = page.child(empty_state(
                    t,
                    "displays-unsupported",
                    "ArrowClockwise",
                    "canvas-rgb needs an update for displays",
                    "The service running is older than this page; update canvas-rgb to set a display up.",
                    Some(button(
                        t,
                        "displays-check-again",
                        "Check again",
                        Variant::Default,
                        false,
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| this.load(cx));
                        },
                    )),
                ));
            }
            DisplaysState::Error(message) => {
                let entity = cx.entity();
                let retry = button(
                    t,
                    "displays-try-again",
                    "Try again",
                    Variant::Primary,
                    self.retrying,
                    move |_, _, cx| {
                        entity
                            .update(cx, |this, cx| {
                                this.retrying = true;
                                this.state = DisplaysState::Loading;
                                cx.notify();
                                this.load(cx);
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
                        .rounded_lg()
                        .border_1()
                        .border_color(Theme::color(t.line))
                        .bg(Theme::color(t.panel))
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .font(crate::fonts::font_ui())
                                .text_color(Theme::color(t.ink))
                                .child("The displays could not be read"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font(crate::fonts::font_ui())
                                .text_color(Theme::color(t.dim))
                                .child("canvas-rgb could not read the displays. The lights are unaffected."),
                        )
                        .child(retry)
                        .child(
                            div()
                                .text_sm()
                                .font(crate::fonts::font_mono())
                                .text_color(Theme::color(t.mute))
                                .child(message.clone()),
                        ),
                );
            }
            DisplaysState::Ready => {
                let rows = self.rows.clone();
                let mut grid = div().flex().flex_col().gap_4();
                for row in &rows {
                    grid = grid.child(self.card(row, cx));
                }
                page = page.child(grid);
            }
        }
        page
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_keys_round_trip() {
        let cases = [
            (DisplaySource::CpuTemp, "cpu_temp"),
            (DisplaySource::GpuTemp, "gpu_temp"),
            (DisplaySource::BoardTemp { sensor: "temp0".into() }, "board:temp0"),
            (DisplaySource::FanRpm { header: "cpu_fan".into() }, "fan:cpu_fan"),
            (DisplaySource::Api, "api"),
        ];
        for (source, key) in &cases {
            assert_eq!(source_key(source), *key);
            assert_eq!(source_from_key(key, &DisplaySource::Api), *source);
        }
        let fixed = DisplaySource::Fixed { value: 42.0 };
        assert_eq!(source_key(&fixed), "fixed");
        assert_eq!(source_from_key("fixed", &fixed), fixed);
        // Switching to fixed from another source starts at 0.
        assert_eq!(
            source_from_key("fixed", &DisplaySource::Api),
            DisplaySource::Fixed { value: 0.0 }
        );
        // An unknown key reads as api.
        assert_eq!(source_from_key("mystery", &fixed), DisplaySource::Api);
    }

    #[test]
    fn display_digits_match_the_adapter_writing() {
        // 46.9°C at 4 digits, 1 decimal, temperature.
        let (cells, dot, fits) = display_digits(Some(46.9), 4, 1, true);
        assert!(fits);
        assert!(dot);
        assert_eq!(cells.len(), 4);
        // It does not fit: negative, and too large.
        assert!(!display_digits(Some(-1.0), 4, 1, true).2);
        assert!(!display_digits(Some(10000.0), 4, 1, true).2);
        // 9999.4 at one decimal scales to 99994, above 10^4 - 1: it does not fit.
        assert!(!display_digits(Some(9999.4), 4, 1, true).2);
        assert!(display_digits(Some(999.9), 4, 1, true).2);
        // No value blanks the cells.
        assert_eq!(display_digits(None, 4, 1, true).0, Vec::<Option<char>>::new());
        // RPM: no dot slot.
        assert!(!display_digits(Some(1200.0), 4, 0, false).1);
        // 0 decimals in temperature mode still has the dot slot.
        assert!(display_digits(Some(46.0), 4, 0, true).1);
        // A leading zero is blank.
        let (cells, _, _) = display_digits(Some(9.0), 4, 1, true);
        assert_eq!(cells[0], None);
    }

    #[test]
    fn connected_count_includes_blocked_displays() {
        use canvas_proto::device::DeviceId;
        use canvas_proto::display::{DisplayInfo, DisplayMode};
        let info = |connected: bool, blocked: Option<&str>| DisplayInfo {
            id: DeviceId("a".into()),
            name: "A".into(),
            digits: 4,
            max_decimals: 1,
            modes: vec![DisplayMode::Temperature],
            connected,
            blocked_by: blocked.map(|s| s.into()),
        };
        let config = |id: &str| DisplayConfig {
            device: DeviceId(id.into()),
            source: DisplaySource::CpuTemp,
            unit: DisplayUnit::Celsius,
            decimals: 1,
            blink_above: None,
        };
        let rows = vec![
            DisplayStatus { info: info(true, None), config: config("a"), shown: None },
            DisplayStatus { info: info(true, Some("ZEUS CAST")), config: config("b"), shown: None },
            DisplayStatus { info: info(false, None), config: config("c"), shown: None },
        ];
        assert_eq!(DisplaysView::connected_count(&rows), 2);
    }

    #[test]
    fn preview_value_converts_to_fahrenheit_and_reads_fixed() {
        let fixed = DisplaySource::Fixed { value: 46.9 };
        assert_eq!(
            preview_value(&fixed, DisplayUnit::Fahrenheit, None, None),
            Some(46.9 * 9.0 / 5.0 + 32.0)
        );
        assert_eq!(preview_value(&fixed, DisplayUnit::Celsius, None, None), Some(46.9));
        // API source with no value yet is blank.
        assert_eq!(preview_value(&DisplaySource::Api, DisplayUnit::Celsius, None, None), None);
    }
}
