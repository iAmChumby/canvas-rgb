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
use crate::components::button::{Variant, button, button_icon, button_icon_wide, icon_button};
use crate::components::daemon_down::daemon_down;
use crate::components::slider::slider;
use crate::theme::{Theme, TokenRadius};
use crate::views::command_palette::{
    CommandPaletteView, PaletteAction, build_commands, build_short, is_palette_toggle_key,
};
use crate::views::devices::DevicesView;
use crate::views::displays::DisplaysView;
use crate::views::effects::EffectsView;
use crate::views::fans::FansView;
use crate::views::layout::LayoutView;
use crate::views::peripherals::PeripheralsView;
use crate::views::settings::{SettingsView, ThemeCell};
use crate::views::updates::UpdatesView;

/// Measure a single line of text in the UI/mono face at `size` px, using
/// GPUI's own text layout so the truncation below matches what is drawn.
pub(crate) fn text_width(window: &Window, text: &str, size: f32, weight: gpui::FontWeight, family: &str) -> f32 {
    let font = gpui::Font {
        family: family.to_string().into(),
        weight,
        style: gpui::FontStyle::Normal,
        features: gpui::FontFeatures::default(),
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
    let layout = window.text_system().layout_line(text, px(size), &[run], None);
    layout.width.into()
}

/// Ellipsis-truncate `text` to `max_w` px (React `text-overflow:
/// ellipsis`). GPUI's `truncate()` hard-cuts without drawing the ellipsis
/// in this version, so the cut is computed here from measured widths: the
/// longest prefix that fits with a "…" appended. Returns the text unchanged
/// when it already fits.
pub(crate) fn ellipsize(
    window: &Window,
    text: &str,
    max_w: f32,
    size: f32,
    weight: gpui::FontWeight,
    family: &str,
) -> String {
    if text_width(window, text, size, weight, family) <= max_w {
        return text.to_string();
    }
    let ellipsis = "…";
    let chars: Vec<char> = text.chars().collect();
    let mut best = 0usize;
    for n in 0..chars.len() {
        let candidate: String = chars[..n].iter().collect::<String>() + ellipsis;
        if text_width(window, &candidate, size, weight, family) <= max_w {
            best = n;
        } else {
            break;
        }
    }
    chars[..best].iter().collect::<String>() + ellipsis
}

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
    /// The Devices page (#209).
    pub devices_view: gpui::Entity<DevicesView>,
    /// The Fans page (#204). The shell builds it, so `main.rs` stays as it is.
    pub fans_view: gpui::Entity<FansView>,
    /// The Settings page (#205).
    pub settings_view: gpui::Entity<SettingsView>,
    /// The Updates page (#206).
    pub updates_view: gpui::Entity<UpdatesView>,
    /// The Peripherals page (#208).
    pub peripherals_view: gpui::Entity<PeripheralsView>,
    /// The Effects page (#210).
    pub effects_view: gpui::Entity<EffectsView>,
    /// The Layout page (#213).
    pub layout_view: gpui::Entity<LayoutView>,
    /// The theme the shell and the settings view share (the Appearance
    /// radio writes it; every render reads it).
    pub theme_cell: ThemeCell,
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
    /// The command palette (#211). The shell owns whether it is open; the
    /// palette's rows run through `run_palette_action` and then close.
    pub palette: gpui::Entity<CommandPaletteView>,
    pub palette_open: bool,
    /// The query `--palette-query` opens with (capture flag only).
    pub palette_query: String,
    /// The status details popover anchor (`Shell.tsx` `detailsOpen`); the
    /// popover itself lives with the strip slice, so this flag only records
    /// the palette action for now.
    pub details_open: bool,
}

impl Shell {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        theme: Theme,
        page: String,
        status: Option<Status>,
        status_error: Option<String>,
        displays: gpui::Entity<DisplaysView>,
        devices_view: gpui::Entity<DevicesView>,
        cx: &mut Context<Self>,
        tray_present: bool,
        tray_rx: Option<std::sync::mpsc::Receiver<canvas_proto::tauri::TrayToEditor>>,
    ) -> Self {
        let failures = if status_error.is_some() { 2 } else { 0 };
        let fans_view = cx.new(|cx| FansView::new(theme, cx));
        let theme_cell: ThemeCell = std::sync::Arc::new(std::sync::Mutex::new(theme));
        let shell_entity = cx.entity();
        let updates_entity = Some(shell_entity.clone());
        let settings_visible = page == "settings";
        let settings_cell = theme_cell.clone();
        let settings_view = cx.new(|cx| {
            SettingsView::new(
                theme,
                settings_cell,
                Some(shell_entity.clone()),
                settings_visible,
                cx,
                tray_present,
                tray_rx,
            )
        });
        let updates_view = cx.new(|cx| {
            UpdatesView::new(theme, cx, tray_present, updates_entity)
        });
        let peripherals_view = cx.new(|cx| PeripheralsView::new(theme, cx));
        let effects_view = cx.new(|cx| EffectsView::new(theme, cx));
        let layout_view = cx.new(|cx| LayoutView::new(theme, cx));
        let palette = cx.new(|cx| CommandPaletteView::new(theme, cx));
        // The palette's callbacks run through the shell: rows execute an
        // action and then close it (`Shell.tsx` `togglePower`/`runEffect`).
        let palette_shell = shell_entity.clone();
        let palette_close_shell = shell_entity.clone();
        palette.update(cx, |p, cx| {
            p.connect(cx);
            p.set_on_run(std::rc::Rc::new(
                move |action: &PaletteAction, _window, cx| {
                    let action = action.clone();
                    palette_shell.update(cx, |this, cx| {
                        this.run_palette_action(&action, cx);
                    });
                },
            ));
            p.set_on_close(std::rc::Rc::new(move |_window, cx| {
                palette_close_shell.update(cx, |this, cx| {
                    this.close_palette(cx);
                });
            }));
        });
        effects_view.update(cx, |view, cx| {
            view.connect_search(cx);
            // The search box's chrome (placeholder, text mode).
            let search = view.search_input.clone();
            search.update(cx, |input, cx| input.set_text_mode(Some("Search effects"), cx));
        });
        let mut shell = Shell {
            theme,
            page,
            status,
            status_error,
            status_failures: failures,
            displays,
            devices_view,
            fans_view,
            updates_view,
            peripherals_view,
            effects_view,
            layout_view,
            settings_view,
            theme_cell,
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
            palette,
            palette_open: false,
            palette_query: String::new(),
            details_open: false,
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

    /// After a navigate: tell the settings view whether it is on screen
    /// (its audio test lease runs only there).
    fn sync_settings_visible(&self, cx: &mut Context<Self>) {
        let visible = self.page == "settings";
        self.settings_view.update(cx, |s, _| {
            s.visible = visible;
        });
    }

    /// One poll of every rail/strip answer (status fast, the rest slow in
    /// React; one fetch here keeps the capture honest — the value shown is
    /// the daemon's, not a constant).
    fn poll_all(&mut self, cx: &mut Context<Self>) {
        // Every answer is asked for on its own, so the strip and the rail do
        // not wait for the slowest of them.
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.get("/api/v1/status")
        });
        cx.spawn(async move |view, cx| {
            let status = bg.await;
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
                cx.notify();
            });
        })
        .detach();
        Self::poll_json::<DevicesResponse>(cx, "/api/v1/devices", |this, parsed| {
            this.devices = Some(parsed);
        });
        Self::poll_json::<Vec<canvas_proto::api::DisplayStatus>>(
            cx,
            "/api/v1/displays",
            |this, rows| {
                this.displays_connected = rows.iter().filter(|r| r.info.connected).count();
                this.displays_count = Some(rows.len());
            },
        );
        Self::poll_json::<LayoutsResponse>(cx, "/api/v1/layouts", |this, parsed| {
            this.layouts = Some(parsed);
        });
        Self::poll_json::<EffectsResponse>(cx, "/api/v1/effects", |this, parsed| {
            this.effects = Some(parsed);
        });
        Self::poll_json::<FansStatus>(cx, "/api/v1/fans", |this, parsed| {
            this.fans = Some(parsed);
        });
        Self::poll_json::<PeripheralsResponse>(cx, "/api/v1/peripherals", |this, parsed| {
            this.peripherals = Some(parsed.peripherals.len());
        });
        Self::poll_json::<RenderState>(cx, "/api/v1/render", |this, parsed| {
            this.render = Some(parsed);
        });
    }

    /// One answer, asked for on its own and put in the shell when it lands.
    fn poll_json<T: serde::de::DeserializeOwned + 'static>(
        cx: &mut Context<Self>,
        path: &'static str,
        apply: impl Fn(&mut Self, T) + 'static,
    ) {
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.get(path).ok()
        });
        cx.spawn(async move |view, cx| {
            let body = bg.await;
            let _ = view.update(cx, |this, cx| {
                if let Some(body) = body {
                    if let Ok(value) = serde_json::from_str::<T>(&body) {
                        apply(this, value);
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
        self.activate_effect(&next.id, cx);
    }

    /// Run the effect `id` (`Shell.tsx` `runEffect`): POST
    /// `/api/v1/effects/{id}/activate` with `{"props":{},"preset":null}`, then
    /// refresh everything. The palette closes at once, before the request
    /// finishes (React's quirk); a failure has no toast in the GPUI shell
    /// yet (there is no NotifyHost in this slice), like the power PATCH.
    fn activate_effect(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.post_json(
                &format!("/api/v1/effects/{id}/activate"),
                r#"{"props":{},"preset":null}"#,
            )
        });
        cx.spawn(async move |view, cx| {
            let _ = bg.await;
            let _ = view.update(cx, |this, cx| this.poll_all(cx));
        })
        .detach();
    }

    /// Open the palette (Ctrl+K, the strip's search button): the search field
    /// is focused 20 ms later, as in React, starting with an empty query.
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.palette
            .update(cx, |p, cx| p.open("", window, cx));
        cx.notify();
    }

    /// Open at start with `query` typed (the capture flag `--palette-query`):
    /// the input is focused on the first frame.
    pub fn open_palette_initial(&mut self, query: &str, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.palette_query = query.to_string();
        self.palette.update(cx, |p, cx| {
            p.open_initial(query, cx);
        });
        cx.notify();
    }

    pub fn close_palette(&mut self, cx: &mut Context<Self>) {
        self.palette_open = false;
        cx.notify();
    }

    /// Run one palette row (`Shell.tsx` commands). The palette closes after
    /// this returns (React runs the row, then closes).
    pub fn run_palette_action(&mut self, action: &PaletteAction, cx: &mut Context<Self>) {
        match action {
            PaletteAction::None => {}
            PaletteAction::OpenDetails => {
                self.details_open = true;
            }
            PaletteAction::Navigate(page) => {
                self.navigate(page);
                self.sync_settings_visible(cx);
            }
            PaletteAction::Theme => {
                self.settings_view
                    .update(cx, |s, cx| s.palette_next_theme(cx));
            }
            PaletteAction::Motion => {
                self.settings_view
                    .update(cx, |s, cx| s.palette_toggle_motion(cx));
            }
            PaletteAction::Power => {
                // React's `togglePower` returns early when status is null:
                // no request, and the palette still closes.
                if self.status.is_some() {
                    self.toggle_power(cx);
                }
            }
            PaletteAction::RunEffect(id) => {
                self.activate_effect(id, cx);
            }
        }
    }

    /// Ctrl+K / Meta+K (`Shell.tsx:97-107`): toggles the palette.
    fn on_shell_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        if is_palette_toggle_key(key, mods.control, mods.platform) {
            if self.palette_open {
                self.close_palette(cx);
            } else {
                self.open_palette(window, cx);
            }
        }
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

    /// The brand easel (`BrandMark.tsx`, `.easel-*`): legs in ink, the
    /// canvas in `--live` (accent blue), red (`--down`) while the daemon is
    /// away. The SVG assets are split in two so each part takes its own
    /// colour; `size` is the width in px (React `BrandMark` default 16).
    fn easel(&self, size: f32) -> impl IntoElement {
        let t = &self.theme;
        let canvas_color = if self.status.is_some() {
            t.live()
        } else if self.status_error.is_some() {
            Theme::color(t.down)
        } else {
            t.live()
        };
        div()
            .relative()
            .w(px(size))
            .h(px(size * 14.0 / 12.0))
            .child(
                gpui::svg()
                    .path("icons/EaselLegs.svg")
                    .absolute()
                    .inset_0()
                    .w_full()
                    .h_full()
                    .text_color(Theme::color(t.ink)),
            )
            .child(
                gpui::svg()
                    .path("icons/EaselCanvas.svg")
                    .absolute()
                    .inset_0()
                    .w_full()
                    .h_full()
                    .text_color(canvas_color),
            )
    }

    fn wordmark(&self) -> impl IntoElement {
        let t = &self.theme;
        // React `.wordmark` (15 px, 650, −0.01em) with the "a" as the easel
        // (0.62em wide ≈ 9.3 px). The −0.01em is −0.15px per character, below
        // a pixel and left out.
        div()
            .flex()
            .items_center()
            .text_size(px(15.0))
            .font_weight(gpui::FontWeight(650.0))
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.ink))
            .child("c")
            .child(self.easel(9.3))
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
            .r_md()
            .text_sm()
            .font_weight(gpui::FontWeight::MEDIUM)
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.ink))
            // React's svg is a replaced element with intrinsic 18 px, so it
            // never shrinks; GPUI's has none and flex-shrank to 13 px on the
            // one row whose value ("Default") overflows the row (#204 G8).
            .child(gpui::svg().path(glyph_name).size(px(18.0)).flex_none().text_color(glyph_color))
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
            // React `.rail-value`: `flex: 1 1 auto; min-width: 0; overflow: hidden;
            // text-align: right`. The value box takes the rest of the row and shrinks,
            // so a value never pushes past the row's padding (the Layout row's
            // "Default" sat 4 px right of the Fans row's).
            Some(v) => row.child(
                div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .justify_end()
                    .child(v),
            ),
            None => row,
        };
        let entity = cx.entity();
        let target = page.to_string();
        row.cursor_pointer().on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.navigate(&target);
                this.sync_settings_visible(cx);
            });
        })
    }

    fn deck(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let values = self.values();
        let paused = values.paused;
        let known = self.status.is_some();
        let brightness = self.status.as_ref().map(|s| s.brightness).unwrap_or(0);
        let devices = self.status.as_ref().map(|s| s.devices_connected);
        let active_name = values.effect.clone().unwrap_or_else(|| {
            if known { "Nothing is running".to_string() } else { "—".to_string() }
        });
        // The name's budget: the deck's inner width (rail 200 − rail
        // padding 24 − deck padding 20 − border 2 = 154) minus the row gap
        // and the device count. GPUI's `truncate()` hard-cuts, so the
        // ellipsis cut is measured here (React `.rail-deck-name`).
        let count_w = match devices {
            Some(n) => text_width(
                window,
                &format!("{n} devices"),
                14.0,
                gpui::FontWeight::NORMAL,
                crate::fonts::UI_FAMILY,
            ),
            None => 0.0,
        };
        // GPUI's measured widths run ~10% narrower than the browser's for
        // the same face (the browser cuts "Nothing is running" to
        // "Nothing i…", GPUI's own measure fits "Nothing is…"), so the
        // budget carries that factor to land on the same cut.
        let name_budget = (154.0 - 8.0 - count_w) * 0.9;
        let active_name = ellipsize(
            window,
            &active_name,
            name_budget,
            15.0,
            gpui::FontWeight::SEMIBOLD,
            crate::fonts::UI_FAMILY,
        );
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
                .r_sm()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .font_family(crate::fonts::UI_FAMILY)
                .child(div().size(px(5.0)).rounded_full().bg(if selected { t.live() } else { Theme::color(t.dot) }))
                .child(label.to_string());
            // React `.seg-thumb`: `box-shadow: inset 0 0 0 1px var(--line-2)`. Both cells carry
            // the 1 px border (transparent when unselected) so they stay the same size.
            let cell = cell.border_1();
            let cell = if selected {
                cell.border_color(Theme::color(t.line_2))
                    .bg(Theme::color(t.raised))
                    .text_color(Theme::color(t.ink))
            } else {
                cell.border_color(gpui::transparent_black()).text_color(Theme::color(t.dim))
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
            .r_md()
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
                .r_md()
                .bg(Theme::color(t.sunk));
            for _ in 0..12 {
                strip = strip.child(div().flex_1().r_sm().bg(gpui::black()));
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
                            // React `.rail-deck-name`: overflow hidden,
                            // text-overflow ellipsis, nowrap. GPUI's
                            // `truncate()` sets the ellipsis, but the flex
                            // item must be allowed to shrink (`min_w 0`) or
                            // it hard-cuts at the deck edge instead.
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(15.0))
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
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.ink))
                                // A plain `.btn` key: the hardware skirt (#204 G8-r3).
                                .child(crate::components::button::skirt_layers(
                                    t.skirt(),
                                    10.0,
                                    32.0,
                                    Theme::color(t.raised),
                                    Theme::color(t.line_2),
                                ))
                                .child(div().size(px(5.0)).rounded_full().bg(if paused { Theme::color(t.warn) } else { Theme::color(t.dot) }))
                                // React's label is a Swap of "Pause"/"Resume":
                                // the cell keeps "Resume"'s width and the
                                // visible word is centred in it, so the dot's
                                // place never moves (#204 G8, measured ~7 px).
                                .child(
                                    div()
                                        .w(px(text_width(window, "Resume", 14.0, gpui::FontWeight::MEDIUM, crate::fonts::UI_FAMILY)))
                                        .text_center()
                                        .child(if paused { "Resume".to_string() } else { "Pause".to_string() }),
                                )
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
                                .text_right()
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
                    // React `.rail-meter-seg{height:4px;border-radius:1px}`: a
                    // rounded rect, not the capsule r_sm(8) gave.
                    bar = bar.child(div().flex_1().h(px(4.0)).r_xs().bg(if i < filled { tone_color } else { Theme::color(t.line) }));
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
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.g1))
            .child(tabs)
            .child(body)
    }

    fn rail(&self, narrow: bool, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .text_right()
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
            col = col.child(self.deck(window, cx));
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
                    .items_center()
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
                // React `.rail-link` in the icon rail: 40 wide, 36 tall (the
                // base height), 38 px pitch with the 2 px gap.
                let tile = div()
                    .id(gpui::SharedString::from(format!("rail-{page}")))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(40.0))
                    .h(px(36.0))
                    .r_md()
                    .child(gpui::svg().path(glyph_name).size(px(18.0)).text_color(if active { t.live() } else { Theme::color(t.dim) }));
                let tile = if active {
                    tile.bg(t.live_soft()).child(
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
                    tile
                };
                col = col.child(tile);
            }
            // No spacer in the icon rail: React hides `.rail-spacer` at
            // 721–1279 px, so Settings sits directly under Fans (2 px gap),
            // not pinned to the bottom.
            let active = self.page == "settings";
            let glyph_name = if active { "icons/Gear-fill.svg" } else { "icons/Gear.svg" };
            let tile = div()
                .id(gpui::SharedString::from("rail-settings"))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .w(px(40.0))
                .h(px(36.0))
                .r_md()
                .child(gpui::svg().path(glyph_name).size(px(18.0)).text_color(if active { t.live() } else { Theme::color(t.dim) }));
            let tile = if active {
                tile.bg(t.live_soft()).child(
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
                tile
            };
            col = col.child(tile);
        }
        col
    }

    fn dock(&self) -> impl IntoElement {
        let t = &self.theme;
        // React `.rail` at ≤720: padding s1/s2 (4/8), links 56 tall with the
        // icon row (18) then the name row (16) at the bottom after a 20 px
        // top offset; the active tile covers the icon+name rows and its top
        // carries the 3 px marker bar.
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
            // React dock link: 56 tall; the quiet links flex-fill, the active
            // one is `flex: 0 0 auto` with `padding: s1 s2 s1` (4px/8px), so
            // the tile is the label's width + 16, not an equal share. The
            // rows (icon 18, name 16, gap 2) sit at the bottom with a 4 px
            // top padding: icon top = cell top + 16 (measured 800 at 390).
            // The tile hugs icon+name: its top is the marker's top, 5 px
            // above the icon (Rail.tsx `--myn: ir.top - 5`), so the tile
            // starts 11 px below the cell top; the marker spans the whole
            // tile (`--mw: ar.width`).
            let cell = div()
                .id(gpui::SharedString::from(format!("dock-{page}")))
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .justify_start()
                .h(px(56.0))
                .pt(px(16.0))
                .gap(px(2.0))
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink));
            let mut cell = if active {
                cell.flex_none().px(px(8.0))
            } else {
                cell.flex_1()
            };
            if active {
                // The active tile sits behind the icon and name (React
                // `::before` at z-index −1): its top is 56 − 18 − 2 − 16 =
                // 20 px from the link top (#204 G8-r3: at 11 it touched the
                // marker and measured 6 px high at 390).
                cell = cell
                    .child(
                        div()
                            .absolute()
                            .top(px(20.0))
                            .bottom(px(0.0))
                            .left_0()
                            .right_0()
                            .r_md()
                            .bg(t.live_soft()),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(11.0))
                            .left_0()
                            .right_0()
                            .h(px(3.0))
                            .rounded_full()
                            .bg(t.live()),
                    );
            }
            cell = cell.child(gpui::svg().path(glyph_name).size(px(18.0)).flex_none().text_color(if active { t.live() } else { Theme::color(t.dim) }));
            if active {
                cell = cell.child(
                    div()
                        .text_sm()
                        .line_height(px(16.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.ink))
                        .child(label.to_string()),
                );
            }
            row = row.child(cell);
        }
        row
    }

    fn strip(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let mut title_text = match self.page.as_str() {
            "layout" => "Layout",
            "effects" => "Effects",
            "devices" => "Devices",
            "displays" => "Displays",
            "peripherals" => "Peripherals",
            "fans" => "Fans",
            "settings" => "Settings",
            "updates" => "Updates",
            _ => "Layout",
        }
        .to_string();
        let mut context_text = if self.page == "displays" {
            format!("{} connected", self.displays_connected)
        } else if self.page == "updates" {
            self.updates_view
                .update(cx, |u, _| u.strip_context_text())
        } else if self.page == "devices" {
            self.devices_view
                .update(cx, |d, _| d.strip_title().1.unwrap_or_default())
        } else if self.page == "peripherals" {
            self.peripherals_view
                .update(cx, |u, _| u.strip_context_text())
        } else if self.page == "effects" {
            self.effects_view
                .update(cx, |u, _| u.strip_title().1.unwrap_or_default())
        } else if self.page == "layout" {
            // The layout's own name stands beside the page's name.
            self.layout_view
                .update(cx, |u, _| u.strip_title().1.unwrap_or_default())
        } else {
            String::new()
        };
        // The page's own actions (React PageActions), on the strip's right.
        let mut page_actions: Option<gpui::AnyElement> = None;
        if self.page == "fans" {
            let fans_view = self.fans_view.clone();
            let (title, context, actions) = fans_view.update(cx, |view, cx| {
                let (title, context) = view.strip_title();
                let actions = view.strip_actions(cx);
                (title, context, actions)
            });
            title_text = title;
            context_text = context.unwrap_or_default();
            page_actions = Some(actions);
        }
        if self.page == "effects" {
            let effects_view = self.effects_view.clone();
            let (title, context, actions) = effects_view.update(cx, |view, cx| {
                let (title, context) = view.strip_title();
                let actions = crate::views::effects::render::strip_actions(view, cx);
                (title, context, actions)
            });
            title_text = title;
            context_text = context.unwrap_or_default();
            page_actions = Some(actions);
        }
        if self.page == "layout" {
            // The Layout page's own actions: undo and redo, the zone count,
            // the save state, Revert, Save and "Map my PC again".
            let layout_view = self.layout_view.clone();
            let (title, context, actions) = layout_view.update(cx, |view, cx| {
                let (title, context) = view.strip_title();
                let actions = crate::views::layout::LayoutView::strip_actions(view, cx);
                (title, context, actions)
            });
            title_text = title;
            context_text = context.unwrap_or_default();
            page_actions = Some(actions);
        }
        // The peripherals page's actions (React `PageActions`): the pending
        // count, Revert and Apply, before Pause and Search. Empty unless
        // there are pending edits.
        if self.page == "peripherals" {
            let (pending, saving) = self
                .peripherals_view
                .update(cx, |v, _| (v.pending_count(), v.saving));
            if pending > 0 {
                let peripherals_entity = self.peripherals_view.clone();
                let revert_entity = peripherals_entity.clone();
                let apply_entity = peripherals_entity.clone();
                let word = if pending == 1 { "change" } else { "changes" };
                let pending_label = format!("{pending} {word} not sent");
                let mut actions = div().flex().items_center().gap_2();
                if !phone {
                    actions = actions.child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.warn))
                            .child(pending_label),
                    );
                }
                actions = actions.child(button_icon(
                    t,
                    "peri-revert",
                    if phone { "" } else { "Revert" },
                    Variant::Ghost,
                    saving,
                    true,
                    "ArrowCounterClockwise",
                    "",
                    16.0,
                    move |_, _, cx| {
                        revert_entity.update(cx, |v, cx| v.revert(cx));
                    },
                ));
                actions = actions.child(button(
                    t,
                    "peri-apply",
                    if saving { "Sending" } else { "Apply" },
                    Variant::Primary,
                    saving,
                    move |_, _, cx| {
                        apply_entity.update(cx, |v, cx| v.apply(cx));
                    },
                ));
                page_actions = Some(actions.into_any_element());
            }
        }
        // React: `.strip-inner{height:54px}` plus the `.strip` border-bottom
        // 1 px = 55 total, the border at y54-55 (GPUI drew the border inside
        // 54, at 53-54, shifting everything below 1 px up). The phone strip
        // is `.strip{height:48px}` with the border inside it, as drawn.
        let mut inner = div()
            .flex()
            .items_center()
            .gap(if phone { px(8.0) } else { px(16.0) })
            .h(if phone { px(48.0) } else { px(55.0) })
            .px(if phone { px(8.0) } else { px(24.0) })
            .border_b_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel));
        if phone {
            inner = inner.child(self.easel(16.0));
        }
        // React `.rd-title`: the h1 and the mono context share one flex row
        // with a 12 px gap (the context sits inside the title slot).
        // `.rd-title` carries `margin-right: var(--s2)` (8 px) so the title
        // never touches the mark (#204 G8-r3: without it the check icon sat
        // 10 px left at 1440). On a phone it is `flex: none` with no right
        // margin, so the title never shrinks under the row.
        let title_row = div()
            .flex()
            .items_baseline()
            .gap_3();
        let mut title_row = if phone {
            title_row.flex_none()
        } else {
            title_row.mr_2()
        };
        title_row = title_row.child(
            div()
                .text_xl()
                .line_height(gpui::relative(1.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child(title_text.to_string()),
        );
        if !context_text.is_empty() && !phone {
            // React's strip gives the room to a page that brings actions of
            // its own: below 1100 px the page's context line goes too
            // (`.strip-inner:has(.rd-actions) .rd-context`).
            let room_for_context = page_actions.is_none() || self.width >= 1100.0;
            if room_for_context {
                title_row = title_row.child(
                    div()
                        .text_sm()
                        .line_height(gpui::relative(1.0))
                        .font_family(crate::fonts::MONO_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(context_text),
                );
            }
        }
        inner = inner.child(title_row);
        // Now playing: hidden at 1280 px and up (the deck carries it there).
        if self.width < 1280.0 {
            let swatch = div()
                .size(px(12.0))
                .r_sm()
                .bg(t.live());
            let swatch = if paused { swatch.opacity(0.4) } else { swatch };
            // React's strip gives the room to a page that brings actions of
            // its own: the effect's name goes, and only its colour stays
            // (`.strip-inner:has(.rd-actions) .strip-now-text`). A phone's
            // strip keeps the name.
            let mut now_row = div().flex().items_center().gap_2().child(swatch);
            if page_actions.is_none() || phone {
                let now_name = if phone {
                    // React's phone strip truncates the now-playing name
                    // (`.strip-now` overflow): "Nothing…".
                    ellipsize(window, &now_name, 76.0, 14.0, gpui::FontWeight::SEMIBOLD, crate::fonts::UI_FAMILY)
                } else {
                    now_name.clone()
                };
                let mut detail_text = div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(now_name),
                    );
                if !phone {
                    detail_text = detail_text.child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child(now_detail.clone()),
                    );
                }
                now_row = now_row
                    .min_w(px(0.0))
                    .child(
                        div()
                            .flex()
                            .min_w(px(0.0))
                            .child(detail_text),
                    );
            }
            inner = inner.child(now_row);
        }
        // The mark: icon always, words at 1280 px and up. React `.strip-mark`
        // has no flex gap between icon and words; GPUI's title advance runs
        // 4 px short of the browser's at 1440, so at 1280 and up the row
        // carries 4 px of left margin and no gap (dot and words both land on
        // React's 333/342 at 1440).
        let mut mark_row = div()
            .flex()
            .items_center()
            .child(gpui::svg().path(mark_icon).size(px(14.0)).text_color(tone_color));
        if self.width >= 1280.0 {
            mark_row = mark_row.ml(px(4.0));
        }
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
        // The page's own actions, on the strip's right, before Pause and
        // Search (React `.rd-actions`).
        if let Some(actions) = page_actions {
            let mut box_ = div()
                .flex()
                .items_center()
                .gap_2()
                .pr_3()
                .mr_1()
                .border_r_1()
                .border_color(Theme::color(t.line))
                .child(actions);
            if phone {
                // React's phone strip: the actions lose their edge and their
                // room (`.rd-actions { border-right: 0; padding-right: 0;
                // margin-right: 0; gap: var(--s1); }`).
                box_ = box_.pr_0().mr_0().gap_1().border_r_0();
            }
            inner = inner.child(box_);
        }
        // Controls at the right (React `.strip-controls`: an inline-flex
        // group with `margin-left: auto` and an 8 px (`--s2`) gap between
        // Pause lights and search, not the strip row's 16 px gap).
        let mut controls = div().flex().items_center().gap(px(8.0));
        if known {
            if phone {
                // The same sm button as the row's (Pause lights), with the
                // words hidden (`.strip-power-text`): icon 14, 10 px padding
                // — 36 wide, 28 tall.
                let entity = cx.entity();
                controls = controls.child(button_icon(
                    t,
                    "strip-power",
                    "",
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
            } else {
                // React's label is a `Swap` of "Pause lights"/"Resume
                // lights": the cell keeps the widest label's width and the
                // visible label is centred in it (#204 G8, measured).
                let widest = text_width(
                    window,
                    "Resume lights",
                    14.0,
                    gpui::FontWeight::MEDIUM,
                    crate::fonts::UI_FAMILY,
                );
                let entity = cx.entity();
                controls = controls.child(button_icon_wide(
                    t,
                    "strip-power",
                    if paused { "Resume lights" } else { "Pause lights" },
                    widest,
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
        // The search button opens the command palette
        // (`StatusStrip.tsx:196-203`, `onOpenPalette`).
        let entity = cx.entity();
        controls = controls.child(icon_button(
            t,
            "strip-search",
            "MagnifyingGlass",
            "",
            true,
            false,
            false,
            move |_, window, cx| {
                entity.update(cx, |this, cx| this.open_palette(window, cx));
            },
        ));
        inner = inner.child(controls);
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
        if self.page == "layout" {
            return self.layout_view.clone().into_any_element();
        }
        if self.page == "devices" {
            return self.devices_view.clone().into_any_element();
        }
        if self.page == "fans" {
            return self.fans_view.clone().into_any_element();
        }
        if self.page == "settings" {
            return self.settings_view.clone().into_any_element();
        }
        if self.page == "updates" {
            return self.updates_view.clone().into_any_element();
        }
        if self.page == "peripherals" {
            return self.peripherals_view.clone().into_any_element();
        }
        if self.page == "effects" {
            return self.effects_view.clone().into_any_element();
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
        let t = self.theme;
        let offline = self.status.is_none() && self.status_error.is_some();
        if offline {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(2.0))
                .bg(Theme::color(t.down))
                .into_any_element()
        } else {
            // The spectrum (`--spectrum`: `linear-gradient(90deg, rgb-1, …
            // rgb-6)`). GPUI gradients take two stops, so paint one 1.5 px
            // quad per column, interpolating the six stops linearly — the
            // same sweep at this height.
            let stops = [
                Theme::color(t.rgb_1),
                Theme::color(t.rgb_2),
                Theme::color(t.rgb_3),
                Theme::color(t.rgb_4),
                Theme::color(t.rgb_5),
                Theme::color(t.rgb_6),
            ];
            canvas(
                move |_, _, _| {},
                move |bounds, _, window, _| {
                    let w: f32 = bounds.size.width.into();
                    let h: f32 = bounds.size.height.into();
                    let step = 1.5f32;
                    let mut x = 0.0f32;
                    while x < w {
                        let u = if w <= 1.0 { 0.0 } else { (x + step / 2.0) / w };
                        // Six stops at 0, .2, .4, .6, .8, 1 (CSS default).
                        let pos = (u * 5.0).clamp(0.0, 4.999_9);
                        let i = pos.floor() as usize;
                        let f = pos - i as f32;
                        let a = stops[i];
                        let b = stops[i + 1];
                        let color = gpui::Rgba {
                            r: a.r + (b.r - a.r) * f,
                            g: a.g + (b.g - a.g) * f,
                            b: a.b + (b.b - a.b) * f,
                            a: 1.0,
                        };
                        window.paint_quad(gpui::fill(
                            Bounds {
                                origin: point(px(x), px(0.0)),
                                size: size(px(step.min(w - x)), px(h)),
                            },
                            color,
                        ));
                        x += step;
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(2.0))
            .into_any_element()
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
                        // React `body`: 1px-radius dots centred on the 22 px
                        // tile, so the first dot is centred at (11, 11); the
                        // 2 px square below spans the same box, centred too.
                        let mut x = px(10.0);
                        while x < w {
                            let mut y = px(10.0);
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
        // The Appearance radio writes the shared cell; every view follows.
        let shared = *self.theme_cell.lock().unwrap();
        if shared != self.theme {
            self.theme = shared;
            let displays = self.displays.clone();
            let fans_view = self.fans_view.clone();
            displays.update(cx, |d, _| d.theme = shared);
            fans_view.update(cx, |f, _| f.theme = shared);
            let devices_view = self.devices_view.clone();
            devices_view.update(cx, |d, _| d.theme = shared);
            self.updates_view.update(cx, |u, _| u.theme = shared);
            self.peripherals_view.update(cx, |v, _| v.theme = shared);
            self.effects_view.update(cx, |v, _| v.theme = shared);
            self.layout_view.update(cx, |v, _| v.theme = shared);
        }
        // The Updates page's tray state comes from the settings view (the
        // single reader of the stdio receiver). Copy it here when the
        // Updates page is open so progress ticks and the strip context
        // follow without a second reader.
        if self.page == "updates" {
            let st = self
                .settings_view
                .update(cx, |s, _| s.tray_update.clone());
            self.updates_view.update(cx, |u, cx| u.set_update(st, cx));
        }
        // The theme follows the settings Appearance radio in the palette too.
        self.palette
            .update(cx, |p, _| p.theme = self.theme);
        // The palette's lists are derived from the shell's answers every
        // render, so Run rows appear as the effects answer lands.
        if self.palette_open {
            let (theme_hint, motion_hint) = self
                .settings_view
                .update(cx, |s, _| (s.theme_pref.clone(), s.motion_pref.clone()));
            let offline = self.status.is_none() && self.status_error.is_some();
            let commands = build_commands(
                self.status.as_ref(),
                offline,
                self.effects.as_ref(),
                &theme_hint,
                &motion_hint,
            );
            let short = build_short(&commands, self.effects.as_ref(), self.status.as_ref());
            self.palette
                .update(cx, |p, _| p.set_data(commands, short));
        }
        let t = &self.theme;
        let phone = self.width <= 720.0;
        let phone_pad_top = px(16.0);
        let content = div()
            .flex_1()
            .flex_col()
            // A page taller than the window must not push the dock off
            // it (React's phone page scrolls as a document; GPUI
            // clips the content box, the dock stays fixed).
            .min_h(px(0.0))
            .overflow_hidden()
            .p_4()
            .child(self.page_content());
        let content = if phone {
            // React phone `.content`: padding s4 (16) on the sides.
            content.px(px(16.0)).pb(px(120.0)).pt(phone_pad_top)
        } else {
            // React `.content { flex: 1 1 auto; min-height: 0; overflow:
            // hidden; }`: the content box holds the page's own height, so a
            // long list scrolls inside its own pane.
            content
                .px(px(24.0))
                .pb(px(24.0))
                .pt(px(16.0))
                .min_h_0()
                .overflow_hidden()
        };
        let main_col = div()
            .flex()
            .flex_col()
            .flex_1()
            // A page taller than the window must shrink this column
            // (the dock stays on screen; the content clips).
            .min_h(px(0.0))
            .child(self.strip(window, cx))
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
                // React `.rail-col{border-right:1px solid var(--line);
                // background:var(--panel)}`: the 1 px line sits over the
                // panel, dark 34,36,39 / light 216,217,218. The wrapper
                // carries the panel background too, or the line composits
                // over the g1 ground (28,29,31 / 197,199,204) (#204 G8-r3).
                .child(
                    div()
                        .w(px(64.0))
                        .h_full()
                        .border_r_1()
                        .border_color(Theme::color(t.line))
                        .bg(Theme::color(t.panel))
                        .child(self.rail(true, window, cx)),
                )
                .child(main_col)
        } else {
            div()
                .flex()
                .size_full()
                .child(
                    div()
                        .w(px(200.0))
                        .h_full()
                        .border_r_1()
                        .border_color(Theme::color(t.line))
                        .bg(Theme::color(t.panel))
                        .child(self.rail(false, window, cx)),
                )
                .child(main_col)
        };
        let mut root = div()
            .relative()
            .size_full()
            .bg(Theme::color(t.g1))
            .line_height(gpui::relative(crate::theme::LINE_HEIGHT))
            .on_key_down(cx.listener(Self::on_shell_key))
            .child(self.background())
            .child(body)
            .child(self.spectrum_line());
        // The palette is rendered outside the page area it covers, so it
        // still opens over the daemon-down page and before the first answer
        // (`Shell.tsx:269-285`).
        if self.palette_open {
            root = root.child(self.palette.clone());
        }
        root
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
