//! Screen Ambience: the primary monitor's picture, scaled to fill the canvas.

use crate::color::saturate;
use crate::props_util as p;
use crate::schema::SchemaBuilder;
use canvas_proto::{Canvas, Effect, Inputs, PropertySchema, Props, ScreenFrame};

/// The Screen Ambience effect.
pub struct ScreenAmbience {
    schema: PropertySchema,
    saturation_boost: f32,
    smoothing: f32,
    brightness: f32,
    /// The previous frame's color, used for the smoothing.
    previous: Vec<[f32; 3]>,
    /// True once `previous` holds a frame.
    primed: bool,
    /// The cached source-to-target mapping for the area-average downscale.
    map: Option<crate::raster::AreaMap>,
}

impl ScreenAmbience {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(
            "screen_ambience",
            "Screen Ambience",
            "The primary monitor's picture, scaled to fill the canvas.",
        )
        .needs_screen()
        .number("saturation", "Saturation Boost", 0.0, 300.0, 1.0, 120.0)
        .number("smoothing", "Smoothing", 0.0, 100.0, 1.0, 30.0)
        .number("brightness", "Brightness", 0.0, 200.0, 1.0, 100.0)
        .preset(
            "Punchy",
            vec![("saturation", number_kind(180.0))],
        )
        .preset(
            "Soft",
            vec![
                ("saturation", number_kind(90.0)),
                ("smoothing", number_kind(60.0)),
            ],
        )
        .preset(
            "Bright",
            vec![("brightness", number_kind(140.0))],
        )
        .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self {
            schema,
            saturation_boost: 1.2,
            smoothing: 0.3,
            brightness: 1.0,
            previous: Vec::new(),
            primed: false,
            map: None,
        };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for ScreenAmbience {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(ScreenAmbience::new())
}

impl Effect for ScreenAmbience {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.saturation_boost = p::number(&self.schema, props, "saturation") as f32 / 100.0;
        self.smoothing = p::number(&self.schema, props, "smoothing") as f32 / 100.0;
        self.brightness = p::number(&self.schema, props, "brightness") as f32 / 100.0;
    }

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
        let frame: Option<std::sync::Arc<ScreenFrame>> = inputs.screen.clone();
        let Some(frame) = frame else {
            // No capture: the canvas goes black.
            canvas.pixels.fill([0.0, 0.0, 0.0]);
            return;
        };
        if frame.width == 0 || frame.height == 0 || frame.pixels.is_empty() {
            canvas.pixels.fill([0.0, 0.0, 0.0]);
            return;
        }
        // The downscale's mapping is cached, so the hot path does not allocate.
        let map = match &self.map {
            Some(m) if m.fits(frame.width, frame.height, canvas.width, canvas.height) => m.clone(),
            _ => {
                let m = crate::raster::AreaMap::new(frame.width, frame.height, canvas.width, canvas.height);
                self.map = Some(m.clone());
                m
            }
        };
        crate::raster::blit_area_average_with(canvas, frame.width, frame.height, &frame.pixels, &map);

        // Apply saturation, brightness and smoothing in place.
        let n = canvas.pixels.len();
        if self.previous.len() != n {
            self.previous = vec![[0.0; 3]; n];
            self.primed = false;
        }
        let mix_amount = self.smoothing.clamp(0.0, 0.95);
        for i in 0..n {
            let mut c = saturate(canvas.pixels[i], self.saturation_boost);
            c = [c[0] * self.brightness, c[1] * self.brightness, c[2] * self.brightness];
            if self.primed && mix_amount > 0.0 {
                let prev = self.previous[i];
                c = [
                    prev[0] + (c[0] - prev[0]) * (1.0 - mix_amount),
                    prev[1] + (c[1] - prev[1]) * (1.0 - mix_amount),
                    prev[2] + (c[2] - prev[2]) * (1.0 - mix_amount),
                ];
            }
            c = [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)];
            canvas.pixels[i] = c;
            self.previous[i] = c;
        }
        self.primed = true;
    }
}

fn number_kind(n: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Number { min: 0.0, max: 300.0, step: 1.0, default: n }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn all_red(w: u32, h: u32) -> Arc<ScreenFrame> {
        Arc::new(ScreenFrame { width: w, height: h, pixels: vec![canvas_proto::Rgb8::new(255, 0, 0); (w * h) as usize] })
    }

    #[test]
    fn the_schema_asks_for_the_screen() {
        assert!(ScreenAmbience::schema().needs.screen);
        assert!(!ScreenAmbience::schema().needs.audio);
    }

    #[test]
    fn an_all_red_screen_gives_a_red_canvas() {
        let mut e = ScreenAmbience::new();
        let mut canvas = Canvas::new(64, 40);
        let inputs = Inputs {
            screen: Some(all_red(320, 200)),
            ..Inputs::default()
        };
        e.update(1.0 / 30.0, &mut canvas, &inputs);
        // The mean is red within 10%.
        let mut r = 0.0;
        let mut g = 0.0;
        let mut b = 0.0;
        for p in canvas.pixels.iter() {
            r += p[0];
            g += p[1];
            b += p[2];
        }
        let n = canvas.pixels.len() as f32;
        let (r, g, b) = (r / n, g / n, b / n);
        assert!((r - 1.0).abs() < 0.1, "mean red {r}");
        assert!(g < 0.1, "mean green {g}");
        assert!(b < 0.1, "mean blue {b}");
    }

    #[test]
    fn no_capture_gives_black() {
        let mut e = ScreenAmbience::new();
        let mut canvas = Canvas::new(16, 16);
        canvas.pixels.fill([1.0, 1.0, 1.0]);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        assert!(canvas.pixels.iter().all(|p| p == &[0.0, 0.0, 0.0]));
    }

    #[test]
    fn the_canvas_is_scaled_to_fill() {
        let mut e = ScreenAmbience::new();
        let mut canvas = Canvas::new(64, 40);
        let mut inputs = Inputs::default();
        // A frame with a red left half and a blue right half.
        let mut pixels = Vec::with_capacity(320 * 200);
        for _ in 0..200 {
            for _ in 0..160 {
                pixels.push(canvas_proto::Rgb8::new(255, 0, 0));
            }
            for _ in 0..160 {
                pixels.push(canvas_proto::Rgb8::new(0, 0, 255));
            }
        }
        inputs.screen = Some(Arc::new(ScreenFrame { width: 320, height: 200, pixels }));
        e.update(1.0 / 30.0, &mut canvas, &inputs);
        let left = canvas.get(4, 20);
        let right = canvas.get(60, 20);
        assert!(left[0] > 0.8 && left[2] < 0.2, "left: {left:?}");
        assert!(right[2] > 0.8 && right[0] < 0.2, "right: {right:?}");
    }
}
