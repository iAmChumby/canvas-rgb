//! The desk's geometry, ported from `app/web/src/lib/zones.ts` (and the
//! case tables in `lib/caseForms.ts`, in `case_forms.rs` beside this).
//!
//! The canvas is 320 x 200 units: a placed zone is a rectangle in that space,
//! and its LEDs sit inside the rectangle at the places the engine samples
//! them. The numbers here are the same ones React produces, computed in
//! `f64` so the last digit matches (`0.8400000000000001`, not `0.84`).
//!
//! The tests at the bottom pin the numbers React's own code produces for the
//! demo desk (`openrgb:mb#3`, `openrgb:kb#0`, `openrgb:mic#0`,
//! `openrgb:dram-a#0`, `openrgb:dram-b#0`).

use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::{PartKind, PlacedZone, ZoneShape};

/// The logical canvas, in canvas units.
pub const CANVAS_W: f64 = 320.0;
pub const CANVAS_H: f64 = 200.0;

/// Millimetres to a canvas unit (`MM_PER_UNIT` in `lib/caseForms.ts`).
pub const MM_PER_UNIT: f64 = 4.0;

/// The unit every size is read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Cm,
    In,
}

impl LengthUnit {
    /// The word the unit is written with (`cm`, `in`).
    pub fn word(self) -> &'static str {
        match self {
            LengthUnit::Cm => "cm",
            LengthUnit::In => "in",
        }
    }

    /// The regions that read in inches (`lib/zones.ts` `IMPERIAL_REGIONS`).
    pub fn for_region(region: &str) -> LengthUnit {
        match region.to_ascii_uppercase().as_str() {
            "US" | "LR" | "MM" => LengthUnit::In,
            _ => LengthUnit::Cm,
        }
    }
}

/// A length in millimetres, from a length in canvas units.
pub fn units_to_mm(units: f64) -> f64 {
    units * MM_PER_UNIT
}

/// A length in canvas units, from a length in millimetres.
pub fn mm_to_units(mm: f64) -> f64 {
    mm / MM_PER_UNIT
}

/// A length in the reader's unit, in words: "12.0 cm" or "4.72 in".
pub fn format_length(mm: f64, unit: LengthUnit) -> String {
    match unit {
        LengthUnit::In => format!("{:.2} in", mm / 25.4),
        LengthUnit::Cm => format!("{:.1} cm", mm / 10.0),
    }
}

/// The same, from a length in canvas units.
pub fn format_units(units: f64, unit: LengthUnit) -> String {
    format_length(units_to_mm(units), unit)
}

/// The number of the reader's unit for a length in millimetres.
pub fn length_number(mm: f64, unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::In => mm / 25.4,
        LengthUnit::Cm => mm / 10.0,
    }
}

/// A length in millimetres, from a number of the reader's unit.
pub fn mm_from_length(value: f64, unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::In => value * 25.4,
        LengthUnit::Cm => value * 10.0,
    }
}

/// A point on the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// The zone's centre on the canvas and its turn, as cos/sin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneTransform {
    pub cx: f64,
    pub cy: f64,
    pub cos: f64,
    pub sin: f64,
}

/// The shape a zone's LEDs really follow, once `auto` is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedShape {
    Strip,
    Ring,
    Outline,
    Point,
    Grid,
    Matrix,
}

/// One LED in the zone's own frame (the origin at the rectangle's centre,
/// before the turn).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalLed {
    pub x: f64,
    pub y: f64,
    /// The cell's size: the region the LED samples.
    pub w: f64,
    pub h: f64,
}

/// Where a zone's LEDs sit.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalLeds {
    pub shape: ResolvedShape,
    /// One entry per LED, in zone order; `None` for an LED with no cell (a
    /// gap in a matrix map).
    pub leds: Vec<Option<LocalLed>>,
    /// How big a dot for one LED may be, in canvas units.
    pub radius: f64,
}

/// A cell on the canvas: where an LED samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasCell {
    pub cx: f64,
    pub cy: f64,
    /// The cell's corners, `x0, y0, x1, y1`.
    pub bounds: [f64; 4],
}

/// The key of one placement: the zone it belongs to and, when the zone is
/// split, where its run of LEDs starts.
pub fn zone_key(z: &PlacedZone) -> String {
    let base = format!("{}#{}", z.device_id.0, z.zone_index);
    match &z.leds {
        Some(slice) => format!("{base}@{}", slice.start),
        None => base,
    }
}

/// The zone's centre on the canvas and its turn.
pub fn zone_transform(z: &PlacedZone) -> ZoneTransform {
    let rad = (z.rotation_deg as f64).to_radians();
    ZoneTransform {
        cx: z.x as f64 + z.width as f64 / 2.0,
        cy: z.y as f64 + z.height as f64 / 2.0,
        cos: rad.cos(),
        sin: rad.sin(),
    }
}

/// The zone's corners on the canvas, clockwise from the top left.
pub fn zone_corners(z: &PlacedZone) -> [Point; 4] {
    let t = zone_transform(z);
    let hw = z.width as f64 / 2.0;
    let hh = z.height as f64 / 2.0;
    let local = [
        Point { x: -hw, y: -hh },
        Point { x: hw, y: -hh },
        Point { x: hw, y: hh },
        Point { x: -hw, y: hh },
    ];
    let mut out = [Point { x: 0.0, y: 0.0 }; 4];
    for (slot, p) in out.iter_mut().zip(local) {
        *slot = Point {
            x: t.cx + p.x * t.cos - p.y * t.sin,
            y: t.cy + p.x * t.sin + p.y * t.cos,
        };
    }
    out
}

/// What `auto` means for a zone: a single zone is a point, a linear one a
/// strip, a matrix its own map.
pub fn resolve_shape(shape: &ZoneShape, kind: &ZoneKind) -> ResolvedShape {
    match shape {
        ZoneShape::Auto => match kind {
            ZoneKind::Single => ResolvedShape::Point,
            ZoneKind::Linear => ResolvedShape::Strip,
            ZoneKind::Matrix { .. } => ResolvedShape::Matrix,
        },
        ZoneShape::Strip => ResolvedShape::Strip,
        ZoneShape::Ring => ResolvedShape::Ring,
        ZoneShape::Outline => ResolvedShape::Outline,
        ZoneShape::Point => ResolvedShape::Point,
        ZoneShape::Grid { .. } => ResolvedShape::Grid,
    }
}

/// Where each LED of a zone sits in the zone's own frame.
///
/// This is the daemon's own geometry (`build_zone_leds` in
/// `crates/canvas-core/src/geometry.rs`), so a dot on the desk and the LED
/// it stands for are the same place:
///
/// - `strip`: cells along the long axis
/// - `ring`: on the inscribed ellipse, clockwise from the top
/// - `outline`: on the rectangle's edge, clockwise from the top left
/// - `point`: every LED samples the whole rectangle
/// - `grid`: row by row, top left first
/// - `matrix`: the device's own map
///
/// `reversed` flips the order of the LEDs along the shape; a matrix keeps
/// its map.
pub fn zone_local_leds(z: &PlacedZone, info: &ZoneInfo) -> LocalLeds {
    let shape = resolve_shape(&z.shape, &info.kind);
    let total = info.led_count as usize;
    let slice = z.leds.as_ref();
    // A matrix zone is never split: its map owns every LED.
    let split = slice.is_some() && shape != ResolvedShape::Matrix;
    let count = match (split, slice) {
        (true, Some(s)) => s.count as usize,
        _ => total,
    };
    let take = match (split, slice) {
        (true, Some(s)) => (s.count as usize).min(total.saturating_sub(s.start as usize)),
        _ => total,
    };
    let w = z.width as f64;
    let h = z.height as f64;
    let mut leds: Vec<Option<LocalLed>> = Vec::new();
    if take == 0 {
        return LocalLeds {
            shape,
            leds,
            radius: 1.0,
        };
    }
    if shape == ResolvedShape::Matrix {
        if let ZoneKind::Matrix {
            width,
            height,
            map,
        } = &info.kind
        {
            let width = *width as usize;
            let height = *height as usize;
            let cw = w / width as f64;
            let ch = h / height as f64;
            leds.resize(total, None);
            for r in 0..height {
                for c in 0..width {
                    let Some(idx) = map.get(r * width + c).copied().flatten() else {
                        continue;
                    };
                    if idx as usize >= total {
                        continue;
                    }
                    leds[idx as usize] = Some(LocalLed {
                        x: -w / 2.0 + (c as f64 + 0.5) * cw,
                        y: -h / 2.0 + (r as f64 + 0.5) * ch,
                        w: cw,
                        h: ch,
                    });
                }
            }
            return LocalLeds {
                shape,
                leds,
                radius: cw.min(ch) * 0.22,
            };
        }
    }
    let horizontal = w >= h;
    let perimeter_of_rect = 2.0 * (w + h);
    let a = w / 2.0;
    let b = h / 2.0;
    // Ramanujan's approximation of the ellipse perimeter, as the daemon uses.
    let perimeter_of_ellipse =
        std::f64::consts::PI * (3.0 * (a + b) - ((3.0 * a + b) * (a + 3.0 * b)).sqrt());
    let cols = match &z.shape {
        ZoneShape::Grid { columns } => (*columns as usize).max(1).min(count),
        _ => 1,
    };
    let rows = count.div_ceil(cols);
    for i in 0..take {
        let slot = if z.reversed { count - 1 - i } else { i };
        let led = match shape {
            ResolvedShape::Point | ResolvedShape::Matrix => LocalLed {
                x: 0.0,
                y: 0.0,
                w,
                h,
            },
            ResolvedShape::Strip => {
                if horizontal {
                    LocalLed {
                        x: -w / 2.0 + ((slot as f64 + 0.5) * w) / count as f64,
                        y: 0.0,
                        w: w / count as f64,
                        h,
                    }
                } else {
                    LocalLed {
                        x: 0.0,
                        y: -h / 2.0 + ((slot as f64 + 0.5) * h) / count as f64,
                        w,
                        h: h / count as f64,
                    }
                }
            }
            ResolvedShape::Ring => {
                let size = (perimeter_of_ellipse / count as f64).min(w.min(h));
                let t = 2.0 * std::f64::consts::PI * slot as f64 / count as f64;
                LocalLed {
                    x: a * t.sin(),
                    y: -b * t.cos(),
                    w: size,
                    h: size,
                }
            }
            ResolvedShape::Outline => {
                let size = (perimeter_of_rect / count as f64).min(w.min(h));
                let d = perimeter_of_rect * slot as f64 / count as f64;
                let (x, y) = if d < w {
                    (-a + d, -b)
                } else if d < w + h {
                    (a, -b + (d - w))
                } else if d < 2.0 * w + h {
                    (a - (d - w - h), b)
                } else {
                    (-a, b - (d - 2.0 * w - h))
                };
                LocalLed {
                    x,
                    y,
                    w: size,
                    h: size,
                }
            }
            ResolvedShape::Grid => {
                let cw = w / cols as f64;
                let ch = h / rows as f64;
                LocalLed {
                    x: -a + ((slot % cols) as f64 + 0.5) * cw,
                    y: -b + ((slot / cols) as f64 + 0.5) * ch,
                    w: cw,
                    h: ch,
                }
            }
        };
        leds.push(Some(led));
    }
    // The dot's size: about a fifth of the gap between neighbours, and no
    // more than a seventh of the zone's cross-section.
    let radius = match shape {
        ResolvedShape::Strip => {
            if horizontal {
                ((w / count as f64) * 0.22).min(h * 0.14)
            } else {
                ((h / count as f64) * 0.22).min(w * 0.14)
            }
        }
        ResolvedShape::Ring => ((perimeter_of_ellipse / count as f64) * 0.22).min(w.min(h) * 0.1),
        ResolvedShape::Outline => ((perimeter_of_rect / count as f64) * 0.22).min(w.min(h) * 0.1),
        ResolvedShape::Grid => (w / cols as f64).min(h / rows as f64) * 0.22,
        _ => w.min(h) * 0.22,
    };
    LocalLeds {
        shape,
        leds,
        radius: radius.max(0.05),
    }
}

/// An LED's cell on the canvas: the zone's transform applied to a local
/// cell.
pub fn canvas_cell(t: &ZoneTransform, led: &LocalLed) -> CanvasCell {
    let hw = led.w / 2.0;
    let hh = led.h / 2.0;
    let mut x0 = f64::INFINITY;
    let mut y0 = f64::INFINITY;
    let mut x1 = f64::NEG_INFINITY;
    let mut y1 = f64::NEG_INFINITY;
    for (dx, dy) in [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)] {
        let lx = led.x + dx;
        let ly = led.y + dy;
        let x = t.cx + lx * t.cos - ly * t.sin;
        let y = t.cy + lx * t.sin + ly * t.cos;
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    CanvasCell {
        cx: t.cx + led.x * t.cos - led.y * t.sin,
        cy: t.cy + led.x * t.sin + led.y * t.cos,
        bounds: [x0, y0, x1, y1],
    }
}

/// The shape a zone gets when it is first placed: fans are rings, RAM and
/// ARGB headers are strips, a single-LED peripheral is a point and a matrix
/// keeps its map.
pub fn default_shape(device_kind: DeviceKind, info: &ZoneInfo) -> ZoneShape {
    if matches!(info.kind, ZoneKind::Matrix { .. }) {
        return ZoneShape::Auto;
    }
    if info.name.to_lowercase().contains("fan") && info.led_count > 1 {
        return ZoneShape::Ring;
    }
    if matches!(info.kind, ZoneKind::Single) || info.led_count == 1 {
        return ZoneShape::Point;
    }
    if device_kind == DeviceKind::Keyboard {
        return ZoneShape::Grid {
            columns: info.led_count.div_ceil(6).max(1),
        };
    }
    ZoneShape::Strip
}

/// The logical size of a zone when it is first placed.
pub fn default_rect(info: &ZoneInfo, device_kind: DeviceKind, shape: &ZoneShape) -> (f64, f64) {
    match shape {
        ZoneShape::Ring => (40.0, 40.0),
        _ if device_kind == DeviceKind::Dram => (16.0, 120.0),
        _ => match &info.kind {
            ZoneKind::Matrix { width, height, .. } => {
                let aspect = *width as f64 / (*height as f64).max(1.0);
                let width = 80.0 * aspect.clamp(0.6, 1.6);
                (width, width / aspect.max(0.1))
            }
            _ => {
                if device_kind == DeviceKind::Keyboard {
                    if matches!(shape, ZoneShape::Grid { .. }) {
                        (120.0, 38.0)
                    } else {
                        (110.0, 30.0)
                    }
                } else if matches!(shape, ZoneShape::Point) {
                    match device_kind {
                        DeviceKind::Mouse => (22.0, 32.0),
                        DeviceKind::Microphone => (16.0, 28.0),
                        DeviceKind::Headset => (30.0, 30.0),
                        _ => (20.0, 20.0),
                    }
                } else {
                    (120.0, 10.0)
                }
            }
        },
    }
}

/// The box a zone takes when its shape changes, so the new shape starts out
/// looking right. The centre never moves.
pub fn reshape_box(z: &PlacedZone, shape: &ZoneShape) -> ReshapedBox {
    let w = z.width as f64;
    let h = z.height as f64;
    let ratio = w.max(h) / w.min(h).max(0.01);
    let mut nw = w;
    let mut nh = h;
    if matches!(shape, ZoneShape::Ring) && ratio > 1.5 {
        nw = (w.max(h) * 0.6).clamp(24.0, 48.0);
        nh = nw;
    } else if matches!(shape, ZoneShape::Strip) && ratio < 3.0 {
        nw = (w.max(h) * 2.5).clamp(60.0, 140.0);
        nh = 10.0;
    }
    if nw == w && nh == h {
        return ReshapedBox {
            shape: shape.clone(),
            width: w,
            height: h,
            x: z.x as f64,
            y: z.y as f64,
        };
    }
    let cx = z.x as f64 + w / 2.0;
    let cy = z.y as f64 + h / 2.0;
    ReshapedBox {
        shape: shape.clone(),
        width: nw,
        height: nh,
        x: (cx - nw / 2.0).clamp(0.0, CANVAS_W - nw),
        y: (cy - nh / 2.0).clamp(0.0, CANVAS_H - nh),
    }
}

/// What a shape change does to a zone's box.
#[derive(Debug, Clone, PartialEq)]
pub struct ReshapedBox {
    pub shape: ZoneShape,
    pub width: f64,
    pub height: f64,
    pub x: f64,
    pub y: f64,
}

/// What a zone is called to a person: the plain name and, muted beside it,
/// the hardware's own.
#[derive(Debug, Clone, PartialEq)]
pub struct PlainName {
    pub name: String,
    pub hardware: Option<String>,
    pub kind: Option<DeviceKind>,
    /// The zone is a fan (a ring of LEDs round a hub).
    pub fan: bool,
}

/// The plain name of a zone: "RAM 2", "ARGB header 1", "Keyboard".
pub fn plain_name(
    devices: &[DeviceInfo],
    device_id: &str,
    zone_index: u32,
) -> PlainName {
    let Some(device) = devices.iter().find(|d| d.id.0 == device_id) else {
        return PlainName {
            name: "Not connected".to_string(),
            hardware: None,
            kind: None,
            fan: false,
        };
    };
    let Some(zone) = device.zones.iter().find(|z| z.index == zone_index) else {
        return PlainName {
            name: "Not connected".to_string(),
            hardware: None,
            kind: None,
            fan: false,
        };
    };
    let zn = zone.name.trim().to_string();
    let fan = zn.to_lowercase().contains("fan") && zone.led_count > 1;
    match device.kind {
        DeviceKind::Dram => {
            let sticks: Vec<&DeviceInfo> =
                devices.iter().filter(|d| d.kind == DeviceKind::Dram).collect();
            let n = sticks
                .iter()
                .position(|d| d.id.0 == device.id.0)
                .map(|i| i + 1)
                .unwrap_or(1);
            PlainName {
                name: format!("RAM {n}"),
                hardware: hardware_name(&zn, &device.name),
                kind: Some(DeviceKind::Dram),
                fan: false,
            }
        }
        DeviceKind::Keyboard => PlainName {
            name: "Keyboard".to_string(),
            hardware: hardware_name(&zn, &device.name),
            kind: Some(DeviceKind::Keyboard),
            fan: false,
        },
        DeviceKind::Mouse => PlainName {
            name: "Mouse".to_string(),
            hardware: hardware_name(&zn, &device.name),
            kind: Some(DeviceKind::Mouse),
            fan: false,
        },
        DeviceKind::Microphone => PlainName {
            name: "Microphone".to_string(),
            hardware: hardware_name(&zn, &device.name),
            kind: Some(DeviceKind::Microphone),
            fan: false,
        },
        DeviceKind::Headset => PlainName {
            name: "Headset".to_string(),
            hardware: hardware_name(&zn, &device.name),
            kind: Some(DeviceKind::Headset),
            fan: false,
        },
        DeviceKind::Gpu => PlainName {
            name: "Graphics card".to_string(),
            hardware: hardware_name(&zn, if zn.is_empty() { &device.name } else { &zn }),
            kind: Some(DeviceKind::Gpu),
            fan: false,
        },
        _ => {
            if fan {
                let hardware = hardware_name(&zn, &device.name);
                return PlainName {
                    name: zn,
                    hardware,
                    kind: Some(device.kind),
                    fan: true,
                };
            }
            if let Some(word) = header_word(&zn) {
                let digits: String = zn
                    .chars()
                    .skip_while(|c| !c.is_ascii_digit())
                    .collect();
                let name = if digits.is_empty() {
                    word.to_string()
                } else {
                    format!("{word} {digits}")
                };
                return PlainName {
                    name,
                    hardware: Some(zn),
                    kind: Some(device.kind),
                    fan: false,
                };
            }
            let hardware = hardware_name(&zn, &device.name);
            PlainName {
                name: if zn.is_empty() {
                    device.name.clone()
                } else {
                    zn
                },
                hardware,
                kind: Some(device.kind),
                fan: false,
            }
        }
    }
}

/// The hardware's own name for a zone, unless it is the zone's own name:
/// the two are the same string when the device calls its zone after itself.
fn hardware_name(zone: &str, candidate: &str) -> Option<String> {
    if !candidate.is_empty() && candidate != zone {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// The word a header zone's name starts with, when it is one of the known
/// headers (`lib/zones.ts`).
fn header_word(name: &str) -> Option<&'static str> {
    let upper = name.trim().to_ascii_uppercase();
    let rest = upper.trim_start_matches(|c: char| c.is_ascii_digit() || c.is_whitespace());
    let known = [
        ("JARGB", "ARGB header"),
        ("JRGB", "RGB header"),
        ("JRAINBOW", "Rainbow header"),
        ("JAF", "Fan header"),
        ("JCORSAIR", "Corsair header"),
        ("JPUMP", "Pump header"),
    ];
    for (prefix, word) in known {
        if rest.starts_with(prefix) {
            let tail = &rest[prefix.len()..];
            if tail.is_empty() || tail.chars().all(|c| c.is_ascii_digit()) {
                return Some(word);
            }
        }
    }
    None
}

/// The name of a placement: the owner's own name, or the plain name of the
/// zone. When one zone is several parts, the parts are numbered.
pub fn placement_name(devices: &[DeviceInfo], zone: &PlacedZone, all: &[PlacedZone]) -> String {
    if let Some(label) = zone.label.as_ref().map(|l| l.trim()) {
        if !label.is_empty() {
            return label.to_string();
        }
    }
    let plain = plain_name(devices, &zone.device_id.0, zone.zone_index).name;
    let mut siblings: Vec<&PlacedZone> = all
        .iter()
        .filter(|z| z.device_id == zone.device_id && z.zone_index == zone.zone_index)
        .collect();
    if siblings.len() < 2 {
        return plain;
    }
    siblings.sort_by_key(|z| z.leds.as_ref().map(|s| s.start).unwrap_or(0));
    let at = siblings
        .iter()
        .position(|z| {
            std::ptr::eq(*z, zone)
                || (z.leds.as_ref().map(|s| s.start) == zone.leds.as_ref().map(|s| s.start)
                    && z.leds.as_ref().map(|s| s.count) == zone.leds.as_ref().map(|s| s.count))
        })
        .unwrap_or(0);
    format!("{plain} {}", at + 1)
}

/// The part a zone is drawn as, when the owner has not said: the device's
/// kind, and a fan for anything whose zone is called a fan.
pub fn part_for_device(kind: DeviceKind, info: &ZoneInfo) -> PartKind {
    match kind {
        DeviceKind::Dram => PartKind::Ram,
        DeviceKind::LedStrip => PartKind::Strip,
        DeviceKind::Gpu => PartKind::Gpu,
        DeviceKind::Keyboard => PartKind::Keyboard,
        DeviceKind::Mouse => PartKind::Mouse,
        DeviceKind::Microphone => PartKind::Microphone,
        DeviceKind::Headset => PartKind::Headset,
        _ => {
            if info.name.to_lowercase().contains("fan") && info.led_count > 1 {
                PartKind::Fan
            } else {
                PartKind::Generic
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::ZoneInfo;

    fn device(kind: DeviceKind, name: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
        DeviceInfo {
            id: canvas_proto::device::DeviceId(format!("dev-{name}")),
            name: name.to_string(),
            vendor: "fake".to_string(),
            kind,
            adapter: "openrgb".to_string(),
            adapter_index: 0,
            serial: String::new(),
            location: String::new(),
            zones,
            connected: true,
        }
    }

    fn linear_zone(index: u32, name: &str, leds: u32) -> ZoneInfo {
        ZoneInfo {
            index,
            name: name.to_string(),
            kind: ZoneKind::Linear,
            led_count: leds,
            min_leds: 0,
            max_leds: leds,
            resizable: false,
        }
    }

    fn matrix_zone(index: u32, name: &str, leds: u32) -> ZoneInfo {
        ZoneInfo {
            index,
            name: name.to_string(),
            kind: ZoneKind::Matrix {
                width: 2,
                height: 2,
                map: vec![Some(0), Some(1), None, Some(2)],
            },
            led_count: leds,
            min_leds: 0,
            max_leds: leds,
            resizable: false,
        }
    }

    fn single_zone(index: u32, name: &str) -> ZoneInfo {
        ZoneInfo {
            index,
            name: name.to_string(),
            kind: ZoneKind::Single,
            led_count: 1,
            min_leds: 0,
            max_leds: 1,
            resizable: false,
        }
    }

    /// The demo desk, as the daemon serves it: the numbers React's own code
    /// produces for these zones are pinned below.
    fn demo() -> (Vec<DeviceInfo>, Vec<PlacedZone>) {
        let devices = vec![
            device(
                DeviceKind::Motherboard,
                "MSI MPG B850 Mystic Light (fake)",
                vec![
                    linear_zone(0, "JARGB1", 0),
                    linear_zone(1, "JARGB2", 0),
                    linear_zone(2, "JARGB3", 0),
                    linear_zone(3, "JRGB1", 3),
                ],
            ),
            device(
                DeviceKind::Keyboard,
                "Fake Keyboard",
                vec![matrix_zone(0, "Keys", 3)],
            ),
            device(
                DeviceKind::Microphone,
                "Fake Microphone",
                vec![single_zone(0, "Logo")],
            ),
            device(
                DeviceKind::Dram,
                "Fake DDR5 A",
                vec![linear_zone(0, "RGB", 8)],
            ),
            device(
                DeviceKind::Dram,
                "Fake DDR5 B",
                vec![linear_zone(0, "RGB", 8)],
            ),
        ];
        let zones = vec![
            PlacedZone {
                device_id: canvas_proto::device::DeviceId("dev-MSI MPG B850 Mystic Light (fake)".into()),
                zone_index: 3,
                x: 109.0,
                y: 31.0,
                width: 102.0,
                height: 4.0,
                rotation_deg: 0.0,
                reversed: false,
                visible: true,
                shape: ZoneShape::Strip,
                label: None,
                leds: None,
                part: None,
                locked: false,
            },
            PlacedZone {
                device_id: canvas_proto::device::DeviceId("dev-Fake Keyboard".into()),
                zone_index: 0,
                x: 14.0,
                y: 158.0,
                width: 110.0,
                height: 30.0,
                rotation_deg: 0.0,
                reversed: false,
                visible: true,
                shape: ZoneShape::Auto,
                label: None,
                leds: None,
                part: None,
                locked: false,
            },
            PlacedZone {
                device_id: canvas_proto::device::DeviceId("dev-Fake Microphone".into()),
                zone_index: 0,
                x: 162.0,
                y: 158.0,
                width: 16.0,
                height: 28.0,
                rotation_deg: 0.0,
                reversed: false,
                visible: true,
                shape: ZoneShape::Point,
                label: None,
                leds: None,
                part: None,
                locked: false,
            },
            PlacedZone {
                device_id: canvas_proto::device::DeviceId("dev-Fake DDR5 A".into()),
                zone_index: 0,
                x: 136.0,
                y: 74.0,
                width: 6.0,
                height: 34.0,
                rotation_deg: 0.0,
                reversed: false,
                visible: true,
                shape: ZoneShape::Strip,
                label: None,
                leds: None,
                part: None,
                locked: false,
            },
            PlacedZone {
                device_id: canvas_proto::device::DeviceId("dev-Fake DDR5 B".into()),
                zone_index: 0,
                x: 144.0,
                y: 74.0,
                width: 6.0,
                height: 34.0,
                rotation_deg: 0.0,
                reversed: false,
                visible: true,
                shape: ZoneShape::Strip,
                label: None,
                leds: None,
                part: None,
                locked: false,
            },
        ];
        (devices, zones)
    }

    #[test]
    fn the_strip_zone_puts_three_cells_along_its_long_axis() {
        let (devices, zones) = demo();
        let zone = &zones[0];
        let info = &devices[0].zones[3];
        let local = zone_local_leds(zone, info);
        assert_eq!(local.shape, ResolvedShape::Strip);
        assert_eq!(local.radius, 0.56);
        let cells: Vec<(f64, f64, f64, f64)> = local
            .leds
            .iter()
            .map(|l| {
                let l = l.expect("a strip has no gaps");
                (l.x, l.y, l.w, l.h)
            })
            .collect();
        assert_eq!(cells, vec![(-34.0, 0.0, 34.0, 4.0), (0.0, 0.0, 34.0, 4.0), (34.0, 0.0, 34.0, 4.0)]);
        let t = zone_transform(zone);
        assert_eq!((t.cx, t.cy, t.cos, t.sin), (160.0, 33.0, 1.0, 0.0));
        let cell = canvas_cell(&t, &local.leds[0].expect("a cell"));
        assert_eq!(cell.cx, 126.0);
        assert_eq!(cell.cy, 33.0);
        assert_eq!(cell.bounds, [109.0, 31.0, 143.0, 35.0]);
    }

    #[test]
    fn the_matrix_zone_keeps_the_devices_own_map() {
        let (devices, zones) = demo();
        let zone = &zones[1];
        let info = &devices[1].zones[0];
        let local = zone_local_leds(zone, info);
        assert_eq!(local.shape, ResolvedShape::Matrix);
        assert_eq!(local.radius, 3.3);
        let cells: Vec<(f64, f64, f64, f64)> = local
            .leds
            .iter()
            .map(|l| {
                let l = l.expect("the map has no gap here");
                (l.x, l.y, l.w, l.h)
            })
            .collect();
        assert_eq!(
            cells,
            vec![
                (-27.5, -7.5, 55.0, 15.0),
                (27.5, -7.5, 55.0, 15.0),
                (27.5, 7.5, 55.0, 15.0)
            ]
        );
        let t = zone_transform(zone);
        let cell = canvas_cell(&t, &local.leds[2].expect("a cell"));
        assert_eq!(cell.cx, 96.5);
        assert_eq!(cell.cy, 180.5);
        assert_eq!(cell.bounds, [69.0, 173.0, 124.0, 188.0]);
    }

    #[test]
    fn the_point_zone_samples_the_whole_rectangle() {
        let (devices, zones) = demo();
        let zone = &zones[2];
        let info = &devices[2].zones[0];
        let local = zone_local_leds(zone, info);
        assert_eq!(local.shape, ResolvedShape::Point);
        assert_eq!(local.radius, 3.52);
        let led = local.leds[0].expect("one LED");
        assert_eq!((led.x, led.y, led.w, led.h), (0.0, 0.0, 16.0, 28.0));
        let t = zone_transform(zone);
        let cell = canvas_cell(&t, &led);
        assert_eq!(cell.cx, 170.0);
        assert_eq!(cell.cy, 172.0);
        assert_eq!(cell.bounds, [162.0, 158.0, 178.0, 186.0]);
    }

    #[test]
    fn the_ram_strip_stacks_its_eight_cells_upwards() {
        let (devices, zones) = demo();
        let zone = &zones[3];
        let info = &devices[3].zones[0];
        let local = zone_local_leds(zone, info);
        assert_eq!(local.shape, ResolvedShape::Strip);
        assert_eq!(local.radius, 0.8400000000000001);
        let ys: Vec<f64> = local
            .leds
            .iter()
            .map(|l| l.expect("a strip has no gaps").y)
            .collect();
        assert_eq!(
            ys,
            vec![
                -14.875, -10.625, -6.375, -2.125, 2.125, 6.375, 10.625, 14.875
            ]
        );
        let t = zone_transform(zone);
        let cell = canvas_cell(&t, &local.leds[0].expect("a cell"));
        assert_eq!(cell.cx, 139.0);
        assert_eq!(cell.cy, 76.125);
        assert_eq!(cell.bounds, [136.0, 74.0, 142.0, 78.25]);
    }

    #[test]
    fn the_zone_corners_are_clockwise_from_the_top_left() {
        let (_devices, zones) = demo();
        let corners = zone_corners(&zones[0]);
        let got: Vec<(f64, f64)> = corners.iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(
            got,
            vec![(109.0, 31.0), (211.0, 31.0), (211.0, 35.0), (109.0, 35.0)]
        );
    }

    #[test]
    fn the_plain_names_are_the_ones_a_person_would_use() {
        let (devices, zones) = demo();
        let mb = plain_name(&devices, "dev-MSI MPG B850 Mystic Light (fake)", 3);
        assert_eq!(mb.name, "RGB header 1");
        assert_eq!(mb.hardware.as_deref(), Some("JRGB1"));
        assert_eq!(mb.kind, Some(DeviceKind::Motherboard));
        assert!(!mb.fan);

        let kb = plain_name(&devices, "dev-Fake Keyboard", 0);
        assert_eq!(kb.name, "Keyboard");
        assert_eq!(kb.hardware.as_deref(), Some("Fake Keyboard"));

        let mic = plain_name(&devices, "dev-Fake Microphone", 0);
        assert_eq!(mic.name, "Microphone");
        assert_eq!(mic.hardware.as_deref(), Some("Fake Microphone"));

        let ram_a = plain_name(&devices, "dev-Fake DDR5 A", 0);
        assert_eq!(ram_a.name, "RAM 1");
        assert_eq!(ram_a.hardware.as_deref(), Some("Fake DDR5 A"));

        let ram_b = plain_name(&devices, "dev-Fake DDR5 B", 0);
        assert_eq!(ram_b.name, "RAM 2");

        let gone = plain_name(&devices, "dev-nope", 0);
        assert_eq!(gone.name, "Not connected");
        assert_eq!(gone.hardware, None);

        // The placement name is the plain name when the zone is not split.
        assert_eq!(placement_name(&devices, &zones[0], &zones), "RGB header 1");
    }

    #[test]
    fn the_sizes_are_read_in_the_readers_unit() {
        assert_eq!(format_units(320.0, LengthUnit::Cm), "128.0 cm");
        assert_eq!(format_units(320.0, LengthUnit::In), "50.39 in");
        assert_eq!(format_units(30.0, LengthUnit::Cm), "12.0 cm");
        assert_eq!(format_units(30.0, LengthUnit::In), "4.72 in");
        assert_eq!(length_number(1000.0, LengthUnit::Cm), 100.0);
        assert_eq!(length_number(1000.0, LengthUnit::In), 39.37007874015748);
        assert_eq!(mm_from_length(10.0, LengthUnit::Cm), 100.0);
        assert_eq!(mm_from_length(1.0, LengthUnit::In), 25.4);
        assert_eq!(LengthUnit::Cm.word(), "cm");
        assert_eq!(LengthUnit::In.word(), "in");
        assert_eq!(LengthUnit::for_region("US"), LengthUnit::In);
        assert_eq!(LengthUnit::for_region("de"), LengthUnit::Cm);
    }

    #[test]
    fn a_default_shape_follows_the_device_and_the_zone() {
        let keyboard = matrix_zone(0, "Keys", 3);
        assert_eq!(
            default_shape(DeviceKind::Keyboard, &keyboard),
            ZoneShape::Auto
        );
        let fan = linear_zone(0, "Front Fan", 8);
        assert_eq!(
            default_shape(DeviceKind::Motherboard, &fan),
            ZoneShape::Ring
        );
        let header = linear_zone(0, "JARGB1", 12);
        assert_eq!(
            default_shape(DeviceKind::Motherboard, &header),
            ZoneShape::Strip
        );
        let logo = single_zone(0, "Logo");
        assert_eq!(
            default_shape(DeviceKind::Microphone, &logo),
            ZoneShape::Point
        );
        let keys = linear_zone(0, "Keys", 12);
        assert_eq!(
            default_shape(DeviceKind::Keyboard, &keys),
            ZoneShape::Grid { columns: 2 }
        );
    }

    #[test]
    fn a_default_rect_fits_the_part() {
        let keys = matrix_zone(0, "Keys", 3);
        assert_eq!(
            default_rect(&keys, DeviceKind::Keyboard, &ZoneShape::Auto),
            (80.0, 80.0)
        );
        let dram = linear_zone(0, "RGB", 8);
        assert_eq!(
            default_rect(&dram, DeviceKind::Dram, &ZoneShape::Strip),
            (16.0, 120.0)
        );
        let logo = single_zone(0, "Logo");
        assert_eq!(
            default_rect(&logo, DeviceKind::Microphone, &ZoneShape::Point),
            (16.0, 28.0)
        );
        let header = linear_zone(0, "JARGB1", 12);
        assert_eq!(
            default_rect(&header, DeviceKind::Motherboard, &ZoneShape::Strip),
            (120.0, 10.0)
        );
    }

    #[test]
    fn a_reshape_keeps_the_centre_and_suits_the_shape() {
        // The same two inputs React's own code is given in the parity harness
        // (`reshapeRing` and `reshapeStrip` in `reference.json`).
        let (_devices, zones) = demo();
        let wide = PlacedZone {
            width: 120.0,
            height: 10.0,
            x: 10.0,
            y: 10.0,
            ..zones[0].clone()
        };
        let ring = reshape_box(&wide, &ZoneShape::Ring);
        assert_eq!(ring.width, 48.0);
        assert_eq!(ring.height, 48.0);
        assert_eq!(ring.x, 46.0);
        assert_eq!(ring.y, 0.0);

        let square = PlacedZone {
            width: 40.0,
            height: 40.0,
            x: 10.0,
            y: 10.0,
            ..zones[0].clone()
        };
        let strip = reshape_box(&square, &ZoneShape::Strip);
        assert_eq!(strip.width, 100.0);
        assert_eq!(strip.height, 10.0);
        assert_eq!(strip.x, 0.0);
        assert_eq!(strip.y, 25.0);
    }
}
