//! Audio Spectrum: one bar per audio band, colored by a gradient.

use crate::color::{mix, to_linear, Rgb, WHITE};
use crate::props_util as p;
use crate::schema::SchemaBuilder;
use canvas_proto::traits::AudioSpectrum as Spectrum;
use canvas_proto::{Canvas, Effect, Inputs, PropertySchema, Props};

/// The Audio Spectrum effect.
pub struct AudioSpectrum {
    schema: PropertySchema,
    low: Rgb,
    high: Rgb,
    sensitivity: f32,
    decay: f32,
    /// How long the peak marker lingers, 0..1.
    peak_hold: f32,
    /// The smoothed magnitude of each band.
    levels: Vec<f32>,
    /// The highest level each band has reached, and how fast it falls.
    peaks: Vec<f32>,
}

impl AudioSpectrum {
    pub fn schema() -> PropertySchema {
        SchemaBuilder::new(
            "audio_spectrum",
            "Audio Spectrum",
            "One bar per audio band, colored by a gradient.",
        )
        .needs_audio()
        .color("color_low", "Low Color", "#6000fc")
        .color("color_high", "High Color", "#00ff9c")
        .number("sensitivity", "Sensitivity", 0.0, 500.0, 1.0, 150.0)
        .number("decay", "Decay", 0.0, 100.0, 1.0, 40.0)
        .number("peak_hold", "Peak Hold", 0.0, 100.0, 1.0, 30.0)
        .preset(
            "Neon",
            vec![
                ("color_low", color_kind("#ff00a0")),
                ("color_high", color_kind("#00fff0")),
            ],
        )
        .preset(
            "Warm",
            vec![
                ("color_low", color_kind("#ff5e00")),
                ("color_high", color_kind("#ffe600")),
            ],
        )
        .preset(
            "Punchy",
            vec![("sensitivity", number_kind(250.0)), ("decay", number_kind(15.0))],
        )
        // The tile feeds the effect a spectrum that moves once per loop. At the defaults the bars
        // clip; a higher sensitivity and a faster decay show them rise and fall.
        .thumbnail(crate::schema::recipe(2.0, 2.0, 6, vec![("sensitivity", crate::schema::num(250.0)), ("decay", crate::schema::num(30.0))]))
        .build()
    }

    pub fn new() -> Self {
        let schema = Self::schema();
        let mut e = Self {
            schema,
            low: [0.0; 3],
            high: [0.0; 3],
            sensitivity: 1.5,
            decay: 0.4,
            peak_hold: 0.3,
            levels: Vec::new(),
            peaks: Vec::new(),
        };
        e.init(&e.schema.defaults());
        e
    }
}

impl Default for AudioSpectrum {
    fn default() -> Self {
        Self::new()
    }
}

/// The factory the registry uses.
pub fn create() -> Box<dyn Effect> {
    Box::new(AudioSpectrum::new())
}

impl Effect for AudioSpectrum {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, props: &Props) {
        self.low = to_linear(p::color(&self.schema, props, "color_low"));
        self.high = to_linear(p::color(&self.schema, props, "color_high"));
        self.sensitivity = p::number(&self.schema, props, "sensitivity") as f32 / 100.0;
        self.decay = p::number(&self.schema, props, "decay") as f32 / 100.0;
        self.peak_hold = p::number(&self.schema, props, "peak_hold") as f32 / 100.0;
    }

    fn update(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
        let spectrum: Option<std::sync::Arc<Spectrum>> = inputs.audio.clone();
        let Some(spectrum) = spectrum else {
            canvas.pixels.fill([0.0, 0.0, 0.0]);
            return;
        };
        let bands = spectrum.magnitudes.len().min(spectrum.centers_hz.len());
        if bands == 0 {
            canvas.pixels.fill([0.0, 0.0, 0.0]);
            return;
        }
        if self.levels.len() != bands {
            self.levels = vec![0.0; bands];
            self.peaks = vec![0.0; bands];
        }

        // Smooth the magnitudes: a fast attack, a slow decay.
        let fall = (1.0 - self.decay.clamp(0.0, 0.99)).powf(dt.max(0.0) * 60.0);
        // The peak marker falls back at a rate set by the peak hold.
        let peak_fall = (1.0 - self.peak_hold.clamp(0.0, 0.99)).powf(dt.max(0.0) * 60.0);
        for i in 0..bands {
            let raw = (spectrum.magnitudes[i] * self.sensitivity).clamp(0.0, 1.0);
            let level = &mut self.levels[i];
            if raw >= *level {
                *level = raw;
            } else {
                *level *= fall.max(0.0);
            }
            let peak = &mut self.peaks[i];
            if *level > *peak {
                *peak = *level;
            } else {
                *peak *= peak_fall.max(0.0);
            }
        }

        // One vertical bar per band, on a cleared canvas: the engine hands the canvas back as
        // the last frame left it, so a bar that fell would otherwise stay painted.
        canvas.pixels.fill([0.0, 0.0, 0.0]);
        let n = canvas.width as usize;
        let bar_w = (n / bands).max(1);
        for b in 0..bands {
            let level = self.levels[b];
            let x0 = b * bar_w;
            let x1 = ((b + 1) * bar_w).min(n);
            let color = mix(self.low, self.high, b as f32 / (bands.max(1) - 1).max(1) as f32);
            let h = (level * canvas.height as f32).round() as u32;
            for x in x0..x1 {
                for y in (canvas.height - h)..canvas.height {
                    let i = (y as usize) * n + x;
                    canvas.pixels[i] = color;
                }
            }
            // The peak marker, a brighter line at the top of the bar.
            let peak_y = (self.peaks[b] * canvas.height as f32).round() as u32;
            if peak_y > 0 && peak_y <= canvas.height {
                for x in x0..x1 {
                    let y = canvas.height - peak_y;
                    let i = (y as usize) * n + x;
                    canvas.pixels[i] = WHITE;
                }
            }
        }
    }
}

fn number_kind(n: f64) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Number { min: 0.0, max: 500.0, step: 1.0, default: n }
}

fn color_kind(s: &str) -> canvas_proto::PropKind {
    canvas_proto::PropKind::Color {
        default: s.parse().unwrap_or(canvas_proto::Rgb8::BLACK),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A spectrum with one band peaking at 1 kHz.
    ///
    /// The centers are log-spaced from 60 Hz to 16 kHz over 32 bands, the usual audio
    /// layout. The band whose centre is closest to 1 kHz is the loud one.
    fn spectrum_peaking_at_1khz() -> Arc<Spectrum> {
        let centers: Vec<f32> = (0..32).map(|i| 60.0 * 2f32.powf(i as f32 / 4.0)).collect();
        let loudest = centers
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (*a - 1000.0)
                    .abs()
                    .partial_cmp(&(*b - 1000.0).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        let magnitudes: Vec<f32> = (0..32)
            .map(|i| if i == loudest { 1.0 } else { 0.02 })
            .collect();
        Arc::new(Spectrum { centers_hz: centers, magnitudes })
    }

    #[test]
    fn the_schema_asks_for_the_audio() {
        assert!(AudioSpectrum::schema().needs.audio);
        assert!(!AudioSpectrum::schema().needs.screen);
    }

    /// The band index whose centre is closest to 1 kHz.
    fn band_at_1khz() -> usize {
        let centers: Vec<f32> = (0..32).map(|i| 60.0 * 2f32.powf(i as f32 / 4.0)).collect();
        centers
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (*a - 1000.0)
                    .abs()
                    .partial_cmp(&(*b - 1000.0).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    #[test]
    fn the_tallest_bar_is_the_loudest_band() {
        let mut e = AudioSpectrum::new();
        let mut canvas = Canvas::new(160, 100);
        let inputs = Inputs {
            audio: Some(spectrum_peaking_at_1khz()),
            ..Inputs::default()
        };
        // Several frames so the smoothing settles.
        for _ in 0..10 {
            e.update(1.0 / 30.0, &mut canvas, &inputs);
        }
        // Find the tallest column. The bars grow upwards from the bottom of the
        // canvas, so the bar's height is the number of lit pixels at the bottom of
        // the column.
        let mut best_x = 0;
        let mut best_h = 0u32;
        for x in 0..canvas.width {
            let h = (0..canvas.height)
                .rev()
                .take_while(|y| canvas.get(x, *y).iter().any(|v| *v > 0.01))
                .count() as u32;
            if h > best_h {
                best_h = h;
                best_x = x;
            }
        }
        assert!(best_h > 50, "no bar was drawn: best_h={best_h} at x={best_x} of {}x{}", canvas.width, canvas.height);
        // The tallest bar must be the 1 kHz band.
        let band = best_x / (canvas.width / 32).max(1);
        assert_eq!(
            band, band_at_1khz() as u32,
            "the tallest bar is in band {band}, not the 1 kHz band"
        );
    }

    #[test]
    fn no_audio_gives_black() {
        let mut e = AudioSpectrum::new();
        let mut canvas = Canvas::new(32, 32);
        canvas.pixels.fill([1.0, 1.0, 1.0]);
        e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
        assert!(canvas.pixels.iter().all(|p| p == &[0.0, 0.0, 0.0]));
    }

    #[test]
    fn the_peak_marker_lingers_above_the_bar() {
        let mut e = AudioSpectrum::new();
        let mut canvas = Canvas::new(160, 100);
        let inputs = Inputs {
            audio: Some(spectrum_peaking_at_1khz()),
            ..Inputs::default()
        };
        for _ in 0..10 {
            e.update(1.0 / 30.0, &mut canvas, &inputs);
        }
        // The peak marker is a white line at the top of the loudest bar.
        let mut white_rows = 0;
        for y in 0..canvas.height {
            if canvas.get(85, y).iter().all(|v| *v > 0.9) {
                white_rows += 1;
            }
        }
        assert!(white_rows >= 1, "no peak marker was drawn");
    }

    #[test]
    fn output_stays_in_range() {
        let mut e = AudioSpectrum::new();
        let mut canvas = Canvas::new(64, 40);
        let inputs = Inputs {
            audio: Some(spectrum_peaking_at_1khz()),
            ..Inputs::default()
        };
        for _ in 0..120 {
            e.update(1.0 / 30.0, &mut canvas, &inputs);
        }
        for p in canvas.pixels.iter() {
            for v in p {
                assert!((0.0..=1.0).contains(v), "{p:?}");
            }
        }
    }
}
