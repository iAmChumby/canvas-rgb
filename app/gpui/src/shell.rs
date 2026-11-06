//! The shell, after `app/web/src/shell/` (`Shell.tsx`, `Rail.tsx`,
//! `RailDeck.tsx`, `StatusStrip.tsx`, `PageHeader.tsx`, `GlobalBanners.tsx`,
//! `DaemonDown`).
//!
//! Layout facts the port follows (from `docs/gpui-parity/shell.md` and
//! `app/web/src/styles/app.css`):
//! - full rail ≥ 1280 px at `--rail-w` (200 px), icon rail 721–1279 px at
//!   `--rail-w-icon` (64 px), a fixed 65 px bottom dock at ≤ 720 px with only
//!   the active link's name drawn;
//! - the strip across the top of the main column (brand on the phone, the
//!   page title and its mono context, now-playing below 1280 px, the mark,
//!   Pause lights and search) — there is no second header bar;
//! - the rail carries the wordmark and the mono version, one row per page
//!   with its Phosphor icon and its mono value, the deck above Settings,
//!   and the engine line at the bottom;
//! - the page ground is the dotted grid (`body` in `app.css`) with the 2 px
//!   spectrum line along the top edge (`.app::before`), red while offline;
//! - the daemon-down page replaces the content when the daemon has not
//!   answered twice in a row; the content is empty until the first answer.
//!
//! The command palette, details popover and update banners are not drawn in
//! this slice: they stay unticked in the checklist.

use canvas_proto::api::{DevicesResponse, EffectsResponse, LayoutsResponse, RenderState, Status};
use canvas_proto::fans::FansStatus;
use canvas_proto::peripheral::PeripheralsResponse;
use gpui::{Context, IntoElement, Window, canvas, div, prelude::*, px};
use gpui::{Bounds, point, size};

use crate::client::DaemonClient;
use crate::components::button::{Variant, button_icon, icon_button};
use crate::components::daemon_down::daemon_down;
use crate::components::slider::slider;
use crate::theme::Theme;
use crate::views::displays::DisplaysView;

/// The pages the rail names, in order (React `Rail.tsx` PAGES, then Settings).
pub const NAV: &[(&str, &str, &str)] = &[
    ("Layout", "layout", "SquaresFour"),
    ("Effects", "effects", "Sparkle"),
    ("Devices", "devices", "Circuitry"),
    ("Displays", "displays", "Monitor"),
    ("Peripherals", "peripherals", "GameController"),
    ("Fans", "fans", "Fan"),
    ("Settings", "settings", "Gear"),
];

pub fn is_valid_page(page: &str) -> bool {
    canvas_proto::tauri::EDITOR_PAGES.contains(&page) || page == "displays" || page == "peripherals"
}

/// The rail's per-page values, after `RailData.ts` `useRailData`.
#[derive(Debug, Clone, Default)]
pub struct RailValues {
    pub layout: Option<String>,
    pub effect: Option<String>,
    pub paused: bool,
    pub devices: Option<(usize, usize)>,
    pub displays: Option<usize>,
    pub peripherals: Option<usize>,
    pub fans: Option<String>,
}

/// The values the rail draws, from the daemon answers the shell polls.
pub fn rail_values(
    status: Option<&Status>,
    effects: Option<&EffectsResponse>,
    devices: Option<&DevicesResponse>,
    displays_count: Option<usize>,
    layouts: Option<&LayoutsResponse>,
    fans: Option<&FansStatus>,
    peripherals: Option<usize>,
) -> RailValues {
    let layout_id = status.and_then(|s| s.active_layout.clone());
    let layout = layout_id.as_ref().map(|id| {
        layouts
            .and_then(|l| l.layouts.iter().find(|x| &x.id == id))
            .map(|l| l.name.clone())
            .unwrap_or_else(|| id.clone())
    });
    let effect_id = status.and_then(|s| s.active_effect.clone());
    let effect = effect_id.as_ref().map(|id| {
        effects
            .and_then(|e| e.effects.iter().find(|x| &x.id == id))
            .map(|e| e.name.clone())
            .unwrap_or_else(|| id.clone())
    });
    let device_counts = devices.map(|d| (d.devices.iter().filter(|x| x.connected).count(), d.devices.len()));
    let fans_word = fans.and_then(|f| {
        if f.headers.is_empty() {
            None
        } else if f.headers.iter().all(|h| h.on_bios) {
            Some("BIOS".to_string())
        } else {
            Some(f.active_profile.clone())
        }
    });
    RailValues {
        layout,
        effect,
        paused: status.is_some_and(|s| s.power == canvas_proto::api::Power::Paused),
        devices: device_counts,
        displays: displays_count,
        peripherals,
        fans: fans_word,
    }
}

/// What is playing, in words (React `lib/status.ts` `playing`).
pub fn playing(status: Option<&Status>, effects: Option<&EffectsResponse>) -> (String, String, bool) {
    let Some(status) = status else {
        return ("—".into(), String::new(), false);
    };
    if status.active_layout.is_none() {
        return ("No layout yet".into(), String::new(), false);
    }
    let effect_id = status.active_effect.clone();
    let name = match &effect_id {
        Some(id) => effects
            .and_then(|e| e.effects.iter().find(|x| &x.id == id))
            .map(|e| e.name.clone())
            .unwrap_or_else(|| id.clone()),
        None => String::new(),
    };
    if name.is_empty() {
        return ("Nothing is running".into(), String::new(), false);
    }
    if status.power == canvas_proto::api::Power::Paused {
        return (name, "paused".into(), false);
    }
    if status.parked {
        return (name, "idle".into(), false);
    }
    let n = status.devices_connected;
    (name, format!("on {n} {}", if n == 1 { "device" } else { "devices" }), true)
}

/// The status words the rail and strip draw from `Status` (React `lib/status.ts`
/// `problems`, worst first; the empty list is "All connected").
pub fn strip_state(status: Option<&Status>, error: Option<&str>) -> (String, String, Option<String>) {
    // Returns (full text, tone, phone short text).
    if error.is_some() && status.is_none() {
        return ("Daemon offline".into(), "down".into(), Some("Daemon".into()));
    }
    if let Some(status) = status {
        if status.openrgb == canvas_proto::api::LinkState::Down {
            return ("OpenRGB down".into(), "down".into(), Some("OpenRGB".into()));
        }
        if status.service == canvas_proto::api::LinkState::Down {
            return ("Fan control off".into(), "down".into(), Some("Fans".into()));
        }
        if status.power == canvas_proto::api::Power::Running
            && !status.parked
            && status.fps_measured < status.fps_target.saturating_sub(1) as f32
        {
            return ("Running slow".into(), "warn".into(), None);
        }
        if status.openrgb == canvas_proto::api::LinkState::Connecting {
            return ("OpenRGB connecting".into(), "work".into(), Some("OpenRGB".into()));
        }
        if status.service == canvas_proto::api::LinkState::Connecting {
            return ("Fan service connecting".into(), "work".into(), None);
        }
        ("All connected".into(), "ok".into(), None)
    } else {
        ("Daemon connecting".into(), "work".into(), Some("Daemon".into()))
    }
}

pub struct Shell {
    pub theme: Theme,
    pub page: String,
    pub status: Option<Status>,
    pub status_error: Option<String>,
    pub status_failures: u32,
    pub displays: gpui::Entity<DisplaysView>,
    pub width: f32,
    // Rail data, polled with the status (React store slow tick + rail poll).
    pub devices: Option<DevicesResponse>,
    pub displays_count: Option<usize>,
    pub displays_connected: usize,
    pub layouts: Option<LayoutsResponse>,
    pub effects: Option<EffectsResponse>,
    pub fans: Option<FansStatus>,
    pub peripherals: Option<usize>,
    pub render: Option<RenderState>,
    pub deck_lights: bool,
}

impl Shell {
    pub fn new(
        theme: Theme,
        page: String,
        status: Option<Status>,
        status_error: Option<String>,
        displays: gpui::Entity<DisplaysView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let failures = if status_error.is_some() { 2 } else { 0 };
        let mut shell = Shell {
            theme,
            page,
            status,
            status_error,
            status_failures: failures,
            displays,
            width: 1280.0,
            devices: None,
            displays_count: None,
            displays_connected: 0,
            layouts: None,
            effects: None,
            fans: None,
            peripherals: None,
            render: None,
            deck_lights: true,
        };
        shell.poll_all(cx);
        shell
    }

    #[allow(dead_code)]
    pub fn navigate(&mut self, page: &str) {
        if is_valid_page(page) {
            self.page = page.to_string();
        }
    }

    /// One poll of every rail/strip answer (status fast, the rest slow in
    /// React; one fetch here keeps the capture honest — the value shown is
    /// the daemon's, not a constant).
    fn poll_all(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            let status = client.get("/api/v1/status");
            let devices = client.get("/api/v1/devices");
            let displays = client.get("/api/v1/displays");
            let layouts = client.get("/api/v1/layouts");
            let effects = client.get("/api/v1/effects");
            let fans = client.get("/api/v1/fans");
            let peripherals = client.get("/api/v1/peripherals");
            let render = client.get("/api/v1/render");
            (status, devices, displays, layouts, effects, fans, peripherals, render)
        });
        cx.spawn(async move |view, cx| {
            let (status, devices, displays, layouts, effects, fans, peripherals, render) = bg.await;
            let _ = view.update(cx, |this, cx| {
                match status {
                    Ok(body) => {
                        if let Ok(parsed) = serde_json::from_str::<Status>(&body) {
                            this.status = Some(parsed);
                            this.status_error = None;
                            this.status_failures = 0;
                        }
                    }
                    Err(e) => {
                        this.status_error = Some(e.to_string());
                        this.status_failures += 1;
                        if this.status_failures >= 2 {
                            this.status = None;
                        }
                    }
                }
                if let Ok(body) = devices {
                    if let Ok(parsed) = serde_json::from_str::<DevicesResponse>(&body) {
                        this.devices = Some(parsed);
                    }
                }
                if let Ok(body) = displays {
                    if let Ok(rows) = serde_json::from_str::<Vec<canvas_proto::api::DisplayStatus>>(&body) {
                        this.displays_connected = rows.iter().filter(|r| r.info.connected).count();
                        this.displays_count = Some(rows.len());
                    }
                }
                if let Ok(body) = layouts {
                    if let Ok(parsed) = serde_json::from_str::<LayoutsResponse>(&body) {
                        this.layouts = Some(parsed);
                    }
                }
                if let Ok(body) = effects {
                    if let Ok(parsed) = serde_json::from_str::<EffectsResponse>(&body) {
                        this.effects = Some(parsed);
                    }
                }
                if let Ok(body) = fans {
                    if let Ok(parsed) = serde_json::from_str::<FansStatus>(&body) {
                        this.fans = Some(parsed);
                    }
                }
                if let Ok(body) = peripherals {
                    if let Ok(parsed) = serde_json::from_str::<PeripheralsResponse>(&body) {
                        this.peripherals = Some(parsed.peripherals.len());
                    }
                }
                if let Ok(body) = render {
                    if let Ok(parsed) = serde_json::from_str::<RenderState>(&body) {
                        this.render = Some(parsed);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn patch_output(&mut self, body: String, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.patch_json("/api/v1/output", &body)
        });
        cx.spawn(async move |view, cx| {
            let _ = bg.await;
            let _ = view.update(cx, |this, cx| {
                // Optimistic power flip, then the daemon's own answer.
                this.poll_all(cx);
            });
        })
        .detach();
    }

    fn toggle_power(&mut self, cx: &mut Context<Self>) {
        let paused = self.status.as_ref().is_some_and(|s| s.power == canvas_proto::api::Power::Paused);
        let body = if paused {
            r#"{"power":"running","brightness":null}"#.to_string()
        } else {
            r#"{"power":"paused","brightness":null}"#.to_string()
        };
        if let Some(status) = &mut self.status {
            status.power = if paused {
                canvas_proto::api::Power::Running
            } else {
                canvas_proto::api::Power::Paused
            };
        }
        cx.notify();
        self.patch_output(body, cx);
    }

    fn step_effect(&mut self, dir: i32, cx: &mut Context<Self>) {
        let list = self.effects.as_ref().map(|e| e.effects.clone()).unwrap_or_default();
        if list.is_empty() {
            return;
        }
        let current = self.status.as_ref().and_then(|s| s.active_effect.clone());
        let at = current
            .as_ref()
            .and_then(|id| list.iter().position(|e| &e.id == id))
            .unwrap_or(0);
        let len = list.len() as i32;
        let next = list[((at as i32 + dir + len) % len) as usize].clone();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.post_json(
                &format!("/api/v1/effects/{}/activate", next.id),
                r#"{"props":{},"preset":null}"#,
            )
        });
        cx.spawn(async move |view, cx| {
            let _ = bg.await;
            let _ = view.update(cx, |this, cx| this.poll_all(cx));
        })
        .detach();
    }

    /// The two consecutive failures that take the status away (the
    /// daemon-down page reaches when it stays cleared).
    pub fn daemon_down(&self) -> bool {
        self.status.is_none() && self.status_failures >= 2 && self.status_error.is_some()
    }

    pub fn answered(&self) -> bool {
        self.status.is_some() || self.status_error.is_some()
    }

    fn values(&self) -> RailValues {
        rail_values(
            self.status.as_ref(),
            self.effects.as_ref(),
            self.devices.as_ref(),
            self.displays_count,
            self.layouts.as_ref(),
            self.fans.as_ref(),
            self.peripherals,
        )
    }

    fn easel(&self, _px_size: f32) -> gpui::Svg {
        let name = if self.theme.is_dark() { "EaselDark" } else { "EaselLight" };
        gpui::svg()
            .path(format!("icons/{name}.svg"))
            .size_3()
            .text_color(Theme::color(self.theme.ink))
    }

    fn wordmark(&self) -> impl IntoElement {
        let t = &self.theme;
        div()
            .flex()
            .items_center()
            .text_base()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.ink))
            .child("c")
            .child(self.easel(10.0))
            .child("nvas")
            .child(
                div()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(t.live())
                    .child("rgb"),
            )
    }

    fn rail_value(&self, page: &str, values: &RailValues) -> Option<gpui::Div> {
        let t = &self.theme;
        let mono = div()
            .text_sm()
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.mute));
        match page {
            "layout" => values.layout.clone().map(|v| mono.child(v)),
            "effects" => values.effect.clone().map(|v| {
                // The effect name is set in the UI face, with its LED.
                let led = if values.paused {
                    Theme::color(t.dot)
                } else {
                    t.live()
                };
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(div().size(px(5.0)).rounded_full().bg(led))
                    .child(v)
            }),
            "devices" => values.devices.map(|(connected, total)| {
                div()
                    .flex()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(
                        div()
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.ok))
                            .child(connected.to_string()),
                    )
                    .child(format!("/{total}"))
            }),
            "displays" => values.displays.map(|v| mono.child(v.to_string())),
            "peripherals" => values.peripherals.map(|v| mono.child(v.to_string())),
            "fans" => values.fans.clone().map(|v| mono.child(v)),
            _ => None,
        }
    }

    fn rail_link(&self, label: &str, page: &str, icon: &str, active: bool, value: Option<gpui::Div>, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = &self.theme;
        let glyph_name = if active { format!("icons/{icon}-fill.svg") } else { format!("icons/{icon}.svg") };
        let glyph_color = if active { t.live() } else { Theme::color(t.dim) };
        let row = div()
            .id(gpui::SharedString::from(format!("rail-{page}")))
            .relative()
            .flex()
            .items_center()
            .gap_3()
            .h(px(36.0))
            .px(px(10.0))
            .rounded_md()
            .text_sm()
            .font_weight(gpui::FontWeight::MEDIUM)
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.ink))
            .child(gpui::svg().path(glyph_name).size_4().text_color(glyph_color))
            .child(
                div()
                    .text_color(if active { Theme::color(t.ink) } else { Theme::color(t.dim) })
                    .child(label.to_string()),
            );
        let row = if active {
            row.bg(t.live_soft())
        } else {
            row
        };
        // The active marker: a 3 px live bar, 6 px left of the link's edge,
        // level with it (React measures it; this is the measured position).
        let row = if active {
            row.child(
                div()
                    .absolute()
                    .left(px(-6.0))
                    .top(px(8.0))
                    .w(px(3.0))
                    .h(px(20.0))
                    .rounded_full()
                    .bg(t.live()),
            )
        } else {
            row
        };
        let row = match value {
            Some(v) => row.child(div().flex_1()).child(v),
            None => row,
        };
        let entity = cx.entity();
        let target = page.to_string();
        row.cursor_pointer().on_click(move |_, _, cx| {
            entity.update(cx, |this, _| this.navigate(&target));
        })
    }

    fn deck(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let values = self.values();
        let paused = values.paused;
        let known = self.status.is_some();
        let brightness = self.status.as_ref().map(|s| s.brightness).unwrap_or(0);
        let devices = self.status.as_ref().map(|s| s.devices_connected);
        let active_name = values.effect.clone().unwrap_or_else(|| {
            if known { "Nothing is running".to_string() } else { "—".to_string() }
        });
        let has_effects = self.effects.as_ref().is_some_and(|e| !e.effects.is_empty());
        // Tabs: two flex-1 cells in the sunk well.
        let deck_entity = cx.entity();
        let tab = |label: &str, selected: bool, lights: bool, entity: gpui::Entity<Self>| {
            let cell = div()
                .id(gpui::SharedString::from(format!("deck-tab-{label}")))
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .h(px(26.0))
                .rounded_md()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .font_family(crate::fonts::UI_FAMILY)
                .child(div().size(px(5.0)).rounded_full().bg(if selected { t.live() } else { Theme::color(t.dot) }))
                .child(label.to_string());
            let cell = if selected {
                cell.bg(Theme::color(t.raised)).text_color(Theme::color(t.ink))
            } else {
                cell.text_color(Theme::color(t.dim))
            };
            cell.cursor_pointer().on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.deck_lights = lights;
                    cx.notify();
                });
            })
        };
        let tabs = div()
            .flex()
            .gap(px(2.0))
            .p(px(2.0))
            .rounded_md()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.sunk))
            .child(tab("Lights", self.deck_lights, true, deck_entity.clone()))
            .child(tab("System", !self.deck_lights, false, deck_entity.clone()));
        let body = if self.deck_lights {
            // Preview strip: twelve cells of the running picture; idle they
            // are the dark frame (React paints the thumbnail/live frame).
            let mut strip = div()
                .flex()
                .gap(px(3.0))
                .h(px(48.0))
                .p(px(6.0))
                .rounded_md()
                .bg(Theme::color(t.sunk));
            for _ in 0..12 {
                strip = strip.child(div().flex_1().rounded_sm().bg(gpui::black()));
            }
            let strip = if paused { strip.opacity(0.25) } else { strip };
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(strip)
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_base()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child(active_name),
                        )
                        .child(match devices {
                            Some(n) => div()
                                .text_sm()
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child(div().flex().child(div().font_family(crate::fonts::MONO_FAMILY).child(n.to_string())).child(" devices")),
                            None => div(),
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(icon_button(
                            t,
                            "deck-prev",
                            "SkipBack",
                            "",
                            false,
                            true,
                            !has_effects,
                            {
                                let entity = deck_entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.step_effect(-1, cx));
                                }
                            },
                        ))
                        .child(
                            div()
                                .id(gpui::SharedString::from("deck-pause"))
                                .flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap(px(6.0))
                                .h(px(32.0))
                                .rounded_md()
                                .border_1()
                                .border_color(Theme::color(t.line_2))
                                .bg(Theme::color(t.raised))
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child(div().size(px(5.0)).rounded_full().bg(if paused { Theme::color(t.warn) } else { Theme::color(t.dot) }))
                                .child(if paused { "Resume".to_string() } else { "Pause".to_string() })
                                .cursor_pointer()
                                .on_click({
                                    let entity = deck_entity.clone();
                                    move |_, _, cx| {
                                        entity.update(cx, |this, cx| this.toggle_power(cx));
                                    }
                                }),
                        )
                        .child(icon_button(
                            t,
                            "deck-next",
                            "SkipForward",
                            "",
                            false,
                            true,
                            !has_effects,
                            {
                                let entity = deck_entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.step_effect(1, cx));
                                }
                            },
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pt(px(2.0))
                        .child(
                            gpui::svg()
                                .path("icons/Sun.svg")
                                .size_4()
                                .text_color(Theme::color(t.mute)),
                        )
                        .child(slider(t, "deck-brightness", brightness, !known, |_, _, _| {}))
                        .child(
                            div()
                                .w(px(38.0))
                                .text_sm()
                                .font_family(crate::fonts::MONO_FAMILY)
                                .text_color(Theme::color(t.dim))
                                .child(if known { format!("{brightness}%") } else { "—".to_string() }),
                        ),
                )
        } else {
            // System: CPU/GPU/Fans/Render meters, ten segments each.
            let cpu = self.fans.as_ref().and_then(|f| {
                f.temps.iter().find(|x| x.source == canvas_proto::fans::TempSource::Cpu).and_then(|x| x.celsius)
            });
            let gpu = self.fans.as_ref().and_then(|f| {
                f.temps.iter().find(|x| x.source == canvas_proto::fans::TempSource::Gpu).and_then(|x| x.celsius)
            });
            let rpm = self.fans.as_ref().and_then(|f| {
                f.headers.iter().filter_map(|h| h.rpm).max()
            });
            let fps = self.status.as_ref().map(|s| s.fps_measured.round() as i32);
            let target = self.render.as_ref().map(|r| r.fps as i32).unwrap_or(60);
            let meters: [(&str, Option<f32>, &str, f32, &str); 4] = [
                ("CPU", cpu, "°C", 100.0, "a3"),
                ("GPU", gpu, "°C", 100.0, "a3"),
                ("Fans", rpm.map(|r| r as f32), "rpm", 2000.0, "a2"),
                ("Render", fps.map(|f| f as f32), "fps", target as f32, "live"),
            ];
            let mut grid = div().flex().flex_wrap().gap_3().pt_1();
            for (label, value, unit, scale, tone) in meters {
                let tone_color = match tone {
                    "a3" => Theme::color(t.a3),
                    "a2" => Theme::color(t.a2),
                    _ => t.live(),
                };
                let filled = match value {
                    None => 0,
                    Some(v) => ((v / scale * 10.0).round().clamp(0.0, 10.0)) as usize,
                };
                let mut bar = div().flex().gap(px(2.0));
                for i in 0..10 {
                    bar = bar.child(div().flex_1().h(px(4.0)).rounded_sm().bg(if i < filled { tone_color } else { Theme::color(t.line) }));
                }
                grid = grid.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .w(px(70.0))
                        .child(div().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.mute)).child(label.to_string()))
                        .child(
                            div()
                                .flex()
                                .items_baseline()
                                .font_family(crate::fonts::MONO_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child(match value {
                                    Some(v) => format!("{v:.0}"),
                                    None => "—".to_string(),
                                })
                                .child(div().ml(px(3.0)).text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.mute)).child(unit.to_string())),
                        )
                        .child(bar),
                );
            }
            grid
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .mb_2()
            .px(px(10.0))
            .pt_2()
            .pb_3()
            .rounded_lg()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.g1))
            .child(tabs)
            .child(body)
    }

    fn rail(&self, narrow: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let values = self.values();
        let engine_word = if self.status.is_some() {
            "engine"
        } else if self.status_error.is_some() {
            "offline"
        } else {
            "connecting"
        };
        let engine_color = if self.status.is_some() {
            Theme::color(t.ok)
        } else if self.status_error.is_some() {
            Theme::color(t.down)
        } else {
            Theme::color(t.work)
        };
        let fps = match &self.status {
            Some(s) => format!("{} fps", s.fps_measured.round()),
            None => "—".to_string(),
        };
        let mut col = div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .h_full()
            .p_3()
            .bg(Theme::color(t.panel));
        if !narrow {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .h(px(40.0))
                    .px(px(10.0))
                    .mb_2()
                    .child(self.wordmark())
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child(
                                self.status
                                    .as_ref()
                                    .map(|s| s.version.clone())
                                    .unwrap_or_else(|| "—".to_string()),
                            ),
                    ),
            );
            for (label, page, icon) in &NAV[..6] {
                let value = self.rail_value(page, &values);
                col = col.child(self.rail_link(label, page, icon, self.page == *page, value, cx));
            }
            col = col.child(div().flex_1());
            col = col.child(self.deck(cx));
            col = col.child(self.rail_link("Settings", "settings", "Gear", self.page == "settings", None, cx));
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(28.0))
                    .px(px(10.0))
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(div().size(px(5.0)).rounded_full().bg(engine_color))
                    .child(
                        div()
                            .flex_1()
                            .text_color(Theme::color(t.mute))
                            .child(engine_word.to_string()),
                    )
                    .child(div().child(fps)),
            );
        } else {
            // The icon rail (721–1279 px): the easel alone, glyphs only.
            col = col.items_center().child(
                div()
                    .flex()
                    .justify_center()
                    .w(px(40.0))
                    .h(px(40.0))
                    .mb_2()
                    .child(self.easel(16.0)),
            );
            for (label, page, icon) in &NAV[..6] {
                let _ = label;
                let active = self.page == *page;
                let glyph_name = if active { format!("icons/{icon}-fill.svg") } else { format!("icons/{icon}.svg") };
                let tile = div()
                    .id(gpui::SharedString::from(format!("rail-{page}")))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(40.0))
                    .rounded_md()
                    .child(gpui::svg().path(glyph_name).size_4().text_color(if active { t.live() } else { Theme::color(t.dim) }));
                let tile = if active {
                    tile.bg(t.live_soft()).child(
                        div()
                            .absolute()
                            .left(px(-6.0))
                            .top(px(10.0))
                            .w(px(3.0))
                            .h(px(20.0))
                            .rounded_full()
                            .bg(t.live()),
                    )
                } else {
                    tile
                };
                col = col.child(tile);
            }
            col = col.child(div().flex_1());
            // Settings sits at the bottom, apart from the page list.
            let active = self.page == "settings";
            let glyph_name = if active { "icons/Gear-fill.svg" } else { "icons/Gear.svg" };
            let tile = div()
                .id(gpui::SharedString::from("rail-settings"))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .size(px(40.0))
                .rounded_md()
                .child(gpui::svg().path(glyph_name).size_4().text_color(if active { t.live() } else { Theme::color(t.dim) }));
            let tile = if active {
                tile.bg(t.live_soft()).child(
                    div()
                        .absolute()
                        .left(px(-6.0))
                        .top(px(10.0))
                        .w(px(3.0))
                        .h(px(20.0))
                        .rounded_full()
                        .bg(t.live()),
                )
            } else {
                tile
            };
            col = col.child(tile);
        }
        col
    }

    fn dock(&self) -> impl IntoElement {
        let t = &self.theme;
        let mut row = div()
            .flex()
            .items_center()
            .justify_around()
            .h(px(65.0))
            .px_2()
            .py_1()
            .border_t_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel));
        for (label, page, icon) in NAV {
            let active = self.page == *page;
            let glyph_name = if active { format!("icons/{icon}-fill.svg") } else { format!("icons/{icon}.svg") };
            let cell = div()
                .id(gpui::SharedString::from(format!("dock-{page}")))
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .justify_end()
                .flex_1()
                .h(px(56.0))
                .py_1()
                .px_1()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child(gpui::svg().path(glyph_name).size_4().text_color(if active { t.live() } else { Theme::color(t.dim) }));
            let cell = if active {
                cell
                    .bg(t.live_soft())
                    .rounded_md()
                    .px_2()
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(label.to_string()),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(-5.0))
                            .left_0()
                            .right_0()
                            .h(px(3.0))
                            .rounded_full()
                            .bg(t.live()),
                    )
            } else {
                cell
            };
            row = row.child(cell);
        }
        row
    }

    fn strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let phone = self.width <= 720.0;
        let (mark, tone, _short) = strip_state(self.status.as_ref(), self.status_error.as_deref());
        let tone_color = match tone.as_str() {
            "ok" => Theme::color(t.ok),
            "warn" => Theme::color(t.warn),
            "work" => Theme::color(t.work),
            _ => Theme::color(t.down),
        };
        let paused = self.status.as_ref().is_some_and(|s| s.power == canvas_proto::api::Power::Paused);
        let known = self.status.is_some();
        let mark_icon = match tone.as_str() {
            "down" => "icons/WarningOctagon-fill.svg",
            "warn" => "icons/Warning-fill.svg",
            "work" => "icons/CircleDashed.svg",
            _ => "icons/CheckCircle-fill.svg",
        };
        let (now_name, now_detail, _now_count) = playing(self.status.as_ref(), self.effects.as_ref());
        // Page title and context (React PageTitle in the strip).
        let title_text = match self.page.as_str() {
            "layout" => "Layout",
            "effects" => "Effects",
            "devices" => "Devices",
            "displays" => "Displays",
            "peripherals" => "Peripherals",
            "fans" => "Fans",
            "settings" => "Settings",
            "updates" => "Updates",
            _ => "Layout",
        };
        let context_text = if self.page == "displays" {
            format!("{} connected", self.displays_connected)
        } else {
            String::new()
        };
        let mut inner = div()
            .flex()
            .items_center()
            .gap(if phone { px(8.0) } else { px(16.0) })
            .h(if phone { px(48.0) } else { px(54.0) })
            .px(if phone { px(8.0) } else { px(24.0) })
            .border_b_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel));
        if phone {
            inner = inner.child(self.easel(16.0));
        }
        inner = inner.child(
            div()
                .flex()
                .items_baseline()
                .gap_3()
                .mr_2()
                .child(
                    div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.ink))
                        .child(title_text.to_string()),
                ),
        );
        if !context_text.is_empty() && !phone {
            inner = inner.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(context_text),
            );
        }
        // Now playing: hidden at 1280 px and up (the deck carries it there).
        if self.width < 1280.0 {
            let swatch = div()
                .size(px(12.0))
                .rounded_md()
                .bg(t.live());
            let swatch = if paused { swatch.opacity(0.4) } else { swatch };
            inner = inner.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(swatch)
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .font_family(crate::fonts::UI_FAMILY)
                                    .text_color(Theme::color(t.ink))
                                    .child(now_name.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_family(crate::fonts::UI_FAMILY)
                                    .text_color(Theme::color(t.mute))
                                    .child(now_detail.clone()),
                            ),
                    ),
            );
        }
        // The mark: icon always, words at 1280 px and up.
        let mark_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(gpui::svg().path(mark_icon).size_4().text_color(tone_color));
        let mark_row = if self.width >= 1280.0 {
            mark_row.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(mark),
            )
        } else {
            mark_row
        };
        inner = inner.child(mark_row).child(div().flex_1());
        // Controls at the right: Pause lights, search.
        if known {
            if phone {
                let entity = cx.entity();
                inner = inner.child(icon_button(
                    t,
                    "strip-power",
                    if paused { "Play" } else { "Pause" },
                    "fill",
                    true,
                    true,
                    false,
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| this.toggle_power(cx));
                    },
                ));
            } else {
                let entity = cx.entity();
                inner = inner.child(button_icon(
                    t,
                    "strip-power",
                    if paused { "Resume lights" } else { "Pause lights" },
                    if paused { Variant::Primary } else { Variant::Default },
                    false,
                    true,
                    if paused { "Play" } else { "Pause" },
                    "fill",
                    14.0,
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| this.toggle_power(cx));
                    },
                ));
            }
        }
        inner = inner.child(icon_button(
            t,
            "strip-search",
            "MagnifyingGlass",
            "",
            true,
            false,
            false,
            |_, _, _| {},
        ));
        inner
    }

    fn page_content(&self) -> gpui::AnyElement {
        let t = &self.theme;
        if !self.answered() {
            // Nothing is drawn until the first status answer (React Shell).
            return div().size_full().into_any_element();
        }
        if self.daemon_down() {
            let displays = self.displays.clone();
            return div()
                .size_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .p_8()
                        .child(daemon_down(
                            t,
                            {
                                let displays = displays.clone();
                                move |_, _, cx| {
                                    displays.update(cx, |_, _| {});
                                }
                            },
                            "not tried yet",
                        )),
                )
                .into_any_element();
        }
        if self.page == "displays" {
            return self.displays.clone().into_any_element();
        }
        // Every other page shows the placeholder until it is ported.
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .child(
                div()
                    .text_base()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child("Not ported yet"),
            )
            .into_any_element()
    }

    fn spectrum_line(&self) -> impl IntoElement {
        let t = &self.theme;
        let offline = self.status.is_none() && self.status_error.is_some();
        if offline {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(2.0))
                .bg(Theme::color(t.down))
        } else {
            // The spectrum (`--spectrum`): the six RGB stops across the top.
            // GPUI gradients take two stops, so the sweep is six 2 px bands,
            // which reads as the same sweep at this height.
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .flex()
                .h(px(2.0))
                .child(div().flex_1().bg(Theme::color(t.rgb_1)))
                .child(div().flex_1().bg(Theme::color(t.rgb_2)))
                .child(div().flex_1().bg(Theme::color(t.rgb_3)))
                .child(div().flex_1().bg(Theme::color(t.rgb_4)))
                .child(div().flex_1().bg(Theme::color(t.rgb_5)))
                .child(div().flex_1().bg(Theme::color(t.rgb_6)))
        }
    }

    fn background(&self) -> impl IntoElement {
        let t = &self.theme;
        let dot = Theme::color(t.dot);
        div()
            .absolute()
            .inset_0()
            .child(
                canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        let w = bounds.size.width;
                        let h = bounds.size.height;
                        let mut x = px(11.0);
                        while x < w {
                            let mut y = px(11.0);
                            while y < h {
                                window.paint_quad(gpui::fill(
                                    Bounds { origin: point(x, y), size: size(px(2.0), px(2.0)) },
                                    dot,
                                ));
                                y += px(22.0);
                            }
                            x += px(22.0);
                        }
                    },
                )
                .size_full(),
            )
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.width = f32::from(window.window_bounds().get_bounds().size.width);
        let t = &self.theme;
        let phone = self.width <= 720.0;
        let phone_pad_top = px(16.0);
        let content = div()
            .flex_1()
            .flex_col()
            .p_4()
            .child(self.page_content());
        let content = if phone {
            content.px(px(16.0)).pb(px(120.0)).pt(phone_pad_top)
        } else {
            content.px(px(24.0)).pb(px(24.0)).pt(px(16.0))
        };
        let main_col = div()
            .flex()
            .flex_col()
            .flex_1()
            .child(self.strip(cx))
            .child(content);
        let body = if phone {
            div()
                .flex()
                .flex_col()
                .size_full()
                .child(main_col)
                .child(self.dock())
        } else if self.width < 1280.0 {
            div()
                .flex()
                .size_full()
                .child(div().w(px(64.0)).h_full().child(self.rail(true, cx)))
                .child(main_col)
        } else {
            div()
                .flex()
                .size_full()
                .child(div().w(px(200.0)).h_full().child(self.rail(false, cx)))
                .child(main_col)
        };
        div()
            .relative()
            .size_full()
            .bg(Theme::color(t.g1))
            .child(self.background())
            .child(body)
            .child(self.spectrum_line())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::api::{LinkState, Power};

    fn status() -> Status {
        Status {
            version: "0.8.4".into(),
            power: Power::Running,
            brightness: 100,
            fps_target: 60,
            fps_measured: 59.6,
            frame_time_ms: 16.7,
            frames_rendered: 100,
            parked: false,
            devices_connected: 2,
            devices_total: 2,
            preview_subscribers: 0,
            active_layout: Some("desk".into()),
            active_effect: None,
            openrgb: LinkState::Connected,
            service: LinkState::Connected,
            service_down_ms: None,
            conflicts: Vec::new(),
            show_zone_panel: false,
            onboarding_complete: true,
        }
    }

    #[test]
    fn strip_says_all_connected_when_nothing_is_wrong() {
        assert_eq!(strip_state(Some(&status()), None).1, "ok");
        assert_eq!(strip_state(Some(&status()), None).0, "All connected");
    }

    #[test]
    fn strip_names_each_link_on_its_own() {
        let mut s = status();
        s.openrgb = LinkState::Down;
        assert_eq!(strip_state(Some(&s), None).0, "OpenRGB down");
        s.openrgb = LinkState::Connecting;
        assert_eq!(strip_state(Some(&s), None).0, "OpenRGB connecting");
        s.openrgb = LinkState::Connected;
        s.service = LinkState::Down;
        assert_eq!(strip_state(Some(&s), None).0, "Fan control off");
        s.service = LinkState::Connecting;
        assert_eq!(strip_state(Some(&s), None).0, "Fan service connecting");
    }

    #[test]
    fn strip_warns_when_running_slow() {
        let mut s = status();
        s.fps_measured = 50.0;
        assert_eq!(strip_state(Some(&s), None).1, "warn");
    }

    #[test]
    fn strip_says_connecting_then_offline() {
        assert_eq!(strip_state(None, None).1, "work");
        assert_eq!(strip_state(None, Some("x")).1, "down");
        assert_eq!(strip_state(None, Some("x")).0, "Daemon offline");
    }

    #[test]
    fn rail_displays_count_comes_from_the_daemon_list() {
        // React RailData: displays is the daemon list's length, null while
        // unknown — never a constant. (The old GPUI rail drew 0 always.)
        let v = rail_values(None, None, None, Some(1), None, None, None);
        assert_eq!(v.displays, Some(1));
        let v = rail_values(None, None, None, None, None, None, None);
        assert_eq!(v.displays, None);
    }

    #[test]
    fn page_names_are_the_navigate_pages_plus_displays() {
        assert!(is_valid_page("layout"));
        assert!(is_valid_page("displays"));
        assert!(is_valid_page("peripherals"));
        assert!(!is_valid_page("nope"));
    }
}
