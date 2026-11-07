//! The Layout page: the desk and the inspector beside it.
//!
//! The port follows `app/web/src/views/LayoutView.tsx` and
//! `app/web/src/views/PreviewStage.tsx`; the checklist is
//! `docs/gpui-parity/layout.md`.
//!
//! The page is three things:
//!
//! - the strip's own actions (undo and redo, the zone count, the save state,
//!   Revert, Save and "Map my PC again"), which the shell draws beside the
//!   page's name;
//! - the desk, in [`stage`], with the parts on it and the cells the engine
//!   samples;
//! - the inspector: "To place", the parts list and (on a phone) the desk's
//!   own settings.

pub mod art;
pub mod geometry;
pub mod preview;
pub mod stage;
pub mod tokens;

use gpui::{Context, IntoElement, Render, Window, div, prelude::*, px};

use canvas_proto::api::{DevicesResponse, LayoutsResponse};
use canvas_proto::device::{DeviceInfo, DeviceKind};
use canvas_proto::layout::{Layout, SamplingMode};

use crate::client::DaemonClient;
use crate::components::button::{Variant, button_icon, icon_button};
use crate::components::panel::panel_with_padding;
use crate::components::segmented::segmented;
use crate::theme::{Theme, TokenRadius};
use crate::views::layout::geometry::{LengthUnit, format_units};
use crate::views::layout::preview::PreviewFeed;
use crate::views::layout::stage::{DeskInput, Sel, StageMode, desk_with_hint, hero_view};
use crate::views::layout::tokens::DeskTokens;

/// The panes below the desk on a phone: one of them at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhonePane {
    Parts,
    Place,
    Render,
}

/// The Layout page.
pub struct LayoutView {
    pub theme: Theme,
    /// The daemon's layouts and the active one.
    pub layouts: Option<LayoutsResponse>,
    pub layouts_error: Option<String>,
    /// The daemon's devices.
    pub devices: Option<Vec<DeviceInfo>>,
    /// The unit every size is read in.
    pub unit: LengthUnit,
    /// What the desk shows: the picture under the parts, or the lights.
    pub mode: StageMode,
    /// The pane below the desk on a phone.
    pub phone_pane: PhonePane,
    /// The daemon's live preview.
    pub feed: PreviewFeed,
    /// The picked parts.
    pub selection: Vec<Sel>,
    /// How many frames the feed has delivered: the page watches this number
    /// to know when the desk has to be drawn again.
    pub frames: u64,
    /// The window's width, for the page's own media queries.
    pub width: f32,
    /// The window's height.
    pub height: f32,
}

impl LayoutView {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        let unit = system_unit();
        let mut view = LayoutView {
            theme,
            layouts: None,
            layouts_error: None,
            devices: None,
            unit,
            mode: StageMode::Effect,
            phone_pane: PhonePane::Parts,
            feed: PreviewFeed::new(),
            selection: Vec::new(),
            frames: 0,
            width: 1280.0,
            height: 860.0,
        };
        view.load(cx);
        view.start_feed();
        view.watch_feed(cx);
        view
    }

    /// The layout the daemon has active.
    pub fn layout(&self) -> Option<&Layout> {
        let layouts = self.layouts.as_ref()?;
        let active = &layouts.active;
        layouts
            .layouts
            .iter()
            .find(|l| &l.id == active)
            .or_else(|| layouts.layouts.first())
    }

    /// The devices, or none yet.
    pub fn devices(&self) -> &[DeviceInfo] {
        self.devices.as_deref().unwrap_or(&[])
    }

    /// `GET /api/v1/layouts` and `GET /api/v1/devices`, off the UI thread.
    fn load(&mut self, cx: &mut Context<Self>) {
        let started = std::time::Instant::now();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            let layouts = client.get("/api/v1/layouts");
            let devices = client.get("/api/v1/devices");
            (layouts, devices)
        });
        cx.spawn(async move |view, cx| {
            let (layouts, devices) = bg.await;
            let _ = std::fs::write(
                "C:/Users/Luke/AppData/Local/Temp/opencode/layout-load.txt",
                format!(
                    "took {:?} layouts={:?} devices={:?}",
                    started.elapsed(),
                    layouts.as_ref().map(|b| b.len()),
                    devices.as_ref().map(|b| b.len())
                ),
            );
            let _ = view.update(cx, |this, cx| {
                match layouts {
                    Ok(body) => match serde_json::from_str::<LayoutsResponse>(&body) {
                        Ok(parsed) => {
                            this.layouts = Some(parsed);
                            this.layouts_error = None;
                        }
                        Err(e) => this.layouts_error = Some(e.to_string()),
                    },
                    Err(e) => this.layouts_error = Some(e.to_string()),
                }
                if let Ok(body) = devices {
                    if let Ok(parsed) = serde_json::from_str::<DevicesResponse>(&body) {
                        this.devices = Some(parsed.devices);
                    }
                }
                cx.notify();
            });
            let timer = cx
                .background_executor()
                .timer(std::time::Duration::from_secs(4));
            timer.await;
            let _ = view.update(cx, |this, cx| {
                this.load(cx);
            });
        })
        .detach();
    }

    /// The live preview feed: the daemon's WebSocket, read in the background.
    fn start_feed(&self) {
        let port = crate::client::api_port();
        let Some(token) = crate::client::api_token() else {
            return;
        };
        preview::spawn(port, token, self.feed.clone());
    }

    /// Watches the feed: the desk is drawn again when a frame arrives, the
    /// way React repaints its overlay for every frame.
    fn watch_feed(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            loop {
                let timer = cx
                    .background_executor()
                    .timer(std::time::Duration::from_millis(100));
                timer.await;
                let _ = view.update(cx, |this, cx| {
                    let count = this.feed.count();
                    if count != this.frames {
                        this.frames = count;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// The page's name and its one-line context, for the strip.
    pub fn strip_title(&self) -> (String, Option<String>) {
        let name = self.layout().map(|l| l.name.clone());
        ("Layout".to_string(), name)
    }

    /// The strip's page actions (React `PageActions`).
    pub fn strip_actions(view: &LayoutView, cx: &mut Context<LayoutView>) -> gpui::AnyElement {
        view.actions_row(cx).into_any_element()
    }

    /// The page's own actions: undo and redo, the zone count, the save state,
    /// Revert, Save and "Map my PC again".
    fn actions_row(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        // React's media queries: below 1439 the quiet key keeps its icon,
        // below 1100 the zone count goes, and below 720 the undo and redo
        // keys move to the desk and the save keys keep their icons.
        let narrow = self.width <= 1439.0;
        let no_count = self.width <= 1099.0;
        let phone = self.width <= 719.0;
        let (visible, total) = match self.layout() {
            Some(l) => (
                l.zones.iter().filter(|z| z.visible).count(),
                l.zones.len(),
            ),
            None => (0, 0),
        };
        let mut row = div().flex().items_center().gap_2();
        if !phone {
            // The undo and redo keys undo work done on the desk.
            row = row
                .child(icon_button(
                    &t,
                    "lay-undo",
                    "ArrowUUpLeft",
                    "",
                    true,
                    false,
                    true,
                    |_, _, _| {},
                ))
                .child(icon_button(
                    &t,
                    "lay-redo",
                    "ArrowUUpRight",
                    "",
                    true,
                    false,
                    true,
                    |_, _, _| {},
                ));
        }
        if !no_count {
            row = row.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(format!("{visible}/{total} zones")),
            );
        }
        // The save state sits beside the save.
        row = row
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("Saved"),
            )
            .child(button_icon(
                &t,
                "lay-revert",
                if phone { "" } else { "Revert" },
                Variant::Ghost,
                true,
                true,
                "ArrowCounterClockwise",
                "",
                16.0,
                |_, _, _| {},
            ))
            .child(button_icon(
                &t,
                "lay-save",
                if phone { "" } else { "Save layout" },
                Variant::Primary,
                true,
                true,
                "FloppyDisk",
                "bold",
                16.0,
                |_, _, _| {},
            ));
        if !phone {
            row = row.child(button_icon(
                &t,
                "lay-remap",
                if narrow { "" } else { "Map my PC again" },
                Variant::Ghost,
                false,
                true,
                "CompassTool",
                "",
                16.0,
                |_, _, _| {},
            ));
        }
        row
    }
}

/// The unit this machine's region uses, until the owner picks one
/// (`systemUnit` in `lib/zones.ts`).
pub fn system_unit() -> LengthUnit {
    locale_region()
        .map(|region| LengthUnit::for_region(&region))
        .unwrap_or(LengthUnit::Cm)
}

/// The user's region, as Windows has it ("en-US").
fn locale_region() -> Option<String> {
    #[cfg(windows)]
    {
        const CAPACITY: i32 = 85;
        let mut buf = [0u16; CAPACITY as usize];
        // SAFETY: `buf` is CAPACITY wide, which is what the call is told,
        // and the call writes at most that many code units into it.
        let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), CAPACITY) };
        if len <= 0 {
            return None;
        }
        let name = String::from_utf16_lossy(&buf[..len as usize]);
        // The region is the tag after the language: "en-US" -> "US".
        let parts: Vec<&str> = name.split(['-', '_']).collect();
        let region = parts
            .get(1)
            .filter(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))?;
        return Some(region.to_string());
    }
    #[cfg(not(windows))]
    None
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultLocaleName(lpLocaleName: *mut u16, cchLocaleName: i32) -> i32;
}

/// The page's own container width: the content box, which is what the page's
/// container queries measure.
pub fn container_width(window_width: f32) -> f32 {
    let phone = window_width <= 720.0;
    let rail = if phone {
        0.0
    } else if window_width < 1280.0 {
        64.0
    } else {
        200.0
    };
    let pad = if phone { 16.0 } else { 24.0 };
    window_width - rail - pad * 2.0
}

/// The inspector's width, by the page's container queries.
pub fn inspector_width(container: f32) -> f32 {
    if container >= 1500.0 {
        360.0
    } else if container <= 999.0 {
        280.0
    } else {
        320.0
    }
}

/// The desk's own box, in css pixels: the size the desk's view is measured
/// from.
///
/// The desk fills the frame, and the frame fills the desk's column of the
/// page's grid. On a phone the column is 46 vh tall and the panes below the
/// desk are the switcher and the bar; on a desktop the column is the page's
/// own height and the bar is one row.
pub fn desk_size(
    window_w: f32,
    window_h: f32,
    container: f32,
    inspector_w: f32,
    one_column: bool,
) -> (f32, f32) {
    let width = if one_column {
        container - 2.0
    } else {
        container - inspector_w - 16.0 - 2.0
    };
    let height = if one_column {
        // The desk's column is 46 vh; the switcher and the bar are under it.
        window_h * 0.46 - 32.0 - 2.0 - 91.0
    } else {
        // The strip, the page's padding and the bar.
        window_h - 55.0 - 16.0 - 24.0 - 2.0 - 51.0
    };
    (width, height)
}

/// One zone that is not on the desk yet.
pub struct Unplaced {
    pub device_id: String,
    pub device_name: String,
    pub zone_index: u32,
    pub zone_name: String,
    pub led_count: u32,
    pub connected: bool,
}

/// The zones that are not on the desk yet (`unplaced` in LayoutView).
pub fn unplaced(layout: Option<&Layout>, devices: &[DeviceInfo]) -> Vec<Unplaced> {
    let mut out = Vec::new();
    let Some(layout) = layout else {
        return out;
    };
    let placed: Vec<String> = layout
        .zones
        .iter()
        .map(|z| format!("{}#{}", z.device_id.0, z.zone_index))
        .collect();
    for device in devices {
        for zone in &device.zones {
            if placed.contains(&format!("{}#{}", device.id.0, zone.index)) {
                continue;
            }
            out.push(Unplaced {
                device_id: device.id.0.clone(),
                device_name: device.name.clone(),
                zone_index: zone.index,
                zone_name: zone.name.clone(),
                led_count: zone.led_count,
                connected: device.connected,
            });
        }
    }
    out
}

/// The icon of a kind of device (`KindIcon` in `components/KindIcon.tsx`).
pub fn kind_icon(kind: Option<DeviceKind>, fan: bool) -> &'static str {
    if fan {
        return "Fan";
    }
    match kind {
        Some(DeviceKind::Dram) => "Memory",
        Some(DeviceKind::Keyboard) => "Keyboard",
        Some(DeviceKind::Mouse) => "Mouse",
        Some(DeviceKind::Microphone) => "Microphone",
        Some(DeviceKind::Headset) => "Headphones",
        Some(DeviceKind::Gpu) => "GraphicsCard",
        Some(DeviceKind::Motherboard) => "Cpu",
        _ => "Lightbulb",
    }
}

/// The row of a part in the parts list.
pub struct PartRow {
    /// The plain name of the zone.
    pub name: String,
    /// The owner's own name and the device's, in the list's own order.
    pub sub: String,
    /// How many LEDs the part has.
    pub leds: Option<u32>,
    /// The icon beside the name.
    pub icon: &'static str,
}

/// The parts list: the zones, then the landmarks.
pub fn part_rows(layout: Option<&Layout>, devices: &[DeviceInfo]) -> Vec<PartRow> {
    let mut out = Vec::new();
    let Some(layout) = layout else {
        return out;
    };
    for z in &layout.zones {
        let device = devices.iter().find(|d| d.id.0 == z.device_id.0);
        let info = device.and_then(|d| d.zones.iter().find(|zone| zone.index == z.zone_index));
        let plain = geometry::plain_name(devices, &z.device_id.0, z.zone_index);
        let friendly = geometry::placement_name(devices, z, &layout.zones);
        let leds = z
            .leds
            .as_ref()
            .map(|s| s.count)
            .or_else(|| info.map(|zone| zone.led_count));
        let device_name = device.map(|d| d.name.clone());
        let mut parts: Vec<String> = Vec::new();
        if !friendly.is_empty() {
            parts.push(plain.name.clone());
        }
        if let Some(name) = device_name {
            if !parts.contains(&name) {
                parts.push(name);
            }
        }
        out.push(PartRow {
            name: if friendly.is_empty() {
                plain.name.clone()
            } else {
                friendly
            },
            sub: parts.join(" · "),
            leds,
            icon: kind_icon(plain.kind, plain.fan),
        });
    }
    out
}

/// The page, at rest.
pub fn render_page(
    view: &mut LayoutView,
    window: &mut Window,
    cx: &mut Context<LayoutView>,
) -> gpui::AnyElement {
    let t = view.theme;
    let size = window.viewport_size();
    view.width = f32::from(size.width);
    view.height = f32::from(size.height);
    let container = container_width(view.width);
    let phone = view.width <= 720.0;
    let one_column = container <= 639.0;
    let inspector_w = inspector_width(container);
    let colours = DeskTokens::for_theme(&t);
    let layout = view.layout().cloned();
    let devices = view.devices().to_vec();

    let page = div().flex().flex_1().flex_col().gap_4().min_h_0().h_full();

    let Some(layout) = layout else {
        // Nothing to draw yet: the page says it is reading the layout.
        return page.child(reading(t)).into_any_element();
    };

    // The desk, in its frame, with the bar along the frame's bottom edge.
    let desk_input = DeskInput {
        zones: layout.zones.clone(),
        landmarks: layout.landmarks.clone(),
        devices: devices.clone(),
        unit: view.unit,
        mode: view.mode,
        sampling: layout.sampling,
        frame: view.feed.frame(),
        selection: view.selection.clone(),
    };
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
        .child(desk_with_hint(&t, &colours, desk_input, true))
        .child(stage_toolbar(view, &t, container, phone, cx));
    let mut stage_column = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(frame);
    if one_column {
        stage_column = stage_column.child(phone_switcher(view, &t, cx));
    }

    // The inspector: the unfinished work first, then the parts.
    let inspector = div()
        .flex()
        .flex_col()
        .gap_4()
        .min_w_0()
        .min_h_0()
        .w(px(inspector_w))
        .child(if one_column && view.phone_pane != PhonePane::Place {
            div().hidden().into_any_element()
        } else {
            place_panel(view, &t, &devices, &layout)
        })
        .child(if one_column && view.phone_pane != PhonePane::Parts {
            div().hidden().into_any_element()
        } else {
            parts_panel(view, &t, &devices, &layout, one_column, cx)
        });
    let inspector = if one_column && view.phone_pane == PhonePane::Render {
        inspector.child(render_panel(view, &t))
    } else {
        inspector
    };

    let grid = if one_column {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_h_0()
            .child(stage_column)
            .child(inspector)
    } else {
        div()
            .flex()
            .gap_4()
            .flex_1()
            .min_h_0()
            .child(stage_column)
            .child(inspector)
    };
    page.child(grid).into_any_element()
}

/// The loading frame: the page says it is reading the layout.
fn reading(t: Theme) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_base()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child("Reading the layout"),
        )
        .into_any_element()
}

/// The switcher between the panes below the desk, on a phone.
fn phone_switcher(
    view: &mut LayoutView,
    t: &Theme,
    cx: &mut Context<LayoutView>,
) -> gpui::AnyElement {
    let entity = cx.entity();
    let selected = view.phone_pane;
    segmented(
        t,
        "lay-tabs",
        &[("parts", "Parts"), ("place", "To place"), ("render", "Render")],
        match selected {
            PhonePane::Parts => "parts",
            PhonePane::Place => "place",
            PhonePane::Render => "render",
        },
        move |value, _window, cx| {
            entity.update(cx, |this, cx| {
                this.phone_pane = match value.as_str() {
                    "place" => PhonePane::Place,
                    "render" => PhonePane::Render,
                    _ => PhonePane::Parts,
                };
                cx.notify();
            });
        },
    )
    .into_any_element()
}

/// The bar along the frame's bottom edge: the scale, the segments and the
/// zoom.
fn stage_toolbar(
    view: &mut LayoutView,
    t: &Theme,
    container: f32,
    phone: bool,
    cx: &mut Context<LayoutView>,
) -> gpui::AnyElement {
    let layout = view.layout().cloned();
    let container_width = container;
    let one_column = container_width <= 639.0;
    let inspector_w = inspector_width(container_width);
    let (desk_w, desk_h) = desk_size(view.width, view.height, container_width, inspector_w, one_column);
    let centre = stage::content_centre(
        &layout.as_ref().map(|l| l.zones.clone()).unwrap_or_default(),
        &layout.as_ref().map(|l| l.landmarks.clone()).unwrap_or_default(),
    );
    let view_state = hero_view(desk_w as f64, desk_h as f64, centre);
    let scale_percent = (view_state.scale * 100.0).round() as i64;

    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .flex_none()
        .border_t_1()
        .border_color(Theme::color(t.line))
        .bg(Theme::color(t.panel))
        .p_2();
    // The scale bar: a real length, in the reader's unit.
    if view.width > 480.0 {
        row = row.child(scale_bar(view, t, view_state.scale, desk_w));
    }
    // The segments in the middle: what the canvas shows, and the desk's own
    // settings.
    let mut segs = div().flex().items_center().gap(px(8.0)).min_w_0();
    let entity = cx.entity();
    let mode = view.mode;
    segs = segs.child(segmented(
        t,
        "lay-mode",
        &[("effect", "Effect"), ("lights", "Lights")],
        match mode {
            StageMode::Effect => "effect",
            StageMode::Lights => "lights",
        },
        {
            let entity = entity.clone();
            move |value, _window, cx| {
                entity.update(cx, |this, cx| {
                    this.mode = if value == "lights" {
                        StageMode::Lights
                    } else {
                        StageMode::Effect
                    };
                    cx.notify();
                });
            }
        },
    ));
    if !phone {
        // The desk's own settings: how much of the desk the effect is drawn
        // at, and how each LED reads the picture. On a narrow desk they fold
        // into one Render key that opens both.
        if desk_w >= 780.0 {
            let scale = layout
                .as_ref()
                .map(|l| l.render_scale.percent())
                .unwrap_or(50);
            segs = segs.child(segmented(
                t,
                "lay-scale",
                &[("25", "¼"), ("50", "½"), ("100", "All")],
                &scale.to_string(),
                |_value, _window, _cx| {},
            ));
            let sampling = layout
                .as_ref()
                .map(|l| sampling_key(l.sampling))
                .unwrap_or("nearest");
            segs = segs.child(segmented(
                t,
                "lay-sampling",
                &[
                    ("nearest", "1"),
                    ("bilinear", "4"),
                    ("area_average", "All px"),
                ],
                sampling,
                |_value, _window, _cx| {},
            ));
        } else {
            segs = segs.child(button_icon(
                t,
                "lay-render-key",
                "Render",
                Variant::Default,
                false,
                true,
                "SlidersHorizontal",
                "",
                15.0,
                |_, _, _| {},
            ));
        }
    } else {
        // On a phone the undo and redo keys move here, to the desk where the
        // work they undo happens.
        segs = segs
            .child(icon_button(
                t,
                "lay-stage-undo",
                "ArrowUUpLeft",
                "",
                true,
                false,
                true,
                |_, _, _| {},
            ))
            .child(icon_button(
                t,
                "lay-stage-redo",
                "ArrowUUpRight",
                "",
                true,
                false,
                true,
                |_, _, _| {},
            ));
    }
    row = row.child(segs);
    // The zoom, on the right.
    row = row.child(
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .ml_auto()
            .p(px(2.0))
            .border_1()
            .border_color(Theme::color(t.line_2))
            .r_md()
            .bg(Theme::color(t.panel))
            .child(icon_button(
                t,
                "lay-zoom-out",
                "Minus",
                "",
                true,
                false,
                false,
                |_, _, _| {},
            ))
            .child(
                div()
                    .w(px(40.0))
                    .text_center()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(format!("{scale_percent}%")),
            )
            .child(icon_button(
                t,
                "lay-zoom-in",
                "Plus",
                "",
                true,
                false,
                false,
                |_, _, _| {},
            ))
            .child(icon_button(
                t,
                "lay-zoom-fit",
                "ArrowsOut",
                "",
                true,
                false,
                false,
                |_, _, _| {},
            )),
    );
    row.into_any_element()
}

/// The sampling's name, as the segment says it.
fn sampling_key(sampling: SamplingMode) -> &'static str {
    match sampling {
        SamplingMode::Nearest => "nearest",
        SamplingMode::Bilinear => "bilinear",
        SamplingMode::AreaAverage => "area_average",
    }
}

/// The scale bar: how long a line on the desk is, in the reader's unit.
fn scale_bar(view: &LayoutView, t: &Theme, scale: f64, desk_w: f32) -> gpui::AnyElement {
    let unit = view.unit;
    let (value, px_wide) = scale_bar_length(unit, scale, desk_w);
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(6.0))
        .py(px(3.0))
        .r_sm()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .bg(Theme::color(t.panel))
        .child(
            div()
                .w(px(px_wide))
                .h(px(0.0))
                .border_t_2()
                .border_l_2()
                .border_r_2()
                .border_color(Theme::color(t.dim)),
        )
        .child(
            div()
                .text_sm()
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.dim))
                .child(value),
        )
        .into_any_element()
}

/// The scale bar's length: a round number of the reader's unit, about 120
/// css pixels wide.
pub fn scale_bar_length(unit: LengthUnit, scale: f64, desk_w: f32) -> (String, f32) {
    let px_per_unit = desk_w as f64 / 320.0 * scale;
    let px_per_cm = px_per_unit * 2.5;
    let steps: &[f64] = match unit {
        LengthUnit::In => &[1.0, 2.0, 5.0, 10.0],
        LengthUnit::Cm => &[1.0, 2.0, 5.0, 10.0, 20.0],
    };
    let raw = 120.0 / px_per_cm;
    let mut value = steps[0];
    for s in steps {
        if *s <= raw {
            value = *s;
        }
    }
    let wide = (value
        * match unit {
            LengthUnit::In => px_per_cm / 2.54,
            LengthUnit::Cm => px_per_cm,
        })
    .round() as f32;
    let text = match unit {
        LengthUnit::In => format!("{value} in"),
        LengthUnit::Cm => format!("{value} cm"),
    };
    (text, wide)
}

/// The "To place" panel: the zones that are not on the desk yet.
fn place_panel(
    view: &LayoutView,
    t: &Theme,
    devices: &[DeviceInfo],
    layout: &Layout,
) -> gpui::AnyElement {
    let rows = unplaced(Some(layout), devices);
    let count = rows.len();
    let actions = div()
        .text_sm()
        .font_family(crate::fonts::MONO_FAMILY)
        .text_color(Theme::color(t.mute))
        .child(format!("{count}"));
    let mut body = div().flex().flex_col();
    if count == 0 {
        body = body.child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.mute))
                .child("Every zone is placed."),
        );
    }
    for (i, u) in rows.iter().enumerate() {
        let device_name = u.device_name.clone();
        let connected = u.connected;
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .min_w_0()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(u.zone_name.clone())
            .child(
                div()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(format!("{}", u.led_count)),
            )
            .truncate();
        let sub = div()
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.mute))
            .child(if connected {
                device_name.clone()
            } else {
                format!("{device_name} · disconnected")
            })
            .truncate();
        let mut row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .py(px(8.0))
            .child(div().flex_1().min_w(px(0.0)).child(title).child(sub));
        if i + 1 < count {
            row = row.border_b_1().border_color(Theme::color(t.line));
        }
        row = row.child(icon_button(
            t,
            &format!("lay-place-{}-{}", u.device_id, u.zone_index),
            "Plus",
            "bold",
            true,
            false,
            false,
            |_, _, _| {},
        ));
        body = body.child(row);
    }
    let panel = panel_with_padding(t, "lay-place", "To place", actions, body, None);
    let _ = view;
    div().flex_none().min_h_0().child(panel).into_any_element()
}

/// The parts panel: the list of what is on the desk.
fn parts_panel(
    view: &mut LayoutView,
    t: &Theme,
    devices: &[DeviceInfo],
    layout: &Layout,
    one_column: bool,
    cx: &mut Context<LayoutView>,
) -> gpui::AnyElement {
    let entity = cx.entity();
    let unit = view.unit;
    let unit_entity = entity.clone();
    let actions = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .child(segmented(
            t,
            "lay-unit",
            &[("cm", "cm"), ("in", "in")],
            match unit {
                LengthUnit::Cm => "cm",
                LengthUnit::In => "in",
            },
            move |value, _window, cx| {
                unit_entity.update(cx, |this, cx| {
                    this.unit = if value == "in" {
                        LengthUnit::In
                    } else {
                        LengthUnit::Cm
                    };
                    cx.notify();
                });
            },
        ))
        .child(button_icon(
            t,
            "lay-add-part",
            "Add part",
            Variant::Ghost,
            false,
            true,
            "Plus",
            "",
            14.0,
            |_, _, _| {},
        ));
    let rows = part_rows(Some(layout), devices);
    let mut body = div()
        .id("lay-parts-body")
        .flex()
        .flex_col()
        .gap_0()
        .mt_3();
    if !one_column {
        body = body.flex_1().min_h_0().overflow_y_scroll();    }
    for row in rows {
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .min_w(px(0.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(
                gpui::svg()
                    .path(format!("icons/{}.svg", row.icon))
                    .size(px(15.0))
                    .flex_none()
                    .text_color(Theme::color(t.ink)),
            )
            .child(row.name.clone())
            .truncate();
        let sub = div()
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.mute))
            .child(row.sub.clone())
            .truncate();
        let count = div()
            .text_sm()
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.mute))
            .child(match row.leds {
                Some(n) => format!("{n}"),
                None => "—".to_string(),
            });
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .w_full()
                .min_h(px(if one_column { 32.0 } else { 52.0 }))
                .px(px(12.0))
                .py(px(if one_column { 4.0 } else { 8.0 }))
                .border_b_1()
                .border_color(Theme::color(t.line))
                .child(title)
                .child(div().flex_1().min_w(px(0.0)).child(sub))
                .child(count),
        );
    }
    let panel = panel_with_padding(t, "lay-parts", "Parts", actions, body, None);
    let panel = if one_column {
        panel
    } else {
        // The parts pane fills the inspector's column and the panel fills the
        // pane, so the list scrolls inside its own panel (R1).
        panel.flex_1().min_h_0()
    };
    let pane = div()
        .flex()
        .flex_col()
        .min_h_0()
        .child(panel);
    let pane = if one_column {
        pane.flex_none()
    } else {
        pane.flex_1()
    };
    pane.into_any_element()
}

/// The Render pane: the desk's own settings, on a phone.
fn render_panel(view: &mut LayoutView, t: &Theme) -> gpui::AnyElement {
    let layout = view.layout().cloned();
    let scale = layout
        .as_ref()
        .map(|l| l.render_scale.percent())
        .unwrap_or(50);
    let sampling = layout
        .as_ref()
        .map(|l| sampling_key(l.sampling))
        .unwrap_or("nearest");
    let body = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .flex_wrap()
        .child(segmented(
            t,
            "lay-pane-scale",
            &[("25", "¼"), ("50", "½"), ("100", "All")],
            &scale.to_string(),
            |_value, _window, _cx| {},
        ))
        .child(segmented(
            t,
            "lay-pane-sampling",
            &[
                ("nearest", "1"),
                ("bilinear", "4"),
                ("area_average", "All px"),
            ],
            sampling,
            |_value, _window, _cx| {},
        ));
    let panel = panel_with_padding(t, "lay-render", "Render", div(), body, None);
    panel.into_any_element()
}

/// The size of a part, in the reader's unit (`formatUnits`).
pub fn part_size(units: f32, unit: LengthUnit) -> String {
    format_units(units as f64, unit)
}

impl Render for LayoutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_page(self, window, cx)
    }
}
