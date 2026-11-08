//! The area picker, after `app/web/src/views/effects/StackAreaPicker.tsx`.
//!
//! A modal on the palette's scrim: the desk drawn at 320 × 200, the four
//! ways to say where a layer paints, and the rectangle drawn by dragging.
//!
#![allow(dead_code)]
//! React draws the desk as an SVG; GPUI has no SVG element, so the desk is
//! painted into a canvas element with the same geometry.

use canvas_proto::device::DeviceInfo;
use canvas_proto::layout::Layout;
use canvas_proto::stack::{LayerArea, StackLayer};

use gpui::{Bounds, IntoElement, MouseButton, Pixels, Window, canvas, div, point, prelude::*, px, size};

use crate::components::button::{Variant, button, icon_button};
use crate::components::segmented::segmented;
use crate::theme::{Theme, TokenRadius};

use super::stack_draft::{
    area_is_on_desk, area_readout, pointer_to_canvas, rect_from_drag, Bounds as AreaBounds,
    PickerView, StageBox, CANVAS_W,
};

/// The four ways a layer can say where it paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    Canvas,
    Device,
    Zone,
    Rect,
}

impl PickerMode {
    /// The mode an area opens in (`modeOf`).
    pub fn of(area: &LayerArea) -> Self {
        match area {
            LayerArea::Canvas => PickerMode::Canvas,
            LayerArea::Device { .. } => PickerMode::Device,
            LayerArea::Zone { .. } => PickerMode::Zone,
            LayerArea::Rect { .. } => PickerMode::Rect,
        }
    }

    /// The mode's name, as the segmented control's value.
    pub fn key(self) -> &'static str {
        match self {
            PickerMode::Canvas => "canvas",
            PickerMode::Device => "device",
            PickerMode::Zone => "zone",
            PickerMode::Rect => "rect",
        }
    }

    /// The mode a name means.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "canvas" => Some(PickerMode::Canvas),
            "device" => Some(PickerMode::Device),
            "zone" => Some(PickerMode::Zone),
            "rect" => Some(PickerMode::Rect),
            _ => None,
        }
    }

    /// The mode's label.
    pub fn label(self) -> &'static str {
        match self {
            PickerMode::Canvas => "Whole canvas",
            PickerMode::Device => "Device",
            PickerMode::Zone => "Zone",
            PickerMode::Rect => "Rectangle",
        }
    }
}

/// The picker's own state.
#[derive(Debug, Clone)]
pub struct PickerState {
    pub mode: PickerMode,
    pub device: String,
    pub zone_index: u32,
    pub rect: Option<AreaBounds>,
    pub view: PickerView,
    /// Where a drag began, in canvas units.
    pub drag_from: Option<(f32, f32)>,
}

impl PickerState {
    /// The state the picker opens with, for a layer's own area.
    pub fn for_layer(layer: &StackLayer, layout: Option<&Layout>) -> Self {
        let (device, zone_index) = match &layer.area {
            LayerArea::Device { device } => (device.0.clone(), 0),
            LayerArea::Zone { device, zone } => (device.0.clone(), *zone),
            _ => {
                let first = layout.and_then(|l| l.zones.iter().find(|z| z.visible));
                (
                    first.map(|z| z.device_id.0.clone()).unwrap_or_default(),
                    first.map(|z| z.zone_index).unwrap_or(0),
                )
            }
        };
        let rect = match &layer.area {
            LayerArea::Rect { x, y, width, height } => {
                Some(AreaBounds { x: *x, y: *y, width: *width, height: *height })
            }
            _ => None,
        };
        Self {
            mode: PickerMode::of(&layer.area),
            device,
            zone_index,
            rect,
            view: PickerView::default(),
            drag_from: None,
        }
    }

    /// The area the picker would use (`chosen` in React).
    pub fn chosen(&self) -> Option<LayerArea> {
        match self.mode {
            PickerMode::Canvas => Some(LayerArea::Canvas),
            PickerMode::Device => {
                if self.device.is_empty() {
                    None
                } else {
                    Some(LayerArea::Device { device: canvas_proto::device::DeviceId(self.device.clone()) })
                }
            }
            PickerMode::Zone => {
                if self.device.is_empty() {
                    None
                } else {
                    Some(LayerArea::Zone {
                        device: canvas_proto::device::DeviceId(self.device.clone()),
                        zone: self.zone_index,
                    })
                }
            }
            PickerMode::Rect => self
                .rect
                .map(|r| LayerArea::Rect { x: r.x, y: r.y, width: r.width, height: r.height }),
        }
    }
}

/// The desk's devices: the ones with a visible zone.
pub fn device_options(layout: Option<&Layout>, devices: Option<&[DeviceInfo]>) -> Vec<(String, String)> {
    let mut ids: Vec<String> = Vec::new();
    for zone in layout.map(|l| l.zones.as_slice()).unwrap_or(&[]) {
        if zone.visible && !ids.contains(&zone.device_id.0) {
            ids.push(zone.device_id.0.clone());
        }
    }
    ids.iter()
        .map(|id| {
            let name = devices
                .and_then(|list| list.iter().find(|d| d.id.0 == *id))
                .map(|d| d.name.clone())
                .unwrap_or_else(|| id.clone());
            (id.clone(), name)
        })
        .collect()
}

/// The zones of the chosen device.
pub fn zone_options(layout: Option<&Layout>, device: &str) -> Vec<(String, String)> {
    layout
        .map(|l| l.zones.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter(|zone| zone.visible && zone.device_id.0 == device)
        .map(|zone| {
            let label = zone
                .label
                .clone()
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| format!("Zone {}", zone.zone_index + 1));
            (zone.zone_index.to_string(), label)
        })
        .collect()
}

/// The picker: the modal, the desk and the keys.
#[allow(clippy::too_many_arguments)]
pub fn area_picker(
    t: &Theme,
    id: &str,
    layer: &StackLayer,
    state: &PickerState,
    layout: Option<&Layout>,
    devices: Option<&[DeviceInfo]>,
    on_mode: impl Fn(String, &mut Window, &mut gpui::App) + Clone + 'static,
    on_device: impl Fn(String, &mut Window, &mut gpui::App) + Clone + 'static,
    on_zone: impl Fn(String, &mut Window, &mut gpui::App) + Clone + 'static,
    on_desk: impl Fn((f32, f32), Option<(f32, f32)>, &mut Window, &mut gpui::App) + Clone + 'static,
    on_use: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    on_close: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let mut dialog = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .w(px(480.0))
        .max_h(px(800.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .r_lg()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .bg(Theme::color(t.panel));

    // The head.
    dialog = dialog.child(
        div()
            .flex()
            .items_start()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .line_height(gpui::relative(1.25))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(format!("Area for {}", layer.name)),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(px(14.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child("Drag on the desk to draw a rectangle, or pick a part."),
                    ),
            )
            .child(icon_button(t, &format!("{id}-close"), "X", "", false, false, false, {
                let on_close = on_close.clone();
                move |event, window, cx| on_close(event, window, cx)
            })),
    );

    // The mode.
    dialog = dialog.child(segmented(
        t,
        &format!("{id}-mode"),
        &[
            ("canvas", "Whole canvas"),
            ("device", "Device"),
            ("zone", "Zone"),
            ("rect", "Rectangle"),
        ],
        state.mode.key(),
        on_mode,
    ));

    // The device and zone choices.
    if matches!(state.mode, PickerMode::Device | PickerMode::Zone) {
        let devices_list = device_options(layout, devices);
        let mut choices = div().grid().grid_cols(2).gap_3();
        let device_options_list: Vec<(&str, &str)> = if devices_list.is_empty() {
            vec![("", "No device on the desk")]
        } else {
            devices_list.iter().map(|(id, name)| (id.as_str(), name.as_str())).collect()
        };
        let device_value = state.device.clone();
        choices = choices.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(px(14.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child("Device"),
                )
                .child(crate::components::select::select_field(
                    t,
                    &format!("{id}-device"),
                    "Device",
                    &device_options_list,
                    &device_value,
                    false,
                    false,
                    true,
                    {
                        let on_device = on_device.clone();
                        move |_w, _cx| {
                            let _ = &on_device;
                        }
                    },
                    {
                        let on_device = on_device.clone();
                        move |value, window, cx| on_device(value, window, cx)
                    },
                )),
        );
        if state.mode == PickerMode::Zone {
            let zones = zone_options(layout, &state.device);
            let zone_options_list: Vec<(&str, &str)> = if zones.is_empty() {
                vec![("0", "No zone on this device")]
            } else {
                zones.iter().map(|(id, name)| (id.as_str(), name.as_str())).collect()
            };
            let zone_value = state.zone_index.to_string();
            choices = choices.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.dim))
                            .child("Zone"),
                    )
                    .child(crate::components::select::select_field(
                        t,
                        &format!("{id}-zone"),
                        "Zone",
                        &zone_options_list,
                        &zone_value,
                        false,
                        false,
                        true,
                        {
                            let on_zone = on_zone.clone();
                            move |_w, _cx| {
                                let _ = &on_zone;
                            }
                        },
                        {
                            let on_zone = on_zone.clone();
                            move |value, window, cx| on_zone(value, window, cx)
                        },
                    )),
            );
        }
        dialog = dialog.child(choices);
    }

    // The desk.
    let desk_box = StageBox::new();
    let desk = div()
        .id(gpui::SharedString::from(format!("{id}-desk")))
        .relative()
        .w_full()
        .r_md()
        .bg(Theme::color(t.sunk))
        .overflow_hidden();
    let layout_owned = layout.cloned();
    let devices_owned = devices.map(|d| d.to_vec());
    let state_owned = state.clone();
    let theme = *t;
    let desk = desk.child(
        canvas(
            move |_bounds, _window, _cx| (),
            move |bounds, _state, window, _cx| {
                paint_desk(
                    window,
                    &theme,
                    bounds,
                    layout_owned.as_ref(),
                    devices_owned.as_deref(),
                    &state_owned,
                );
            },
        )
        .absolute()
        .inset_0(),
    );
    let on_desk_down = on_desk.clone();
    let desk = desk.on_mouse_down(MouseButton::Left, move |event: &gpui::MouseDownEvent, window, cx| {
        if event.button != MouseButton::Left {
            return;
        }
        let at = event.position;
        on_desk_down((f32::from(at.x), f32::from(at.y)), None, window, cx);
    });
    let on_desk_move = on_desk.clone();
    let desk = desk.on_mouse_move(move |event: &gpui::MouseMoveEvent, window, cx| {
        let at = event.position;
        on_desk_move((f32::from(at.x), f32::from(at.y)), None, window, cx);
    });
    let _ = desk_box;
    dialog = dialog.child(desk);

    // The readout.
    let chosen = state.chosen();
    let readout = match chosen.as_ref().map(|c| area_readout(c, layout, devices)) {
        Some(readout) => div()
            .flex()
            .items_baseline()
            .gap_3()
            .min_h(px(20.0))
            .text_size(px(14.0))
            .line_height(gpui::relative(1.4))
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.dim))
            .child(readout.title)
            .child(
                readout
                    .detail
                    .map(|d| div().text_color(Theme::color(t.mute)).child(d))
                    .unwrap_or_else(|| div()),
            ),
        None => div()
            .flex()
            .items_baseline()
            .gap_3()
            .min_h(px(20.0))
            .text_size(px(14.0))
            .line_height(gpui::relative(1.4))
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.mute))
            .child(if state.mode == PickerMode::Rect {
                "Drag on the desk to draw a rectangle."
            } else {
                "Pick a part on the desk."
            }),
    };
    dialog = dialog.child(readout);

    // The foot.
    let usable = chosen
        .as_ref()
        .map(|c| matches!(c, LayerArea::Canvas | LayerArea::Rect { .. }) || area_is_on_desk(c, layout))
        .unwrap_or(false);
    dialog = dialog.child(
        div()
            .flex()
            .items_center()
            .justify_end()
            .gap_2()
            .child(button(t, &format!("{id}-cancel"), "Cancel", Variant::Ghost, false, on_close))
            .child(button_icon_check(t, &format!("{id}-use"), usable, on_use)),
    );

    // The scrim.
    div()
        .id(gpui::SharedString::from(format!("{id}-scrim")))
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .bg(Theme::color(t.scrim))
        .child(dialog)
}

/// The "Use this area" key.
fn button_icon_check(
    t: &Theme,
    id: &str,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    crate::components::button::button_icon(
        t,
        id,
        "Use this area",
        Variant::Primary,
        !enabled,
        false,
        "Check",
        "",
        15.0,
        on_click,
    )
}

/// Paints the desk: the line, the landmarks and the zones.
fn paint_desk(
    window: &mut Window,
    t: &Theme,
    bounds: Bounds<Pixels>,
    layout: Option<&Layout>,
    devices: Option<&[DeviceInfo]>,
    state: &PickerState,
) {
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let k = w / CANVAS_W;
    let to_x = |x: f32| f32::from(bounds.origin.x) + x * k;
    let to_y = |y: f32| f32::from(bounds.origin.y) + y * k;

    // The desk line at y 190.
    window.paint_quad(gpui::fill(
        Bounds {
            origin: point(px(to_x(0.0)), px(to_y(190.0))),
            size: size(px(w), px(1.0)),
        },
        Theme::color(t.line),
    ));

    // The landmarks, as cases.
    if let Some(layout) = layout {
        for landmark in layout.landmarks.iter().filter(|l| l.visible) {
            let mut el = div();
            let _ = &mut el;
            let x = to_x(landmark.x);
            let y = to_y(landmark.y);
            let lw = landmark.width * k;
            let lh = landmark.height * k;
            window.paint_quad(gpui::fill(
                Bounds { origin: point(px(x), px(y)), size: size(px(lw), px(lh)) },
                gpui::Rgba { a: 0.0, ..Theme::color(t.line_2) },
            ));
            // The 1.5 px outline.
            for (ox, oy, bw, bh) in [
                (x, y, lw, 1.5),
                (x, y + lh - 1.5, lw, 1.5),
                (x, y, 1.5, lh),
                (x + lw - 1.5, y, 1.5, lh),
            ] {
                window.paint_quad(gpui::fill(
                    Bounds { origin: point(px(ox), px(oy)), size: size(px(bw), px(bh)) },
                    Theme::color(t.line_2),
                ));
            }
        }
        // The zones.
        for zone in layout.zones.iter().filter(|z| z.visible) {
            let lit = state.device == zone.device_id.0
                && (state.mode == PickerMode::Device || state.zone_index == zone.zone_index);
            let x = to_x(zone.x);
            let y = to_y(zone.y);
            let zw = zone.width * k;
            let zh = zone.height * k;
            let fill = if lit {
                gpui::Rgba { a: 0.22, ..Theme::color(t.a1) }
            } else {
                Theme::color(t.raised)
            };
            window.paint_quad(gpui::fill(
                Bounds { origin: point(px(x), px(y)), size: size(px(zw), px(zh)) },
                fill,
            ));
            let stroke = if lit { Theme::color(t.a1) } else { Theme::color(t.line_2) };
            for (ox, oy, bw, bh) in [
                (x, y, zw, 1.0),
                (x, y + zh - 1.0, zw, 1.0),
                (x, y, 1.0, zh),
                (x + zw - 1.0, y, 1.0, zh),
            ] {
                window.paint_quad(gpui::fill(
                    Bounds { origin: point(px(ox), px(oy)), size: size(px(bw), px(bh)) },
                    stroke,
                ));
            }
        }
    }

    // The rectangle, in Rectangle mode.
    if state.mode == PickerMode::Rect {
        if let Some(rect) = state.rect {
            let x = to_x(rect.x);
            let y = to_y(rect.y);
            let rw = rect.width * k;
            let rh = rect.height * k;
            window.paint_quad(gpui::fill(
                Bounds { origin: point(px(x), px(y)), size: size(px(rw), px(rh)) },
                gpui::Rgba { a: 0.15, ..Theme::color(t.a1) },
            ));
            for (ox, oy, bw, bh) in [
                (x, y, rw, 1.5),
                (x, y + rh - 1.5, rw, 1.5),
                (x, y, 1.5, rh),
                (x + rw - 1.5, y, 1.5, rh),
            ] {
                window.paint_quad(gpui::fill(
                    Bounds { origin: point(px(ox), px(oy)), size: size(px(bw), px(bh)) },
                    Theme::color(t.a1),
                ));
            }
            for (hx, hy) in [(x, y), (x + rw, y), (x, y + rh), (x + rw, y + rh)] {
                window.paint_quad(gpui::fill(
                    Bounds { origin: point(px(hx - 2.0), px(hy - 2.0)), size: size(px(4.0), px(4.0)) },
                    Theme::color(t.a1),
                ));
            }
        }
    }
    let _ = devices;
}

/// The pointer's place on the desk, in canvas units.
pub fn desk_point(
    at: (f32, f32),
    bounds: Bounds<Pixels>,
    view: &PickerView,
) -> (f32, f32) {
    let box_ = StageBox {
        left: f32::from(bounds.origin.x),
        top: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    };
    pointer_to_canvas(at.0, at.1, &box_, view)
}

/// The rectangle a drag draws.
pub fn drag_rect(from: (f32, f32), to: (f32, f32)) -> AreaBounds {
    rect_from_drag(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modes_label_and_key_are_the_pickers_own() {
        assert_eq!(PickerMode::Canvas.label(), "Whole canvas");
        assert_eq!(PickerMode::Device.label(), "Device");
        assert_eq!(PickerMode::Zone.label(), "Zone");
        assert_eq!(PickerMode::Rect.label(), "Rectangle");
        assert_eq!(PickerMode::from_key("zone"), Some(PickerMode::Zone));
        assert_eq!(PickerMode::from_key("nope"), None);
    }

    #[test]
    fn a_layer_opens_the_picker_in_its_own_mode() {
        let mut layer = StackLayer {
            id: "l1".into(),
            name: "Heat".into(),
            effect: "fire".into(),
            props: Default::default(),
            area: LayerArea::Rect { x: 16.0, y: 150.0, width: 110.0, height: 46.0 },
            blend: canvas_proto::stack::BlendMode::Normal,
            opacity: 100,
            visible: true,
        };
        let state = PickerState::for_layer(&layer, None);
        assert_eq!(state.mode, PickerMode::Rect);
        assert_eq!(state.rect, Some(AreaBounds { x: 16.0, y: 150.0, width: 110.0, height: 46.0 }));
        layer.area = LayerArea::Canvas;
        assert_eq!(PickerState::for_layer(&layer, None).mode, PickerMode::Canvas);
    }

    #[test]
    fn the_chosen_area_follows_the_mode() {
        let mut state = PickerState {
            mode: PickerMode::Canvas,
            device: String::new(),
            zone_index: 0,
            rect: None,
            view: PickerView::default(),
            drag_from: None,
        };
        assert_eq!(state.chosen(), Some(LayerArea::Canvas));
        state.mode = PickerMode::Rect;
        assert_eq!(state.chosen(), None);
        state.rect = Some(AreaBounds { x: 1.0, y: 2.0, width: 3.0, height: 4.0 });
        assert_eq!(
            state.chosen(),
            Some(LayerArea::Rect { x: 1.0, y: 2.0, width: 3.0, height: 4.0 })
        );
        state.mode = PickerMode::Device;
        state.device = "openrgb:kb".into();
        assert_eq!(
            state.chosen(),
            Some(LayerArea::Device { device: canvas_proto::device::DeviceId("openrgb:kb".into()) })
        );
    }

    #[test]
    fn a_drag_draws_the_rectangle_the_picker_makes() {
        let r = drag_rect((10.0, 10.0), (20.0, 30.0));
        assert_eq!(r, AreaBounds { x: 10.0, y: 10.0, width: 10.0, height: 20.0 });
    }
}
