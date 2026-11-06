//! `FanCurveEditor`, after `app/web/src/views/FanCurveEditor.tsx`.
//!
//! A 2 to 8 point curve on a temperature by percent grid. The shaded band at
//! the bottom is the floor. From the source's hot limit (issue #130: the
//! user's "don't exceed" line, drawn here and draggable) a manual header is
//! raised to its hot floor, so that band is drawn from the line. Past the
//! fixed safety ceiling a hatched band says every manual header runs at full
//! speed. Points drag, or take the arrow keys.
//!
//! The drawing is GPUI's canvas (paths), the labels are ordinary elements
//! placed over it. The geometry, the maths and the words are the React ones:
//! the same margins, the same grid, the same bands.
//!
//! One deviation: GPUI 0.2.2 cannot turn text, so the "Full speed" caption
//! stands up as a column of letters in the hatch instead of a turned line.

use std::cell::Cell;
use std::rc::Rc;

use canvas_proto::fans::{CurvePoint, HotKind, HotLimitStatus};
use gpui::{
    div, point, prelude::*, px, IntoElement, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Window,
};

use crate::components::fan_bits::{circle_points, live, tint};
use crate::theme::Theme;

pub const TEMP_MIN: f32 = 20.0;
pub const TEMP_MAX: f32 = 100.0;
pub const MARGIN_LEFT: f32 = 48.0;
pub const MARGIN_RIGHT: f32 = 14.0;
pub const MARGIN_TOP: f32 = 14.0;
pub const MARGIN_BOTTOM: f32 = 46.0;
pub const ASPECT: f32 = 0.62;
/// The hot line may not be set below this.
pub const HOT_MIN: f32 = 40.0;

/// The plot's box inside the chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geom {
    pub width: f32,
    pub height: f32,
    pub plot_w: f32,
    pub plot_h: f32,
}

/// The chart's box for a width, the way `FanCurveEditor` measures it: the
/// height is the width times 0.62, rounded, and the plot sits inside the
/// margins.
pub fn curve_geometry(width: f32) -> Geom {
    let width = width.max(260.0);
    let height = (width * ASPECT).round();
    Geom {
        width,
        height,
        plot_w: width - MARGIN_LEFT - MARGIN_RIGHT,
        plot_h: height - MARGIN_TOP - MARGIN_BOTTOM,
    }
}

/// A temperature's x in the plot.
pub fn x_of(geom: &Geom, temp: f32) -> f32 {
    MARGIN_LEFT + ((temp - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)) * geom.plot_w
}

/// A percent's y in the plot.
pub fn y_of(geom: &Geom, percent: f32) -> f32 {
    MARGIN_TOP + (1.0 - percent / 100.0) * geom.plot_h
}

/// The temperature an x stands for.
pub fn temp_of_x(geom: &Geom, x: f32) -> f32 {
    TEMP_MIN + ((x - MARGIN_LEFT) / geom.plot_w) * (TEMP_MAX - TEMP_MIN)
}

/// The percent a y stands for.
pub fn pct_of_y(geom: &Geom, y: f32) -> f32 {
    (1.0 - (y - MARGIN_TOP) / geom.plot_h) * 100.0
}

/// The points sorted by temperature, the way the daemon evaluates them.
pub fn sorted_points(points: &[CurvePoint]) -> Vec<CurvePoint> {
    let mut out = points.to_vec();
    out.sort_by(|a, b| {
        a.temp_c
            .partial_cmp(&b.temp_c)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// The duty a curve asks for at a temperature: the same evaluation the daemon
/// does (flat outside, linear between).
pub fn duty_for(points: &[CurvePoint], temp: Option<f32>) -> Option<f32> {
    let temp = temp?;
    if points.is_empty() {
        return None;
    }
    let sorted = sorted_points(points);
    let first = sorted.first()?;
    let last = sorted.last()?;
    if temp <= first.temp_c {
        return Some(f32::from(first.percent));
    }
    if temp >= last.temp_c {
        return Some(f32::from(last.percent));
    }
    for i in 1..sorted.len() {
        let a = sorted[i - 1];
        let b = sorted[i];
        if temp <= b.temp_c {
            let f = if b.temp_c == a.temp_c {
                0.0
            } else {
                (temp - a.temp_c) / (b.temp_c - a.temp_c)
            };
            return Some(f32::from(a.percent) + (f32::from(b.percent) - f32::from(a.percent)) * f);
        }
    }
    Some(f32::from(last.percent))
}

/// The curve a header starts on, after `defaultCurve` in `FansView.tsx`.
pub fn default_curve(floor: u8) -> Vec<CurvePoint> {
    vec![
        CurvePoint {
            temp_c: 30.0,
            percent: floor.max(30),
        },
        CurvePoint {
            temp_c: 50.0,
            percent: floor.max(45),
        },
        CurvePoint {
            temp_c: 70.0,
            percent: floor.max(70),
        },
        CurvePoint {
            temp_c: 85.0,
            percent: 100,
        },
    ]
}

/// The temperature a point may take: between its neighbours, a degree apart.
pub fn bounds_for(points: &[CurvePoint], index: usize) -> (f32, f32) {
    let mut lo = TEMP_MIN;
    let mut hi = TEMP_MAX;
    let Some(me) = points.get(index) else {
        return (lo, hi);
    };
    for (j, p) in points.iter().enumerate() {
        if j == index {
            continue;
        }
        if p.temp_c <= me.temp_c {
            lo = lo.max(p.temp_c + 1.0);
        } else {
            hi = hi.min(p.temp_c - 1.0);
        }
    }
    (lo, hi)
}

/// The lowest percent a header may be set to (`fan-rules.ts`'s `baseFloor`).
pub fn base_floor(header: &str, configured: u8) -> u8 {
    let floor = configured.max(canvas_proto::fans::DEFAULT_FLOOR_PERCENT);
    if header == canvas_proto::fans::CPU_FAN_HEADER {
        floor.max(canvas_proto::fans::CPU_FAN_MIN_PERCENT)
    } else if header.starts_with(canvas_proto::fans::GPU_FAN_HEADER_PREFIX) {
        floor.max(canvas_proto::fans::GPU_FAN_MIN_PERCENT)
    } else {
        floor
    }
}

/// The floor a manual header is raised to while its curve temperature is at or
/// above the hot line (`fan-rules.ts`'s `hotFloor`).
pub fn hot_floor(header: &str, configured: u8) -> u8 {
    let base = base_floor(header, configured);
    if header == canvas_proto::fans::CPU_FAN_HEADER {
        base.max(canvas_proto::fans::HOT_CPU_FAN_FLOOR_PERCENT)
    } else {
        base.max(canvas_proto::fans::HOT_FLOOR_PERCENT)
    }
}

/// G4's rule, with the numbers filled in (`fan-rules.ts`'s `hotRuleText`).
pub fn hot_rule_text(limit_c: Option<f32>) -> String {
    let line = match limit_c {
        None => "its hot limit".to_string(),
        Some(c) => format!("its hot limit of {c} {}", "\u{00B0}C"),
    };
    format!(
        "Below {}% (CPU_FAN {}%) is raised at or above {line}",
        canvas_proto::fans::HOT_FLOOR_PERCENT,
        canvas_proto::fans::HOT_CPU_FAN_FLOOR_PERCENT
    )
}

/// The hot row of a header's source kind.
pub fn hot_row(hot: &[HotLimitStatus], kind: HotKind) -> Option<&HotLimitStatus> {
    hot.iter().find(|h| h.kind == kind)
}

/// The chart's box in the window, measured when it is painted. GPUI 0.2.2 has
/// no "give me this element's bounds" call, so the chart measures itself.
#[derive(Clone)]
pub struct ChartBox(Rc<Cell<(f32, f32, f32, f32)>>);

impl Default for ChartBox {
    fn default() -> Self {
        Self::new()
    }
}

impl ChartBox {
    pub fn new() -> Self {
        ChartBox(Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0))))
    }

    pub fn set(&self, x: f32, y: f32, w: f32, h: f32) {
        self.0.set((x, y, w, h));
    }

    /// The chart's box in the window: (x, y, width, height).
    pub fn get(&self) -> (f32, f32, f32, f32) {
        self.0.get()
    }

    /// The chart's width, as measured.
    pub fn width(&self) -> f32 {
        self.0.get().2
    }
}

/// Everything the chart draws, gathered so the paint callback can take it.
struct ChartPaint {
    geom: Geom,
    points: Vec<CurvePoint>,
    floor: u8,
    hot_floor: u8,
    current_temp: Option<f32>,
    current_duty: Option<f32>,
    hot: Option<HotLimitStatus>,
    hot_value: f32,
    line: gpui::Rgba,
    down: gpui::Rgba,
    warn: gpui::Rgba,
    ink: gpui::Rgba,
    live: gpui::Rgba,
    a2: gpui::Rgba,
}

/// One hatch line of the ceiling band: the segment of `y = x + c` inside the
/// band.
fn hatch_segment(x0: f32, y0: f32, x1: f32, y1: f32, c: f32) -> Option<(f32, f32, f32, f32)> {
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let yl = x0 + c;
    if yl >= y0 && yl <= y1 {
        pts.push((x0, yl));
    }
    let yr = x1 + c;
    if yr >= y0 && yr <= y1 {
        pts.push((x1, yr));
    }
    let xt = y0 - c;
    if xt >= x0 && xt <= x1 {
        pts.push((xt, y0));
    }
    let xb = y1 - c;
    if xb >= x0 && xb <= x1 {
        pts.push((xb, y1));
    }
    if pts.len() < 2 {
        return None;
    }
    Some((pts[0].0, pts[0].1, pts[1].0, pts[1].1))
}

/// The chart itself.
#[allow(clippy::too_many_arguments)]
pub fn fan_curve_chart(
    t: &Theme,
    id: &str,
    points: &[CurvePoint],
    floor: u8,
    current_temp: Option<f32>,
    current_duty: Option<f32>,
    hot: Option<HotLimitStatus>,
    hot_value: Option<f32>,
    header_id: &str,
    box_measure: ChartBox,
    on_point_down: impl Fn(usize, &mut Window, &mut gpui::App) + Clone + 'static,
    on_point_move: impl Fn(f32, f32, &mut Window, &mut gpui::App) + Clone + 'static,
    on_point_up: impl Fn(f32, f32, &mut Window, &mut gpui::App) + Clone + 'static,
    on_hot_down: impl Fn(&mut Window, &mut gpui::App) + Clone + 'static,
    on_hot_move: impl Fn(f32, &mut Window, &mut gpui::App) + Clone + 'static,
    on_hot_up: impl Fn(f32, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let measured = box_measure.width();
    let geom = curve_geometry(if measured > 0.0 { measured } else { 360.0 });
    let hot_value = hot_value
        .or_else(|| hot.map(|h| h.limit_c))
        .unwrap_or(canvas_proto::fans::HOT_TEMP_C);
    let sorted = sorted_points(points);

    let paint = ChartPaint {
        geom,
        points: sorted.clone(),
        floor,
        hot_floor: hot_floor(header_id, floor),
        current_temp,
        current_duty,
        hot,
        hot_value,
        line: Theme::color(t.line),
        down: Theme::color(t.down),
        warn: Theme::color(t.warn),
        ink: Theme::color(t.ink),
        live: live(t),
        a2: Theme::color(t.a2),
    };

    let mut chart = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .w_full()
        .max_w(px(520.0))
        .h(px(geom.height));

    // The drawing.
    chart = chart.child(
        gpui::canvas(
            {
                let box_measure = box_measure.clone();
                move |bounds, _window, _cx| {
                    box_measure.set(
                        f32::from(bounds.origin.x),
                        f32::from(bounds.origin.y),
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                    );
                }
            },
            move |bounds, _state, window, _cx| {
                let p = &paint;
                let g = p.geom;
                // The canvas paints in window coordinates: the plot's origin is
                // the canvas' own.
                let ox = f32::from(bounds.origin.x);
                let oy = f32::from(bounds.origin.y);
                let at = |x: f32, y: f32| point(px(ox + x), px(oy + y));
                let plot_right = MARGIN_LEFT + g.plot_w;
                let plot_bottom = MARGIN_TOP + g.plot_h;
                // The percent grid.
                for pct in [0.0_f32, 20.0, 40.0, 60.0, 80.0, 100.0] {
                    let y = y_of(&g, pct);
                    let mut b = gpui::PathBuilder::stroke(px(1.0));
                    b.move_to(at(MARGIN_LEFT, y));
                    b.line_to(at(plot_right, y));
                    if let Ok(path) = b.build() {
                        window.paint_path(path, p.line);
                    }
                }
                // The temperature grid.
                for temp in [20.0_f32, 40.0, 60.0, 80.0, 100.0] {
                    let x = x_of(&g, temp);
                    let mut b = gpui::PathBuilder::stroke(px(1.0));
                    b.move_to(at(x, MARGIN_TOP));
                    b.line_to(at(x, plot_bottom));
                    if let Ok(path) = b.build() {
                        window.paint_path(path, p.line);
                    }
                }
                // The floor band.
                let floor_top = y_of(&g, f32::from(p.floor));
                let mut band = gpui::PathBuilder::fill();
                band.move_to(at(MARGIN_LEFT, floor_top));
                band.line_to(at(plot_right, floor_top));
                band.line_to(at(plot_right, plot_bottom));
                band.line_to(at(MARGIN_LEFT, plot_bottom));
                band.close();
                if let Ok(path) = band.build() {
                    window.paint_path(path, tint(p.down, 0.18));
                }
                // The safety ceiling: a hatched band from the ceiling to the right.
                if let Some(hot) = p.hot {
                    let ceil = hot.ceiling_c.clamp(TEMP_MIN, TEMP_MAX);
                    let cx = x_of(&g, ceil);
                    let mut c = gpui::PathBuilder::stroke(px(1.5));
                    let mut k = cx - plot_bottom;
                    while k <= plot_right - MARGIN_TOP {
                        if let Some((x1, y1, x2, y2)) =
                            hatch_segment(cx, MARGIN_TOP, plot_right, plot_bottom, k)
                        {
                            c.move_to(at(x1, y1));
                            c.line_to(at(x2, y2));
                        }
                        k += 8.0;
                    }
                    if let Ok(path) = c.build() {
                        window.paint_path(path, tint(p.down, 0.45));
                    }
                    let mut dashed =
                        gpui::PathBuilder::stroke(px(1.0)).dash_array(&[px(2.0), px(3.0)]);
                    dashed.move_to(at(cx, MARGIN_TOP));
                    dashed.line_to(at(cx, plot_bottom));
                    if let Ok(path) = dashed.build() {
                        window.paint_path(path, p.down);
                    }
                }
                // The hot floor band and its line.
                let hot_left = x_of(&g, p.hot_value);
                let hot_top = y_of(&g, f32::from(p.hot_floor));
                let mut band = gpui::PathBuilder::fill();
                band.move_to(at(hot_left, hot_top));
                band.line_to(at(plot_right, hot_top));
                band.line_to(at(plot_right, plot_bottom));
                band.line_to(at(hot_left, plot_bottom));
                band.close();
                if let Ok(path) = band.build() {
                    window.paint_path(path, tint(p.warn, 0.16));
                }
                let mut hot_line =
                    gpui::PathBuilder::stroke(px(1.0)).dash_array(&[px(4.0), px(4.0)]);
                hot_line.move_to(at(hot_left, MARGIN_TOP));
                hot_line.line_to(at(hot_left, plot_bottom));
                if let Ok(path) = hot_line.build() {
                    window.paint_path(path, p.warn);
                }
                // The current temperature.
                if let Some(current) = p.current_temp {
                    let cx = x_of(&g, current.clamp(TEMP_MIN, TEMP_MAX));
                    let mut dashed =
                        gpui::PathBuilder::stroke(px(1.0)).dash_array(&[px(2.0), px(3.0)]);
                    dashed.move_to(at(cx, MARGIN_TOP));
                    dashed.line_to(at(cx, plot_bottom));
                    if let Ok(path) = dashed.build() {
                        window.paint_path(path, p.ink);
                    }
                }
                // The current duty.
                if let (Some(current), Some(duty)) = (p.current_temp, p.current_duty) {
                    let cx = x_of(&g, current.clamp(TEMP_MIN, TEMP_MAX));
                    let cy = y_of(&g, duty.clamp(0.0, 100.0));
                    let mut dot = gpui::PathBuilder::fill();
                    dot.add_polygon(&circle_points(at(cx, cy), 6.0, 32), true);
                    if let Ok(path) = dot.build() {
                        window.paint_path(path, p.live);
                    }
                    let mut ring = gpui::PathBuilder::stroke(px(1.5));
                    ring.add_polygon(&circle_points(at(cx, cy), 10.0, 32), true);
                    if let Ok(path) = ring.build() {
                        window.paint_path(path, tint(p.live, 0.5));
                    }
                }
                // The curve.
                if p.points.len() > 1 {
                    let mut line = gpui::PathBuilder::stroke(px(2.5));
                    line.move_to(at(
                        x_of(&g, p.points[0].temp_c),
                        y_of(&g, f32::from(p.points[0].percent)),
                    ));
                    for point in p.points.iter().skip(1) {
                        line.line_to(at(
                            x_of(&g, point.temp_c),
                            y_of(&g, f32::from(point.percent)),
                        ));
                    }
                    if let Ok(path) = line.build() {
                        window.paint_path(path, p.a2);
                    }
                }
                // The points.
                for point in &p.points {
                    let cx = x_of(&g, point.temp_c);
                    let cy = y_of(&g, f32::from(point.percent));
                    let mut dot = gpui::PathBuilder::fill();
                    dot.add_polygon(&circle_points(at(cx, cy), 7.0, 32), true);
                    if let Ok(path) = dot.build() {
                        window.paint_path(path, p.a2);
                    }
                }
            },
        )
        .absolute()
        .inset_0(),
    );

    // The labels over the drawing.
    let plot_right = MARGIN_LEFT + geom.plot_w;
    let plot_bottom = MARGIN_TOP + geom.plot_h;
    for pct in [0.0_f32, 20.0, 40.0, 60.0, 80.0, 100.0] {
        let y = y_of(&geom, pct);
        chart = chart.child(
            div()
                .absolute()
                .top(px(y - 8.0))
                .left_0()
                .w(px(MARGIN_LEFT - 8.0))
                .h(px(16.0))
                .flex()
                .items_center()
                .justify_end()
                .text_sm()
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.mute))
                .child(format!("{pct}%")),
        );
    }
    for temp in [20.0_f32, 40.0, 60.0, 80.0, 100.0] {
        let x = x_of(&geom, temp);
        chart = chart.child(
            div()
                .absolute()
                .top(px(plot_bottom + 9.0))
                .left(px(x - 20.0))
                .w(px(40.0))
                .text_sm()
                .text_center()
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.mute))
                .child(format!("{temp}")),
        );
    }
    // The axis caption.
    chart = chart.child(
        div()
            .absolute()
            .top(px(plot_bottom + 29.0))
            .left(px(plot_right - 140.0))
            .w(px(140.0))
            .text_sm()
            .text_right()
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.mute))
            .child("temperature \u{00B0}C"),
    );
    // The hot line's figure, at the top of the plot.
    let hot_left = x_of(&geom, hot_value);
    let short = format!("{} {}", hot_value.round() as i32, "\u{00B0}C");
    let char_w = 8.5;
    let text_w = short.chars().count() as f32 * char_w;
    let (place_x, right_aligned) = if hot_left - 6.0 - text_w >= MARGIN_LEFT {
        (hot_left - 6.0, true)
    } else if hot_left + 6.0 + text_w <= plot_right {
        (hot_left + 6.0, false)
    } else {
        (plot_right, true)
    };
    chart = chart.child(
        div()
            .absolute()
            .top(px(MARGIN_TOP + 1.0))
            .left(px(place_x - if right_aligned { text_w } else { 0.0 }))
            .w(px(text_w.max(1.0)))
            .text_sm()
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.warn))
            .child(short),
    );
    // The ceiling's caption, standing up in the hatch.
    if let Some(hot) = hot {
        let ceil = hot.ceiling_c.clamp(TEMP_MIN, TEMP_MAX);
        let cx = x_of(&geom, ceil);
        let label_x = if cx + 16.0 <= plot_right {
            cx + 16.0
        } else {
            cx - 6.0
        };
        let mut stack = div()
            .absolute()
            .top(px(plot_bottom - 98.0))
            .left(px(label_x))
            .flex()
            .flex_col_reverse()
            .items_center()
            .text_size(px(13.0))
            .line_height(px(9.0))
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.down));
        for ch in "Full speed".chars() {
            stack = stack.child(ch.to_string());
        }
        chart = chart.child(stack);
    }

    // The hot line's handle: a strip over the dashed line.
    if hot.is_some() {
        let hot_down = on_hot_down.clone();
        chart = chart.child(
            div()
                .id(gpui::SharedString::from(format!("{id}-hot-handle")))
                .absolute()
                .top(px(MARGIN_TOP))
                .left(px(hot_left - 7.0))
                .w(px(14.0))
                .h(px(geom.plot_h))
                .cursor_ew_resize()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    move |_: &MouseDownEvent, window, cx| hot_down(window, cx),
                ),
        );
    }

    // The points' handles.
    for (index, point) in points.iter().enumerate() {
        let cx = x_of(&geom, point.temp_c);
        let cy = y_of(&geom, f32::from(point.percent));
        let down = on_point_down.clone();
        chart = chart.child(
            div()
                .id(gpui::SharedString::from(format!("{id}-point-{index}")))
                .absolute()
                .top(px(cy - 8.0))
                .left(px(cx - 8.0))
                .size(px(16.0))
                .cursor_grab()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    move |_: &MouseDownEvent, window, cx| down(index, window, cx),
                ),
        );
    }

    // The drag: the chart's own box tells the pointer where it is.
    let move_box = box_measure.clone();
    let move_handler = on_point_move.clone();
    let up_handler = on_point_up.clone();
    let hot_move = on_hot_move.clone();
    let hot_up = on_hot_up.clone();
    let _ = (hot_move, hot_up);
    chart
        .on_mouse_move({
            let move_box = move_box.clone();
            move |event: &MouseMoveEvent, window, cx| {
                let (ox, oy, _, _) = move_box.get();
                move_handler(
                    f32::from(event.position.x) - ox,
                    f32::from(event.position.y) - oy,
                    window,
                    cx,
                );
            }
        })
        .on_mouse_up(
            gpui::MouseButton::Left,
            move |event: &MouseUpEvent, window, cx| {
                let (ox, oy, _, _) = move_box.get();
                up_handler(
                    f32::from(event.position.x) - ox,
                    f32::from(event.position.y) - oy,
                    window,
                    cx,
                );
            },
        )
}
#[cfg(test)]
mod tests {
    use super::*;

    fn point(temp: f32, percent: u8) -> CurvePoint {
        CurvePoint {
            temp_c: temp,
            percent,
        }
    }

    #[test]
    fn the_chart_box_is_the_react_one() {
        let g = curve_geometry(400.0);
        assert_eq!(g.height, 248.0);
        assert_eq!(g.plot_w, 400.0 - 48.0 - 14.0);
        assert_eq!(g.plot_h, 248.0 - 14.0 - 46.0);
        // A narrow chart keeps the 260 px floor.
        assert_eq!(curve_geometry(100.0).width, 260.0);
    }

    #[test]
    fn the_axes_run_from_20_to_100() {
        let g = curve_geometry(400.0);
        assert_eq!(x_of(&g, 20.0), 48.0);
        assert_eq!(x_of(&g, 100.0), 48.0 + g.plot_w);
        assert_eq!(y_of(&g, 0.0), 14.0 + g.plot_h);
        assert_eq!(y_of(&g, 100.0), 14.0);
        // The inverse agrees.
        let x = x_of(&g, 63.0);
        assert!((temp_of_x(&g, x) - 63.0).abs() < 0.001);
        let y = y_of(&g, 42.0);
        assert!((pct_of_y(&g, y) - 42.0).abs() < 0.001);
    }

    #[test]
    fn the_duty_is_flat_outside_and_linear_between() {
        let points = vec![point(30.0, 30), point(70.0, 100)];
        assert_eq!(duty_for(&points, Some(20.0)), Some(30.0));
        assert_eq!(duty_for(&points, Some(30.0)), Some(30.0));
        assert_eq!(duty_for(&points, Some(50.0)), Some(65.0));
        assert_eq!(duty_for(&points, Some(70.0)), Some(100.0));
        assert_eq!(duty_for(&points, Some(90.0)), Some(100.0));
        assert_eq!(duty_for(&points, None), None);
        assert_eq!(duty_for(&[], Some(50.0)), None);
    }

    #[test]
    fn the_duty_uses_the_points_sorted_by_temperature() {
        let points = vec![point(70.0, 100), point(30.0, 30)];
        assert_eq!(duty_for(&points, Some(50.0)), Some(65.0));
    }

    #[test]
    fn the_default_curve_follows_the_floor() {
        let curve = default_curve(20);
        assert_eq!(curve.len(), 4);
        assert_eq!(curve[0].percent, 30);
        assert_eq!(curve[1].percent, 45);
        assert_eq!(curve[2].percent, 70);
        assert_eq!(curve[3].percent, 100);
        // A floor of 40 lifts the points under it.
        let curve = default_curve(40);
        assert_eq!(curve[0].percent, 40);
        assert_eq!(curve[1].percent, 45);
        assert_eq!(curve[2].percent, 70);
        assert_eq!(curve[3].percent, 100);
    }

    #[test]
    fn a_point_stays_between_its_neighbours() {
        let points = vec![point(30.0, 30), point(50.0, 50), point(70.0, 70)];
        assert_eq!(bounds_for(&points, 0), (20.0, 49.0));
        assert_eq!(bounds_for(&points, 1), (31.0, 69.0));
        assert_eq!(bounds_for(&points, 2), (51.0, 100.0));
    }

    #[test]
    fn the_floors_are_the_daemons() {
        assert_eq!(base_floor("sys_fan1", 20), 20);
        assert_eq!(base_floor("cpu_fan", 20), 30);
        assert_eq!(base_floor("cpu_fan", 40), 40);
        assert_eq!(base_floor("gpu_fan1", 20), 30);
        assert_eq!(base_floor("sys_fan1", 0), 20);
        // The hot floor lifts the manual headers.
        assert_eq!(hot_floor("sys_fan1", 20), 35);
        assert_eq!(hot_floor("cpu_fan", 20), 50);
    }

    #[test]
    fn the_floor_rule_names_the_hot_limit() {
        assert_eq!(
            hot_rule_text(Some(74.0)),
            "Below 35% (CPU_FAN 50%) is raised at or above its hot limit of 74 \u{00B0}C"
        );
        assert_eq!(
            hot_rule_text(None),
            "Below 35% (CPU_FAN 50%) is raised at or above its hot limit"
        );
    }

    #[test]
    fn the_hatch_line_is_clipped_to_the_band() {
        // The line y = x crosses a square band from corner to corner.
        let seg = hatch_segment(0.0, 0.0, 20.0, 20.0, 0.0).expect("a segment");
        assert_eq!(seg, (0.0, 0.0, 20.0, 20.0));
        // A line far below the band has no segment in it.
        assert!(hatch_segment(10.0, 0.0, 20.0, 10.0, 100.0).is_none());
    }
}
