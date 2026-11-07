//! The property controls, after `app/web/src/views/PropertyControls.tsx` and
//! the `.prop-item` / `.prop-control` rules. Every effect and every
//! peripheral renders its settings from the schema alone through
//! [`control`]: a number is a slider plus a number field, a colour a swatch
//! with its picker, a bool a switch, a choice a segmented (up to four
//! options) or a select, a hue a ring plus degrees, an angle a dial plus
//! degrees.
//!
//! The panel layouts that place the controls (the row grid, the label) stay
//! with the views; this module owns the controls themselves and the pure
//! rules they share. Peripherals (which built these first) and the Effects
//! page both call into it, so the two pages cannot drift apart.

use canvas_proto::color::Rgb8;
use canvas_proto::props::{PropKind, PropValue, Props, PropertyDef};
use gpui::{Bounds, IntoElement, Window, canvas, div, point, prelude::*, px, size};

use crate::components::number_field::number_field;
use crate::components::segmented::{segmented, segmented_disabled};
use crate::components::select::select_field;
use crate::components::slider::{Origin, slider_range};
use crate::components::toggle::{toggle, toggle_disabled};
use crate::theme::{Theme, TokenRadius};

// ---------------------------------------------------------------------------
// Logic.

// Decimal places a step needs.

pub fn decimals_of(step: f64) -> usize {
    let s = step.to_string();
    match s.find('.') {
        None => 0,
        Some(dot) => (s.len() - dot - 1).min(2),
    }
}

/// The default value for a missing key.
pub fn default_for(kind: &PropKind) -> PropValue {
    kind.default_value()
}

/// Wrap degrees into 0..360.
pub fn wrap_deg(deg: f64) -> f64 {
    ((deg % 360.0) + 360.0) % 360.0
}

/// A hue step (arrows: 3 degrees, shift 15).
#[allow(dead_code)]
pub fn hue_step(current: f64, delta: f64) -> f64 {
    wrap_deg(current + delta)
}

/// An angle step (arrows: 1 degree, shift 15, Home 0, End 359).
#[allow(dead_code)]
pub fn angle_step(current: f64, delta: f64) -> f64 {
    wrap_deg(current + delta)
}

/// The angle read-out: rounded and wrapped to 0..=359.
pub fn angle_now(deg: f64) -> i32 {
    ((wrap_deg(deg).round() as i64) % 360) as i32
}

/// The HSV conversions `ColorPicker` works in.
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

/// HSV to `#rrggbb`.
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

/// Parse `#rrggbb` (or a bare `rrggbb`) to `Rgb8`.
pub fn parse_hex(hex: &str) -> Option<Rgb8> {
    hex.parse::<Rgb8>().ok()
}

/// Whether a value is a valid `#rrggbb`.
#[allow(dead_code)]
pub fn is_hex(value: &str) -> bool {
    parse_hex(value).is_some()
}

/// The swatch list for a colour field: the other colours of the same schema,
/// excluding the field's own colour and any that do not parse.
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

// ---------------------------------------------------------------------------
// The controls.

/// One property's value change.
pub type OnChange = std::rc::Rc<dyn Fn(PropValue, &mut Window, &mut gpui::App)>;

/// Flips a view's open state (a colour picker, a select list).
pub type OnToggle = std::rc::Rc<dyn Fn(&mut Window, &mut gpui::App)>;

/// The control for one property (`control()` in PropertyControls).
///
/// `color_open` / `select_open` say whether the view holds the picker and
/// the select list open for this key; `on_toggle_color` / `on_toggle_select`
/// flip them. `disabled` draws every control at half strength (the Effects
/// page never passes it; Peripherals does for its read-only devices).
#[allow(clippy::too_many_arguments)]
pub fn control(
    t: &Theme,
    key: &str,
    label: &str,
    kind: &PropKind,
    value: &PropValue,
    schema_properties: &[PropertyDef],
    values: &Props,
    disabled: bool,
    color_open: bool,
    select_open: bool,
    on_change: OnChange,
    on_toggle_color: crate::components::prop_controls::OnToggle,
    on_toggle_select: crate::components::prop_controls::OnToggle,
) -> gpui::AnyElement {
    match kind {
        PropKind::Number { min, max, step, default } => {
            let n = match value {
                PropValue::Number(n) => *n as f32,
                _ => *default as f32,
            };
            let min_f = *min as f32;
            let max_f = *max as f32;
            let step_f = *step as f32;
            let change = {
                let on = on_change.clone();
                move |v: f32, window: &mut Window, cx: &mut gpui::App| {
                    let stepped = quantise_step(v, min_f, max_f, step_f);
                    on(PropValue::Number(stepped as f64), window, cx);
                }
            };
            let field_change = {
                let on = on_change.clone();
                move |v: f32, window: &mut Window, cx: &mut gpui::App| {
                    let stepped = quantise_step(v, min_f, max_f, step_f);
                    on(PropValue::Number(stepped as f64), window, cx);
                }
            };
            div()
                .flex()
                .items_center()
                .gap_3()
                .flex_1()
                .min_w(px(0.0))
                .child(slider_range(
                    t,
                    &format!("{key}-slider"),
                    n,
                    min_f,
                    max_f,
                    disabled,
                    Origin::new(),
                    change.clone(),
                    change,
                ))
                .child(number_field(
                    t,
                    &format!("{key}-field"),
                    n,
                    min_f,
                    max_f,
                    step_f,
                    None,
                    field_change,
                ))
                .into_any_element()
        }
        PropKind::Bool { default } => {
            let on = match value {
                PropValue::Bool(b) => *b,
                _ => *default,
            };
            let inner = {
                let on_change = on_change.clone();
                move |v: bool, window: &mut Window, cx: &mut gpui::App| {
                    on_change(PropValue::Bool(v), window, cx);
                }
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
                // The Effects page shows the option's id (no `optionLabel`);
                // Peripherals passes its words by wrapping this call's
                // closures over ids — this fn always shows words here, so the
                // peripherals caller remaps options before calling through
                // `control_plain`.
                control_plain(t, key, label, kind, value, schema_properties, values, disabled, color_open, select_open, on_change, on_toggle_color, on_toggle_select)
            } else {
                choice_select(t, key, label, options, &current, disabled, select_open, on_toggle_select, on_change).into_any_element()
            }
        }
        PropKind::Color { default } => {
            let hex = match value {
                PropValue::Color(c) => c.to_hex(),
                _ => default.to_hex(),
            };
            color_field(t, key, label, &hex, schema_properties, values, disabled, color_open, on_change, on_toggle_color).into_any_element()
        }
        PropKind::Hue { default } => {
            let deg = match value {
                PropValue::Hue(n) => *n as f32,
                _ => *default as f32,
            };
            hue_control(t, key, deg, disabled, on_change).into_any_element()
        }
        PropKind::Angle { default } => {
            let deg = match value {
                PropValue::Angle(n) => *n as f32,
                PropValue::Hue(n) => *n as f32,
                PropValue::Number(n) => (*n).clamp(0.0, 360.0) as f32,
                _ => *default as f32,
            };
            angle_control(t, key, deg, disabled, on_change).into_any_element()
        }
    }
}

/// A choice's control with the labels the caller wants (Peripherals' words,
/// or the raw ids): `options` are (value, label) pairs.
#[allow(clippy::too_many_arguments)]
pub fn control_words(
    t: &Theme,
    key: &str,
    label: &str,
    value: &PropValue,
    options: &[(String, String)],
    disabled: bool,
    select_open: bool,
    on_change: OnChange,
    on_toggle_select: crate::components::prop_controls::OnToggle,
) -> gpui::AnyElement {
    let current = match value {
        PropValue::Choice(s) => s.clone(),
        _ => options.first().map(|(v, _)| v.clone()).unwrap_or_default(),
    };
    if options.len() <= 4 {
        let refs: Vec<(&str, &str)> = options.iter().map(|(v, l)| (v.as_str(), l.as_str())).collect();
        if disabled {
            segmented_disabled(t, &format!("{key}-seg"), &refs, &current, |_, _, _| {}).into_any_element()
        } else {
            segmented(t, &format!("{key}-seg"), &refs, &current, {
                let on = on_change.clone();
                move |v: String, window: &mut Window, cx: &mut gpui::App| {
                    on(PropValue::Choice(v), window, cx);
                }
            })
            .into_any_element()
        }
    } else {
        let refs: Vec<(&str, &str)> = options.iter().map(|(v, l)| (v.as_str(), l.as_str())).collect();
        let _ = label;
        select_field(
            t,
            &format!("{key}-select"),
            label,
            &refs,
            &current,
            select_open,
            disabled,
            false,
            {
                let toggle = on_toggle_select.clone();
                move |window: &mut Window, cx: &mut gpui::App| toggle(window, cx)
            },
            {
                let on = on_change.clone();
                move |v: String, window: &mut Window, cx: &mut gpui::App| {
                    on(PropValue::Choice(v), window, cx);
                }
            },
        )
    }
}

/// A choice control with the option's id as its label (the Effects page
/// passes no `optionLabel`).
#[allow(clippy::too_many_arguments)]
fn control_plain(
    t: &Theme,
    key: &str,
    label: &str,
    _kind: &PropKind,
    value: &PropValue,
    _schema: &[PropertyDef],
    _values: &Props,
    disabled: bool,
    _color_open: bool,
    select_open: bool,
    on_change: OnChange,
    _toggle_color: crate::components::prop_controls::OnToggle,
    on_toggle_select: crate::components::prop_controls::OnToggle,
) -> gpui::AnyElement {
    let PropKind::Choice { options, default } = _kind else {
        return div().into_any_element();
    };
    let pairs: Vec<(String, String)> = options.iter().map(|o| (o.clone(), o.clone())).collect();
    let _ = default;
    control_words(t, key, label, value, &pairs, disabled, select_open, on_change, on_toggle_select)
}

/// A choice with more than 4 options (`Select`).
#[allow(clippy::too_many_arguments)]
pub fn choice_select(
    t: &Theme,
    key: &str,
    label: &str,
    options: &[String],
    current: &str,
    disabled: bool,
    open: bool,
    on_toggle: crate::components::prop_controls::OnToggle,
    on_change: OnChange,
) -> impl IntoElement {
    let opts: Vec<(String, String)> = options.iter().map(|o| (o.clone(), o.clone())).collect();
    let refs: Vec<(&str, &str)> = opts.iter().map(|(v, l)| (v.as_str(), l.as_str())).collect();
    let _ = label;
    select_field(
        t,
        &format!("{key}-select"),
        label,
        &refs,
        current,
        open,
        disabled,
        false,
        {
            let toggle = on_toggle.clone();
            move |window: &mut Window, cx: &mut gpui::App| toggle(window, cx)
        },
        {
            let on = on_change.clone();
            move |v: String, window: &mut Window, cx: &mut gpui::App| {
                on(PropValue::Choice(v), window, cx);
            }
        },
    )
}

/// A colour field (`ColorField`): the swatch, the hex, and the picker
/// popover with the other colours of the schema as swatches.
#[allow(clippy::too_many_arguments)]
pub fn color_field(
    t: &Theme,
    key: &str,
    label: &str,
    hex: &str,
    schema_properties: &[PropertyDef],
    values: &Props,
    disabled: bool,
    open: bool,
    on_change: OnChange,
    on_toggle_color: crate::components::prop_controls::OnToggle,
) -> impl IntoElement {
    let hex_owned = hex.to_string();
    let swatches = colour_swatches(schema_properties, values, key, hex);
    let _ = label;
    let mut row = div().flex().items_center().gap_2();
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
        let toggle = on_toggle_color.clone();
        swatch.cursor_pointer().on_click(move |_, window, cx| toggle(window, cx))
    };
    row = row.child(swatch).child(
        div()
            .text_sm()
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.dim))
            .child(hex_owned.clone()),
    );
    if open && !disabled {
        row = row.child(color_picker(t, key, hex, &swatches, on_change));
    }
    row
}

/// The picker popover (`ColorPicker`): SV square, hue strip, hex row and the
/// other colours of the schema.
pub fn color_picker(
    t: &Theme,
    key: &str,
    hex: &str,
    swatches: &[(String, String)],
    on_change: OnChange,
) -> impl IntoElement {
    let (hue, sat, val) = hex_to_hsv(hex, 0.0);
    let hue_hex = hsv_to_hex(hue, 1.0, 1.0);
    let hex_owned = hex.to_string();
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
                            let (hr, hg, hb) = {
                                let raw = hue_hex.strip_prefix('#').unwrap_or(&hue_hex);
                                let p = |k: usize| u8::from_str_radix(&raw[k..k + 2], 16).unwrap_or(0) as f32 / 255.0;
                                (p(0), p(2), p(4))
                            };
                            let r = hr + (1.0 - hr) * (1.0 - s);
                            let g = hg + (1.0 - hg) * (1.0 - s);
                            let b = hb + (1.0 - hb) * (1.0 - s);
                            let c = gpui::Rgba { r: r * v, g: g * v, b: b * v, a: 1.0 };
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
        )
        .cursor_pointer()
        .on_click({
            let on = on_change.clone();
            move |_, window, cx| {
                // A click commits the current hex (the popover's sv/hue drags
                // stream into the draft; a GPUI click commits at once).
                if let Some(parsed) = parse_hex(&hex_owned) {
                    on(PropValue::Color(parsed), window, cx);
                }
            }
        });
    picker = picker.child(sv);
    // Hue strip.
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
                        let raw = hsv_to_hex(h, 0.8, 0.62);
                        let s = raw.strip_prefix('#').unwrap_or(&raw);
                        let p = |k: usize| u8::from_str_radix(&s[k..k + 2], 16).unwrap_or(0) as f32 / 255.0;
                        let (r, g, b) = (p(0), p(2), p(4));
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
        .on_click(|_, _, _| {});
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
        for (sw, _sw_label) in swatches {
            let on = on_change.clone();
            let sw_owned = sw.clone();
            row = row.child(
                div()
                    .id(gpui::SharedString::from(format!("{key}-swatch")))
                    .size(px(22.0))
                    .r_sm()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .bg(Theme::color(sw))
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        if let Some(c) = parse_hex(&sw_owned) {
                            on(PropValue::Color(c), window, cx);
                        }
                    }),
            );
        }
        picker = picker.child(row);
    }
    picker
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

/// A hue control (`HueRing` plus degrees field).
pub fn hue_control(t: &Theme, key: &str, deg: f32, disabled: bool, on_change: OnChange) -> impl IntoElement {
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
    let ring = if disabled { ring.opacity(0.5) } else { ring.cursor_pointer() };
    let field = number_field(t, &format!("{key}-degrees"), deg.round(), 0.0, 360.0, 1.0, Some("°"), {
        let on = on_change.clone();
        move |v: f32, window: &mut Window, cx: &mut gpui::App| on(PropValue::Hue(v as f64), window, cx)
    });
    div().flex().items_center().gap_3().child(ring).child(field)
}

/// An angle control (`AngleDial` plus degrees field): 0 points right, 90
/// points down, growing clockwise, as on the canvas.
pub fn angle_control(t: &Theme, key: &str, deg: f32, disabled: bool, on_change: OnChange) -> impl IntoElement {
    let deg_wrapped = wrap_deg(deg as f64) as f32;
    let mute = Theme::color(t.mute);
    let dial_ring = Theme::color(t.dial_ring);
    let ink = Theme::color(t.ink);
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
                    for (x1, y1, x2, y2) in [(95.0, 50.0, 99.0, 50.0), (50.0, 95.0, 50.0, 99.0), (5.0, 50.0, 1.0, 50.0), (50.0, 1.0, 50.0, 5.0)] {
                        let mut b = gpui::PathBuilder::stroke(px(2.0 * s));
                        b.move_to(point(px(cx + (x1 - 50.0) * s), px(cy + (y1 - 50.0) * s)));
                        b.line_to(point(px(cx + (x2 - 50.0) * s), px(cy + (y2 - 50.0) * s)));
                        if let Ok(path) = b.build() {
                            window.paint_path(path, mute);
                        }
                    }
                    let mut ring = gpui::PathBuilder::stroke(px(7.0 * s));
                    ring.add_polygon(&circle_points(point(px(cx), px(cy)), 40.0 * s, 48), true);
                    if let Ok(path) = ring.build() {
                        window.paint_path(path, dial_ring);
                    }
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
    let now = angle_now(deg as f64);
    let field = number_field(t, &format!("{key}-degrees"), now as f32, 0.0, 359.0, 1.0, Some("°"), {
        let on = on_change.clone();
        move |v: f32, window: &mut Window, cx: &mut gpui::App| on(PropValue::Angle(v as f64), window, cx)
    });
    div().flex().items_center().gap_3().child(dial).child(field)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_and_angle_wrap_to_a_turn() {
        assert_eq!(angle_now(0.0), 0);
        assert_eq!(angle_now(359.6), 0);
        assert_eq!(angle_now(90.0), 90);
        assert_eq!(angle_now(-1.0), 359);
        assert_eq!((hue_step(359.0, 3.0) as i32), 2);
        assert_eq!((angle_step(359.0, 1.0) as i32), 0);
    }

    #[test]
    fn hsv_round_trips_through_hex() {
        assert_eq!(hsv_to_hex(0.0, 1.0, 1.0), "#ff0000");
        let (h, s, v) = hex_to_hsv("#ff0000", 0.0);
        assert_eq!((h as i32, (s * 100.0) as i32, (v * 100.0) as i32), (0, 100, 100));
        assert_eq!(hex_to_hsv("#00ff00", 0.0).0 as i32, 120);
        assert_eq!(hex_to_hsv("#0000ff", 0.0).0 as i32, 240);
    }

    #[test]
    fn is_hex_needs_the_daemon_form() {
        assert!(is_hex("#aabbcc"));
        assert!(!is_hex("aabbcc"));
        assert!(!is_hex("#abc"));
        assert!(!is_hex("zz"));
        assert_eq!(parse_hex("#aabbcc").map(|c| c.to_hex()).as_deref(), Some("#aabbcc"));
    }

    #[test]
    fn quantise_step_clamps_and_snaps() {
        assert_eq!(quantise_step(33.4, 0.0, 100.0, 1.0), 33.0);
        assert_eq!(quantise_step(33.6, 0.0, 100.0, 1.0), 34.0);
        assert_eq!(quantise_step(150.0, 0.0, 100.0, 1.0), 100.0);
        assert_eq!(quantise_step(-5.0, 0.0, 100.0, 1.0), 0.0);
    }

    #[test]
    fn decimals_of_steps() {
        assert_eq!(decimals_of(1.0), 0);
        assert_eq!(decimals_of(0.5), 1);
        assert_eq!(decimals_of(0.25), 2);
        assert_eq!(decimals_of(0.1), 1);
    }

    #[test]
    fn format_number_drops_the_point_for_whole_numbers() {
        assert_eq!(format_number(30.0, 0), "30");
        assert_eq!(format_number(30.5, 1), "30.5");
        assert_eq!(format_number(30.55, 2), "30.55");
    }

    fn prop(key: &str, kind: PropKind) -> PropertyDef {
        PropertyDef { key: key.to_string(), label: key.to_string(), kind }
    }

    #[test]
    fn colour_swatches_exclude_the_own_colour() {
        let props = vec![
            prop("ink", PropKind::Color { default: "#ff0000".parse().unwrap() }),
            prop("paper", PropKind::Color { default: "#ffffff".parse().unwrap() }),
            prop("speed", PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 30.0 }),
        ];
        let values: Props = [("ink".to_string(), PropValue::Color("#ff0000".parse().unwrap()))]
            .into_iter()
            .collect();
        let swatches = colour_swatches(&props, &values, "ink", "#ff0000");
        assert_eq!(swatches, vec![("#ffffff".to_string(), "paper".to_string())]);
        // A value that does not parse is skipped, and the own colour is skipped in either case.
        let mut bad = values.clone();
        bad.insert("paper".to_string(), PropValue::Color("#ff0000".parse().unwrap()));
        let swatches = colour_swatches(&props, &bad, "ink", "#FF0000");
        assert!(swatches.is_empty());
    }
}
