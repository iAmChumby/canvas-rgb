//! Effect thumbnails for `GET /api/v1/effects/{id}/thumbnail`.
//!
//! An effect is rendered headless at its schema defaults: no device is touched and the
//! engine is not involved. The render runs on a blocking thread (see the API handler).
//! The output is `u16 LE width, u16 LE height, u16 LE frames`, then frames x width x
//! height x 3 RGB bytes.
//!
//! The effect is rendered on the 320x200 canvas, averaged down to 64x40, after a one
//! second warm-up so trail effects have content. Screen Ambience and Audio Spectrum get
//! synthetic input, so their thumbnails are not black.

use canvas_proto::props::Props;
use canvas_proto::traits::{AudioSpectrum, Canvas, Effect, Inputs, ScreenFrame};
use canvas_proto::Rgb8;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

pub const WIDTH: u16 = 64;
pub const HEIGHT: u16 = 40;
pub const FRAMES: u16 = 24;
/// The frame rate the frames are stepped at.
const FPS: f64 = 12.0;
/// One second of steps before the first recorded frame.
const WARM_UP_FRAMES: u32 = 12;
/// The canvas the effect is rendered on.
const RENDER_WIDTH: u32 = 320;
const RENDER_HEIGHT: u32 = 200;
/// The synthetic spectrum's bands, log-spaced from 30 Hz to 16 kHz.
const SPECTRUM_BANDS: usize = 32;

/// Renders the thumbnail of `effect` at `props`. Returns the whole binary body.
pub fn render(effect: &mut dyn Effect, props: &Props) -> Vec<u8> {
    let needs = effect.schema().needs;
    effect.init(props);

    let synthetic = Synthetic {
        screen: needs.screen.then(|| Arc::new(gradient_screen())),
        audio: needs.audio.then(|| Arc::new(sloped_spectrum())),
    };
    let mut canvas = Canvas::new(RENDER_WIDTH, RENDER_HEIGHT);
    let step = 1.0 / FPS;
    let mut time = 0.0f64;

    for _ in 0..WARM_UP_FRAMES {
        effect.update(step as f32, &mut canvas, &synthetic.inputs(time));
        time += step;
    }

    let mut out = Vec::with_capacity(6 + FRAMES as usize * WIDTH as usize * HEIGHT as usize * 3);
    out.extend_from_slice(&WIDTH.to_le_bytes());
    out.extend_from_slice(&HEIGHT.to_le_bytes());
    out.extend_from_slice(&FRAMES.to_le_bytes());
    for _ in 0..FRAMES {
        effect.update(step as f32, &mut canvas, &synthetic.inputs(time));
        downsample(&canvas, &mut out);
        time += step;
    }
    out
}

/// The synthetic capture data handed to an effect, for the inputs it asks for.
struct Synthetic {
    screen: Option<Arc<ScreenFrame>>,
    audio: Option<Arc<AudioSpectrum>>,
}

impl Synthetic {
    fn inputs(&self, time: f64) -> Inputs {
        Inputs { time, screen: self.screen.clone(), audio: self.audio.clone() }
    }
}

/// Averages the canvas down to `WIDTH` x `HEIGHT`, one box of canvas pixels per output
/// pixel, and appends the RGB bytes to `out`.
fn downsample(canvas: &Canvas, out: &mut Vec<u8>) {
    let (sw, sh) = (canvas.width as usize, canvas.height as usize);
    let (dw, dh) = (WIDTH as usize, HEIGHT as usize);
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = ((dy + 1) * sh / dh).max(y0 + 1);
        for dx in 0..dw {
            let x0 = dx * sw / dw;
            let x1 = ((dx + 1) * sw / dw).max(x0 + 1);
            let mut sum = [0.0f64; 3];
            let mut count = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    if let Some(p) = canvas.pixels.get(y * sw + x) {
                        sum[0] += p[0] as f64;
                        sum[1] += p[1] as f64;
                        sum[2] += p[2] as f64;
                        count += 1;
                    }
                }
            }
            let n = count.max(1) as f64;
            let average = [(sum[0] / n) as f32, (sum[1] / n) as f32, (sum[2] / n) as f32];
            let c = Rgb8::from_f32(average);
            out.extend_from_slice(&[c.r, c.g, c.b]);
        }
    }
}

/// A fixed gradient standing in for the primary monitor.
fn gradient_screen() -> ScreenFrame {
    let (w, h) = (RENDER_WIDTH, RENDER_HEIGHT);
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.push(Rgb8 {
                r: (x * 255 / (w - 1)) as u8,
                g: (y * 255 / (h - 1)) as u8,
                b: 128,
            });
        }
    }
    ScreenFrame { width: w, height: h, pixels }
}

/// A fixed spectrum sloping down from the bass to the treble.
fn sloped_spectrum() -> AudioSpectrum {
    let lo = 30.0f32.ln();
    let hi = 16_000.0f32.ln();
    let last = (SPECTRUM_BANDS - 1) as f32;
    let centers_hz = (0..SPECTRUM_BANDS)
        .map(|i| (lo + (hi - lo) * i as f32 / last).exp())
        .collect();
    let magnitudes = (0..SPECTRUM_BANDS)
        .map(|i| 1.0 - 0.8 * i as f32 / last)
        .collect();
    AudioSpectrum { centers_hz, magnitudes }
}

/// The rendered thumbnails, keyed by effect id.
///
/// Every entry belongs to one generation of the user-effects set. A `get` for a different
/// generation drops everything first, so an edited or removed user effect never serves a
/// stale thumbnail.
#[derive(Default)]
pub struct ThumbnailCache {
    inner: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    generation: u64,
    entries: HashMap<String, Arc<Vec<u8>>>,
}

impl ThumbnailCache {
    /// The cached thumbnail for `id`, if the cache belongs to `generation`.
    pub fn get(&self, generation: u64, id: &str) -> Option<Arc<Vec<u8>>> {
        let mut state = lock(&self.inner);
        if state.generation != generation {
            state.entries.clear();
            state.generation = generation;
        }
        state.entries.get(id).cloned()
    }

    /// Stores a thumbnail rendered for `generation`. A render that finished after the set
    /// changed is dropped rather than cached.
    pub fn put(&self, generation: u64, id: &str, bytes: Arc<Vec<u8>>) {
        let mut state = lock(&self.inner);
        if state.generation == generation {
            state.entries.insert(id.to_string(), bytes);
        }
    }

    /// How many thumbnails are cached.
    pub fn len(&self) -> usize {
        lock(&self.inner).entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn lock(state: &Mutex<CacheState>) -> MutexGuard<'_, CacheState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::props::PropertySchema;
    use canvas_proto::traits::Effect;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Counts its updates and paints every pixel with the update count.
    struct Counter {
        updates: Arc<AtomicU32>,
    }

    impl Effect for Counter {
        fn schema(&self) -> PropertySchema {
            PropertySchema {
                id: "counter".into(),
                name: "Counter".into(),
                description: String::new(),
                needs: Default::default(),
                properties: Vec::new(),
                presets: Vec::new(),
            }
        }

        fn init(&mut self, _props: &Props) {}

        fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
            let n = self.updates.fetch_add(1, Ordering::SeqCst) + 1;
            let v = (n % 255) as f32 / 255.0;
            for p in canvas.pixels.iter_mut() {
                *p = [v, v, v];
            }
        }
    }

    #[test]
    fn the_header_and_length_match_the_format() {
        let updates = Arc::new(AtomicU32::new(0));
        let mut effect = Counter { updates: updates.clone() };
        let bytes = render(&mut effect, &Props::new());

        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), WIDTH);
        assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), HEIGHT);
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), FRAMES);
        let expected = 6 + FRAMES as usize * WIDTH as usize * HEIGHT as usize * 3;
        assert_eq!(bytes.len(), expected);
        // One warm-up second plus the recorded frames.
        assert_eq!(updates.load(Ordering::SeqCst), WARM_UP_FRAMES + FRAMES as u32);
    }

    #[test]
    fn the_cache_drops_its_entries_when_the_generation_changes() {
        let cache = ThumbnailCache::default();
        assert!(cache.get(0, "a").is_none());
        cache.put(0, "a", Arc::new(vec![1, 2, 3]));
        assert_eq!(cache.get(0, "a").as_deref(), Some(&vec![1, 2, 3]));

        // A new generation finds nothing, and a late put for the old one is dropped.
        assert!(cache.get(1, "a").is_none());
        cache.put(0, "a", Arc::new(vec![9]));
        assert!(cache.is_empty());
    }

    #[test]
    fn the_spectrum_slopes_down_and_spans_the_bands() {
        let spectrum = sloped_spectrum();
        assert_eq!(spectrum.centers_hz.len(), SPECTRUM_BANDS);
        assert!(spectrum.centers_hz.windows(2).all(|w| w[0] < w[1]));
        assert!(spectrum.magnitudes.windows(2).all(|w| w[0] > w[1]));
    }
}
