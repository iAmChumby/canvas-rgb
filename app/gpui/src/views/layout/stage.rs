//! The desk: the canvas the parts stand on.
//!
//! This is `app/web/src/views/PreviewStage.tsx` ported, at rest: the desk
//! with its grid, the furniture (the desk rule, the frame of the effect area
//! and the rulers), the cells the engine samples, and the parts themselves.
//!
//! The desk is one canvas: React draws the same picture in a layer of
//! elements over a canvas, and the GPUI editor draws it in one paint pass,
//! so the parts and the cells cannot drift apart.

use gpui::{Bounds, PathBuilder, Pixels, Window, canvas, div, point, prelude::*, px};

use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo};
use canvas_proto::layout::{Facing, Landmark, PartKind, PlacedZone, SamplingMode};

use crate::theme::{Theme, TokenRadius};
use crate::views::layout::art;
use crate::views::layout::geometry::{self, CANVAS_H, CANVAS_W, LengthUnit, ResolvedShape};
use crate::views::layout::preview::PreviewFrame;
use crate::views::layout::tokens::DeskTokens;

/// The desk's ruler band, in css pixels (`RulerMarks`' `BAND_PX`).
const BAND_PX: f32 = 13.0;
/// How long a minor tick is, in css pixels.
const MINOR_PX: f32 = 4.0;
/// How long a major tick is, in css pixels.
const MAJOR_PX: f32 = 8.0;
/// The effect area: the whole logical canvas.
const EFFECT_W: f64 = 320.0;
const EFFECT_H: f64 = 200.0;
/// Where the desk's surface is, in canvas units (`DESK_Y` in `lib/caseForms.ts`).
const DESK_Y: f64 = 152.0;

/// What the desk shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageMode {
    /// The picture under the parts, with every LED's own cell.
    Effect,
    /// The lights themselves: every LED a dot in its own colour.
    Lights,
}

/// One picked part. Round 2's hit-testing picks these; at rest nothing is
/// picked yet.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sel {
    Zone(usize),
    Landmark(usize),
}

/// The view of the desk: the zoom and the pan. A view transform only: it
/// never reaches the layout the daemon stores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub scale: f64,
    pub pan_x: f64,
    pub pan_y: f64,
}

/// The view the desk opens in: the desk fills the stage's height, so the PC
/// is as big as the page allows (`heroView` in PreviewStage).
pub fn hero_view(w: f64, h: f64, centre: f64) -> View {
    if w <= 0.0 || h <= 0.0 {
        return View {
            scale: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        };
    }
    let base = w / CANVAS_W;
    let height_fit = h / CANVAS_H / base;
    let scale = 1.0f64.max(height_fit.min(1.35));
    // The view looks at the parts, not at the empty desk around them, but the
    // desk's own edges never come into view.
    let span = CANVAS_W * (1.0 / scale - 1.0);
    let pan_x = (CANVAS_W / (2.0 * scale) - centre).clamp(span, 0.0);
    View {
        scale,
        pan_x,
        pan_y: 0.0,
    }
}

/// The middle of the parts on the desk, in canvas units: the view opens
/// looking at them (`contentCentre`).
pub fn content_centre(zones: &[PlacedZone], landmarks: &[Landmark]) -> f64 {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    for z in zones {
        min_x = min_x.min(z.x as f64);
        max_x = max_x.max(z.x as f64 + z.width as f64);
    }
    for l in landmarks {
        min_x = min_x.min(l.x as f64);
        max_x = max_x.max(l.x as f64 + l.width as f64);
    }
    if !min_x.is_finite() || !max_x.is_finite() {
        return CANVAS_W / 2.0;
    }
    (min_x + max_x) / 2.0
}

/// What a drag is drawing over the desk: the marquee, the snap guides and
/// the readout. All in css pixels relative to the desk, except the guides,
/// which are in canvas units as React draws them as spans over the desk.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DragOverlay {
    /// The marquee band, css px (x, y, w, h).
    pub marquee: Option<(f32, f32, f32, f32)>,
    /// Snap guides, canvas units (vertical, second vertical, horizontal).
    pub guides: (Option<f64>, Option<f64>, Option<f64>),
    /// The move/resize/rotate readout, css px (text, x, y).
    pub readout: Option<(String, f32, f32)>,
}

/// Everything the desk draws.
pub struct DeskInput {
    pub zones: Vec<PlacedZone>,
    pub landmarks: Vec<Landmark>,
    /// The form of the case the layout names: a layout with a form but no
    /// case landmark draws the case from the form's measurements.
    pub case_form: canvas_proto::layout::CaseForm,
    pub devices: Vec<DeviceInfo>,
    pub unit: LengthUnit,
    pub mode: StageMode,
    pub sampling: SamplingMode,
    /// The daemon's newest frame, when one has arrived.
    pub frame: Option<PreviewFrame>,
    pub selection: Vec<Sel>,
    /// The view the desk is zoomed/panned to: `None` is the hero view.
    pub view: Option<View>,
    pub overlay: DragOverlay,
}

/// The desk: a canvas that draws the whole picture.
pub fn desk_with_origin(
    t: &Theme,
    colours: &DeskTokens,
    input: DeskInput,
    origin: crate::components::slider::Origin,
) -> gpui::AnyElement {
    let t = *t;
    let colours = *colours;
    let zones = input.zones;
    let landmarks = input.landmarks;
    let case_form = input.case_form;
    let devices = input.devices;
    let unit = input.unit;
    let mode = input.mode;
    let sampling = input.sampling;
    let frame = input.frame;
    let selection = input.selection;
    let stored_view = input.view;
    let overlay = input.overlay;

    let measure = origin.clone();
    let element = canvas(
        move |bounds, _window, _cx| {
            measure.set(
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            );
        },
        move |bounds, _ready, window, _cx| {
            let w: f32 = bounds.size.width.into();
            let h: f32 = bounds.size.height.into();
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let view = stored_view.unwrap_or_else(|| {
                hero_view(w as f64, h as f64, content_centre(&zones, &landmarks))
            });
            let base = w as f64 / CANVAS_W;
            let k = (base * view.scale) as f32;
            let ox: f32 = bounds.origin.x.into();
            let oy: f32 = bounds.origin.y.into();

            // The desk's own ground: the sunk colour, with the quiet grid on it.
            paint_grid(window, bounds, &t, &colours, mode);

            // The furniture: the desk rule, the frame of the effect area and
            // the rulers. React draws it in css pixels (the svg has no
            // viewBox), so it is drawn here in css pixels too.
            paint_furniture(window, bounds, &colours, unit, k);

            // The case drawn from the layout's form when there is no case
            // landmark of its own: a picture, not a part, dashed and below
            // everything else (`derivedCase` in PreviewStage).
            if !landmarks.iter().any(|l| l.kind == PartKind::Case) {
                if let Some(case) = geometry::case_landmark_for(case_form) {
                    paint_derived_case(window, &case, k, ox, oy, view, &colours);
                }
            }

            // The cells the engine samples, in the colours they read.
            paint_cells(
                window,
                bounds,
                &colours,
                &zones,
                &devices,
                frame.as_ref(),
                sampling,
                k,
                view,
            );

            // The parts.
            for (i, z) in zones.iter().enumerate() {
                let info = find_zone_info(&devices, z);
                let kind = part_of(z, &devices, info.map(|(_, zone)| zone));
                let facing = z.part.as_ref().map(|p| p.facing).unwrap_or(Facing::Viewer);
                let selected = selection
                    .iter()
                    .any(|s| matches!(s, Sel::Zone(n) if *n == i));
                let line = if selected { t.live() } else { colours.zone_line };
                paint_part(
                    window,
                    kind,
                    facing,
                    z.x as f64 + view.pan_x,
                    z.y as f64 + view.pan_y,
                    z.width as f64,
                    z.height as f64,
                    z.rotation_deg as f64,
                    k,
                    ox,
                    oy,
                    line,
                    &colours,
                );
            }
            for (i, l) in landmarks.iter().enumerate() {
                if !l.visible {
                    continue;
                }
                let selected = selection
                    .iter()
                    .any(|s| matches!(s, Sel::Landmark(n) if *n == i));
                let line = if selected {
                    t.live()
                } else {
                    colours.landmark_line
                };
                paint_part(
                    window,
                    l.kind,
                    l.facing,
                    l.x as f64 + view.pan_x,
                    l.y as f64 + view.pan_y,
                    l.width as f64,
                    l.height as f64,
                    l.rotation_deg as f64,
                    k,
                    ox,
                    oy,
                    line,
                    &colours,
                );
            }

            // The drag overlay: snap guides, marquee and readout.
            paint_overlay(window, bounds, &t, &overlay, k, ox, oy);

            // The selection frame and handles (one part picked).
            paint_selection_frame(window, &t, &zones, &landmarks, &selection, k, ox, oy, view);
        },
    );
    element.flex_1().min_h_0().min_w_0().into_any_element()
}

/// The snap guides (rose, `--a4`), the marquee band and the drag readout.
fn paint_overlay(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    t: &Theme,
    overlay: &DragOverlay,
    k: f32,
    ox: f32,
    oy: f32,
) {
    let rose = Theme::color(t.a4);
    let (gx, gx2, gy) = overlay.guides;
    if let Some(x) = gx {
        let px_x = ox + x as f32 * k;
        window.paint_quad(gpui::fill(
            Bounds {
                origin: point(px(px_x), px(oy)),
                size: gpui::size(px(1.0), bounds.size.height),
            },
            rose,
        ));
    }
    if let Some(x) = gx2 {
        let px_x = ox + x as f32 * k;
        window.paint_quad(gpui::fill(
            Bounds {
                origin: point(px(px_x), px(oy)),
                size: gpui::size(px(1.0), bounds.size.height),
            },
            rose,
        ));
    }
    if let Some(y) = gy {
        let px_y = oy + y as f32 * k;
        window.paint_quad(gpui::fill(
            Bounds {
                origin: point(px(ox), px(px_y)),
                size: gpui::size(bounds.size.width, px(1.0)),
            },
            rose,
        ));
    }
    if let Some((x, y, w, h)) = overlay.marquee {
        let rect = Bounds {
            origin: point(px(ox + x), px(oy + y)),
            size: gpui::size(px(w), px(h)),
        };
        let fill = gpui::Rgba {
            r: 0.5,
            g: 0.7,
            b: 1.0,
            a: 0.12,
        };
        window.paint_quad(gpui::fill(rect, fill));
        window.paint_quad(gpui::outline(rect, t.live(), gpui::BorderStyle::default()));
    }
    if let Some((text, rx, ry)) = &overlay.readout {
        paint_text(
            window,
            text,
            ox + rx + 7.0,
            oy + ry + 12.0,
            14.0,
            Theme::color(t.ink),
        );
    }
}

/// The selection frame: a live outline, eight handles and the rotate grip,
/// for exactly one picked part (PreviewStage `.frame`).
#[allow(clippy::too_many_arguments)]
fn paint_selection_frame(
    window: &mut Window,
    t: &Theme,
    zones: &[PlacedZone],
    landmarks: &[Landmark],
    selection: &[Sel],
    k: f32,
    ox: f32,
    oy: f32,
    view: View,
) {
    if selection.len() != 1 {
        return;
    }
    let (x, y, w, h, rot, locked): (f64, f64, f64, f64, f64, bool) = match selection[0] {
        Sel::Zone(i) => {
            let Some(z) = zones.get(i) else { return };
            (
                z.x as f64 + view.pan_x,
                z.y as f64 + view.pan_y,
                z.width as f64,
                z.height as f64,
                z.rotation_deg as f64,
                z.locked,
            )
        }
        Sel::Landmark(i) => {
            let Some(l) = landmarks.get(i) else { return };
            (
                l.x as f64 + view.pan_x,
                l.y as f64 + view.pan_y,
                l.width as f64,
                l.height as f64,
                l.rotation_deg as f64,
                l.locked,
            )
        }
    };
    let left = ox + x as f32 * k;
    let top = oy + y as f32 * k;
    let width = w as f32 * k;
    let height = h as f32 * k;
    let x0 = left.round() - 0.5;
    let y0 = top.round() - 0.5;
    let x1 = (left + width).round() - 0.5;
    let y1 = (top + height).round() - 0.5;
    paint_outline(window, x0, y0, x1, y1, 3.0, 1.5, t.live());
    if locked {
        return;
    }
    // Handles: 9 px squares, the rotate grip 11 px on a 24 px stem.
    let rad = rot.to_radians();
    let (s, c) = (rad.sin() as f32, rad.cos() as f32);
    let cx = left + width / 2.0;
    let cy = top + height / 2.0;
    let rotate = |lx: f32, ly: f32| (cx + lx * c - ly * s, cy + lx * s + ly * c);
    let narrow_x = width < 44.0;
    let narrow_y = height < 44.0;
    let handles: [(f32, f32, bool); 8] = [
        (-width / 2.0, -height / 2.0, true),
        (0.0, -height / 2.0, !narrow_x),
        (width / 2.0, -height / 2.0, true),
        (width / 2.0, 0.0, !narrow_y),
        (width / 2.0, height / 2.0, true),
        (0.0, height / 2.0, !narrow_x),
        (-width / 2.0, height / 2.0, true),
        (-width / 2.0, 0.0, !narrow_y),
    ];
    for (lx, ly, show) in handles {
        if !show {
            continue;
        }
        let (hx, hy) = rotate(lx, ly);
        window.paint_quad(gpui::fill(
            Bounds {
                origin: point(px(hx - 4.5), px(hy - 4.5)),
                size: gpui::size(px(9.0), px(9.0)),
            },
            Theme::color(t.panel),
        ));
        window.paint_quad(gpui::outline(
            Bounds {
                origin: point(px(hx - 4.5), px(hy - 4.5)),
                size: gpui::size(px(9.0), px(9.0)),
            },
            t.live(),
            gpui::BorderStyle::default(),
        ));
    }
    // Stem + grip.
    let (gx, gy) = rotate(0.0, -height / 2.0 - 24.0);
    paint_line(window, cx, cy, gx, gy, 1.5, t.live());
    window.paint_quad(gpui::fill(
        Bounds {
            origin: point(px(gx - 5.5), px(gy - 5.5)),
            size: gpui::size(px(11.0), px(11.0)),
        },
        Theme::color(t.panel),
    ));
    window.paint_quad(gpui::outline(
        Bounds {
            origin: point(px(gx - 5.5), px(gy - 5.5)),
            size: gpui::size(px(11.0), px(11.0)),
        },
        t.live(),
        gpui::BorderStyle::default(),
    ));
}

/// The desk's grid: a line every 12.5% of the desk, in the dot colour.
fn paint_grid(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    t: &Theme,
    colours: &DeskTokens,
    mode: StageMode,
) {
    let (ground, grid) = if mode == StageMode::Lights {
        (colours.stage_lights, colours.stage_lights_grid)
    } else {
        (Theme::color(t.sunk), Theme::color(t.dot))
    };
    window.paint_quad(gpui::fill(bounds, ground));
    let w: f32 = bounds.size.width.into();
    let h: f32 = bounds.size.height.into();
    let left: f32 = bounds.origin.x.into();
    let top: f32 = bounds.origin.y.into();
    // The grid: a line every 12.5% of the desk. The browser draws it as a
    // repeating background gradient, so a line is a one pixel band that
    // starts at the tile's edge and is antialiased over the two pixels it
    // touches; this draws the same band the same way.
    let mut x = left + w / 8.0;
    while x < left + w {
        paint_aa_vline(window, x, top, h, grid);
        x += w / 8.0;
    }
    let mut y = top + h / 8.0;
    while y < top + h {
        paint_aa_hline(window, left, y, w, grid);
        y += h / 8.0;
    }
}

/// A one pixel band that starts at `x`, antialiased over the two pixels it
/// touches, the way the browser draws a background gradient's edge.
fn paint_aa_vline(window: &mut Window, x: f32, top: f32, h: f32, colour: gpui::Rgba) {
    let base = x.floor();
    let frac = x - base;
    let first = (1.0 - frac) * colour.a;
    let second = frac * colour.a;
    if first > 0.0 {
        window.paint_quad(gpui::fill(
            gpui::Bounds {
                origin: gpui::point(gpui::px(base), gpui::px(top)),
                size: gpui::size(gpui::px(1.0), gpui::px(h)),
            },
            with_alpha(colour, first),
        ));
    }
    if second > 0.0 {
        window.paint_quad(gpui::fill(
            gpui::Bounds {
                origin: gpui::point(gpui::px(base + 1.0), gpui::px(top)),
                size: gpui::size(gpui::px(1.0), gpui::px(h)),
            },
            with_alpha(colour, second),
        ));
    }
}

/// The same band, the other way round.
fn paint_aa_hline(window: &mut Window, left: f32, y: f32, w: f32, colour: gpui::Rgba) {
    let base = y.floor();
    let frac = y - base;
    let first = (1.0 - frac) * colour.a;
    let second = frac * colour.a;
    if first > 0.0 {
        window.paint_quad(gpui::fill(
            gpui::Bounds {
                origin: gpui::point(gpui::px(left), gpui::px(base)),
                size: gpui::size(gpui::px(w), gpui::px(1.0)),
            },
            with_alpha(colour, first),
        ));
    }
    if second > 0.0 {
        window.paint_quad(gpui::fill(
            gpui::Bounds {
                origin: gpui::point(gpui::px(left), gpui::px(base + 1.0)),
                size: gpui::size(gpui::px(w), gpui::px(1.0)),
            },
            with_alpha(colour, second),
        ));
    }
}

/// The same colour at another strength.
fn with_alpha(colour: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    gpui::Rgba {
        r: colour.r,
        g: colour.g,
        b: colour.b,
        a: alpha,
    }
}

/// The ruler's marks: a tick every 2 cm (or every inch) and a number every
/// 10 cm (or every 5 in).
pub struct Mark {
    /// The place on the canvas, in canvas units.
    pub at: f64,
    pub major: bool,
    pub label: String,
}

/// The ruler's marks, in the reader's unit.
pub fn ruler_marks(unit: LengthUnit) -> Vec<Mark> {
    let mut out = Vec::new();
    let per_cm = 2.5; // canvas units to a centimetre
    let per_in = 25.4 / 4.0; // canvas units to an inch
    let minor = match unit {
        LengthUnit::In => per_in,
        LengthUnit::Cm => 2.0 * per_cm,
    };
    let count = (EFFECT_W.max(EFFECT_H) / minor).floor();
    for i in 0..=count as i64 {
        let at = i as f64 * minor;
        let mm = at * 4.0;
        out.push(Mark {
            at,
            major: i % 5 == 0,
            label: match unit {
                LengthUnit::In => format!("{}", ((mm / 25.4) * 100.0).round() / 100.0),
                LengthUnit::Cm => format!("{}", (mm / 10.0).round()),
            },
        });
    }
    out
}

/// The desk rule, the frame of the effect area and the rulers.
///
/// React draws them in canvas units inside one svg per layer, at scale 1
/// (the furniture covers the top-left 320 x 200 of the desk, whatever the
/// view's zoom: `Rulers.tsx`'s ticks are `MINOR_PX / k` canvas units, so they
/// keep their screen size, and its numbers are `11px / k`). What follows is
/// that, in canvas units, measured the same way.
fn paint_furniture(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    colours: &DeskTokens,
    unit: LengthUnit,
    k: f32,
) {
    let left: f32 = bounds.origin.x.into();
    let top: f32 = bounds.origin.y.into();
    // The desk: the surface the case stands on and the peripherals sit on.
    paint_line(
        window,
        left - 2.0,
        top + DESK_Y as f32,
        left + 322.0,
        top + DESK_Y as f32,
        1.0,
        colours.ruler_ink,
    );
    // The frame of the effect area: where the picture ends.
    let mut builder = PathBuilder::stroke(px(1.0));
    builder.move_to(point(px(left), px(top)));
    builder.line_to(point(px(left + EFFECT_W as f32), px(top)));
    builder.line_to(point(px(left + EFFECT_W as f32), px(top + EFFECT_H as f32)));
    builder.line_to(point(px(left), px(top + EFFECT_H as f32)));
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, colours.ruler_ink);
    }
    // The rulers: along the top edge and along the left edge. Every line is
    // drawn twice: a dark keyline under a light one.
    let marks = ruler_marks(unit);
    for m in &marks {
        if m.at <= EFFECT_W + 0.001 {
            let len = if m.major { MAJOR_PX } else { MINOR_PX } / k;
            paint_line(
                window,
                left + m.at as f32,
                top,
                left + m.at as f32,
                top + len,
                if m.major { 3.0 } else { 2.0 },
                colours.ruler_key,
            );
            paint_line(
                window,
                left + m.at as f32,
                top,
                left + m.at as f32,
                top + len,
                1.0,
                colours.ruler_ink,
            );
        }
    }
    for m in &marks {
        if m.at <= EFFECT_H + 0.001 {
            let len = if m.major { MAJOR_PX } else { MINOR_PX } / k;
            paint_line(
                window,
                left,
                top + m.at as f32,
                left + len,
                top + m.at as f32,
                if m.major { 3.0 } else { 2.0 },
                colours.ruler_key,
            );
            paint_line(
                window,
                left,
                top + m.at as f32,
                left + len,
                top + m.at as f32,
                1.0,
                colours.ruler_ink,
            );
        }
    }
    // The numbers: the reader's own unit, never canvas units.
    for m in &marks {
        if m.at <= EFFECT_W + 0.001 && m.major {
            paint_text(
                window,
                &m.label,
                left + m.at as f32 + 2.0 / k,
                top + (BAND_PX - 2.0) / k,
                11.0 / k,
                colours.ruler_num,
            );
        }
    }
    for m in &marks {
        if m.at <= EFFECT_H + 0.001 && m.major {
            // The left-hand numbers stand on their side.
            paint_text_rotated(
                window,
                &m.label,
                left + (BAND_PX - 2.0) / k,
                top + m.at as f32 - 2.0 / k,
                11.0 / k,
                colours.ruler_num,
            );
        }
    }
}

fn paint_line(
    window: &mut Window,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    width: f32,
    colour: gpui::Rgba,
) {
    let mut builder = PathBuilder::stroke(px(width));
    builder.move_to(point(px(x1), px(y1)));
    builder.line_to(point(px(x2), px(y2)));
    if let Ok(path) = builder.build() {
        window.paint_path(path, colour);
    }
}

/// The figures face at the ruler's size. Built directly: `TextStyle`'s
/// accessors spell the banned call, and the family, weight and style are all
/// written here anyway.
fn ruler_style(_window: &Window, _size: f32) -> gpui::Font {
    gpui::Font {
        family: crate::fonts::MONO_FAMILY.into(),
        features: gpui::FontFeatures::default(),
        fallbacks: None,
        weight: gpui::FontWeight::MEDIUM,
        style: gpui::FontStyle::default(),
    }
}

/// The ruler's numbers, in the figures face.
fn paint_text(
    window: &mut Window,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    colour: gpui::Rgba,
) {
    let hsla = gpui::Hsla::from(colour);
    let font = ruler_style(window, size);
    let runs = [gpui::TextRun {
        len: text.len(),
        font: font.clone(),
        color: hsla,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let layout = window.text_system().layout_line(text, px(size), &runs, None);
    for run in &layout.runs {
        for glyph in &run.glyphs {
            let at = point(
                px(x + f32::from(glyph.position.x)),
                px(y + f32::from(glyph.position.y)),
            );
            let _ = window.paint_glyph(at, run.font_id, glyph.id, px(size), hsla);
        }
    }
}

/// The ruler's numbers on the left edge, turned on their side.
///
/// React turns them with `rotate(-90 …)` and anchors them at their end, so
/// the text runs up the edge from the mark.
fn paint_text_rotated(
    window: &mut Window,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    colour: gpui::Rgba,
) {
    let hsla = gpui::Hsla::from(colour);
    let font = ruler_style(window, size);
    let runs = [gpui::TextRun {
        len: text.len(),
        font: font.clone(),
        color: hsla,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let layout = window.text_system().layout_line(text, px(size), &runs, None);
    // The text is anchored at its end, so it starts `width` to the left.
    let width = layout.width;
    for run in &layout.runs {
        for glyph in &run.glyphs {
            // A turn of -90 degrees: (dx, dy) becomes (dy, -dx).
            let gx = f32::from(glyph.position.y);
            let gy = -(f32::from(glyph.position.x) - f32::from(width));
            let at = point(px(x + gx), px(y + gy));
            let _ = window.paint_glyph(at, run.font_id, glyph.id, px(size), hsla);
        }
    }
}

/// The cells the engine samples, in the colours they read.
#[allow(clippy::too_many_arguments)]
fn paint_cells(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    colours: &DeskTokens,
    zones: &[PlacedZone],
    devices: &[DeviceInfo],
    frame: Option<&PreviewFrame>,
    sampling: SamplingMode,
    k: f32,
    view: View,
) {
    for z in zones {
        if !z.visible {
            continue;
        }
        let Some((device, info)) = find_zone_info(devices, z) else {
            continue;
        };
        let local = geometry::zone_local_leds(z, info);
        let transform = geometry::zone_transform(z);
        let off = !device.connected;
        let count = if local.shape == ResolvedShape::Point {
            local.leds.len().min(1)
        } else {
            local.leds.len()
        };
        for led in local.leds.iter().take(count) {
            let Some(led) = led else { continue };
            let cell = geometry::canvas_cell(&transform, led);
            let x = f32::from(bounds.origin.x) + (cell.cx + view.pan_x) as f32 * k;
            let y = f32::from(bounds.origin.y) + (cell.cy + view.pan_y) as f32 * k;
            let w = (led.w as f32 * k).max(1.0);
            let h = (led.h as f32 * k).max(1.0);
            let colour = match (frame, off) {
                (Some(pic), false) => sample_led(pic, &cell, sampling),
                _ => None,
            };
            let fill = match colour {
                Some([r, g, b]) => gpui::Rgba {
                    r: r as f32 / 255.0,
                    g: g as f32 / 255.0,
                    b: b as f32 / 255.0,
                    a: 1.0,
                },
                None => colours.dot_unlit,
            };
            // The cell is the LED's own ground: its colour, solid, with a
            // dark line round it, so two cells that touch read as two tiles.
            let corners = rotated_corners(x, y, w, h, z.rotation_deg as f64);
            window.paint_quad(gpui::fill(quad_bounds(corners), fill));
            let mut builder = PathBuilder::stroke(px(1.0));
            builder.move_to(point(px(corners[0].0), px(corners[0].1)));
            for c in &corners[1..] {
                builder.line_to(point(px(c.0), px(c.1)));
            }
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(
                    path,
                    gpui::Rgba {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.55,
                    },
                );
            }
        }
    }
}

/// The four corners of a cell, turned with the part.
fn rotated_corners(x: f32, y: f32, w: f32, h: f32, rotation_deg: f64) -> [(f32, f32); 4] {
    let rad = rotation_deg.to_radians();
    let (sin, cos) = (rad.sin() as f32, rad.cos() as f32);
    let mut out = [(0.0f32, 0.0f32); 4];
    for (i, (dx, dy)) in [
        (-w / 2.0, -h / 2.0),
        (w / 2.0, -h / 2.0),
        (w / 2.0, h / 2.0),
        (-w / 2.0, h / 2.0),
    ]
    .iter()
    .enumerate()
    {
        out[i] = (
            x + dx * cos - dy * sin,
            y + dx * sin + dy * cos,
        );
    }
    out
}

/// The box round four corners.
fn quad_bounds(corners: [(f32, f32); 4]) -> Bounds<Pixels> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    Bounds {
        origin: point(px(min_x), px(min_y)),
        size: gpui::size(px(max_x - min_x), px(max_y - min_y)),
    }
}

/// A part's outline and its drawing.
#[allow(clippy::too_many_arguments)]
fn paint_part(
    window: &mut Window,
    kind: PartKind,
    facing: Facing,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rotation_deg: f64,
    k: f32,
    ox: f32,
    oy: f32,
    line: gpui::Rgba,
    colours: &DeskTokens,
) {
    let left = ox + x as f32 * k;
    let top = oy + y as f32 * k;
    let width = w as f32 * k;
    let height = h as f32 * k;
    // The browser snaps a border box to whole pixels before it paints the
    // border, and the border itself is a whole pixel wide. The keyline sits
    // just outside it, so it holds the outline on any picture.
    let x0 = left.round();
    let y0 = top.round();
    let x1 = (left + width).round();
    let y1 = (top + height).round();
    paint_outline(
        window,
        x0 - 0.5,
        y0 - 0.5,
        x1 - 0.5,
        y1 - 0.5,
        4.5,
        1.0,
        colours.zone_halo,
    );
    paint_outline(
        window,
        x0 + 0.5,
        y0 + 0.5,
        x1 - 0.5,
        y1 - 0.5,
        3.5,
        1.0,
        line,
    );
    // The drawing inside it, on the part's own padding box (the browser
    // draws the picture inside the border, not on the border box).
    art::paint(
        window,
        kind,
        facing,
        x,
        y,
        w,
        h,
        rotation_deg,
        ox,
        oy,
        k,
        line,
        1.25,
    );
}

/// The derived case: a dashed landmark-line outline with the case's own
/// drawing inside, below every part (`desk.derived` in `desk.module.css`:
/// the border is dashed, the art takes the landmark's colour).
fn paint_derived_case(
    window: &mut Window,
    case: &Landmark,
    k: f32,
    ox: f32,
    oy: f32,
    view: View,
    colours: &DeskTokens,
) {
    let left = ox + (case.x as f64 + view.pan_x) as f32 * k;
    let top = oy + (case.y as f64 + view.pan_y) as f32 * k;
    let width = case.width * k;
    let height = case.height * k;
    let x0 = left.round();
    let y0 = top.round();
    let x1 = (left + width).round();
    let y1 = (top + height).round();
    paint_dashed_outline(
        window,
        x0 - 0.5,
        y0 - 0.5,
        x1 - 0.5,
        y1 - 0.5,
        4.5,
        1.0,
        colours.zone_halo,
    );
    paint_dashed_outline(window, x0 + 0.5, y0 + 0.5, x1 - 0.5, y1 - 0.5, 3.5, 1.25, colours.landmark_line);
    // The drawing inside, on the part's own padding box.
    art::paint(
        window,
        PartKind::Case,
        case.facing,
        case.x as f64 + view.pan_x,
        case.y as f64 + view.pan_y,
        case.width as f64,
        case.height as f64,
        0.0,
        ox,
        oy,
        k,
        colours.landmark_line,
        1.25,
    );
}

/// A dashed rounded outline: a dash of `2 * width` and a gap of `2 * width`
/// along the outline, the way the browser dashes a border.
#[allow(clippy::too_many_arguments)]
fn paint_dashed_outline(
    window: &mut Window,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radius: f32,
    width: f32,
    colour: gpui::Rgba,
) {
    let points = rounded_box(x0, y0, x1, y1, radius);
    let dash = 2.0 * width;
    let gap = 2.0 * width;
    // Sample the outline every pixel and paint the samples that fall inside
    // a dash: the sub-paths are the dashes.
    let mut samples: Vec<(f32, f32)> = Vec::new();
    let mut total = 0.0f32;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let d = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let steps = d.ceil() as usize;
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            samples.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
        total += d;
    }
    let period = dash + gap;
    let mut builder = PathBuilder::stroke(px(width));
    let mut prev: Option<(f32, f32)> = None;
    for (i, p) in samples.iter().enumerate() {
        let along = i as f32;
        let in_dash = along % period < dash;
        if in_dash {
            if prev.is_none() {
                builder.move_to(point(px(p.0), px(p.1)));
            }
            builder.line_to(point(px(p.0), px(p.1)));
            prev = Some(*p);
        } else {
            prev = None;
        }
        let _ = total;
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, colour);
    }
}

/// A part's outline: a rounded box, turned with the part.
#[allow(clippy::too_many_arguments)]
fn paint_outline(
    window: &mut Window,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    radius: f32,
    width: f32,
    colour: gpui::Rgba,
) {
    let points = rounded_box(x0, y0, x1, y1, radius);
    let mut builder = PathBuilder::stroke(px(width));
    builder.move_to(point(px(points[0].0), px(points[0].1)));
    for p in &points[1..] {
        builder.line_to(point(px(p.0), px(p.1)));
    }
    builder.close();
    if let Ok(path) = builder.build() {
        window.paint_path(path, colour);
    }
}

/// A box with rounded corners, as the points of its outline.
fn rounded_box(x0: f32, y0: f32, x1: f32, y1: f32, radius: f32) -> Vec<(f32, f32)> {
    let w = x1 - x0;
    let h = y1 - y0;
    let r = radius.min(w / 2.0).min(h / 2.0);
    let steps = 6;
    // The corners: top left, top right, bottom right, bottom left.
    let corners = [
        (x0 + r, y0 + r),
        (x1 - r, y0 + r),
        (x1 - r, y1 - r),
        (x0 + r, y1 - r),
    ];
    let mut points: Vec<(f32, f32)> = Vec::with_capacity(4 * (steps + 1));
    for (i, centre) in corners.iter().enumerate() {
        // Each corner's arc runs from the edge before it to the edge after it:
        // the top left's arc goes from the left edge up to the top edge.
        let from = (i as f64 + 2.0) * std::f64::consts::FRAC_PI_2;
        let to = (i as f64 + 3.0) * std::f64::consts::FRAC_PI_2;
        for step in 0..=steps {
            let a = from + (to - from) * (step as f64 / steps as f64);
            points.push((
                centre.0 + r * (a.cos() as f32),
                centre.1 + r * (a.sin() as f32),
            ));
        }
    }
    points
}

/// The device and the zone a placement belongs to.
pub fn find_zone_info<'a>(
    devices: &'a [DeviceInfo],
    z: &PlacedZone,
) -> Option<(&'a DeviceInfo, &'a ZoneInfo)> {
    let device = devices.iter().find(|d| d.id.0 == z.device_id.0)?;
    let zone = device.zones.iter().find(|zone| zone.index == z.zone_index)?;
    Some((device, zone))
}

/// The part a zone is drawn as, when the owner has not said (`partOf`).
pub fn part_of(z: &PlacedZone, devices: &[DeviceInfo], info: Option<&ZoneInfo>) -> PartKind {
    if let Some(part) = z.part.as_ref() {
        return part.kind;
    }
    let Some(info) = info else {
        return PartKind::Generic;
    };
    let kind = devices
        .iter()
        .find(|d| d.id.0 == z.device_id.0)
        .map(|d| d.kind)
        .unwrap_or(DeviceKind::Other);
    geometry::part_for_device(kind, info)
}

/// The colour one LED reads from the picture.
fn sample_led(
    pic: &PreviewFrame,
    cell: &geometry::CanvasCell,
    mode: SamplingMode,
) -> Option<[u8; 3]> {
    let sx = pic.width as f64 / CANVAS_W;
    let sy = pic.height as f64 / CANVAS_H;
    let x = cell.cx * sx;
    let y = cell.cy * sy;
    match mode {
        SamplingMode::Bilinear => sample_bilinear(pic, x, y),
        SamplingMode::AreaAverage => sample_area(
            pic,
            x,
            y,
            [
                cell.bounds[0] * sx,
                cell.bounds[1] * sy,
                cell.bounds[2] * sx,
                cell.bounds[3] * sy,
            ],
        ),
        SamplingMode::Nearest => sample_nearest(pic, x, y),
    }
}

fn sample_nearest(pic: &PreviewFrame, x: f64, y: f64) -> Option<[u8; 3]> {
    let px = (x.round() as i64).clamp(0, pic.width as i64 - 1);
    let py = (y.round() as i64).clamp(0, pic.height as i64 - 1);
    pic.at(px, py)
}

fn sample_bilinear(pic: &PreviewFrame, x: f64, y: f64) -> Option<[u8; 3]> {
    let max_x = pic.width as f64 - 1.0;
    let max_y = pic.height as f64 - 1.0;
    let fx = x.clamp(0.0, max_x);
    let fy = y.clamp(0.0, max_y);
    let x0 = fx.floor();
    let y0 = fy.floor();
    let x1 = (x0 + 1.0).min(max_x);
    let y1 = (y0 + 1.0).min(max_y);
    let tx = fx - x0;
    let ty = fy - y0;
    let p00 = pic.at(x0 as i64, y0 as i64)?;
    let p10 = pic.at(x1 as i64, y0 as i64)?;
    let p01 = pic.at(x0 as i64, y1 as i64)?;
    let p11 = pic.at(x1 as i64, y1 as i64)?;
    let mut out = [0u8; 3];
    for c in 0..3 {
        let a0 = p00[c] as f64;
        let a1 = p10[c] as f64;
        let b0 = p01[c] as f64;
        let b1 = p11[c] as f64;        let top = a0 + (a1 - a0) * tx;
        let bottom = b0 + (b1 - b0) * tx;
        out[c] = (top + (bottom - top) * ty).round() as u8;
    }
    Some(out)
}

fn sample_area(pic: &PreviewFrame, x: f64, y: f64, bounds: [f64; 4]) -> Option<[u8; 3]> {
    let max_x = pic.width as f64 - 1.0;
    let max_y = pic.height as f64 - 1.0;
    let x0 = (bounds[0] - 0.5).ceil().clamp(0.0, max_x);
    let y0 = (bounds[1] - 0.5).ceil().clamp(0.0, max_y);
    let x1 = (bounds[2] - 0.5).floor().clamp(0.0, max_x);
    let y1 = (bounds[3] - 0.5).floor().clamp(0.0, max_y);
    if x0 > x1 || y0 > y1 {
        return sample_nearest(pic, x, y);
    }
    let mut sum = [0u32; 3];
    let mut n = 0u32;
    for py in y0 as i64..=y1 as i64 {
        for px in x0 as i64..=x1 as i64 {
            let Some(colour) = pic.at(px, py) else {
                continue;
            };
            for c in 0..3 {
                sum[c] += colour[c] as u32;
            }
            n += 1;
        }
    }
    if n == 0 {
        return sample_nearest(pic, x, y);
    }
    Some([
        (sum[0] as f64 / n as f64).round() as u8,
        (sum[1] as f64 / n as f64).round() as u8,
        (sum[2] as f64 / n as f64).round() as u8,
    ])
}

/// The desk element, with the one-time hint over it.
pub fn desk_with_hint(
    t: &Theme,
    colours: &DeskTokens,
    input: DeskInput,
    origin: crate::components::slider::Origin,
    hint: bool,
) -> gpui::AnyElement {
    let canvas = desk_with_origin(t, colours, input, origin);
    if !hint {
        return canvas;
    }
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .child(canvas)
        .child(
            div()
                .absolute()
                .top(px(10.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .px(px(8.0))
                        .py(px(3.0))
                        .r_sm()
                        .border_1()
                        .border_color(Theme::color(t.line_2))
                        .bg(Theme::color(t.panel))
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child(
                            div()
                                .font_family(crate::fonts::MONO_FAMILY)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(Theme::color(t.ink))
                                .child("L"),
                        )
                        .child(" lights · ")
                        .child(
                            div()
                                .font_family(crate::fonts::MONO_FAMILY)
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(Theme::color(t.ink))
                                .child("E"),
                        )
                        .child(" effect"),
                ),
        )
        .into_any_element()
}


