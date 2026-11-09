//! White: a white of one colour temperature, warm to cool, across the whole canvas.

use crate::color::{KELVIN_MAX, KELVIN_MIN, kelvin_to_led};
use crate::props_util as p;
use crate::schema::{SchemaBuilder, num};
use canvas_proto::{Canvas, Effect, PropertySchema, Props};

/// The temperature White starts at: a neutral white.
pub const DEFAULT_KELVIN: f64 = 4000.0;

/// The White effect.
pub struct White {
    schema: PropertySchema,
    color: [f32; 3],
}

impl White {
    pub fn schema() -> PropertySchema {
        // 2700-6500 K is govee-cli's Set Temperature range, so a desk lamp and the PC can be
        // set to the same number; the range reaches down to candlelight below that.
        SchemaBuilder::new("white", "White", "A white of the colour temperature you pick, warm to cool.")
            .number("temperature", "Kelvin", KELVIN_MIN as f64, KELVIN_MAX as f64, 100.0, DEFAULT_KELVIN)
            .preset_values("Candle", vec![("temperature", num(1800.0))])
            .preset_values("Warm", vec![("temperature", num(2700.0))])
            .preset_values("Neutral", vec![("temperature", num(4000.0))])
            .preset_values("Daylight", vec![("temperature", num(5000.0))])
            .preset_values("Cool", vec![("temperature", num(6500.0))])
            .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self { schema, color: [0.0; 3] };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for White {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(White::new())
}

impl Effect for White {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        // LED drive levels, not display values: sent unchanged, the display white reads
        // yellow on an LED instead of amber (#50, `canvas_proto::color::kelvin_to_led`).
        self.color = kelvin_to_led(p::number(&self.schema, props, "temperature") as f32);
    }

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::Inputs) {
        canvas.pixels.fill(self.color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::{Inputs, PropValue};

    fn render(props: &Props) -> Canvas {
        let mut e = White::new();
        e.init(props);
        let mut canvas = Canvas::new(16, 8);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        canvas
    }

    fn at(kelvin: f64) -> Props {
        let mut props = Props::new();
        props.insert("temperature".into(), PropValue::Number(kelvin));
        props
    }

    #[test]
    fn every_pixel_is_the_white_of_the_chosen_temperature() {
        for k in [1800.0, 2700.0, 4000.0, 6500.0] {
            let canvas = render(&at(k));
            let want = kelvin_to_led(k as f32);
            assert!(canvas.pixels.iter().all(|p| *p == want), "{k} K is not one flat {want:?}");
        }
    }

    #[test]
    fn the_default_is_a_neutral_white() {
        let canvas = render(&Props::new());
        assert_eq!(canvas.pixels[0], kelvin_to_led(DEFAULT_KELVIN as f32));
    }

    #[test]
    fn warm_and_cool_presets_draw_different_whites() {
        let schema = White::schema();
        let preset = |name: &str| {
            let p = schema.presets.iter().find(|p| p.name == name).expect("preset");
            render(&p.props).pixels[0]
        };
        let (warm, cool) = (preset("Warm"), preset("Cool"));
        assert!(warm[2] < cool[2], "Warm is no warmer than Cool: {warm:?} vs {cool:?}");
        assert_eq!(warm, kelvin_to_led(2700.0));
    }

    /// #50 regression: v0.10.0 sent the display white, 1800 K = #ff7e1f, which the lights
    /// showed far less warm than the Govee lamps.
    #[test]
    fn the_warm_end_drives_the_leds_amber() {
        let c = canvas_proto::Rgb8::from_f32(render(&at(1800.0)).pixels[0]);
        assert_eq!(c.to_hex(), "#ff3603");
    }

    #[test]
    fn a_temperature_outside_the_slider_is_clamped() {
        assert_eq!(render(&at(500.0)).pixels[0], kelvin_to_led(KELVIN_MIN));
        assert_eq!(render(&at(99_000.0)).pixels[0], kelvin_to_led(KELVIN_MAX));
    }
}
