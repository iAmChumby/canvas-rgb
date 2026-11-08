//! The stack builder's model, after `app/web/src/views/effects/stackDraft.ts`.
//!
//! The draft's layers, the values, the area's names and geometry, and the
//! session's epoch and sequence. Everything here is pure, so the builder's
//! rules are tested without a window (React's D95.6 tests 15-17).

#![allow(dead_code)]

use canvas_proto::api::StackLayerState;
use canvas_proto::device::DeviceInfo;
use canvas_proto::layout::Layout;
use canvas_proto::props::{PropKind, PropValue, PropertySchema, Props};
use canvas_proto::stack::{BlendMode, LayerArea, StackLayer};

use std::collections::BTreeMap;

/// `STACK_MAX_LAYERS`, `STACK_HTML_MAX`, `STACK_NAME_MAX` and `LAYER_NAME_MAX`
/// from the spine.
pub const MAX_LAYERS: usize = 8;
pub const MAX_HTML_LAYERS: usize = 2;
pub const MAX_STACK_NAME: usize = 60;
pub const MAX_LAYER_NAME: usize = 40;

/// The canvas the engine composes over (`CANVAS_WIDTH` x `CANVAS_HEIGHT`).
pub const CANVAS_W: f32 = 320.0;
pub const CANVAS_H: f32 = 200.0;

/// The smallest a dragged rectangle gets, in canvas units.
pub const MIN_AREA: f32 = 4.0;

/// Whether an effect is an HTML one (the slow, capped kind).
pub fn is_html_effect(effect: &str) -> bool {
    effect.starts_with("html:")
}

/// How many of the layers are HTML ones.
pub fn html_layers(layers: &[StackLayer]) -> usize {
    layers.iter().filter(|layer| is_html_effect(&layer.effect)).count()
}

/// The next free layer id (`l1` .. `l8`), or `None` when the stack is full.
pub fn next_layer_id(layers: &[StackLayer]) -> Option<String> {
    for n in 1..=MAX_LAYERS {
        let id = format!("l{n}");
        if !layers.iter().any(|layer| layer.id == id) {
            return Some(id);
        }
    }
    None
}

/// Why a layer cannot be added, or `None` when one can.
pub fn add_layer_refusal(layers: &[StackLayer], effect: Option<&str>) -> Option<String> {
    if layers.len() >= MAX_LAYERS {
        return Some(format!("A stack holds {MAX_LAYERS} layers"));
    }
    if let Some(effect) = effect {
        if is_html_effect(effect) && html_layers(layers) >= MAX_HTML_LAYERS {
            return Some(format!("A stack holds {MAX_HTML_LAYERS} HTML layers"));
        }
    }
    None
}

/// The value a property kind starts at.
pub fn default_value(kind: &PropKind) -> PropValue {
    kind.default_value()
}

/// A layer's props over the schema's defaults: the map its controls show.
pub fn layer_values(schema: Option<&PropertySchema>, props: &Props) -> Props {
    let mut values: Props = Props::new();
    for def in schema.map(|s| &s.properties).unwrap_or(&Vec::new()) {
        values.insert(def.key.clone(), default_value(&def.kind));
    }
    for (key, value) in props {
        values.insert(key.clone(), value.clone());
    }
    values
}

/// A new layer: whole-canvas, Normal, 100 %, visible, with the effect's saved
/// values or its defaults.
pub fn make_layer(
    effect: &str,
    name: &str,
    schema: Option<&PropertySchema>,
    props: &Props,
) -> StackLayer {
    StackLayer {
        id: "l1".to_string(),
        name: name.chars().take(MAX_LAYER_NAME).collect(),
        effect: effect.to_string(),
        props: layer_values(schema, props),
        area: LayerArea::Canvas,
        blend: BlendMode::Normal,
        opacity: 100,
        visible: true,
    }
}

/// The layer list with a new layer on top (the last one painted).
pub fn add_layer(layers: &[StackLayer], layer: StackLayer) -> Vec<StackLayer> {
    let mut next = layers.to_vec();
    next.push(layer);
    next
}

/// The layer list without one layer.
pub fn remove_layer(layers: &[StackLayer], id: &str) -> Vec<StackLayer> {
    layers.iter().filter(|layer| layer.id != id).cloned().collect()
}

/// Moves a layer to a bottom-first index; the others keep their order.
pub fn move_layer(layers: &[StackLayer], id: &str, to: usize) -> Vec<StackLayer> {
    let Some(from) = layers.iter().position(|layer| layer.id == id) else {
        return layers.to_vec();
    };
    let target = to.min(layers.len().saturating_sub(1));
    if target == from {
        return layers.to_vec();
    }
    let mut next = layers.to_vec();
    let moved = next.remove(from);
    next.insert(target, moved);
    next
}

/// A copy with the layer's values and a fresh id, placed directly above it.
pub fn duplicate_layer(layers: &[StackLayer], id: &str) -> (Vec<StackLayer>, Option<String>) {
    let Some(from) = layers.iter().position(|layer| layer.id == id) else {
        return (layers.to_vec(), None);
    };
    let source = &layers[from];
    let Some(fresh) = next_layer_id(layers) else {
        return (layers.to_vec(), None);
    };
    if is_html_effect(&source.effect) && html_layers(layers) >= MAX_HTML_LAYERS {
        return (layers.to_vec(), None);
    }
    let mut copy = source.clone();
    copy.id = fresh.clone();
    copy.name = format!("{} copy", source.name).chars().take(MAX_LAYER_NAME).collect();
    let mut next = layers.to_vec();
    next.insert(from + 1, copy);
    (next, Some(fresh))
}

/// The layer list with one layer changed.
pub fn patch_layer(layers: &[StackLayer], id: &str, patch: &LayerPatch) -> Vec<StackLayer> {
    layers
        .iter()
        .map(|layer| {
            if layer.id == id {
                patch.apply(layer)
            } else {
                layer.clone()
            }
        })
        .collect()
}

/// The layer list with one layer's visibility flipped.
pub fn toggle_layer(layers: &[StackLayer], id: &str) -> Vec<StackLayer> {
    layers
        .iter()
        .map(|layer| {
            if layer.id == id {
                let mut copy = layer.clone();
                copy.visible = !layer.visible;
                copy
            } else {
                layer.clone()
            }
        })
        .collect()
}

/// The changes one layer can take (`Partial<StackLayer>` in React).
#[derive(Debug, Clone, Default)]
pub struct LayerPatch {
    pub name: Option<String>,
    pub effect: Option<String>,
    pub props: Option<Props>,
    pub area: Option<LayerArea>,
    pub blend: Option<BlendMode>,
    pub opacity: Option<u8>,
    pub visible: Option<bool>,
}

impl LayerPatch {
    /// The changed layer.
    pub fn apply(&self, layer: &StackLayer) -> StackLayer {
        let mut copy = layer.clone();
        if let Some(v) = &self.name {
            copy.name = v.clone();
        }
        if let Some(v) = &self.effect {
            copy.effect = v.clone();
        }
        if let Some(v) = &self.props {
            copy.props = v.clone();
        }
        if let Some(v) = &self.area {
            copy.area = v.clone();
        }
        if let Some(v) = &self.blend {
            copy.blend = *v;
        }
        if let Some(v) = self.opacity {
            copy.opacity = v;
        }
        if let Some(v) = self.visible {
            copy.visible = v;
        }
        copy
    }
}

/// The schemas the builder knows, by effect id.
pub type Schemas = BTreeMap<String, Option<PropertySchema>>;

/// Every layer's values, namespaced `{layer.id}.{key}`: what a stack preset
/// stores (D95.1.7).
pub fn merged_props(layers: &[StackLayer], schemas: &Schemas) -> Props {
    let mut merged: Props = Props::new();
    for layer in layers {
        let values = layer_values(schemas.get(&layer.effect).and_then(|s| s.as_ref()), &layer.props);
        for (key, value) in values {
            merged.insert(format!("{}.{}", layer.id, key), value);
        }
    }
    merged
}

/// A merged map put back into the layers: each layer takes its own keys, and
/// a key its effect does not have is dropped, as the daemon's activate drops
/// one (D95.4.3).
pub fn apply_merged_props(layers: &[StackLayer], merged: &Props, schemas: &Schemas) -> Vec<StackLayer> {
    layers
        .iter()
        .map(|layer| {
            let prefix = format!("{}.", layer.id);
            let known: Vec<&str> = schemas
                .get(&layer.effect)
                .and_then(|s| s.as_ref())
                .map(|s| s.properties.iter().map(|d| d.key.as_str()).collect())
                .unwrap_or_default();
            let mut props = layer.props.clone();
            for (key, value) in merged {
                let Some(own) = key.strip_prefix(&prefix) else {
                    continue;
                };
                // Only the keys the effect has. A schema that is not here yet is
                // not a licence to write any key: the layer keeps its own values
                // until the schema says what it takes.
                if !known.contains(&own) {
                    continue;
                }
                props.insert(own.to_string(), value.clone());
            }
            let mut copy = layer.clone();
            copy.props = props;
            copy
        })
        .collect()
}

/// The stack's schema as the API derives it: every layer's properties,
/// namespaced (D95.4.3).
pub fn stack_schema(name: &str, layers: &[StackLayer], schemas: &Schemas) -> PropertySchema {
    let mut properties = Vec::new();
    let mut needs_screen = false;
    let mut needs_audio = false;
    let mut names: Vec<String> = Vec::new();
    for layer in layers {
        match schemas.get(&layer.effect).and_then(|s| s.as_ref()) {
            Some(schema) => {
                needs_screen |= schema.needs.screen;
                needs_audio |= schema.needs.audio;
                names.push(schema.name.clone());
                for def in &schema.properties {
                    properties.push(canvas_proto::props::PropertyDef {
                        key: format!("{}.{}", layer.id, def.key),
                        label: format!("{} · {}", layer.name, def.label),
                        kind: def.kind.clone(),
                    });
                }
            }
            None => names.push(layer.effect.clone()),
        }
    }
    let count = names.len();
    PropertySchema {
        id: format!("stack:{name}"),
        name: name.to_string(),
        description: if count == 0 {
            "No layers yet".to_string()
        } else {
            format!("{count} {}: {}", if count == 1 { "layer" } else { "layers" }, names.join(", "))
        },
        needs: canvas_proto::props::InputNeeds { screen: needs_screen, audio: needs_audio },
        properties,
        presets: vec![],
        thumbnail: None,
    }
}

// ---------------------------------------------------------------------------
// Names.

const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

fn name_problem(name: &str, max: usize, what: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Some(format!("A {what} needs a name"));
    }
    if trimmed != name {
        return Some(format!("\"{name}\" has a space at either end"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '.' || c == '_' || c == '-')
    {
        return Some(format!("\"{name}\" may only hold letters, digits, spaces, '-', '_' and '.'"));
    }
    if name.contains("..") {
        return Some(format!("\"{name}\" must not hold '..'"));
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Some(format!("\"{name}\" must not end in a space or a dot"));
    }
    let base = name.split('.').next().unwrap_or(name);
    if RESERVED.contains(&base.to_lowercase().as_str()) {
        return Some(format!("\"{base}\" is a name Windows reserves"));
    }
    if name.chars().count() > max {
        return Some(format!("A {what} is at most {max} characters"));
    }
    None
}

/// What is wrong with a stack's name, or `None`. The daemon is the authority;
/// this is the prompt.
pub fn stack_name_problem(name: &str, others: &[String]) -> Option<String> {
    let basic = name_problem(name, MAX_STACK_NAME, "stack name");
    if basic.is_some() {
        return basic;
    }
    if name.to_lowercase() == "preview" {
        return Some(format!("\"{name}\" is reserved for the stack preview route"));
    }
    if others.iter().any(|other| other.to_lowercase() == name.to_lowercase()) {
        return Some(format!("\"{name}\" is already another stack's name"));
    }
    None
}

/// What is wrong with a layer's name, or `None`.
pub fn layer_name_problem(name: &str) -> Option<String> {
    name_problem(name, MAX_LAYER_NAME, "layer name")
}

// ---------------------------------------------------------------------------
// The session.

/// 16 random hex characters, made when the builder opens (D95.4.4).
pub fn new_epoch() -> String {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E3779B97F4A7C15);
    // A xorshift64* walk: the epoch only has to be unique per session.
    let mut next = || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545F4914F6CDD1D)
    };
    let mut out = String::with_capacity(16);
    for _ in 0..8 {
        out.push_str(&format!("{:02x}", (next() & 0xff) as u8));
    }
    out
}

/// A builder session's epoch and sequence.
///
/// `next()` is the sequence for the next PUT; `observe()` takes the daemon's
/// answer, so a superseded PUT (`applied: false`) advances the sequence past
/// the one that owns the preview. A dropped PUT is never resent: only a new
/// change sends again.
#[derive(Debug, Clone)]
pub struct StackSession {
    pub epoch: String,
    pub seq: u64,
}

impl StackSession {
    pub fn new(epoch: String) -> Self {
        Self { epoch, seq: 0 }
    }

    /// The epoch and sequence for the next PUT.
    pub fn next(&mut self) -> (String, u64) {
        self.seq += 1;
        (self.epoch.clone(), self.seq)
    }

    /// Takes the daemon's answer.
    pub fn observe(&mut self, epoch: Option<&str>, seq: Option<u64>) {
        if let (Some(epoch), Some(seq)) = (epoch, seq) {
            if epoch == self.epoch && seq > self.seq {
                self.seq = seq;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Areas.

/// A rectangle in canvas units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The layers as the daemon must receive them (#118): every rect area clamped
/// to the canvas.
pub fn layers_for_server(layers: &[StackLayer]) -> Vec<StackLayer> {
    layers
        .iter()
        .map(|layer| {
            let LayerArea::Rect { x, y, width, height } = layer.area else {
                return layer.clone();
            };
            let x0 = x.clamp(0.0, CANVAS_W);
            let y0 = y.clamp(0.0, CANVAS_H);
            let x1 = (x + width).min(CANVAS_W).max(x0);
            let y1 = (y + height).min(CANVAS_H).max(y0);
            if x0 == x && y0 == y && x1 - x0 == width && y1 - y0 == height {
                return layer.clone();
            }
            let mut copy = layer.clone();
            copy.area = LayerArea::Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
            copy
        })
        .collect()
}

/// Where an area paints on the 320 x 200 canvas: a rect, or the union of its
/// placed zones.
pub fn area_bounds(area: &LayerArea, layout: Option<&Layout>) -> Option<Bounds> {
    match area {
        LayerArea::Canvas => Some(Bounds { x: 0.0, y: 0.0, width: CANVAS_W, height: CANVAS_H }),
        LayerArea::Rect { x, y, width, height } => {
            Some(Bounds { x: *x, y: *y, width: *width, height: *height })
        }
        LayerArea::Device { device } | LayerArea::Zone { device, .. } => {
            let layout = layout?;
            let zones: Vec<&canvas_proto::layout::PlacedZone> = layout
                .zones
                .iter()
                .filter(|zone| {
                    zone.visible
                        && &zone.device_id == device
                        && match area {
                            LayerArea::Device { .. } => true,
                            LayerArea::Zone { zone: index, .. } => zone.zone_index == *index,
                            _ => false,
                        }
                })
                .collect();
            let first = zones.first()?;
            let mut x0 = first.x;
            let mut y0 = first.y;
            let mut x1 = first.x + first.width;
            let mut y1 = first.y + first.height;
            for zone in zones.iter().skip(1) {
                x0 = x0.min(zone.x);
                y0 = y0.min(zone.y);
                x1 = x1.max(zone.x + zone.width);
                y1 = y1.max(zone.y + zone.height);
            }
            Some(Bounds { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
        }
    }
}

fn device_name(devices: Option<&[DeviceInfo]>, id: &str) -> String {
    if let Some(found) = devices.and_then(|list| list.iter().find(|d| d.id.0 == id)) {
        return found.name.clone();
    }
    // `id.replace(/^[a-z0-9]+:/i, "")`: the vendor's prefix goes.
    let mut out = id;
    let bytes = id.as_bytes();
    let mut at = 0;
    while at < bytes.len() && (bytes[at].is_ascii_alphanumeric()) {
        at += 1;
    }
    if at < bytes.len() && bytes[at] == b':' {
        out = &id[at + 1..];
    }
    out.to_string()
}

/// A device or a zone on the layout, by the id and index the area names.
pub fn area_is_on_desk(area: &LayerArea, layout: Option<&Layout>) -> bool {
    match area {
        LayerArea::Canvas | LayerArea::Rect { .. } => true,
        LayerArea::Device { device } | LayerArea::Zone { device, .. } => {
            let Some(layout) = layout else {
                return false;
            };
            layout.zones.iter().any(|zone| {
                zone.visible
                    && &zone.device_id == device
                    && match area {
                        LayerArea::Device { .. } => true,
                        LayerArea::Zone { zone: index, .. } => zone.zone_index == *index,
                        _ => false,
                    }
            })
        }
    }
}

/// What an area is called in a layer row and in the picked layer's area line.
pub fn area_name(area: &LayerArea, layout: Option<&Layout>, devices: Option<&[DeviceInfo]>) -> String {
    match area {
        LayerArea::Canvas => "Whole canvas".to_string(),
        LayerArea::Rect { width, height, .. } => {
            format!("Rectangle {} × {}", width.round(), height.round())
        }
        LayerArea::Device { device } | LayerArea::Zone { device, .. } => {
            if !area_is_on_desk(area, layout) {
                return "Not on the desk".to_string();
            }
            if let LayerArea::Device { .. } = area {
                return device_name(devices, &device.0);
            }
            let LayerArea::Zone { zone, .. } = area else {
                return device_name(devices, &device.0);
            };
            let label = layout.and_then(|l| {
                l.zones
                    .iter()
                    .find(|placed| placed.device_id == *device && placed.zone_index == *zone)
                    .and_then(|placed| placed.label.clone())
            });
            let name = device_name(devices, &device.0);
            match label.filter(|l| !l.is_empty()) {
                Some(label) => format!("{name} · {label}"),
                None => format!("{name} · Zone {}", zone + 1),
            }
        }
    }
}

/// A rectangle's position, said under its size.
pub fn area_position(area: &LayerArea) -> Option<String> {
    match area {
        LayerArea::Rect { x, y, .. } => Some(format!("x {}, y {}", x.round(), y.round())),
        _ => None,
    }
}

/// A rectangle with whole-pixel edges.
pub fn round_rect(bounds: &Bounds) -> Bounds {
    Bounds {
        x: bounds.x.round(),
        y: bounds.y.round(),
        width: bounds.width.round(),
        height: bounds.height.round(),
    }
}

// ---------------------------------------------------------------------------
// Picker geometry.

/// The desk's view in the picker: 1 is the whole desk in the stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickerView {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
}

impl Default for PickerView {
    fn default() -> Self {
        Self { zoom: 1.0, pan_x: 0.0, pan_y: 0.0 }
    }
}

/// The stage's box on the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StageBox {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl StageBox {
    /// The stage's box at the origin.
    pub fn new() -> Self {
        Self { left: 0.0, top: 0.0, width: 0.0, height: 0.0 }
    }
}

impl Default for StageBox {
    fn default() -> Self {
        Self::new()
    }
}

/// Pointer to canvas through the stage's view (D95.2.6):
/// `base = stage.clientWidth / 320`, `canvas_x = css_x / (base * zoom) - pan_x`.
/// A point outside the desk clamps at its edge, so a drag that starts beside
/// the desk still draws from the edge.
pub fn pointer_to_canvas(
    client_x: f32,
    client_y: f32,
    box_: &StageBox,
    view: &PickerView,
) -> (f32, f32) {
    let base = box_.width / CANVAS_W;
    let k = if base * view.zoom == 0.0 { 1.0 } else { base * view.zoom };
    let x = (client_x - box_.left) / k - view.pan_x;
    let y = (client_y - box_.top) / k - view.pan_y;
    (x.clamp(0.0, CANVAS_W), y.clamp(0.0, CANVAS_H))
}

/// The rectangle two canvas points draw, whole pixels and at least one.
pub fn rect_from_drag(a: (f32, f32), b: (f32, f32)) -> Bounds {
    let x0 = a.0.min(b.0);
    let y0 = a.1.min(b.1);
    let x1 = a.0.max(b.0);
    let y1 = a.1.max(b.1);
    let x = x0.clamp(0.0, CANVAS_W - 1.0).round();
    let y = y0.clamp(0.0, CANVAS_H - 1.0).round();
    Bounds {
        x,
        y,
        width: 1.0f32.max((x1 - x0).min(CANVAS_W - x).round()),
        height: 1.0f32.max((y1 - y0).min(CANVAS_H - y).round()),
    }
}

/// A rectangle from its four edges, each rounded to a whole unit on its own,
/// so the right and bottom edges never round past the canvas the way a
/// rounded x plus a rounded width can (219.5 + 100.5 is 220 + 101 = 321).
pub fn bounds_from_edges(x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds {
    let left = x0.round();
    let top = y0.round();
    Bounds { x: left, y: top, width: x1.round() - left, height: y1.round() - top }
}

/// A rectangle moved by a canvas-unit delta, kept whole inside the canvas.
pub fn move_bounds(start: &Bounds, dx: f32, dy: f32) -> Bounds {
    let x = (start.x + dx).min(CANVAS_W - start.width).max(0.0);
    let y = (start.y + dy).min(CANVAS_H - start.height).max(0.0);
    bounds_from_edges(x, y, x + start.width, y + start.height)
}

/// A rectangle resized by dragging one handle by a canvas-unit delta.
///
/// `sx` and `sy` are the handle's direction (-1 the left or top edge, 1 the
/// right or bottom, 0 an axis it leaves alone); the opposite edge stays put.
/// The result stays inside the canvas and at least `MIN_AREA` each way.
/// `keep_aspect` (a corner with Shift) holds the starting ratio.
pub fn resize_bounds(
    start: &Bounds,
    sx: i32,
    sy: i32,
    dx: f32,
    dy: f32,
    keep_aspect: bool,
) -> Bounds {
    let right = start.x + start.width;
    let bottom = start.y + start.height;
    // The smallest a side gets is MIN_AREA, or the side's own length when it
    // started shorter (the picker draws one-unit rectangles): a resize never
    // grows a small area and never leaves the canvas.
    let min_w = MIN_AREA.min(start.width);
    let min_h = MIN_AREA.min(start.height);
    let mut x0 = start.x;
    let mut x1 = right;
    let mut y0 = start.y;
    let mut y1 = bottom;
    if sx < 0 {
        x0 = (start.x + dx).max(0.0).min(right - min_w);
    }
    if sx > 0 {
        x1 = (right + dx).min(CANVAS_W).max(start.x + min_w);
    }
    if sy < 0 {
        y0 = (start.y + dy).max(0.0).min(bottom - min_h);
    }
    if sy > 0 {
        y1 = (bottom + dy).min(CANVAS_H).max(start.y + min_h);
    }
    if keep_aspect && sx != 0 && sy != 0 {
        let room_w = if sx > 0 { CANVAS_W - start.x } else { right };
        let room_h = if sy > 0 { CANVAS_H - start.y } else { bottom };
        let most = (room_w / start.width).min(room_h / start.height);
        let least = (min_w / start.width).max(min_h / start.height);
        let want = ((x1 - x0) / start.width).max((y1 - y0) / start.height);
        let scale = most.min(least.max(want));
        let w = start.width * scale;
        let h = start.height * scale;
        if sx < 0 {
            x0 = right - w;
        } else {
            x1 = start.x + w;
        }
        if sy < 0 {
            y0 = bottom - h;
        } else {
            y1 = start.y + h;
        }
    }
    bounds_from_edges(x0, y0, x1, y1)
}

/// The picker's readout: the area's title and its detail.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaReadout {
    /// The line's first part: "Rectangle 110 × 46", "Whole canvas",
    /// "Keyboard · Zone 1".
    pub title: String,
    /// The muted second part: "x 16, y 150".
    pub detail: Option<String>,
}

/// The area's readout.
pub fn area_readout(
    area: &LayerArea,
    layout: Option<&Layout>,
    devices: Option<&[DeviceInfo]>,
) -> AreaReadout {
    if let LayerArea::Rect { width, height, .. } = area {
        return AreaReadout {
            title: format!("Rectangle {} × {}", width.round(), height.round()),
            detail: area_position(area),
        };
    }
    AreaReadout { title: area_name(area, layout, devices), detail: None }
}

/// The tone a layer's status is drawn in: warn for a layer that is not
/// running.
pub fn status_words(state: Option<StackLayerState>, last_ms: Option<f32>) -> Option<String> {
    let state = state?;
    if state == StackLayerState::Ok {
        return None;
    }
    let ms = match last_ms {
        Some(v) => format!(" · {} ms", v.round()),
        None => String::new(),
    };
    Some(match state {
        StackLayerState::Held => format!("Held: too slow{ms}"),
        StackLayerState::Faulted => format!("Faulted: too slow{ms}"),
        StackLayerState::MissingEffect => "Missing".to_string(),
        StackLayerState::NestedStack => "Nested stack".to_string(),
        StackLayerState::NoArea => "Not on the desk".to_string(),
        StackLayerState::Ok => return None,
    })
}

/// The blend's name, as the layer row says it.
pub fn blend_label(blend: BlendMode) -> &'static str {
    match blend {
        BlendMode::Normal => "Normal",
        BlendMode::Add => "Add",
        BlendMode::Multiply => "Multiply",
        BlendMode::Screen => "Screen",
    }
}

/// The blend's note, as the picked layer says it.
pub fn blend_note(blend: BlendMode) -> &'static str {
    match blend {
        BlendMode::Normal => "Normal: covers what is under it; a black part hides it.",
        BlendMode::Add => "Add: lightens what is under it; a black part leaves it alone.",
        BlendMode::Multiply => "Multiply: darkens what is under it; a black part makes it black.",
        BlendMode::Screen => "Screen: lifts what is under it; a black part is invisible.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::color::Rgb8;

    fn layer(id: &str, name: &str, effect: &str) -> StackLayer {
        StackLayer {
            id: id.into(),
            name: name.into(),
            effect: effect.into(),
            props: Props::new(),
            area: LayerArea::Canvas,
            blend: BlendMode::Normal,
            opacity: 100,
            visible: true,
        }
    }

    fn schema(name: &str, props: &[(&str, PropKind)]) -> PropertySchema {
        PropertySchema {
            id: "s".into(),
            name: name.into(),
            description: String::new(),
            needs: Default::default(),
            properties: props
                .iter()
                .map(|(key, kind)| canvas_proto::props::PropertyDef {
                    key: (*key).into(),
                    label: (*key).into(),
                    kind: kind.clone(),
                })
                .collect(),
            presets: vec![],
            thumbnail: None,
        }
    }

    #[test]
    fn the_limits_are_the_spines() {
        assert_eq!(MAX_LAYERS, 8);
        assert_eq!(MAX_HTML_LAYERS, 2);
        assert_eq!(MAX_STACK_NAME, 60);
        assert_eq!(MAX_LAYER_NAME, 40);
        assert_eq!(CANVAS_W, 320.0);
        assert_eq!(CANVAS_H, 200.0);
        assert_eq!(MIN_AREA, 4.0);
    }

    #[test]
    fn the_next_free_layer_id_counts_from_one() {
        assert_eq!(next_layer_id(&[]), Some("l1".to_string()));
        assert_eq!(next_layer_id(&[layer("l1", "A", "solid")]), Some("l2".to_string()));
        let full: Vec<StackLayer> = (1..=8).map(|n| layer(&format!("l{n}"), "L", "solid")).collect();
        assert_eq!(next_layer_id(&full), None);
    }

    #[test]
    fn a_full_stack_refuses_a_layer_and_two_html_layers_refuse_a_third() {
        let full: Vec<StackLayer> = (1..=8).map(|n| layer(&format!("l{n}"), "L", "solid")).collect();
        assert_eq!(
            add_layer_refusal(&full, None),
            Some("A stack holds 8 layers".to_string())
        );
        let two_html = vec![layer("l1", "A", "html:a"), layer("l2", "B", "html:b")];
        assert_eq!(
            add_layer_refusal(&two_html, Some("html:c")),
            Some("A stack holds 2 HTML layers".to_string())
        );
        assert_eq!(add_layer_refusal(&two_html, Some("solid")), None);
    }

    #[test]
    fn a_layers_values_are_its_own_over_the_defaults() {
        let s = schema("Rain", &[("speed", PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 30.0 })]);
        let mut props = Props::new();
        props.insert("speed".into(), PropValue::Number(70.0));
        let values = layer_values(Some(&s), &props);
        assert_eq!(values.get("speed"), Some(&PropValue::Number(70.0)));
        let empty = layer_values(Some(&s), &Props::new());
        assert_eq!(empty.get("speed"), Some(&PropValue::Number(30.0)));
    }

    #[test]
    fn a_new_layer_is_whole_canvas_normal_and_full() {
        let l = make_layer("solid", "Base", None, &Props::new());
        assert_eq!(l.id, "l1");
        assert_eq!(l.area, LayerArea::Canvas);
        assert_eq!(l.blend, BlendMode::Normal);
        assert_eq!(l.opacity, 100);
        assert!(l.visible);
    }

    #[test]
    fn a_new_layers_name_is_cut_to_forty_characters() {
        let long = "x".repeat(60);
        let l = make_layer("solid", &long, None, &Props::new());
        assert_eq!(l.name.chars().count(), MAX_LAYER_NAME);
    }

    #[test]
    fn moving_a_layer_keeps_the_others_in_order() {
        let layers = vec![layer("l1", "A", "solid"), layer("l2", "B", "solid"), layer("l3", "C", "solid")];
        let moved = move_layer(&layers, "l1", 2);
        let ids: Vec<&str> = moved.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, vec!["l2", "l3", "l1"]);
        // A move past the end stops at the end.
        let clamped = move_layer(&layers, "l1", 99);
        let ids: Vec<&str> = clamped.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, vec!["l2", "l3", "l1"]);
    }

    #[test]
    fn a_copy_sits_just_above_the_original_with_a_fresh_id() {
        let layers = vec![layer("l1", "A", "solid"), layer("l2", "B", "solid")];
        let (next, id) = duplicate_layer(&layers, "l1");
        assert_eq!(id, Some("l3".to_string()));
        assert_eq!(next[1].name, "A copy");
        assert_eq!(next[1].id, "l3");
        assert_eq!(next[2].id, "l2");
    }

    #[test]
    fn a_copy_of_an_html_layer_stops_at_the_html_cap() {
        let layers = vec![
            layer("l1", "A", "html:a"),
            layer("l2", "B", "html:b"),
            layer("l3", "C", "solid"),
        ];
        let (next, id) = duplicate_layer(&layers, "l1");
        assert_eq!(id, None);
        assert_eq!(next.len(), 3);
    }

    #[test]
    fn the_merged_map_namespaces_every_layers_keys() {
        let s = schema("Rain", &[("speed", PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 30.0 })]);
        let mut schemas = Schemas::new();
        schemas.insert("rain".into(), Some(s));
        let mut l = layer("l1", "Rain", "rain");
        l.props.insert("speed".into(), PropValue::Number(70.0));
        let merged = merged_props(&[l], &schemas);
        assert_eq!(merged.get("l1.speed"), Some(&PropValue::Number(70.0)));
    }

    #[test]
    fn a_merged_map_writes_only_the_keys_the_effect_has() {
        let s = schema("Rain", &[("speed", PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 30.0 })]);
        let mut schemas = Schemas::new();
        schemas.insert("rain".into(), Some(s));
        let mut merged = Props::new();
        merged.insert("l1.speed".into(), PropValue::Number(70.0));
        merged.insert("l1.unknown".into(), PropValue::Number(1.0));
        let l = layer("l1", "Rain", "rain");
        let applied = apply_merged_props(&[l], &merged, &schemas);
        assert_eq!(applied[0].props.get("speed"), Some(&PropValue::Number(70.0)));
        assert!(!applied[0].props.contains_key("unknown"));
    }

    #[test]
    fn a_stacks_schema_counts_its_layers_and_names_them() {
        let s = schema("Rain", &[("speed", PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 30.0 })]);
        let mut schemas = Schemas::new();
        schemas.insert("rain".into(), Some(s));
        let l = layer("l1", "Rain", "rain");
        let built = stack_schema("Glow", &[l], &schemas);
        assert_eq!(built.id, "stack:Glow");
        assert_eq!(built.description, "1 layer: Rain");
        assert_eq!(built.properties[0].key, "l1.speed");
        assert_eq!(built.properties[0].label, "Rain · speed");
    }

    #[test]
    fn a_stacks_name_may_not_be_empty_or_hold_a_reserved_word() {
        assert_eq!(
            stack_name_problem("", &[]),
            Some("A stack name needs a name".to_string())
        );
        assert_eq!(
            stack_name_problem("preview", &[]),
            Some("\"preview\" is reserved for the stack preview route".to_string())
        );
        assert_eq!(
            stack_name_problem("Sunset/glow", &[]),
            Some("\"Sunset/glow\" may only hold letters, digits, spaces, '-', '_' and '.'".to_string())
        );
        assert_eq!(
            stack_name_problem("Sunset glow", &["sunset glow".to_string()]),
            Some("\"Sunset glow\" is already another stack's name".to_string())
        );
        assert_eq!(stack_name_problem("Sunset glow", &[]), None);
    }

    #[test]
    fn a_layers_name_may_not_be_empty() {
        assert_eq!(
            layer_name_problem(""),
            Some("A layer name needs a name".to_string())
        );
        assert_eq!(
            layer_name_problem("a:b"),
            Some("\"a:b\" may only hold letters, digits, spaces, '-', '_' and '.'".to_string())
        );
        assert_eq!(layer_name_problem("Heat"), None);
    }

    #[test]
    fn the_session_counts_up_and_takes_the_daemons_answer() {
        let mut session = StackSession::new("abc".into());
        assert_eq!(session.next(), ("abc".to_string(), 1));
        assert_eq!(session.next(), ("abc".to_string(), 2));
        // Another epoch's answer is ignored.
        session.observe(Some("other"), Some(9));
        assert_eq!(session.seq, 2);
        session.observe(Some("abc"), Some(5));
        assert_eq!(session.seq, 5);
    }

    #[test]
    fn a_rect_area_is_clamped_to_the_canvas_for_the_daemon() {
        let mut l = layer("l1", "A", "solid");
        l.area = LayerArea::Rect { x: -5.0, y: 0.0, width: 400.0, height: 46.0 };
        let sent = layers_for_server(&[l]);
        assert_eq!(
            sent[0].area,
            LayerArea::Rect { x: 0.0, y: 0.0, width: 320.0, height: 46.0 }
        );
    }

    #[test]
    fn a_whole_canvas_area_is_the_whole_canvas() {
        let bounds = area_bounds(&LayerArea::Canvas, None).unwrap();
        assert_eq!(bounds, Bounds { x: 0.0, y: 0.0, width: 320.0, height: 200.0 });
    }

    #[test]
    fn a_rectangle_area_is_named_and_placed() {
        let area = LayerArea::Rect { x: 16.0, y: 150.0, width: 110.0, height: 46.0 };
        assert_eq!(area_name(&area, None, None), "Rectangle 110 × 46");
        assert_eq!(area_position(&area), Some("x 16, y 150".to_string()));
        let readout = area_readout(&area, None, None);
        assert_eq!(readout.title, "Rectangle 110 × 46");
        assert_eq!(readout.detail, Some("x 16, y 150".to_string()));
    }

    #[test]
    fn a_whole_canvas_area_has_no_position() {
        assert_eq!(area_position(&LayerArea::Canvas), None);
        assert_eq!(area_readout(&LayerArea::Canvas, None, None).detail, None);
    }

    #[test]
    fn a_device_area_that_is_not_on_the_desk_says_so() {
        let area = LayerArea::Device { device: canvas_proto::device::DeviceId("x:1".into()) };
        assert_eq!(area_name(&area, None, None), "Not on the desk");
        assert!(!area_is_on_desk(&area, None));
    }

    #[test]
    fn a_drag_draws_a_whole_rectangle_at_least_one_pixel() {
        let r = rect_from_drag((10.4, 20.6), (30.2, 40.1));
        assert_eq!(r, Bounds { x: 10.0, y: 21.0, width: 20.0, height: 19.0 });
        // A drag in the other direction draws the same rectangle.
        let back = rect_from_drag((30.2, 40.1), (10.4, 20.6));
        assert_eq!(back, r);
        // A click draws one pixel.
        let dot = rect_from_drag((5.0, 5.0), (5.0, 5.0));
        assert_eq!(dot.width, 1.0);
        assert_eq!(dot.height, 1.0);
    }

    #[test]
    fn a_drag_stays_on_the_canvas() {
        let r = rect_from_drag((-40.0, -40.0), (900.0, 900.0));
        assert!(r.x >= 0.0 && r.y >= 0.0);
        assert!(r.x + r.width <= CANVAS_W);
        assert!(r.y + r.height <= CANVAS_H);
    }

    #[test]
    fn a_moved_rectangle_stays_inside_the_canvas() {
        let start = Bounds { x: 300.0, y: 190.0, width: 20.0, height: 10.0 };
        let moved = move_bounds(&start, 50.0, 50.0);
        assert_eq!(moved.x, 300.0);
        assert_eq!(moved.y, 190.0);
        let back = move_bounds(&start, -500.0, -500.0);
        assert_eq!(back.x, 0.0);
        assert_eq!(back.y, 0.0);
    }

    #[test]
    fn a_resized_rectangle_resizes_from_the_opposite_side() {
        let start = Bounds { x: 10.0, y: 10.0, width: 20.0, height: 20.0 };
        // The south-east corner: the top-left stays put.
        let grown = resize_bounds(&start, 1, 1, 10.0, 10.0, false);
        assert_eq!(grown, Bounds { x: 10.0, y: 10.0, width: 30.0, height: 30.0 });
        // The north-west corner: the bottom-right stays put.
        let shrunk = resize_bounds(&start, -1, -1, -5.0, -5.0, false);
        assert_eq!(shrunk, Bounds { x: 5.0, y: 5.0, width: 25.0, height: 25.0 });
    }

    #[test]
    fn a_resize_is_never_smaller_than_the_minimum_area() {
        let start = Bounds { x: 10.0, y: 10.0, width: 20.0, height: 20.0 };
        let tiny = resize_bounds(&start, 1, 1, -100.0, -100.0, false);
        assert_eq!(tiny.width, MIN_AREA);
        assert_eq!(tiny.height, MIN_AREA);
    }

    #[test]
    fn a_corner_with_shift_holds_the_ratio() {
        let start = Bounds { x: 0.0, y: 0.0, width: 20.0, height: 10.0 };
        let held = resize_bounds(&start, 1, 1, 20.0, 0.0, true);
        assert!((held.width / held.height - 2.0).abs() < 0.01, "{held:?}");
    }

    #[test]
    fn a_pointer_maps_through_the_stages_view() {
        let box_ = StageBox { left: 10.0, top: 20.0, width: 320.0, height: 200.0 };
        let view = PickerView::default();
        assert_eq!(pointer_to_canvas(10.0, 20.0, &box_, &view), (0.0, 0.0));
        assert_eq!(pointer_to_canvas(330.0, 220.0, &box_, &view), (320.0, 200.0));
        // A point outside the desk clamps at its edge.
        assert_eq!(pointer_to_canvas(-100.0, -100.0, &box_, &view), (0.0, 0.0));
    }

    #[test]
    fn a_layers_status_words_say_what_is_wrong() {
        assert_eq!(status_words(None, None), None);
        assert_eq!(status_words(Some(StackLayerState::Ok), None), None);
        assert_eq!(
            status_words(Some(StackLayerState::Held), Some(12.4)),
            Some("Held: too slow · 12 ms".to_string())
        );
        assert_eq!(
            status_words(Some(StackLayerState::Faulted), None),
            Some("Faulted: too slow".to_string())
        );
        assert_eq!(
            status_words(Some(StackLayerState::MissingEffect), None),
            Some("Missing".to_string())
        );
        assert_eq!(
            status_words(Some(StackLayerState::NestedStack), None),
            Some("Nested stack".to_string())
        );
        assert_eq!(
            status_words(Some(StackLayerState::NoArea), None),
            Some("Not on the desk".to_string())
        );
    }

    #[test]
    fn the_epoch_is_sixteen_hex_characters() {
        let epoch = new_epoch();
        assert_eq!(epoch.len(), 16);
        assert!(epoch.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(epoch, new_epoch());
    }

    #[test]
    fn the_blend_names_and_notes_are_reacts() {
        assert_eq!(blend_label(BlendMode::Normal), "Normal");
        assert_eq!(
            blend_note(BlendMode::Screen),
            "Screen: lifts what is under it; a black part is invisible."
        );
    }

    #[test]
    fn a_colour_property_is_read_as_its_rgb() {
        let mut props = Props::new();
        props.insert("color".into(), PropValue::Color(Rgb8::new(1, 2, 3)));
        assert_eq!(layer_values(None, &props).get("color"), Some(&PropValue::Color(Rgb8::new(1, 2, 3))));
    }
}
