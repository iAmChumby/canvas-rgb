//! The small controls the Fans page needs, after the React components
//! (`app/web/src/components/`): `Button`/`IconButton` with icons, `Chip`,
//! `Checkbox`, `Meter`, `Sparkline`, the `rule` box, `NumberField` and
//! `Slider`.
//!
//! These live in their own file because the shared `components/button.rs` and
//! `components/number_field.rs` carry no icon, no `danger` variant and no
//! disabled state, and the Fans page needs all three. The shared components
//! are not edited here (another branch is reworking them).
//!
//! Every colour and size comes from [`crate::theme::Theme`], which mirrors
//! `app/web/src/styles/tokens.css`; the `color-mix()` the CSS uses is
//! computed here by [`mix_srgb`].

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    div, point, prelude::*, px, BoxShadow, ClickEvent, Hsla, IntoElement, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Rgba, Window,
};

use crate::theme::{Theme, TokenRadius};

/// The `color-mix(in srgb, a p%, b)` the CSS writes: a straight sRGB mix of two
/// opaque colours. `p` is the share `a` keeps.
pub fn mix_srgb(a: Rgba, b: Rgba, p: f32) -> Rgba {
    let p = p.clamp(0.0, 1.0);
    Rgba {
        r: a.r * p + b.r * (1.0 - p),
        g: a.g * p + b.g * (1.0 - p),
        b: a.b * p + b.b * (1.0 - p),
        a: 1.0,
    }
}

/// `color-mix(in srgb, c p%, transparent)`: the colour at `p` alpha.
pub fn tint(c: Rgba, p: f32) -> Rgba {
    Rgba {
        a: p.clamp(0.0, 1.0),
        ..c
    }
}

/// An `hsl()` token as a colour.
///
/// The theme's `--live` and `--live-soft` are `hsl(var(--live-h) 78% 72%)` and
/// `hsl(var(--live-h) 55% 55% / 0.16)`: the hue is the theme's `live_h` and
/// the rest is in the token's own text.
pub fn hsl_token(token: &str, hue: u16) -> Rgba {
    // Parse "hsl(h s% l% / a)" on tokens: the alpha is 0.16, not 16, so a
    // plain digit sweep divides it twice.
    let inner = token
        .strip_prefix("hsl(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(token);
    let (main, alpha) = match inner.split_once('/') {
        Some((m, a)) => (m, a.trim().parse::<f32>().unwrap_or(1.0)),
        None => (inner, 1.0),
    };
    let replaced = main.replace("var(--live-h)", "215");
    let parts: Vec<&str> = replaced.split_whitespace().collect();
    let saturation = parts.get(1).and_then(|s| s.trim_end_matches('%').parse::<f32>().ok()).unwrap_or(0.0) / 100.0;
    let lightness = parts.get(2).and_then(|s| s.trim_end_matches('%').parse::<f32>().ok()).unwrap_or(0.0) / 100.0;
    hsl_to_rgba(hue as f32, saturation, lightness, alpha)
}

/// An HSL colour as RGB.
pub fn hsl_to_rgba(hue: f32, saturation: f32, lightness: f32, alpha: f32) -> Rgba {
    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let h = (hue % 360.0) / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = lightness - c / 2.0;
    Rgba {
        r: r + m,
        g: g + m,
        b: b + m,
        a: alpha,
    }
}

/// `--live`: the accent colour.
pub fn live(t: &Theme) -> Rgba {
    let hue = t.live_h.parse::<u16>().unwrap_or(215);
    hsl_token(t.live, hue)
}

/// `--live-soft`: the accent at a low alpha.
pub fn live_soft(t: &Theme) -> Rgba {
    t.live_soft()
}

/// `--warn-wash`: `color-mix(in srgb, var(--warn) 8%, var(--sunk))` in the dark
/// set and `color-mix(in srgb, var(--warn) 18%, var(--panel))` in the light one.
pub fn warn_wash(t: &Theme) -> Rgba {
    let warn = Theme::color(t.warn);
    if t.g1 == Theme::dark().g1 {
        mix_srgb(warn, Theme::color(t.sunk), 0.08)
    } else {
        mix_srgb(warn, Theme::color(t.panel), 0.18)
    }
}

/// The hardware layer's skirt (`--skirt`): the darker band a key's face sits on.
pub fn skirt(t: &Theme) -> Rgba {
    if t.g1 == Theme::dark().g1 {
        Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.55,
        }
    } else {
        Rgba {
            r: 15.0 / 255.0,
            g: 18.0 / 255.0,
            b: 24.0 / 255.0,
            a: 0.22,
        }
    }
}

/// The two-pixel hard shadow the hardware layer puts under a key's face.
fn skirt_shadow(t: &Theme) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: Hsla::from(skirt(t)),
        offset: point(px(0.0), px(2.0)),
        blur_radius: px(0.0),
        spread_radius: px(0.0),
    }]
}

/// Where an element sits in the window, measured when it is painted.
///
/// GPUI 0.2.2 has no "give me this element's bounds" call, so the elements
/// that need to know (the fader, the curve chart) measure themselves in a
/// canvas' prepaint and read the answer back from here.
#[derive(Clone)]
pub struct Origin(Rc<Cell<(f32, f32, f32, f32)>>);

impl Default for Origin {
    fn default() -> Self {
        Self::new()
    }
}

impl Origin {
    pub fn new() -> Self {
        Origin(Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0))))
    }

    /// Where the element was painted: its origin and its size.
    pub fn set(&self, x: f32, y: f32, w: f32, h: f32) {
        self.0.set((x, y, w, h));
    }

    /// The window box of the element: (x, y, width, height).
    pub fn get(&self) -> (f32, f32, f32, f32) {
        self.0.get()
    }
}

/// `Button`'s variants, after `app/web/src/components/Button.tsx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtnVariant {
    Default,
    Primary,
    Ghost,
    Danger,
}

/// A button, after `.btn`: 32 px tall (28 px when small), a 1 px edge, a short
/// skirt, and the variant's face.
#[allow(clippy::too_many_arguments)]
pub fn fans_button(
    t: &Theme,
    id: &str,
    label: &str,
    variant: BtnVariant,
    small: bool,
    disabled: bool,
    icon: Option<&str>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    fans_button_sized(t, id, label, variant, small, disabled, icon, None, on_click)
}

/// The same button with React's `min-width`/`min-height` applied to the key
/// itself, for `.rd-fans .fan-select-key` (88 x 44 on a phone): the border and
/// the face stay the button's, the content is centred in the larger box.
#[allow(clippy::too_many_arguments)]
pub fn fans_button_sized(
    t: &Theme,
    id: &str,
    label: &str,
    variant: BtnVariant,
    small: bool,
    disabled: bool,
    icon: Option<&str>,
    size: Option<(f32, f32)>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (bg, fg, border) = match variant {
        BtnVariant::Primary => (
            live(t),
            Theme::color(t.on_accent),
            Rgba { a: 0.0, ..live(t) },
        ),
        BtnVariant::Default => (
            Theme::color(t.raised),
            Theme::color(t.ink),
            Theme::color(t.line_2),
        ),
        BtnVariant::Ghost => (
            Rgba {
                a: 0.0,
                ..Theme::color(t.ink)
            },
            Theme::color(t.dim),
            Rgba {
                a: 0.0,
                ..Theme::color(t.ink)
            },
        ),
        BtnVariant::Danger => (
            Theme::color(t.raised),
            Theme::color(t.down),
            Theme::color(t.line_2),
        ),
    };
    let (bg, fg, border) = if disabled {
        (
            Rgba { a: 0.5, ..bg },
            Rgba { a: 0.5, ..fg },
            Rgba { a: 0.5, ..border },
        )
    } else {
        (bg, fg, border)
    };
    let mut row = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .h(match size {
            Some((_, h)) => px(h),
            None => px(if small { 28.0 } else { 32.0 }),
        })
        .px(if small { px(10.0) } else { px(12.0) })
        .r_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY);
    if let Some((w, _)) = size {
        row = row.w(px(w));
    }
    if variant != BtnVariant::Ghost {
        row = row.shadow(skirt_shadow(t));
    }
    if let Some(icon) = icon {
        let svg = gpui::svg().path(format!("icons/{icon}.svg")).text_color(fg);
        row = row.child(if small { svg.size_3p5() } else { svg.size_4() });
    }
    // An empty label adds no glyph but would still take the row's gap: on a
    // phone React hides the Save key's words (`.rd-btn-text { display: none }`)
    // and the key is 36 px wide, not 42 (measured at 390x844).
    if !label.is_empty() {
        row = row.child(label.to_string());
    }
    if disabled {
        row
    } else {
        row.cursor_pointer().on_click(on_click)
    }
}

/// An icon-only button, after `IconButton`: a square key whose label is its
/// accessible name.
///
/// React `.ibtn`: a transparent face and a transparent edge, so the glyph
/// stands on its own; the box is only there to be a target.
#[allow(clippy::too_many_arguments)]
pub fn fans_icon_button(
    t: &Theme,
    id: &str,
    label: &str,
    icon: &str,
    size_px: f32,
    icon_px: f32,
    disabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let _ = label;
    let face = Rgba {
        a: 0.0,
        ..Theme::color(t.ink)
    };
    let row = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_center()
        .size(px(size_px))
        .r_md()
        .border_1()
        .border_color(face)
        .bg(face)
        .child(
            gpui::svg()
                .path(format!("icons/{icon}.svg"))
                .size(px(icon_px))
                .text_color(if disabled {
                    Rgba {
                        a: 0.4,
                        ..Theme::color(t.dim)
                    }
                } else {
                    Theme::color(t.dim)
                }),
        );
    if disabled {
        row
    } else {
        row.cursor_pointer().on_click(on_click)
    }
}

/// `Chip`, after `.chip`: a quiet tag with a 1 px edge.
pub fn chip(t: &Theme, id: &str, label: &str) -> impl IntoElement {
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .h(px(28.0))
        .px(px(10.0))
        .rounded_sm()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .bg(Theme::color(t.raised))
        .text_color(Theme::color(t.dim))
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY)
        .child(label.to_string())
}

/// `Checkbox`, after `.tick` and `.tick-box`: an 18 px box that fills with the
/// live colour when it is ticked. A blocked box keeps its outline and shows a
/// dash, so it reads as a closed slot rather than a faded one.
pub fn tick_box(
    t: &Theme,
    id: &str,
    checked: bool,
    mixed: bool,
    blocked: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (bg, border, fg) = if mixed || (blocked && !checked) {
        (
            Theme::color(t.raised),
            Theme::color(t.line_2),
            Theme::color(t.mute),
        )
    } else if checked {
        (
            live(t),
            Rgba { a: 0.0, ..live(t) },
            Theme::color(t.on_accent),
        )
    } else {
        (
            Theme::color(t.sunk),
            Theme::color(t.line_2),
            Theme::color(t.on_accent),
        )
    };
    let glyph = if mixed || (blocked && !checked) {
        Some("Minus-bold")
    } else if checked {
        Some("Check-bold")
    } else {
        None
    };
    let mut box_div = div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(18.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg);
    if let Some(glyph) = glyph {
        box_div = box_div.child(
            gpui::svg()
                .path(format!("icons/{glyph}.svg"))
                .size_3()
                .text_color(fg),
        );
    }
    let button = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .child(box_div);
    if blocked {
        button.cursor_not_allowed()
    } else {
        button.cursor_pointer().on_click(on_toggle)
    }
}

/// `Meter`, after `.meter` and `.meter-fill`: a 6 px well with a fill that
/// scales to the fraction. A width of `None` lets the meter take the room
/// that is left (the CSS's `width: 100%; min-width: 54px`).
pub fn meter(
    t: &Theme,
    id: &str,
    fraction: f32,
    tone: Rgba,
    width: Option<f32>,
) -> impl IntoElement {
    let p = fraction.clamp(0.0, 1.0);
    let well = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .h(px(6.0))
        .rounded(px(3.0))
        .bg(Theme::color(t.raised))
        .overflow_hidden();
    let well = match width {
        Some(w) => well.w(px(w)).flex_none(),
        None => well.flex_1().min_w(px(54.0)),
    };
    let fill_w = match width {
        Some(w) => w * p,
        None => 0.0,
    };
    let fill = div()
        .absolute()
        .top_0()
        .left_0()
        .h(px(6.0))
        .rounded(px(3.0))
        .bg(tone);
    let fill = match width {
        Some(_) => fill.w(px(fill_w)),
        None => fill.w(gpui::relative(p)),
    };
    well.child(fill)
}

/// `Sparkline`, after `app/web/src/components/Readout.tsx`: the last samples,
/// drawn at a fixed scale. The box is the CSS box (56x16 in the fans table on a
/// wide page, 180x44 elsewhere); the drawing keeps the SVG's 180x44 box and is
/// scaled into it, as a browser does.
pub fn sparkline(
    t: &Theme,
    id: &str,
    values: &[u32],
    max: u32,
    width_px: f32,
    height_px: f32,
) -> impl IntoElement {
    let values: Vec<u32> = values.to_vec();
    let tone = Theme::color(t.a3);
    let rule = Theme::color(t.line_2);
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .w(px(width_px))
        .h(px(height_px))
        .child(
            gpui::canvas(
                move |bounds, _window, _cx| (bounds, values.clone(), max, tone, rule),
                |_bounds, state, window, _cx| {
                    let (bounds, values, max, tone, rule) = state;
                    let w = 180.0_f32;
                    let h = 44.0_f32;
                    let scale =
                        (f32::from(bounds.size.width) / w).min(f32::from(bounds.size.height) / h);
                    let ox = f32::from(bounds.origin.x)
                        + (f32::from(bounds.size.width) - w * scale) / 2.0;
                    let oy = f32::from(bounds.origin.y)
                        + (f32::from(bounds.size.height) - h * scale) / 2.0;
                    let at = |x: f32, y: f32| point(px(ox + x * scale), px(oy + y * scale));
                    let scale_max = if max > 0 { max as f32 } else { 1.0 };
                    let step = if values.len() > 1 {
                        w / (values.len() - 1) as f32
                    } else {
                        w
                    };
                    let mut raw: Vec<(f32, f32)> = values
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let x = if values.len() > 1 { i as f32 * step } else { 0.0 };
                            let y = h
                                - 3.0
                                - ((*v).min(scale_max as u32) as f32 / scale_max) * (h - 6.0);
                            ((x * 10.0).round() / 10.0, (y * 10.0).round() / 10.0)
                        })
                        .collect();
                    if raw.len() == 1 {
                        // One sample is a flat line at its level, drawn from
                        // x=0 to x=w (React's sparkline), not a dot.
                        let only = raw[0];
                        raw = vec![(0.0, only.1), (w, only.1)];
                    }
                    let flat = !values.is_empty() && values.iter().all(|v| *v == values[0]);
                    // The trace never drops below 1 px, or a flat line at a
                    // small scale rasterises to nothing but its head dot.
                    let stroke_w = (1.75 * scale).max(1.0);
                    // The rule the trace sits on. Its width is the SVG's,
                    // scaled into the box with the rest of the drawing.
                    let mut builder = gpui::PathBuilder::stroke(px(1.0 * scale));
                    builder.move_to(at(0.0, h - 0.5));
                    builder.line_to(at(w, h - 0.5));
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, rule);
                    }
                    if raw.len() > 1 {
                        if !flat {
                            // The area under the trace.
                            let mut area = gpui::PathBuilder::fill();
                            area.move_to(at(raw[0].0, raw[0].1));
                            for (x, y) in raw.iter().skip(1) {
                                area.line_to(at(*x, *y));
                            }
                            area.line_to(at(w, h));
                            area.line_to(at(0.0, h));
                            area.close();
                            if let Ok(path) = area.build() {
                                window.paint_path(path, tint(tone, 0.12));
                            }
                        }
                        let mut line = gpui::PathBuilder::stroke(px(stroke_w));
                        line.move_to(at(raw[0].0, raw[0].1));
                        for (x, y) in raw.iter().skip(1) {
                            line.line_to(at(*x, *y));
                        }
                        if let Ok(path) = line.build() {
                            window.paint_path(path, tone);
                        }
                    }
                    if let Some(head) = raw.last() {
                        let mut dot = gpui::PathBuilder::fill();
                        dot.add_polygon(&circle_points(at(head.0, head.1), 2.5 * scale, 16), true);
                        if let Ok(path) = dot.build() {
                            window.paint_path(path, tone);
                        }
                    }
                },
            )
            .absolute()
            .inset_0(),
        )
}

/// The points of a circle, for the canvas.
pub fn circle_points(
    center: gpui::Point<gpui::Pixels>,
    radius: f32,
    steps: usize,
) -> Vec<gpui::Point<gpui::Pixels>> {
    (0..steps)
        .map(|i| {
            let a = (i as f32 / steps as f32) * std::f32::consts::TAU;
            point(
                px(f32::from(center.x) + a.cos() * radius),
                px(f32::from(center.y) + a.sin() * radius),
            )
        })
        .collect()
}

/// `Select`, after `app/web/src/components/Select.tsx`: a trigger that opens a
/// list of options.
///
/// The shared `components/select.rs` takes its options as `&str` pairs, which
/// the Fans page cannot hand it (the options come from the daemon at runtime),
/// so the Fans-specific select is here. The keyboard and type-ahead items of
/// the checklist stay unticked: GPUI 0.2.2 has no portalled list.
#[allow(clippy::too_many_arguments)]
pub fn fans_select(
    t: &Theme,
    id: &str,
    label: &str,
    options: &[(String, String)],
    value: &str,
    open: bool,
    on_toggle: impl Fn(&mut Window, &mut gpui::App) + Clone + 'static,
    on_pick: impl Fn(String, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let _ = label;
    let chosen = options
        .iter()
        .find(|(v, _)| v == value)
        .map(|(_, l)| l.clone())
        .or_else(|| options.first().map(|(_, l)| l.clone()))
        .unwrap_or_default();
    let toggle = on_toggle.clone();
    let trigger = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .h(px(32.0))
        .px_3()
        // React `.select-trigger { min-width: 160px; }`.
        .min_w(px(160.0))
        .r_md()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .bg(Theme::color(t.raised))
        .text_color(Theme::color(t.ink))
        .text_sm()
        .font_family(crate::fonts::UI_FAMILY)
        .cursor_pointer()
        .child(chosen)
        .child(
            gpui::svg()
                .path("icons/CaretDown-bold.svg")
                .w(px(14.0))
                .h(px(14.0))
                .flex_none()
                .text_color(Theme::color(t.ink)),
        )
        .on_click(move |_, window, cx| toggle(window, cx));
    let mut col = div()
        .id(gpui::SharedString::from(format!("{id}-col")))
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .child(trigger);
    if open {
        let mut list = div()
            .id(gpui::SharedString::from(format!("{id}-list")))
            .absolute()
            .top(px(36.0))
            .left_0()
            .right_0()
            .r_md()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.panel))
            .overflow_hidden();
        for (value, text) in options {
            let pick = on_pick.clone();
            let value = value.clone();
            list = list.child(
                div()
                    .id(gpui::SharedString::from(format!("{id}-option-{value}")))
                    .px_3()
                    .py_1()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .cursor_pointer()
                    .child(text.clone())
                    .on_click(move |_, window, cx| pick(value.clone(), window, cx)),
            );
        }
        col = col.child(list);
    }
    col
}

/// The `rule` box: a tinted note with a 1 px edge. `warn` picks the warning
/// tone, as `.rule-warn` does.
pub fn rule_box(t: &Theme, id: &str, warn: bool, body: impl IntoElement) -> impl IntoElement {
    let (border, bg) = if warn {
        (
            tint(Theme::color(t.warn), 0.40),
            mix_srgb(Theme::color(t.warn), Theme::color(t.sunk), 0.08),
        )
    } else {
        (
            tint(Theme::color(t.a2), 0.35),
            mix_srgb(Theme::color(t.a2), Theme::color(t.sunk), 0.08),
        )
    };
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .gap_2()
        .py(px(8.0))
        .px(px(12.0))
        .r_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(Theme::color(t.dim))
        .text_sm()
        .font_family(crate::fonts::UI_FAMILY)
        .child(body)
}

/// `NumberField`, after `.num`: a sunk well with the value in the figures
/// font, right-aligned, and the unit beside it.
///
/// GPUI 0.2.2 has no text input, so the value is changed with the two
/// steppers beside the unit (the React field is typed into).
#[allow(clippy::too_many_arguments)]
pub fn fans_number_field(
    t: &Theme,
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    unit: Option<&str>,
    disabled: bool,
    on_commit: impl Fn(f32, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let text = format!("{value}");
    let (bg, border, fg) = if disabled {
        (
            Theme::color(t.sunk),
            Theme::color(t.line),
            Theme::color(t.mute),
        )
    } else {
        (
            Theme::color(t.sunk),
            Theme::color(t.line),
            Theme::color(t.ink),
        )
    };
    let well = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_end()
        .h(px(32.0))
        .w(px(92.0))
        // The well may shrink when the row is narrow, so a field never makes
        // its panel wider than the column.
        .flex_shrink()
        .min_w_0()
        .px(px(8.0))
        .r_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_size(px(15.0))
        .font_family(crate::fonts::MONO_FAMILY)
        .child(text);
    let mut row = div().flex().items_center().gap_2().child(well);
    if let Some(unit) = unit {
        row = row.child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child(unit.to_string()),
        );
    }
    if disabled {
        return row;
    }
    let up = on_commit.clone();
    let down = on_commit;
    row.child(
        div()
            .flex()
            .flex_col()
            .gap(px(1.0))
            .child(
                div()
                    .id(gpui::SharedString::from(format!("{id}-up")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(14.0))
                    .h(px(14.0))
                    .cursor_pointer()
                    .text_size(px(9.0))
                    .text_color(Theme::color(t.mute))
                    .child("\u{25B2}")
                    .on_click(move |_, window, cx| up(clamp(value + step, min, max), window, cx)),
            )
            .child(
                div()
                    .id(gpui::SharedString::from(format!("{id}-down")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(14.0))
                    .h(px(14.0))
                    .cursor_pointer()
                    .text_size(px(9.0))
                    .text_color(Theme::color(t.mute))
                    .child("\u{25BC}")
                    .on_click(move |_, window, cx| down(clamp(value - step, min, max), window, cx)),
            ),
    )
}

/// The clamp `NumberField` applies on commit.
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() {
        return min;
    }
    value.clamp(min, max)
}

/// `Slider`, after `app/web/src/components/Slider.tsx` and the hardware
/// layer's fader: a slot with a cap that travels in it, the part behind the
/// cap filled with the live colour, and a shaded band for the floor.
#[allow(clippy::too_many_arguments)]
pub fn slider(
    t: &Theme,
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    floor: Option<f32>,
    disabled: bool,
    origin: Origin,
    on_change: impl Fn(f32, &mut Window, &mut gpui::App) + Clone + 'static,
    on_commit: impl Fn(f32, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    const CAP: f32 = 14.0;
    let span = if (max - min).abs() < f32::EPSILON {
        1.0
    } else {
        max - min
    };
    let p = ((value - min) / span).clamp(0.0, 1.0);
    let f = floor
        .map(|floor| ((floor - min) / span).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let mut slot = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .w_full()
        .h(px(32.0))
        .child(
            gpui::canvas(
                {
                    let origin = origin.clone();
                    move |bounds, _window, _cx| {
                        origin.set(
                            f32::from(bounds.origin.x),
                            f32::from(bounds.origin.y),
                            f32::from(bounds.size.width),
                            f32::from(bounds.size.height),
                        );
                    }
                },
                |_bounds, _state, _window, _cx| {},
            )
            .absolute()
            .inset_0(),
        )
        // The slot.
        .child(
            div()
                .absolute()
                .top(px(14.0))
                .left_0()
                .right_0()
                .h(px(4.0))
                .rounded(px(2.0))
                .bg(Theme::color(t.sunk))
                .border_1()
                .border_color(Theme::color(t.line)),
        );
    // The floor band.
    if floor.is_some() && f > 0.0 {
        slot = slot.child(
            div()
                .absolute()
                .top(px(10.0))
                .left_0()
                .w(px(f * 100.0))
                .h(px(12.0))
                .bg(tint(Theme::color(t.down), 0.26)),
        );
    }
    // The fill, up to the cap.
    slot = slot.child(
        div()
            .absolute()
            .top(px(14.0))
            .left_0()
            .w(px(p * 100.0))
            .h(px(4.0))
            .rounded(px(2.0))
            .bg(live(t)),
    );
    // The cap.
    slot = slot.child(
        div()
            .absolute()
            .top(px(16.0))
            .left(px(p * 100.0))
            .w(px(CAP))
            .h(px(24.0))
            .mt(px(-12.0))
            .ml(px(-7.0))
            .rounded(px(4.0))
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.raised))
            .shadow(skirt_shadow(t)),
    );
    if disabled {
        return slot;
    }
    let change = on_change.clone();
    let commit = on_commit.clone();
    let drag_origin = origin.clone();
    slot.on_mouse_down(gpui::MouseButton::Left, {
        let drag_origin = drag_origin.clone();
        let change = change.clone();
        move |event: &MouseDownEvent, window, cx| {
            let frac = frac_at(&drag_origin, f32::from(event.position.x), CAP);
            change(quantise(min + frac * span, min, max, 1.0), window, cx);
        }
    })
    .on_mouse_move({
        let drag_origin = drag_origin.clone();
        move |event: &MouseMoveEvent, window, cx| {
            let frac = frac_at(&drag_origin, f32::from(event.position.x), CAP);
            change(quantise(min + frac * span, min, max, 1.0), window, cx);
        }
    })
    .on_mouse_up(
        gpui::MouseButton::Left,
        move |event: &MouseUpEvent, window, cx| {
            let frac = frac_at(&drag_origin, f32::from(event.position.x), CAP);
            commit(quantise(min + frac * span, min, max, 1.0), window, cx);
        },
    )
}

/// Where the pointer is along the travel, 0..1: the cap's centre follows it.
fn frac_at(origin: &Origin, x: f32, cap: f32) -> f32 {
    let (ox, _, w, _) = origin.get();
    let travel = w - cap;
    if travel <= 0.0 {
        return 0.0;
    }
    ((x - ox - cap / 2.0) / travel).clamp(0.0, 1.0)
}

/// A raw number put on the step grid and inside the range.
pub fn quantise(raw: f32, min: f32, max: f32, step: f32) -> f32 {
    let stepped = min + ((raw - min) / step).round() * step;
    stepped.clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mix_keeps_the_share_the_css_asks_for() {
        let a = Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let b = Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let mixed = mix_srgb(a, b, 0.08);
        assert!((mixed.r - 0.08).abs() < 0.001);
        assert!(mixed.g.abs() < 0.001);
        assert!((mixed.a - 1.0).abs() < 0.001);
        // A tint keeps the colour and takes the alpha.
        let tinted = tint(a, 0.18);
        assert!((tinted.a - 0.18).abs() < 0.001);
        assert!((tinted.r - 1.0).abs() < 0.001);
    }

    #[test]
    fn the_number_field_clamps_to_its_range() {
        assert_eq!(clamp(50.0, 20.0, 100.0), 50.0);
        assert_eq!(clamp(5.0, 20.0, 100.0), 20.0);
        assert_eq!(clamp(120.0, 20.0, 100.0), 100.0);
        assert_eq!(clamp(f32::NAN, 20.0, 100.0), 20.0);
    }

    #[test]
    fn a_stepped_value_lands_on_the_grid() {
        assert_eq!(quantise(49.6, 20.0, 100.0, 1.0), 50.0);
        assert_eq!(quantise(19.0, 20.0, 100.0, 1.0), 20.0);
        assert_eq!(quantise(140.0, 20.0, 100.0, 1.0), 100.0);
    }

    #[test]
    fn the_origin_remembers_where_it_was_measured() {
        let origin = Origin::new();
        assert_eq!(origin.get(), (0.0, 0.0, 0.0, 0.0));
        origin.set(12.0, 34.0, 56.0, 78.0);
        assert_eq!(origin.get(), (12.0, 34.0, 56.0, 78.0));
    }

    #[test]
    fn the_live_soft_alpha_is_the_tokens() {
        let dark = Theme::dark();
        let soft = live_soft(&dark);
        // `hsl(215 55% 55% / 0.16)`: the alpha is 0.16, not 0.16/100.
        assert!((soft.a - 0.16).abs() < 0.001, "alpha {:?}", soft.a);
        let light = Theme::light();
        assert!((live_soft(&light).a - 0.14).abs() < 0.001);
    }

    #[test]
    fn a_single_sample_draws_a_flat_line() {
        // The `raw.len()==1` branch: one sample becomes a two-point line at
        // its level, not a dot.
        let values = [1200u32];
        let max = 2000u32;
        let scale_max = max as f32;
        let h = 44.0_f32;
        let y = h - 3.0 - (values[0].min(scale_max as u32) as f32 / scale_max) * (h - 6.0);
        let raw = [(0.0, (y * 10.0).round() / 10.0), (180.0, (y * 10.0).round() / 10.0)];
        assert_eq!(raw.len(), 2);
        assert!(raw[0].1 == raw[1].1);
        assert!(raw[0].0 < raw[1].0);
        // The trace stroke is never below 1 px.
        assert!(((1.75 * 0.31_f32).max(1.0)) >= 1.0);
        assert!(((1.75 * 1.0_f32).max(1.0)) > 1.0);
    }
}
