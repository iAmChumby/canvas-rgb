//! Round-2 interactions, ported from React's pure logic.
//!
//! The desk's pointer and keyboard behaviour lives in
//! `app/web/src/lib/{hitTest,rubberBand}.ts` and
//! `app/web/src/views/{PreviewStage,LayoutView}.tsx`. Everything here is a
//! pure function of that logic, so the tests below pin React's numbers
//! without a window.

#![allow(dead_code)]

use canvas_proto::layout::{Landmark, PartKind, PlacedZone};

use crate::views::layout::geometry::{CANVAS_H, CANVAS_W};

pub const MIN_SIZE: f64 = 2.0;
pub const SNAP_PIXELS: f64 = 8.0;
pub const EDGE_HANDLE_MIN_PX: f64 = 44.0;
pub const ROTATE_GRIP_PX: f64 = 24.0;
pub const EDGE_BAND_PX: f64 = 8.0;
pub const MIN_TARGET_PX: f64 = 12.0;
pub const RUBBER_C: f64 = 0.55;
pub const MIN_SCALE: f64 = 0.25;
pub const MAX_SCALE: f64 = 8.0;
pub const NUDGE_MM: f64 = 1.0;
pub const NUDGE_MM_BIG: f64 = 10.0;
pub const MM_PER_UNIT: f64 = 4.0;

/// A point in canvas units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ref {
    Zone(usize),
    Landmark(usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitPart {
    pub r: Ref,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rot: f64,
    pub is_case: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Nw,
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
}

impl Handle {
    pub fn sx(self) -> i32 {
        match self {
            Handle::Nw | Handle::Sw | Handle::W => -1,
            Handle::Ne | Handle::E | Handle::Se => 1,
            _ => 0,
        }
    }
    pub fn sy(self) -> i32 {
        match self {
            Handle::Nw | Handle::N | Handle::Ne => -1,
            Handle::Sw | Handle::S | Handle::Se => 1,
            _ => 0,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Handle::Nw => "nw",
            Handle::N => "n",
            Handle::Ne => "ne",
            Handle::E => "e",
            Handle::Se => "se",
            Handle::S => "s",
            Handle::Sw => "sw",
            Handle::W => "w",
        }
    }
}

pub const HANDLES: [Handle; 8] = [
    Handle::Nw,
    Handle::N,
    Handle::Ne,
    Handle::E,
    Handle::Se,
    Handle::S,
    Handle::Sw,
    Handle::W,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Part(Ref, bool),
    CaseEdge(Ref, bool),
    CaseBody(Ref, bool),
    Handle(Ref, Handle),
    Rotate(Ref),
    Desk,
}

pub fn drag_threshold_px(touch: bool) -> f64 {
    if touch { 8.0 } else { 4.0 }
}

pub fn handle_reach_px(touch: bool) -> f64 {
    if touch { 12.0 } else { 8.0 }
}

fn to_local(p: &HitPart, pt: Pt) -> Pt {
    let rad = p.rot.to_radians();
    let (s, c) = (rad.sin(), rad.cos());
    let dx = pt.x - (p.x + p.w / 2.0);
    let dy = pt.y - (p.y + p.h / 2.0);
    Pt {
        x: dx * c + dy * s,
        y: -dx * s + dy * c,
    }
}

pub fn part_corners(p: &HitPart) -> [Pt; 4] {
    let rad = p.rot.to_radians();
    let (s, c) = (rad.sin(), rad.cos());
    let (cx, cy) = (p.x + p.w / 2.0, p.y + p.h / 2.0);
    let (hw, hh) = (p.w / 2.0, p.h / 2.0);
    let local = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)];
    let mut out = [Pt { x: 0.0, y: 0.0 }; 4];
    for (i, (lx, ly)) in local.iter().enumerate() {
        out[i] = Pt {
            x: cx + lx * c - ly * s,
            y: cy + lx * s + ly * c,
        };
    }
    out
}

fn on_body(p: &HitPart, pt: Pt, k: f64) -> bool {
    let q = to_local(p, pt);
    let pad = if p.is_case { 0.0 } else { MIN_TARGET_PX / 2.0 / k };
    let hw = (p.w / 2.0).max(pad);
    let hh = (p.h / 2.0).max(pad);
    q.x.abs() <= hw && q.y.abs() <= hh
}

fn on_edge_band(p: &HitPart, pt: Pt, k: f64) -> bool {
    let q = to_local(p, pt);
    let band = EDGE_BAND_PX / k;
    p.w / 2.0 - q.x.abs() < band || p.h / 2.0 - q.y.abs() < band
}

fn area(p: &HitPart) -> f64 {
    p.w * p.h
}

/// Everything under the point, top of the stack first (hitTest.ts hitStack).
pub fn hit_stack(parts: &[HitPart], point: Pt, k: f64) -> Vec<Hit> {
    let hits: Vec<&HitPart> = parts.iter().filter(|p| on_body(p, point, k)).collect();
    let mut plain: Vec<&HitPart> = hits.iter().copied().filter(|p| !p.is_case).collect();
    plain.sort_by(|a, b| {
        area(a)
            .partial_cmp(&area(b))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| match (a.r, b.r) {
                (Ref::Zone(_), Ref::Landmark(_)) => std::cmp::Ordering::Less,
                (Ref::Landmark(_), Ref::Zone(_)) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| {
                let ia = match a.r { Ref::Zone(i) | Ref::Landmark(i) => i };
                let ib = match b.r { Ref::Zone(i) | Ref::Landmark(i) => i };
                ib.cmp(&ia)
            })
    });
    let mut cases: Vec<&HitPart> = hits.iter().copied().filter(|p| p.is_case).collect();
    cases.sort_by(|a, b| {
        area(a)
            .partial_cmp(&area(b))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let ia = match a.r { Ref::Zone(i) | Ref::Landmark(i) => i };
                let ib = match b.r { Ref::Zone(i) | Ref::Landmark(i) => i };
                ib.cmp(&ia)
            })
    });
    let edges: Vec<&HitPart> = cases
        .iter()
        .copied()
        .filter(|c| on_edge_band(c, point, k))
        .collect();
    let bodies: Vec<&HitPart> = cases
        .iter()
        .copied()
        .filter(|c| !edges.contains(c))
        .collect();
    let mut out = Vec::new();
    for p in plain {
        out.push(Hit::Part(p.r, p.locked));
    }
    for p in edges {
        out.push(Hit::CaseEdge(p.r, p.locked));
    }
    for p in bodies {
        out.push(Hit::CaseBody(p.r, p.locked));
    }
    out
}

fn handle_at(parts: &[HitPart], selection: &[Ref], point: Pt, k: f64, touch: bool) -> Option<Hit> {
    if selection.len() != 1 {
        return None;
    }
    let r = selection[0];
    let part = parts.iter().find(|p| p.r == r)?;
    if part.locked {
        return None;
    }
    let q = to_local(part, point);
    let reach = handle_reach_px(touch) / k;
    let narrow_x = part.w * k < EDGE_HANDLE_MIN_PX;
    let narrow_y = part.h * k < EDGE_HANDLE_MIN_PX;
    let mut best: Option<(Hit, f64)> = None;
    let offer = |hit: Hit, lx: f64, ly: f64, best: &mut Option<(Hit, f64)>| {
        let ex = (q.x - lx).abs();
        let ey = (q.y - ly).abs();
        if ex > reach || ey > reach {
            return;
        }
        let d = ex.hypot(ey);
        if best.as_ref().is_none_or(|(_, bd)| d < *bd) {
            *best = Some((hit, d));
        }
    };
    for h in HANDLES {
        if h.sx() == 0 && narrow_x {
            continue;
        }
        if h.sy() == 0 && narrow_y {
            continue;
        }
        offer(
            Hit::Handle(r, h),
            (h.sx() as f64 * part.w) / 2.0,
            (h.sy() as f64 * part.h) / 2.0,
            &mut best,
        );
    }
    offer(
        Hit::Rotate(r),
        0.0,
        -part.h / 2.0 - ROTATE_GRIP_PX / k,
        &mut best,
    );
    best.map(|(h, _)| h)
}

/// What a press at the point lands on (hitTest.ts hitTest).
pub fn hit_test(parts: &[HitPart], selection: &[Ref], point: Pt, k: f64, touch: bool, under: bool) -> Hit {
    if !under {
        if let Some(h) = handle_at(parts, selection, point, k, touch) {
            return h;
        }
    }
    let stack = hit_stack(parts, point, k);
    let Some(first) = stack.first().copied() else {
        return Hit::Desk;
    };
    if !under {
        return first;
    }
    let picked = selection.first().copied();
    if let Some(picked) = picked {
        if let Some(at) = stack.iter().position(|t| t.r() == picked) {
            return stack[(at + 1) % stack.len()];
        }
    }
    first
}

impl Hit {
    pub fn r(self) -> Ref {
        match self {
            Hit::Part(r, _) | Hit::CaseEdge(r, _) | Hit::CaseBody(r, _) => r,
            Hit::Handle(r, _) | Hit::Rotate(r) => r,
            Hit::Desk => Ref::Zone(usize::MAX),
        }
    }
    pub fn locked(self) -> bool {
        match self {
            Hit::Part(_, l) | Hit::CaseEdge(_, l) | Hit::CaseBody(_, l) => l,
            _ => false,
        }
    }
}

/// The parts a case carries when it moves (hitTest.ts partsInsideCase).
pub fn parts_inside_case(case: &HitPart, parts: &[HitPart]) -> Vec<Ref> {
    let mut out = Vec::new();
    for p in parts {
        if p.is_case || p.locked || p.r == case.r {
            continue;
        }
        let mid = Pt {
            x: p.x + p.w / 2.0,
            y: p.y + p.h / 2.0,
        };
        let q = to_local(case, mid);
        if q.x.abs() <= case.w / 2.0 && q.y.abs() <= case.h / 2.0 {
            out.push(p.r);
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

fn aabb(p: &HitPart) -> Rect {
    let c = part_corners(p);
    let xs = c.iter().map(|p| p.x);
    let ys = c.iter().map(|p| p.y);
    let x = xs.clone().fold(f64::INFINITY, f64::min);
    let y = ys.clone().fold(f64::INFINITY, f64::min);
    let x1 = xs.fold(f64::NEG_INFINITY, f64::max);
    let y1 = ys.fold(f64::NEG_INFINITY, f64::max);
    Rect {
        x,
        y,
        w: x1 - x,
        h: y1 - y,
    }
}

/// What a rubber band picks (hitTest.ts marqueePick).
pub fn marquee_pick(band: Rect, parts: &[HitPart]) -> Vec<Ref> {
    let mut out = Vec::new();
    for p in parts {
        let b = aabb(p);
        let touches = b.x < band.x + band.w
            && band.x < b.x + b.w
            && b.y < band.y + band.h
            && band.y < b.y + b.h;
        if !touches {
            continue;
        }
        if p.is_case {
            let holds = b.x >= band.x
                && b.y >= band.y
                && b.x + b.w <= band.x + band.w
                && b.y + b.h <= band.y + band.h;
            if !holds {
                continue;
            }
        }
        out.push(p.r);
    }
    out
}

// --- rubber band ---

pub fn resisted_overshoot(overshoot: f64, dimension: f64, c: f64) -> f64 {
    if !overshoot.is_finite() || !dimension.is_finite() || overshoot <= 0.0 || dimension <= 0.0 {
        return 0.0;
    }
    (overshoot * c * dimension) / (dimension + c * overshoot)
}

pub fn rubber_band(value: f64, min: f64, max: f64) -> f64 {
    rubber_band_c(value, min, max, RUBBER_C)
}

pub fn rubber_band_c(value: f64, min: f64, max: f64, c: f64) -> f64 {
    let span = max - min;
    if !value.is_finite() || !span.is_finite() || span <= 0.0 {
        return value;
    }
    if value < min {
        return min - resisted_overshoot(min - value, span, c);
    }
    if value > max {
        return max + resisted_overshoot(value - max, span, c);
    }
    value
}

pub fn settle_to_limit(value: f64, min: f64, max: f64) -> f64 {
    value.min(max).max(min)
}

pub fn settle_rect(rect: Rect, canvas_w: f64, canvas_h: f64, min_size: f64) -> Rect {
    let w = settle_to_limit(
        if rect.w.is_finite() { rect.w } else { min_size },
        min_size,
        canvas_w,
    );
    let h = settle_to_limit(
        if rect.h.is_finite() { rect.h } else { min_size },
        min_size,
        canvas_h,
    );
    let x = settle_to_limit(
        if rect.x.is_finite() { rect.x } else { 0.0 },
        0.0,
        (canvas_w - w).max(0.0),
    );
    let y = settle_to_limit(
        if rect.y.is_finite() { rect.y } else { 0.0 },
        0.0,
        (canvas_h - h).max(0.0),
    );
    Rect { x, y, w, h }
}

// --- snap ---

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    pub x: f64,
    pub y: f64,
    pub guide_x: Option<f64>,
    pub guide_y: Option<f64>,
    pub guide_x2: Option<f64>,
    pub gap_x: Option<f64>,
    pub gap_y: Option<f64>,
}

pub fn zone_lines(r: Rect, axis_x: bool) -> [f64; 3] {
    let (a, b) = if axis_x { (r.x, r.w) } else { (r.y, r.h) };
    [a, a + b / 2.0, a + b]
}

pub fn nearest_line(value: f64, lines: &[f64], threshold: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for &line in lines {
        if (line - value).abs() <= threshold
            && best.is_none_or(|b| (line - value).abs() < (b - value).abs())
        {
            best = Some(line);
        }
    }
    best
}

/// Where a moving part lands (PreviewStage snapMove).
pub fn snap_move(moving: Rect, others: &[Rect], panels_x: &[f64], panels_y: &[f64], threshold: f64, free: bool) -> Snap {
    let none = Snap {
        x: moving.x,
        y: moving.y,
        guide_x: None,
        guide_y: None,
        guide_x2: None,
        gap_x: None,
        gap_y: None,
    };
    if free {
        return none;
    }
    let mut all_x: Vec<f64> = others.iter().flat_map(|o| zone_lines(*o, true)).collect();
    all_x.extend_from_slice(panels_x);
    let mut all_y: Vec<f64> = others.iter().flat_map(|o| zone_lines(*o, false)).collect();
    all_y.extend_from_slice(panels_y);
    let mut best_x: Option<(f64, f64)> = None;
    let mut best_y: Option<(f64, f64)> = None;
    for &line in &all_x {
        for &mine in &zone_lines(moving, true) {
            let delta = line - mine;
            if delta.abs() <= threshold
                && best_x.is_none_or(|(bd, _)| delta.abs() < bd.abs())
            {
                best_x = Some((delta, line));
            }
        }
    }
    for &line in &all_y {
        for &mine in &zone_lines(moving, false) {
            let delta = line - mine;
            if delta.abs() <= threshold
                && best_y.is_none_or(|(bd, _)| delta.abs() < bd.abs())
            {
                best_y = Some((delta, line));
            }
        }
    }
    Snap {
        x: best_x.map(|(d, _)| moving.x + d).unwrap_or(moving.x),
        y: best_y.map(|(d, _)| moving.y + d).unwrap_or(moving.y),
        guide_x: best_x.map(|(_, l)| l),
        guide_y: best_y.map(|(_, l)| l),
        guide_x2: None,
        gap_x: best_x.map(|(d, _)| d.abs()),
        gap_y: best_y.map(|(d, _)| d.abs()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqualSpacing {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub line_a: Option<f64>,
    pub line_b: Option<f64>,
}

/// The equal-spacing snap (PreviewStage equalSpacing).
pub fn equal_spacing(moving: Rect, others: &[Rect], threshold: f64) -> EqualSpacing {
    let mut best = EqualSpacing {
        x: None,
        y: None,
        line_a: None,
        line_b: None,
    };
    let overlaps_y = |a: Rect, b: Rect| a.y < b.y + b.h && b.y < a.y + a.h;
    let overlaps_x = |a: Rect, b: Rect| a.x < b.x + b.w && b.x < a.x + a.w;
    for a in others {
        for b in others {
            if std::ptr::eq(a, b) {
                continue;
            }
            #[allow(clippy::collapsible_if)]
            if a.x + a.w <= moving.x + 0.001 && b.x >= moving.x + moving.w - 0.001 {
                if overlaps_y(*a, moving) && overlaps_y(*b, moving) {
                    let gap_l = moving.x - (a.x + a.w);
                    let gap_r = b.x - (moving.x + moving.w);
                    if (gap_l - gap_r).abs() <= threshold {
                        let x = (a.x + a.w + b.x - moving.w) / 2.0;
                        if best.x.is_none_or(|bx| (x - moving.x).abs() < (bx - moving.x).abs()) {
                            best = EqualSpacing {
                                x: Some(x),
                                y: best.y,
                                line_a: Some(a.x + a.w),
                                line_b: Some(b.x),
                            };
                        }
                    }
                }
            }
            #[allow(clippy::collapsible_if)]
            if a.y + a.h <= moving.y + 0.001 && b.y >= moving.y + moving.h - 0.001 {
                if overlaps_x(*a, moving) && overlaps_x(*b, moving) {
                    let gap_t = moving.y - (a.y + a.h);
                    let gap_b = b.y - (moving.y + moving.h);
                    if (gap_t - gap_b).abs() <= threshold {
                        let y = (a.y + a.h + b.y - moving.h) / 2.0;
                        if best.y.is_none_or(|by| (y - moving.y).abs() < (by - moving.y).abs()) {
                            best = EqualSpacing {
                                x: best.x,
                                y: Some(y),
                                line_a: Some(a.y + a.h),
                                line_b: Some(b.y),
                            };
                        }
                    }
                }
            }
        }
    }
    best
}

// --- resize / rotate / zoom ---

/// Resizes a part by one handle (PreviewStage resizeBy).
pub fn resize_by(orig: Rect, rot_deg: f64, handle: Handle, p: Pt, shift: bool, alt: bool) -> Rect {
    let rad = rot_deg.to_radians();
    let (s, c) = (rad.sin(), rad.cos());
    let cx0 = orig.x + orig.w / 2.0;
    let cy0 = orig.y + orig.h / 2.0;
    let dx = p.x - cx0;
    let dy = p.y - cy0;
    let qx = dx * c + dy * s;
    let qy = -dx * s + dy * c;
    let mut w = orig.w;
    let mut h = orig.h;
    if handle.sx() != 0 {
        w = if alt {
            2.0 * handle.sx() as f64 * qx
        } else {
            handle.sx() as f64 * qx + orig.w / 2.0
        };
    }
    if handle.sy() != 0 {
        h = if alt {
            2.0 * handle.sy() as f64 * qy
        } else {
            handle.sy() as f64 * qy + orig.h / 2.0
        };
    }
    w = rubber_band(w, MIN_SIZE, CANVAS_W);
    h = rubber_band(h, MIN_SIZE, CANVAS_H);
    if shift {
        let want_s = if handle.sx() != 0 && handle.sy() != 0 {
            (w / orig.w).max(h / orig.h)
        } else if handle.sx() != 0 {
            w / orig.w
        } else {
            h / orig.h
        };
        let s_min = (MIN_SIZE / orig.w).max(MIN_SIZE / orig.h);
        let s_max = (CANVAS_W / orig.w).min(CANVAS_H / orig.h);
        let sc = want_s.min(s_max).max(s_min);
        w = orig.w * sc;
        h = orig.h * sc;
    }
    let (mut lx, mut ly) = (0.0, 0.0);
    if !alt {
        if handle.sx() != 0 {
            lx = (handle.sx() as f64 * (w - orig.w)) / 2.0;
        }
        if handle.sy() != 0 {
            ly = (handle.sy() as f64 * (h - orig.h)) / 2.0;
        }
    }
    let cx = cx0 + lx * c - ly * s;
    let cy = cy0 + lx * s + ly * c;
    Rect {
        x: cx - w / 2.0,
        y: cy - h / 2.0,
        w,
        h,
    }
}

/// The rotate handle's angle (PreviewStage): from the pointer, 0.1° rounds.
pub fn rotate_angle(centre: Pt, p: Pt, shift: bool) -> f64 {
    let mut deg = (p.y - centre.y).atan2(p.x - centre.x).to_degrees() + 90.0;
    deg = ((deg % 360.0) + 360.0) % 360.0;
    if shift {
        deg = (deg / 15.0).round() * 15.0;
    }
    ((deg % 360.0) * 10.0).round() / 10.0
}

/// Zoom about the view centre (PreviewStage zoomBy): factor 1.3 per key.
pub fn zoom_scale(scale: f64, factor: f64) -> f64 {
    (scale * factor).clamp(MIN_SCALE, MAX_SCALE)
}

/// Wheel zoom about the pointer (PreviewStage wheel): exp(-deltaY*0.0016).
pub fn wheel_factor(delta_y: f64) -> f64 {
    (-delta_y * 0.0016).exp()
}

/// The view that shows the whole desk (PreviewStage wholeView).
pub fn whole_view(w: f64, h: f64) -> (f64, f64, f64) {
    if w <= 0.0 || h <= 0.0 {
        return (1.0, 0.0, 0.0);
    }
    let base = w / CANVAS_W;
    let height_fit = h / CANVAS_H / base;
    (height_fit.min(1.0), 0.0, 0.0)
}

pub fn zoom_percent(scale: f64) -> i64 {
    (scale * 100.0).round() as i64
}

/// One arrow-key nudge step in canvas units (PreviewStage step).
pub fn nudge_step(shift: bool) -> f64 {
    (if shift { NUDGE_MM_BIG } else { NUDGE_MM }) / MM_PER_UNIT
}

pub fn clamp(v: f64, min: f64, max: f64) -> f64 {
    v.min(max).max(min)
}

/// Keeps the canvas in the stage (PreviewStage clampView).
pub fn clamp_view(scale: f64, pan_x: f64, pan_y: f64) -> (f64, f64, f64) {
    let scale = scale.clamp(MIN_SCALE, MAX_SCALE);
    let pan_x = if scale >= 1.0 {
        clamp(pan_x, CANVAS_W * (1.0 / scale - 1.0), 0.0)
    } else {
        CANVAS_W * (1.0 / scale - 1.0) / 2.0
    };
    let pan_y = if scale >= 1.0 {
        clamp(pan_y, CANVAS_H * (1.0 / scale - 1.0), 0.0)
    } else {
        CANVAS_H * (1.0 / scale - 1.0) / 2.0
    };
    (scale, pan_x, pan_y)
}

// --- undo history (LayoutView Edit) ---

/// The draft and its undo stacks: every commit pushes the old draft onto
/// `past` and clears `future` (LayoutView `commit`).
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    pub draft: Option<canvas_proto::layout::Layout>,
    pub past: Vec<canvas_proto::layout::Layout>,
    pub future: Vec<canvas_proto::layout::Layout>,
}

impl History {
    pub fn new() -> Self {
        History {
            draft: None,
            past: Vec::new(),
            future: Vec::new(),
        }
    }

    pub fn reset(&mut self, layout: canvas_proto::layout::Layout) {
        self.draft = Some(layout);
        self.past.clear();
        self.future.clear();
    }

    pub fn commit(&mut self, f: impl FnOnce(&canvas_proto::layout::Layout) -> canvas_proto::layout::Layout) {
        let Some(draft) = self.draft.clone() else { return };
        self.past.push(draft.clone());
        self.future.clear();
        self.draft = Some(f(&draft));
    }

    pub fn undo(&mut self) {
        let (Some(draft), Some(prev)) = (self.draft.clone(), self.past.last().cloned()) else {
            return;
        };
        self.past.pop();
        self.future.insert(0, draft);
        self.draft = Some(prev);
    }

    pub fn redo(&mut self) {
        let (Some(draft), Some(next)) = (self.draft.clone(), self.future.first().cloned()) else {
            return;
        };
        self.future.remove(0);
        self.past.push(draft);
        self.draft = Some(next);
    }
}

/// `withoutEffect` (LayoutView): the draft compared to the server copy
/// without its effect.
pub fn without_effect(l: &canvas_proto::layout::Layout) -> String {
    let mut v = serde_json::to_value(l).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.insert("effect".to_string(), serde_json::Value::Null);
    }
    serde_json::to_string(&v).unwrap_or_default()
}

pub fn is_dirty(draft: Option<&canvas_proto::layout::Layout>, base: Option<&canvas_proto::layout::Layout>) -> bool {
    match (draft, base) {
        (Some(d), Some(b)) => without_effect(d) != without_effect(b),
        _ => false,
    }
}

/// What Save sends (LayoutView save): the draft with the server's effect.
pub fn save_body(draft: &canvas_proto::layout::Layout, server_effect: Option<canvas_proto::layout::ActiveEffect>) -> canvas_proto::layout::Layout {
    let mut body = draft.clone();
    body.effect = server_effect;
    body
}

// --- place / add ---

/// Where a part starts out (editor/parts.ts PART_SIZE).
pub fn part_size(kind: PartKind) -> (f64, f64) {
    match kind {
        PartKind::Fan => (30.0, 30.0),
        PartKind::Radiator => (90.0, 10.0),
        PartKind::Pump => (20.0, 20.0),
        PartKind::Ram => (6.0, 34.0),
        PartKind::Gpu => (70.0, 14.0),
        PartKind::Cpu => (16.0, 16.0),
        PartKind::Strip => (120.0, 5.0),
        PartKind::Keyboard => (110.0, 30.0),
        PartKind::Mouse => (22.0, 32.0),
        PartKind::Microphone => (16.0, 28.0),
        PartKind::Headset => (30.0, 30.0),
        PartKind::Case => (120.0, 122.0),
        PartKind::Generic => (24.0, 24.0),
    }
}

pub fn part_label(kind: PartKind) -> &'static str {
    match kind {
        PartKind::Fan => "Fan",
        PartKind::Radiator => "Radiator",
        PartKind::Pump => "Pump",
        PartKind::Ram => "RAM",
        PartKind::Gpu => "Graphics card",
        PartKind::Cpu => "CPU",
        PartKind::Strip => "Strip",
        PartKind::Keyboard => "Keyboard",
        PartKind::Mouse => "Mouse",
        PartKind::Microphone => "Microphone",
        PartKind::Headset => "Headset",
        PartKind::Case => "Case",
        PartKind::Generic => "Box",
    }
}

pub const LANDMARK_KINDS: [PartKind; 4] = [
    PartKind::Cpu,
    PartKind::Gpu,
    PartKind::Generic,
    PartKind::Case,
];

/// A landmark at the desk centre (LayoutView addLandmark).
pub fn landmark_at_centre(kind: PartKind) -> Landmark {
    let (w, h) = part_size(kind);
    Landmark {
        kind,
        x: (CANVAS_W / 2.0 - w / 2.0) as f32,
        y: (CANVAS_H / 2.0 - h / 2.0) as f32,
        width: w as f32,
        height: h as f32,
        rotation_deg: 0.0,
        facing: canvas_proto::layout::Facing::Viewer,
        visible: true,
        label: None,
        locked: false,
    }
}

/// A zone placed at the desk centre (LayoutView place).
pub fn placed_zone(
    device_id: String,
    zone_index: u32,
    shape: canvas_proto::layout::ZoneShape,
    w: f64,
    h: f64,
    at: Option<(f64, f64)>,
) -> PlacedZone {
    let (cx, cy) = at.unwrap_or((CANVAS_W / 2.0, CANVAS_H / 2.0));
    PlacedZone {
        device_id: canvas_proto::device::DeviceId(device_id),
        zone_index,
        x: clamp(cx - w / 2.0, 0.0, CANVAS_W - w) as f32,
        y: clamp(cy - h / 2.0, 0.0, CANVAS_H - h) as f32,
        width: w as f32,
        height: h as f32,
        rotation_deg: 0.0,
        reversed: false,
        visible: true,
        shape,
        label: None,
        leds: None,
        part: None,
        locked: false,
    }
}

/// The preview line (LayoutView previewLine): returns (text, down).
/// `preview_state`: "live" | "refused" | other. Line 452's string is
/// unreachable in React (the checklist notes it); the port returns it only
/// for the same combination, and the test pins that the earlier lines win.
pub fn preview_line(
    state: &str,
    error: Option<&str>,
    editor_draws: bool,
    effect_needs_input: bool,
    running_effect: bool,
) -> Option<(String, bool)> {
    if state == "live" {
        return None;
    }
    if editor_draws {
        return Some((
            "Preview drawn by the editor · the live preview is not available yet".to_string(),
            false,
        ));
    }
    if state == "refused" {
        let text = format!("The live preview is not connected. {}", error.unwrap_or("")).trim().to_string();
        return Some((text, true));
    }
    if effect_needs_input {
        return Some((
            "This effect reads the screen or the audio, so the preview is the frame from canvas-rgb.".to_string(),
            true,
        ));
    }
    if !running_effect {
        return Some(("No effect is running on this layout.".to_string(), true));
    }
    Some(("The live preview is not available yet.".to_string(), false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(r: Ref, x: f64, y: f64, w: f64, h: f64) -> HitPart {
        HitPart {
            r,
            x,
            y,
            w,
            h,
            rot: 0.0,
            is_case: false,
            locked: false,
        }
    }

    #[test]
    fn drag_and_handle_reach_are_4_and_8_px_for_a_mouse() {
        assert_eq!(drag_threshold_px(false), 4.0);
        assert_eq!(drag_threshold_px(true), 8.0);
        assert_eq!(handle_reach_px(false), 8.0);
        assert_eq!(handle_reach_px(true), 12.0);
    }

    #[test]
    fn hit_test_picks_the_smallest_part_then_the_case() {
        let big = part(Ref::Zone(0), 0.0, 0.0, 100.0, 100.0);
        let small = part(Ref::Zone(1), 40.0, 40.0, 20.0, 20.0);
        let mut case = part(Ref::Landmark(0), 0.0, 0.0, 200.0, 200.0);
        case.is_case = true;
        let parts = vec![big, small, case];
        let hit = hit_test(&parts, &[], Pt { x: 50.0, y: 50.0 }, 1.0, false, false);
        assert_eq!(hit, Hit::Part(Ref::Zone(1), false));
        // Inside the case but no other part: the case body.
        let hit = hit_test(&parts, &[], Pt { x: 180.0, y: 180.0 }, 1.0, false, false);
        assert_eq!(hit, Hit::CaseBody(Ref::Landmark(0), false));
        // At the case's edge band: case-edge.
        let hit = hit_test(&parts, &[], Pt { x: 4.0, y: 150.0 }, 1.0, false, false);
        assert_eq!(hit, Hit::CaseEdge(Ref::Landmark(0), false));
        // Empty desk.
        assert_eq!(
            hit_test(&parts, &[], Pt { x: 300.0, y: 300.0 }, 1.0, false, false),
            Hit::Desk
        );
    }

    #[test]
    fn alt_click_takes_the_next_part_down() {
        let big = part(Ref::Zone(0), 0.0, 0.0, 100.0, 100.0);
        let small = part(Ref::Zone(1), 40.0, 40.0, 20.0, 20.0);
        let parts = vec![big, small];
        let hit = hit_test(
            &parts,
            &[Ref::Zone(1)],
            Pt { x: 50.0, y: 50.0 },
            1.0,
            false,
            true,
        );
        assert_eq!(hit, Hit::Part(Ref::Zone(0), false));
    }

    #[test]
    fn handles_are_offered_for_a_single_unlocked_pick() {
        let p = part(Ref::Zone(0), 100.0, 100.0, 40.0, 40.0);
        let parts = vec![p];
        // On the se corner.
        let hit = hit_test(&parts, &[Ref::Zone(0)], Pt { x: 140.0, y: 140.0 }, 1.0, false, false);
        assert_eq!(hit, Hit::Handle(Ref::Zone(0), Handle::Se));
        // On the rotate grip: 24 px above the top edge.
        let hit = hit_test(&parts, &[Ref::Zone(0)], Pt { x: 120.0, y: 76.0 }, 1.0, false, false);
        assert_eq!(hit, Hit::Rotate(Ref::Zone(0)));
        // Locked: no handles.
        let locked = vec![HitPart { locked: true, ..p }];
        let hit = hit_test(&locked, &[Ref::Zone(0)], Pt { x: 140.0, y: 140.0 }, 1.0, false, false);
        assert_eq!(hit, Hit::Part(Ref::Zone(0), true));
    }

    #[test]
    fn a_narrow_part_drops_its_edge_handles() {
        // 40 px wide at k=1: under 44, so n/s are gone but corners stay.
        let p = part(Ref::Zone(0), 100.0, 100.0, 40.0, 100.0);
        let parts = vec![p];
        let hit = hit_test(&parts, &[Ref::Zone(0)], Pt { x: 120.0, y: 100.0 }, 1.0, false, false);
        assert_ne!(hit, Hit::Handle(Ref::Zone(0), Handle::N));
        let hit = hit_test(&parts, &[Ref::Zone(0)], Pt { x: 100.0, y: 100.0 }, 1.0, false, false);
        assert_eq!(hit, Hit::Handle(Ref::Zone(0), Handle::Nw));
    }

    #[test]
    fn the_rubber_band_resists_past_the_limit() {
        // Inside: exact.
        assert_eq!(rubber_band(50.0, 0.0, 320.0), 50.0);
        // Past the max: resisted with RUBBER_C 0.55.
        let got = rubber_band(400.0, 0.0, 320.0);
        assert!(got > 320.0 && got < 400.0);
        assert_eq!(got, 320.0 + resisted_overshoot(80.0, 320.0, RUBBER_C));
        // The settle clamps back.
        assert_eq!(settle_to_limit(got, 0.0, 320.0), 320.0);
        assert_eq!(settle_rect(Rect { x: -10.0, y: 0.0, w: 400.0, h: 10.0 }, 320.0, 200.0, 2.0), Rect { x: 0.0, y: 0.0, w: 320.0, h: 10.0 });
    }

    #[test]
    fn snap_move_lines_up_with_a_neighbour() {
        let moving = Rect { x: 98.0, y: 50.0, w: 20.0, h: 20.0 };
        let other = Rect { x: 118.0, y: 50.0, w: 20.0, h: 20.0 };
        // Moving's right edge (118) is other's left edge (118): already there.
        let snap = snap_move(moving, &[other], &[], &[], 2.0, false);
        assert_eq!(snap.x, 98.0);
        assert_eq!(snap.guide_x, Some(118.0));
        // Free: untouched.
        let snap = snap_move(moving, &[other], &[], &[], 2.0, true);
        assert_eq!(snap.guide_x, None);
        assert_eq!(snap.x, 98.0);
    }

    #[test]
    fn equal_spacing_centres_a_part_between_two() {
        let moving = Rect { x: 101.0, y: 50.0, w: 20.0, h: 20.0 };
        let left = Rect { x: 60.0, y: 50.0, w: 20.0, h: 20.0 };
        let right = Rect { x: 140.0, y: 50.0, w: 20.0, h: 20.0 };
        let eq = equal_spacing(moving, &[left, right], 5.0);
        // Gaps 21 and 19: within 5, centre at (80+140-20)/2 = 100.
        assert_eq!(eq.x, Some(100.0));
        assert_eq!(eq.line_a, Some(80.0));
        assert_eq!(eq.line_b, Some(140.0));
    }

    #[test]
    fn resize_keeps_the_opposite_edge_fixed() {
        let orig = Rect { x: 100.0, y: 100.0, w: 40.0, h: 40.0 };
        // Drag the se corner to (150,150): width 50, left edge stays 100.
        let next = resize_by(orig, 0.0, Handle::Se, Pt { x: 150.0, y: 150.0 }, false, false);
        assert_eq!(next, Rect { x: 100.0, y: 100.0, w: 50.0, h: 50.0 });
        // Alt resizes about the centre.
        let next = resize_by(orig, 0.0, Handle::Se, Pt { x: 150.0, y: 150.0 }, false, true);
        assert_eq!(next, Rect { x: 90.0, y: 90.0, w: 60.0, h: 60.0 });
    }

    #[test]
    fn rotate_angle_reads_from_the_pointer() {
        let centre = Pt { x: 0.0, y: 0.0 };
        // Straight up is 0°.
        assert_eq!(rotate_angle(centre, Pt { x: 0.0, y: -10.0 }, false), 0.0);
        // Right is 90°.
        assert_eq!(rotate_angle(centre, Pt { x: 10.0, y: 0.0 }, false), 90.0);
        // Shift snaps to 15°.
        assert_eq!(rotate_angle(centre, Pt { x: 10.0, y: -1.0 }, true), 90.0);
    }

    #[test]
    fn zoom_and_wheel_numbers() {
        assert_eq!(zoom_scale(1.0, 1.3), 1.3);
        assert_eq!(zoom_scale(1.0, 1.0 / 1.3), 1.0 / 1.3);
        assert_eq!(zoom_scale(8.0, 1.3), 8.0);
        assert_eq!(zoom_scale(0.25, 1.0 / 1.3), 0.25);
        assert!((wheel_factor(100.0) - (-100.0f64 * 0.0016).exp()).abs() < 1e-12);
        assert_eq!(zoom_percent(1.35), 135);
        assert_eq!(whole_view(320.0, 200.0), (1.0, 0.0, 0.0));
    }

    #[test]
    fn nudge_is_1mm_or_10mm_with_shift() {
        assert_eq!(nudge_step(false), 0.25);
        assert_eq!(nudge_step(true), 2.5);
    }

    #[test]
    fn history_is_one_step_per_commit() {
        use canvas_proto::layout::Layout;
        let base: Layout = serde_json::from_str(
            r#"{"id":"d","name":"Default","render_scale":50,"sampling":"area_average","zones":[],"effect":null,"case_form":"none","landmarks":[]}"#,
        )
        .unwrap();
        let mut h = History::new();
        h.reset(base.clone());
        assert!(!is_dirty(h.draft.as_ref(), Some(&base)));
        h.commit(|l| {
            let mut l = l.clone();
            l.name = "Changed".to_string();
            l
        });
        assert_eq!(h.past.len(), 1);
        assert!(is_dirty(h.draft.as_ref(), Some(&base)));
        h.undo();
        assert_eq!(h.draft.as_ref().unwrap().name, "Default");
        assert_eq!(h.future.len(), 1);
        h.redo();
        assert_eq!(h.draft.as_ref().unwrap().name, "Changed");
    }

    #[test]
    fn a_case_carries_the_parts_inside_it() {
        let mut case = part(Ref::Landmark(0), 0.0, 0.0, 200.0, 200.0);
        case.is_case = true;
        case.locked = false;
        let inside = part(Ref::Zone(0), 50.0, 50.0, 20.0, 20.0);
        let outside = part(Ref::Zone(1), 210.0, 210.0, 20.0, 20.0);
        let locked = HitPart { locked: true, ..inside };
        let got = parts_inside_case(&case, &[case, inside, outside, locked]);
        assert_eq!(got, vec![Ref::Zone(0)]);
    }

    #[test]
    fn the_marquee_takes_a_case_only_when_it_holds_all_of_it() {
        let mut case = part(Ref::Landmark(0), 0.0, 0.0, 100.0, 100.0);
        case.is_case = true;
        let zone = part(Ref::Zone(0), 10.0, 10.0, 20.0, 20.0);
        // Band inside the case, touching the zone: zone only.
        let got = marquee_pick(Rect { x: 0.0, y: 0.0, w: 50.0, h: 50.0 }, &[case, zone]);
        assert_eq!(got, vec![Ref::Zone(0)]);
        // Band holding the whole case: both.
        let got = marquee_pick(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }, &[case, zone]);
        assert_eq!(got, vec![Ref::Landmark(0), Ref::Zone(0)]);
    }

    #[test]
    fn add_part_sizes_and_centres() {
        let cpu = landmark_at_centre(PartKind::Cpu);
        assert_eq!((cpu.x, cpu.y, cpu.width, cpu.height), (152.0, 92.0, 16.0, 16.0));
        let case = landmark_at_centre(PartKind::Case);
        assert_eq!((case.x, case.y, case.width, case.height), (100.0, 39.0, 120.0, 122.0));
        assert_eq!(part_label(PartKind::Generic), "Box");
        assert_eq!(
            LANDMARK_KINDS,
            [PartKind::Cpu, PartKind::Gpu, PartKind::Generic, PartKind::Case]
        );
    }

    #[test]
    fn the_preview_line_prefers_the_editor_line() {
        // Running effect, no input: editor line wins over line 452's string.
        let line = preview_line("other", None, true, false, true).unwrap();
        assert!(!line.1);
        assert!(line.0.contains("Preview drawn by the editor"));
        // No effect: warn.
        let line = preview_line("other", None, false, false, false).unwrap();
        assert_eq!(line.0, "No effect is running on this layout.");
        assert!(line.1);
        // Live: none.
        assert_eq!(preview_line("live", None, false, false, false), None);
    }
}
