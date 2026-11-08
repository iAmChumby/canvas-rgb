//! The stack compositor's own maths, after `app/web/src/lib/stackFrame.ts`.
//!
//! The daemon draws a stack itself; this is the copy the demo mode uses, and
//! the copy the GPUI port keeps as the rule the compositor is measured
//! against: the four blends' arithmetic, and the way a stack's layers are
//! mapped into their areas.
//!
//! `demoFrameFor` is ported from `app/web/src/lib/thumb-demo.ts`: the demo's
//! one generic source, used for the effects that have no look of their own.

#![allow(dead_code)]

use canvas_proto::layout::Layout;
use canvas_proto::props::{PropValue, Props};
use canvas_proto::stack::{BlendMode, Stack, StackLayer};

use crate::views::effects::stack_draft::{area_bounds, CANVAS_H, CANVAS_W};

/// The blends' arithmetic, on 0..1 channels (`blendChannel`).
///
/// `d` is what is under, `s` what is over, `a` the layer's opacity.
pub fn blend_channel(mode: BlendMode, d: f32, s: f32, a: f32) -> f32 {
    match mode {
        BlendMode::Add => (d + a * ((d + s).min(1.0) - d)).min(1.0),
        BlendMode::Multiply => d * (1.0 - a + a * s),
        BlendMode::Screen => (d + a * s * (1.0 - d)).min(1.0),
        BlendMode::Normal => s * a + d * (1.0 - a),
    }
}

/// The colour a property holds, or the fallback (`colorOf`).
fn color_of(value: Option<&PropValue>, fallback: (u8, u8, u8)) -> (u8, u8, u8) {
    match value {
        Some(PropValue::Color(c)) => (c.r, c.g, c.b),
        _ => fallback,
    }
}

/// The number a property holds, or the fallback (`numberProp`).
fn number_prop(layer: &StackLayer, key: &str, fallback: f32) -> f32 {
    match layer.props.get(key) {
        Some(PropValue::Number(v)) => *v as f32,
        _ => fallback,
    }
}

/// The degrees an angle (or an old hue) property holds.
fn angle_prop(layer: &StackLayer, key: &str, fallback: f32) -> f32 {
    match layer.props.get(key) {
        Some(PropValue::Angle(v)) | Some(PropValue::Hue(v)) => *v as f32,
        _ => fallback,
    }
}

/// One layer's demo picture. A handful of effects have a look built from
/// their props (`demoLayerPicture`).
fn demo_layer_picture(layer: &StackLayer, width: usize, height: usize, seconds: f32) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 3];
    match layer.effect.as_str() {
        "solid" => {
            let (r, g, b) = color_of(layer.props.get("color"), (0x60, 0x00, 0xfc));
            for i in 0..width * height {
                out[i * 3] = r;
                out[i * 3 + 1] = g;
                out[i * 3 + 2] = b;
            }
            out
        }
        "gradient" => {
            let from = color_of(layer.props.get("color1"), (0xff, 0x2d, 0x55));
            let to = color_of(layer.props.get("color2"), (0xff, 0xb7, 0x03));
            let angle = (angle_prop(layer, "angle", 90.0) % 360.0) * std::f32::consts::PI / 180.0;
            let dx = angle.cos();
            let dy = angle.sin();
            let mut o = 0usize;
            for y in 0..height {
                for x in 0..width {
                    let nx = (x as f32 + 0.5) / width as f32 - 0.5;
                    let ny = (y as f32 + 0.5) / height as f32 - 0.5;
                    let t = (0.5 + nx * dx + ny * dy).clamp(0.0, 1.0);
                    out[o] = trunc_u8(from.0 as f32 + (to.0 as f32 - from.0 as f32) * t);
                    out[o + 1] = trunc_u8(from.1 as f32 + (to.1 as f32 - from.1 as f32) * t);
                    out[o + 2] = trunc_u8(from.2 as f32 + (to.2 as f32 - from.2 as f32) * t);
                    o += 3;
                }
            }
            out
        }
        "fire" => {
            let palette = match layer.props.get("palette") {
                Some(PropValue::Choice(v)) => v.as_str(),
                _ => "Classic",
            };
            let cool: (f32, f32, f32) = match palette {
                "Blue" => (40.0, 90.0, 255.0),
                "Green" => (40.0, 255.0, 110.0),
                "Mono" => (255.0, 255.0, 255.0),
                _ => (255.0, 170.0, 60.0),
            };
            let intensity = (number_prop(layer, "intensity", 55.0).clamp(0.0, 100.0)) / 100.0;
            let mut o = 0usize;
            for y in 0..height {
                for x in 0..width {
                    let nx = (x as f32 + 0.5) / width as f32 - 0.5;
                    let ny = (y as f32 + 0.5) / height as f32;
                    let flicker = 0.16 * (x as f32 * 0.31 + seconds * 3.1).sin()
                        * (y as f32 * 0.17 - seconds * 1.7).sin();
                    let heat = (1.0 - ((nx * 2.1).powi(2) + (1.0 - ny).powi(2)).sqrt() + flicker)
                        .clamp(0.0, 1.0)
                        * intensity;
                    out[o] = (cool.0 * heat + 12.0).min(255.0) as u8;
                    out[o + 1] = (cool.1 * heat).min(255.0) as u8;
                    out[o + 2] = (cool.2 * heat).min(255.0) as u8;
                    o += 3;
                }
            }
            out
        }
        "rain" => {
            // A sparse layer: a dark sky with bright grains.
            let drop = color_of(layer.props.get("drop_color"), (0xa8, 0xd8, 0xff));
            let bg = color_of(layer.props.get("background"), (0x05, 0x08, 0x14));
            for i in 0..width * height {
                out[i * 3] = bg.0;
                out[i * 3 + 1] = bg.1;
                out[i * 3 + 2] = bg.2;
            }
            for i in 0..30usize {
                let x = (((i as f32 * 61.0 + 17.0) / 67.0) % 1.0 * width as f32).floor() as usize
                    % width.max(1);
                let y = (((i as f32 * 37.0 + seconds * (12.0 + (i % 5) as f32 * 4.0)) / 43.0) % 1.0
                    * height as f32)
                    .floor() as usize
                    % height.max(1);
                let twinkle = 0.55 + 0.45 * (seconds * 3.0 + i as f32).sin();
                let at = (y * width + x) * 3;
                out[at] = (drop.0 as f32 * twinkle).min(255.0) as u8;
                out[at + 1] = (drop.1 as f32 * twinkle).min(255.0) as u8;
                out[at + 2] = (drop.2 as f32 * twinkle).min(255.0) as u8;
            }
            out
        }
        other => demo_frame_for(other, seconds, width, height),
    }
}

/// A float to a byte, the way a `Uint8Array` stores it: toward zero.
fn trunc_u8(v: f32) -> u8 {
    if v <= 0.0 {
        0
    } else if v >= 255.0 {
        255
    } else {
        v as u8
    }
}

/// One frame of the stack: every visible layer mapped into its area, bottom
/// first (`composeStack`).
pub fn compose_stack(
    stack: &Stack,
    layout: Option<&Layout>,
    seconds: f32,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 3];
    let mut acc = vec![0f32; width * height * 3];
    for layer in &stack.layers {
        if !layer.visible || layer.opacity == 0 {
            continue;
        }
        let Some(bounds) = area_bounds(&layer.area, layout) else {
            continue;
        };
        if bounds.width <= 0.0 || bounds.height <= 0.0 {
            continue;
        }
        let picture = demo_layer_picture(layer, width, height, seconds);
        let x0 = ((bounds.x / CANVAS_W) * width as f32).floor().max(0.0) as usize;
        let y0 = ((bounds.y / CANVAS_H) * height as f32).floor().max(0.0) as usize;
        let x1 = (((bounds.x + bounds.width) / CANVAS_W) * width as f32)
            .ceil()
            .min(width as f32) as usize;
        let y1 = (((bounds.y + bounds.height) / CANVAS_H) * height as f32)
            .ceil()
            .min(height as f32) as usize;
        let span_x = (x1 as isize - x0 as isize).max(1) as f32;
        let span_y = (y1 as isize - y0 as isize).max(1) as f32;
        let a = (layer.opacity.min(100) as f32) / 100.0;
        for y in y0..y1 {
            let sy = (((y as f32 + 0.5 - y0 as f32) / span_y) * height as f32)
                .floor()
                .min(height as f32 - 1.0) as usize;
            for x in x0..x1 {
                let sx = (((x as f32 + 0.5 - x0 as f32) / span_x) * width as f32)
                    .floor()
                    .min(width as f32 - 1.0) as usize;
                let d = (y * width + x) * 3;
                let s = (sy * width + sx) * 3;
                for c in 0..3 {
                    let under = acc[d + c];
                    let over = picture.get(s + c).copied().unwrap_or(0) as f32 / 255.0;
                    acc[d + c] = blend_channel(layer.blend, under, over, a);
                }
            }
        }
    }
    for (i, v) in acc.iter().enumerate() {
        out[i] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    out
}

// ---------------------------------------------------------------------------
// The demo's one generic source (`app/web/src/lib/thumb-demo.ts`).

const TAU: f32 = std::f32::consts::PI * 2.0;

/// The demo's loop, in seconds.
pub const DEMO_LOOP_SECONDS: f32 = 2.0;

fn fract(v: f32) -> f32 {
    v - v.floor()
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// HSV to RGB on the 0..255 scale.
fn hsv(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let hh = fract(h) * 6.0;
    let i = hh.floor();
    let f = hh - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let r = v * (1.0 - s * (1.0 - f));
    let c = match (i as i32) % 6 {
        0 => (v, r, p),
        1 => (q, v, p),
        2 => (p, v, r),
        3 => (p, q, v),
        4 => (r, p, v),
        _ => (v, p, q),
    };
    (c.0 * 255.0, c.1 * 255.0, c.2 * 255.0)
}

/// A stable number in 0..1 from an effect id, so each demo effect gets its
/// own colours.
fn seed_of(id: &str) -> f32 {
    let mut h: u32 = 2166136261;
    for byte in id.encode_utf16() {
        h ^= byte as u32;
        h = h.wrapping_mul(16777619);
    }
    ((h % 1000) as f32) / 1000.0
}

/// One frame of an effect's demo look, at the size the preview feed uses.
pub fn demo_frame_for(id: &str, seconds: f32, width: usize, height: usize) -> Vec<u8> {
    let base = seed_of(id);
    let t = ((seconds / DEMO_LOOP_SECONDS) % 1.0 + 1.0) % 1.0;
    let mut rgb = vec![0u8; width * height * 3];
    let mut o = 0usize;
    for py in 0..height {
        for px in 0..width {
            let x = (px as f32 + 0.5) / width as f32;
            let y = (py as f32 + 0.5) / height as f32;
            let ripple = (TAU * (x * 2.0 + y - t)).sin() * (TAU * (y * 3.0 - t)).sin();
            let c = hsv(
                base + 0.18 * x + 0.12 * (TAU * (y + t)).sin(),
                0.8,
                0.55 + 0.4 * (0.5 + 0.5 * ripple),
            );
            rgb[o] = (clamp01(c.0 / 255.0) * 255.0).round() as u8;
            rgb[o + 1] = (clamp01(c.1 / 255.0) * 255.0).round() as u8;
            rgb[o + 2] = (clamp01(c.2 / 255.0) * 255.0).round() as u8;
            o += 3;
        }
    }
    rgb
}

/// The layer's own values over the schema's defaults (React's `layerValues`),
/// as the compositor sees them.
pub fn merged_layer_values(layer: &StackLayer, schema: Option<&canvas_proto::props::PropertySchema>) -> Props {
    crate::views::effects::stack_draft::layer_values(schema, &layer.props)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::color::Rgb8;
    use canvas_proto::stack::LayerArea;

    #[test]
    fn the_normal_blend_mixes_by_the_opacity() {
        // s * a + d * (1 - a)
        assert!((blend_channel(BlendMode::Normal, 0.0, 1.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Normal, 0.5, 1.0, 0.5) - 0.75).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Normal, 0.5, 0.0, 0.5) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn the_add_blend_lightens_and_stops_at_one() {
        // min(1, d + a * (min(1, d + s) - d))
        assert!((blend_channel(BlendMode::Add, 0.2, 0.3, 1.0) - 0.5).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Add, 0.2, 0.3, 0.5) - 0.35).abs() < 1e-6);
        assert_eq!(blend_channel(BlendMode::Add, 0.9, 0.9, 1.0), 1.0);
    }

    #[test]
    fn the_multiply_blend_darkens() {
        // d * (1 - a + a * s)
        assert!((blend_channel(BlendMode::Multiply, 0.5, 0.5, 1.0) - 0.25).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Multiply, 0.5, 0.5, 0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_screen_blend_lifts_and_stops_at_one() {
        // min(1, d + a * s * (1 - d))
        assert!((blend_channel(BlendMode::Screen, 0.2, 0.5, 1.0) - 0.6).abs() < 1e-6);
        assert!((blend_channel(BlendMode::Screen, 0.8, 0.8, 1.0) - 0.96).abs() < 1e-6);
        assert_eq!(blend_channel(BlendMode::Screen, 0.5, 1.0, 1.0), 1.0);
    }

    fn layer(effect: &str, props: Props, area: LayerArea) -> StackLayer {
        StackLayer {
            id: "l1".into(),
            name: "L".into(),
            effect: effect.into(),
            props,
            area,
            blend: BlendMode::Normal,
            opacity: 100,
            visible: true,
        }
    }

    #[test]
    fn a_solid_layer_paints_the_whole_canvas_with_its_colour() {
        let mut props = Props::new();
        props.insert("color".into(), PropValue::Color(Rgb8::new(0x60, 0x00, 0xfc)));
        let stack = Stack {
            id: "stack:s".into(),
            layers: vec![layer("solid", props, LayerArea::Canvas)],
        };
        let rgb = compose_stack(&stack, None, 0.0, 320, 200);
        assert_eq!(rgb[0], 0x60);
        assert_eq!(rgb[1], 0x00);
        assert_eq!(rgb[2], 0xfc);
        assert_eq!(rgb.len(), 320 * 200 * 3);
    }

    #[test]
    fn a_hidden_layer_paints_nothing() {
        let mut l = layer("solid", Props::new(), LayerArea::Canvas);
        l.visible = false;
        let stack = Stack { id: "stack:s".into(), layers: vec![l] };
        let rgb = compose_stack(&stack, None, 0.0, 8, 4);
        assert!(rgb.iter().all(|v| *v == 0));
    }

    #[test]
    fn a_layer_with_no_opacity_paints_nothing() {
        let mut l = layer("solid", Props::new(), LayerArea::Canvas);
        l.opacity = 0;
        let stack = Stack { id: "stack:s".into(), layers: vec![l] };
        let rgb = compose_stack(&stack, None, 0.0, 8, 4);
        assert!(rgb.iter().all(|v| *v == 0));
    }

    #[test]
    fn a_gradient_runs_from_its_first_colour_to_its_second() {
        let mut props = Props::new();
        props.insert("color1".into(), PropValue::Color(Rgb8::new(0, 0, 0)));
        props.insert("color2".into(), PropValue::Color(Rgb8::new(255, 255, 255)));
        props.insert("angle".into(), PropValue::Angle(90.0));
        let stack = Stack { id: "stack:s".into(), layers: vec![layer("gradient", props, LayerArea::Canvas)] };
        let rgb = compose_stack(&stack, None, 0.0, 32, 20);
        // At 90 degrees the gradient runs down the canvas: the top row is
        // nearly the first colour and the bottom row nearly the second.
        // React's own numbers: the first pixel is `0 + 255 * 0.025` = 6.
        assert_eq!(rgb[0], 6);
        assert!(rgb[(19 * 32) * 3] > 200, "the last row is nearly the second colour");
    }

    #[test]
    fn a_rectangle_area_paints_only_its_box() {
        let mut props = Props::new();
        props.insert("color".into(), PropValue::Color(Rgb8::new(255, 255, 255)));
        let stack = Stack {
            id: "stack:s".into(),
            layers: vec![layer(
                "solid",
                props,
                LayerArea::Rect { x: 0.0, y: 0.0, width: 160.0, height: 100.0 },
            )],
        };
        let rgb = compose_stack(&stack, None, 0.0, 320, 200);
        // The top-left corner is inside the rectangle; the bottom-right is not.
        assert_eq!(rgb[0], 255);
        let last = ((199 * 320) + 319) * 3;
        assert_eq!(rgb[last], 0);
    }
}
