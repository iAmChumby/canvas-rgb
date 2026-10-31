//! Effect thumbnails for `GET /api/v1/effects/{id}/thumbnail`.
//!
//! An effect is rendered headless: no device is touched and the engine is not involved. The
//! render runs on a blocking thread (see the API handler). The output is
//! `u16 LE width, u16 LE height, u16 LE frames`, then frames x width x height x 3 RGB bytes.
//!
//! The effect is rendered on the 320x200 canvas and averaged down to 64x40. How it is
//! rendered comes from its [`ThumbnailRecipe`] (`PropertySchema::thumbnail`): the warm-up,
//! the loop length, a crossfade of the loop's end into its start, and showcase props laid
//! over the schema defaults. An effect with no recipe gets [`ThumbnailRecipe::default`]: one
//! second of warm-up and a two second loop. Screen Ambience and Audio Spectrum get synthetic
//! input, so their thumbnails are not black.
//!
//! The quality bar every tile has to pass lives here too ([`score`], [`problems`] and the
//! `BAR_*` constants), so the test over every effect and `canvas-rgb effects check` read
//! the same numbers against the same thresholds.

use canvas_proto::props::{Props, ThumbnailRecipe};
use canvas_proto::traits::{AudioSpectrum, Canvas, Effect, Inputs, ScreenFrame};
use canvas_proto::Rgb8;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

pub const WIDTH: u16 = 64;
pub const HEIGHT: u16 = 40;
pub const FRAMES: u16 = 24;
/// The longest step the effect is updated by: 1/15 s is four of the 60 Hz frames an HTML effect
/// runs per update, the most its runtime will catch up in one call. A longer step would lose time.
const MAX_STEP_S: f64 = 1.0 / 15.0;
/// The canvas the effect is rendered on.
const RENDER_WIDTH: u32 = 320;
const RENDER_HEIGHT: u32 = 200;
/// The synthetic spectrum's bands, log-spaced from 30 Hz to 16 kHz.
const SPECTRUM_BANDS: usize = 32;
/// The longest warm-up or loop a recipe may ask for, in seconds.
const MAX_RECIPE_SECONDS: f64 = 30.0;
/// The shortest loop a recipe may ask for, in seconds.
const MIN_LOOP_SECONDS: f64 = 0.5;

/// A pixel is lit when its lightness (CIE L*, 0..=100) is at least this.
///
/// Lightness, not the sum of the channels: the sum counted a near-black grey as lit, so a
/// black tile could pass the lit bar. L* measures what a pixel looks like, so a blue or a
/// violet counts for what it is and not for how little green it has. A neutral grey has to
/// be brighter than about 31 of 255 to count.
pub const LIT_LIGHTNESS: f64 = 12.0;
/// The quality bar: the average lit share of the frames is at least this.
pub const BAR_LIT_AVG: f64 = 0.60;
/// The quality bar: the last frame differs from the first by at most this share of full
/// scale (mean absolute difference over every channel of every pixel).
pub const BAR_SEAM: f64 = 0.10;
/// The quality bar: every row and every column is lit, over the whole loop, in at least this
/// share of its pixels (a single lit pixel does not make a row lit).
pub const BAR_LINE_SHARE: f64 = 0.25;
/// The longest a thumbnail render may run, in wall-clock time, before it is given up. A
/// recipe at its limits (30 s of warm-up, a 30 s loop and a crossfade) asks an HTML effect for
/// thousands of frames at up to 100 ms each; this stops that costing minutes of a core.
pub const RENDER_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

/// Renders the thumbnail of `effect` at `props`, within [`RENDER_BUDGET`]. Returns the whole
/// binary body, or why the render was given up.
///
/// The effect's own recipe applies: its showcase props are laid over `props`.
pub fn render(effect: &mut dyn Effect, props: &Props) -> Result<Vec<u8>, String> {
    render_within(effect, props, RENDER_BUDGET)
}

/// [`render`] with the time budget given.
pub fn render_within(
    effect: &mut dyn Effect,
    props: &Props,
    budget: std::time::Duration,
) -> Result<Vec<u8>, String> {
    let started = std::time::Instant::now();
    let over_budget = || {
        (started.elapsed() > budget)
            .then(|| format!("the thumbnail render ran past its {} s budget and was stopped", budget.as_secs()))
    };
    let schema = effect.schema();
    let needs = schema.needs;
    let recipe = sanitize(schema.thumbnail.unwrap_or_default());
    let mut props = props.clone();
    props.extend(recipe.props.clone());
    effect.init(&props);

    let synthetic = Synthetic { screen: needs.screen, audio: needs.audio, loop_s: recipe.loop_s };
    let mut canvas = Canvas::new(RENDER_WIDTH, RENDER_HEIGHT);
    let mut time = 0.0f64;

    let warm_up_steps = steps_for(recipe.warm_up_s);
    let warm_up_dt = if warm_up_steps == 0 { 0.0 } else { recipe.warm_up_s / warm_up_steps as f64 };
    for _ in 0..warm_up_steps {
        effect.update(warm_up_dt as f32, &mut canvas, &synthetic.inputs(time));
        time += warm_up_dt;
        if let Some(why) = over_budget() {
            return Err(why);
        }
    }

    // One recorded frame covers loop_s / FRAMES seconds, cut into steps of at most MAX_STEP_S.
    let frame_dt = recipe.loop_s / FRAMES as f64;
    let sub_steps = steps_for(frame_dt).max(1);
    let sub_dt = frame_dt / sub_steps as f64;
    let total = FRAMES as usize + recipe.crossfade_frames as usize;
    let mut tiles: Vec<Vec<u8>> = Vec::with_capacity(total);
    for _ in 0..total {
        for _ in 0..sub_steps {
            effect.update(sub_dt as f32, &mut canvas, &synthetic.inputs(time));
            time += sub_dt;
            if let Some(why) = over_budget() {
                return Err(why);
            }
        }
        let mut tile = Vec::with_capacity(WIDTH as usize * HEIGHT as usize * 3);
        downsample(&canvas, &mut tile);
        tiles.push(tile);
    }
    let tiles = crossfade(tiles, recipe.crossfade_frames as usize);

    let mut out = Vec::with_capacity(6 + FRAMES as usize * WIDTH as usize * HEIGHT as usize * 3);
    out.extend_from_slice(&WIDTH.to_le_bytes());
    out.extend_from_slice(&HEIGHT.to_le_bytes());
    out.extend_from_slice(&FRAMES.to_le_bytes());
    for tile in tiles {
        out.extend_from_slice(&tile);
    }
    Ok(out)
}

/// How many updates cover `seconds` with no step longer than [`MAX_STEP_S`].
fn steps_for(seconds: f64) -> u32 {
    ((seconds / MAX_STEP_S) - 1e-9).ceil().max(0.0) as u32
}

/// Keeps a recipe's numbers inside what the renderer can run: finite, and no longer than
/// [`MAX_RECIPE_SECONDS`].
fn sanitize(mut recipe: ThumbnailRecipe) -> ThumbnailRecipe {
    let defaults = ThumbnailRecipe::default();
    if !recipe.warm_up_s.is_finite() {
        recipe.warm_up_s = defaults.warm_up_s;
    }
    if !recipe.loop_s.is_finite() {
        recipe.loop_s = defaults.loop_s;
    }
    recipe.warm_up_s = recipe.warm_up_s.clamp(0.0, MAX_RECIPE_SECONDS);
    recipe.loop_s = recipe.loop_s.clamp(MIN_LOOP_SECONDS, MAX_RECIPE_SECONDS);
    recipe.crossfade_frames = recipe.crossfade_frames.min(FRAMES as u32 / 2);
    recipe
}

/// Closes a loop that has no period. `tiles` holds the loop's frames followed by `fade`
/// extra frames that carry on past its end. The first `fade` output frames blend those extra
/// frames into the loop's own start, so the last frame leads straight into the first.
fn crossfade(mut tiles: Vec<Vec<u8>>, fade: usize) -> Vec<Vec<u8>> {
    if fade == 0 || tiles.len() <= fade {
        tiles.truncate(FRAMES as usize);
        return tiles;
    }
    let looped = tiles.len() - fade;
    let tail = tiles.split_off(looped);
    for (i, extra) in tail.iter().enumerate() {
        // The extra frame fades out as the loop's own frame fades in.
        let t = (i + 1) as f32 / (fade + 1) as f32;
        for (own, past) in tiles[i].iter_mut().zip(extra) {
            *own = (*past as f32 * (1.0 - t) + *own as f32 * t).round() as u8;
        }
    }
    tiles
}

/// What a tile measures, the way the issue's table did: a pixel is lit when its lightness is
/// at least [`LIT_LIGHTNESS`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileScore {
    /// The share of lit pixels in the first frame, 0..=1.
    pub lit_first: f64,
    /// The share of lit pixels in the last frame.
    pub lit_last: f64,
    /// The share of lit pixels, averaged over the frames.
    pub lit_avg: f64,
    /// Rows lit over the loop (at least [`BAR_LINE_SHARE`] of the row's pixels lit in some
    /// frame), out of [`HEIGHT`].
    pub rows_lit: usize,
    /// Columns lit over the loop, out of [`WIDTH`].
    pub cols_lit: usize,
    /// The last frame against the first: mean absolute difference over every channel,
    /// as a share of full scale. This is the jump when the loop restarts.
    pub seam: f64,
    /// The same difference between consecutive frames, averaged: how much the tile moves.
    pub step: f64,
    /// The largest difference between two consecutive frames, inside the loop: a hard cut
    /// shows here.
    pub max_step: f64,
}

/// Measures a thumbnail body (the binary format above). `None` when the body is malformed.
pub fn score(body: &[u8]) -> Option<TileScore> {
    if body.len() < 6 {
        return None;
    }
    let w = u16::from_le_bytes([body[0], body[1]]) as usize;
    let h = u16::from_le_bytes([body[2], body[3]]) as usize;
    let n = u16::from_le_bytes([body[4], body[5]]) as usize;
    let frame_len = w * h * 3;
    if w == 0 || h == 0 || n == 0 || body.len() != 6 + n * frame_len {
        return None;
    }
    let frames: Vec<&[u8]> = body[6..].chunks_exact(frame_len).collect();

    // Which pixels are lit in some frame, and the lit share of each frame.
    let mut ever = vec![false; w * h];
    let mut shares = Vec::with_capacity(n);
    for frame in &frames {
        let mut lit = 0usize;
        for i in 0..w * h {
            let px = &frame[i * 3..i * 3 + 3];
            if lightness(px) >= LIT_LIGHTNESS {
                lit += 1;
                ever[i] = true;
            }
        }
        shares.push(lit as f64 / (w * h) as f64);
    }
    let rows_lit = (0..h)
        .filter(|y| (0..w).filter(|x| ever[y * w + x]).count() as f64 >= BAR_LINE_SHARE * w as f64)
        .count();
    let cols_lit = (0..w)
        .filter(|x| (0..h).filter(|y| ever[y * w + x]).count() as f64 >= BAR_LINE_SHARE * h as f64)
        .count();
    let diff = |a: &[u8], b: &[u8]| -> f64 {
        let sum: u64 = a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64).sum();
        sum as f64 / (a.len() as f64 * 255.0)
    };
    let steps: Vec<f64> = frames.windows(2).map(|p| diff(p[0], p[1])).collect();
    Some(TileScore {
        lit_first: shares[0],
        lit_last: shares[n - 1],
        lit_avg: shares.iter().sum::<f64>() / n as f64,
        rows_lit,
        cols_lit,
        seam: diff(frames[0], frames[n - 1]),
        step: if steps.is_empty() { 0.0 } else { steps.iter().sum::<f64>() / steps.len() as f64 },
        max_step: steps.iter().copied().fold(0.0, f64::max),
    })
}

/// The lightness of one pixel, 0..=100: the CIE L* of its sRGB values.
///
/// L* is the perceptual lightness: it is 0 for black and 100 for white, and it weighs the
/// channels the way the eye does after the gamma curve, so a blue or a violet is as light as
/// it looks and not as light as its channel sum says.
pub fn lightness(px: &[u8]) -> f64 {
    let y = 0.2126 * linear(px[0]) + 0.7152 * linear(px[1]) + 0.0722 * linear(px[2]);
    if y <= 0.008_856 {
        903.3 * y
    } else {
        116.0 * y.cbrt() - 16.0
    }
}

/// One sRGB channel (0..=255) as a linear light, 0..=1.
fn linear(channel: u8) -> f64 {
    static LINEAR: std::sync::OnceLock<[f64; 256]> = std::sync::OnceLock::new();
    let table = LINEAR.get_or_init(|| {
        let mut t = [0.0f64; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f64 / 255.0;
            *v = if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
        }
        t
    });
    table[channel as usize]
}

/// What a tile fails of the bar, as sentences. Empty when it passes.
///
/// A recipe that sets `dark_on_purpose` is exempt from the lit-share bar only, and one that
/// sets `loop_jump_on_purpose` from the seam bar only; each states its reason.
pub fn problems(score: &TileScore, recipe: Option<&ThumbnailRecipe>) -> Vec<String> {
    let mut out = Vec::new();
    let dark = recipe.and_then(|r| r.dark_on_purpose.as_deref()).is_some();
    let jumps = recipe.and_then(|r| r.loop_jump_on_purpose.as_deref()).is_some();
    if !dark && score.lit_avg < BAR_LIT_AVG {
        out.push(format!(
            "the tile is {:.0} % lit on average; the bar is {:.0} % (or set dark_on_purpose in the recipe, with the reason)",
            score.lit_avg * 100.0,
            BAR_LIT_AVG * 100.0
        ));
    }
    if score.rows_lit < HEIGHT as usize {
        out.push(format!(
            "only {} of {} rows are lit (at least {:.0} % of the row, over the loop)",
            score.rows_lit,
            HEIGHT,
            BAR_LINE_SHARE * 100.0
        ));
    }
    if score.cols_lit < WIDTH as usize {
        out.push(format!(
            "only {} of {} columns are lit (at least {:.0} % of the column, over the loop)",
            score.cols_lit,
            WIDTH,
            BAR_LINE_SHARE * 100.0
        ));
    }
    if !jumps && score.seam > BAR_SEAM {
        out.push(format!(
            "the loop jumps by {:.1} % where it restarts; the bar is {:.0} % (or set loop_jump_on_purpose in the recipe, with the reason)",
            score.seam * 100.0,
            BAR_SEAM * 100.0
        ));
    }
    out
}

/// The synthetic capture data handed to an effect, for the inputs it asks for.
///
/// The data moves, once round per loop: the picture or the spectrum at `time` equals the one
/// at `time + loop_s`, so the tile of an effect that follows its input closes its loop and is
/// not a still frame.
struct Synthetic {
    screen: bool,
    audio: bool,
    loop_s: f64,
}

impl Synthetic {
    fn inputs(&self, time: f64) -> Inputs {
        let phase = (time / self.loop_s).fract();
        Inputs {
            time,
            screen: self.screen.then(|| Arc::new(gradient_screen(phase))),
            audio: self.audio.then(|| Arc::new(moving_spectrum(phase))),
        }
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
/// The picture slides sideways by one whole width over a loop (`phase` 0..1), so it closes.
fn gradient_screen(phase: f64) -> ScreenFrame {
    let (w, h) = (RENDER_WIDTH, RENDER_HEIGHT);
    let tau = std::f64::consts::TAU;
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let g = (y * 255 / (h - 1)) as u8;
        for x in 0..w {
            let along = x as f64 / w as f64 + phase;
            pixels.push(Rgb8 {
                r: (127.5 + 127.5 * (tau * along).sin()) as u8,
                g,
                b: (127.5 + 127.5 * (tau * (along + 0.33)).sin()) as u8,
            });
        }
    }
    ScreenFrame { width: w, height: h, pixels }
}

/// The sloped spectrum with a wave running up through its bands once per loop (`phase` 0..1).
fn moving_spectrum(phase: f64) -> AudioSpectrum {
    let mut spectrum = sloped_spectrum();
    let bands = spectrum.magnitudes.len() as f64;
    for (i, magnitude) in spectrum.magnitudes.iter_mut().enumerate() {
        let wave = (std::f64::consts::TAU * (phase + 2.0 * i as f64 / bands)).sin();
        *magnitude *= (0.7 + 0.3 * wave) as f32;
    }
    spectrum
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
pub struct ThumbnailCache {
    inner: Mutex<CacheState>,
    /// How many renders may run at once. The library asks for every tile together, and each
    /// render can take seconds of a core.
    gate: tokio::sync::Semaphore,
}

/// Thumbnails rendered at the same time, at most.
pub const CONCURRENT_RENDERS: usize = 2;

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self {
            inner: Mutex::default(),
            gate: tokio::sync::Semaphore::new(CONCURRENT_RENDERS),
        }
    }
}

#[derive(Default)]
struct CacheState {
    generation: u64,
    entries: HashMap<String, Arc<Vec<u8>>>,
}

impl ThumbnailCache {
    /// Waits for one of the [`CONCURRENT_RENDERS`] render slots. Drop the permit when the
    /// render is done.
    pub async fn render_slot(&self) -> tokio::sync::SemaphorePermit<'_> {
        // The semaphore is never closed.
        self.gate.acquire().await.expect("the render gate is never closed")
    }

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
        recipe: Option<ThumbnailRecipe>,
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
                thumbnail: self.recipe.clone(),
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

    /// A tile of one flat colour, scored.
    fn score_of_a_flat_colour(rgb: [u8; 3]) -> TileScore {
        let mut body = Vec::new();
        for v in [8u16, 8, 1] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        for _ in 0..8 * 8 {
            body.extend_from_slice(&rgb);
        }
        score(&body).expect("a well formed tile")
    }

    #[test]
    fn a_dim_grey_is_not_lit_and_a_vivid_colour_is() {
        // A near-black grey adds up to 48, so the old "R + G + B above 45" rule counted it
        // as lit.
        let dim = score_of_a_flat_colour([16, 16, 16]);
        assert_eq!(dim.lit_avg, 0.0, "a near-black grey is lit");
        // A dim grey is not lit either.
        assert_eq!(score_of_a_flat_colour([30, 30, 30]).lit_avg, 0.0, "a dim grey is lit");
        // A mid grey is light enough.
        assert_eq!(score_of_a_flat_colour([45, 45, 45]).lit_avg, 1.0, "a mid grey is not lit");
        // A vivid violet, the Solid effect's default, is lit: lightness weighs the channels
        // the way the eye does, so the blue and the red count.
        assert_eq!(score_of_a_flat_colour([96, 0, 252]).lit_avg, 1.0, "a violet is not lit");
        // A vivid blue too.
        assert_eq!(score_of_a_flat_colour([0, 0, 255]).lit_avg, 1.0, "a blue is not lit");
        // A lit grey.
        assert_eq!(score_of_a_flat_colour([90, 90, 90]).lit_avg, 1.0, "a light grey is not lit");
    }

    #[test]
    fn the_lightness_of_a_pixel_is_its_perceptual_light() {
        assert!(lightness(&[0, 0, 0]) < 0.001);
        assert!((lightness(&[255, 255, 255]) - 100.0).abs() < 0.01);
        // A dim grey is well under the bar; a vivid colour is over it.
        assert!(lightness(&[30, 30, 30]) < LIT_LIGHTNESS);
        assert!(lightness(&[96, 0, 252]) >= LIT_LIGHTNESS);
    }

    #[test]
    fn the_header_and_length_match_the_format() {
        let updates = Arc::new(AtomicU32::new(0));
        let mut effect = Counter { updates: updates.clone(), recipe: None };
        let bytes = render(&mut effect, &Props::new()).expect("renders");

        assert_eq!(u16::from_le_bytes([bytes[0], bytes[1]]), WIDTH);
        assert_eq!(u16::from_le_bytes([bytes[2], bytes[3]]), HEIGHT);
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), FRAMES);
        let expected = 6 + FRAMES as usize * WIDTH as usize * HEIGHT as usize * 3;
        assert_eq!(bytes.len(), expected);
        // One warm-up second in 15 steps, then two steps for each recorded frame.
        assert_eq!(updates.load(Ordering::SeqCst), 15 + 2 * FRAMES as u32);
    }

    #[test]
    fn a_recipe_sets_the_warm_up_and_the_loop() {
        let updates = Arc::new(AtomicU32::new(0));
        let recipe = ThumbnailRecipe { warm_up_s: 3.0, loop_s: 4.0, ..Default::default() };
        let mut effect = Counter { updates: updates.clone(), recipe: Some(recipe) };
        let bytes = render(&mut effect, &Props::new()).expect("renders");
        assert_eq!(bytes.len(), 6 + FRAMES as usize * WIDTH as usize * HEIGHT as usize * 3);
        // 3 s of warm-up is 45 steps; a 4 s loop is 4 / 24 s a frame, 3 steps each.
        assert_eq!(updates.load(Ordering::SeqCst), 45 + 3 * FRAMES as u32);
    }

    #[test]
    fn a_crossfade_renders_extra_frames_and_keeps_the_frame_count() {
        let updates = Arc::new(AtomicU32::new(0));
        let recipe = ThumbnailRecipe { crossfade_frames: 6, ..Default::default() };
        let mut effect = Counter { updates: updates.clone(), recipe: Some(recipe) };
        let bytes = render(&mut effect, &Props::new()).expect("renders");
        assert_eq!(bytes.len(), 6 + FRAMES as usize * WIDTH as usize * HEIGHT as usize * 3);
        assert_eq!(updates.load(Ordering::SeqCst), 15 + 2 * (FRAMES as u32 + 6));
    }

    #[test]
    fn a_crossfade_leads_the_last_frame_into_the_first() {
        // Frames 0..=5 are the loop (values 0, 10, ...), frames 6 and 7 carry on past its end.
        let tiles: Vec<Vec<u8>> = (0..8u8).map(|i| vec![i * 10]).collect();
        let faded = crossfade(tiles, 2);
        assert_eq!(faded.len(), 6);
        // The last frame is untouched; the first blends the frame after it (60) with its own (0).
        assert_eq!(faded[5], vec![50]);
        assert_eq!(faded[0], vec![40]); // 60 * 2/3 + 0 * 1/3
        assert_eq!(faded[1], vec![30]); // 70 * 1/3 + 10 * 2/3
        assert_eq!(faded[2], vec![20]);
    }

    #[test]
    fn a_recipe_with_nonsense_numbers_is_held_inside_the_limits() {
        let recipe = sanitize(ThumbnailRecipe {
            warm_up_s: f64::NAN,
            loop_s: 1e9,
            crossfade_frames: 999,
            ..Default::default()
        });
        assert_eq!(recipe.warm_up_s, 1.0);
        assert_eq!(recipe.loop_s, MAX_RECIPE_SECONDS);
        assert_eq!(recipe.crossfade_frames, FRAMES as u32 / 2);
    }

    #[test]
    fn the_score_reads_the_tile_the_bar_measures() {
        // Two 2x1 frames: the first has one lit pixel, the second two; the seam is the
        // difference between them. The pixels are ones the bar counts as lit (a dark red
        // and a dark green): the bar counts a pixel by its lightness, so the channel a
        // colour sits in does not decide it.
        let mut body = Vec::new();
        for v in [2u16, 1, 2] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&[100, 0, 0, 0, 0, 0]);
        body.extend_from_slice(&[100, 0, 0, 0, 50, 0]);
        let s = score(&body).expect("well formed");
        assert_eq!((s.lit_first, s.lit_last, s.lit_avg), (0.5, 1.0, 0.75));
        assert_eq!((s.rows_lit, s.cols_lit), (1, 2));
        assert!((s.seam - 50.0 / (6.0 * 255.0)).abs() < 1e-9);
        assert!(score(&body[..body.len() - 1]).is_none());
    }

    #[test]
    fn the_seam_bar_is_strict_and_only_a_stated_reason_lifts_it() {
        let lit = TileScore {
            lit_first: 1.0,
            lit_last: 1.0,
            lit_avg: 1.0,
            rows_lit: HEIGHT as usize,
            cols_lit: WIDTH as usize,
            seam: 0.09,
            step: 0.01,
            max_step: 0.02,
        };
        assert!(problems(&lit, None).is_empty());
        // 30 % over a tile that normally moves 25 % a frame is still a pop.
        let flicker = TileScore { seam: 0.30, step: 0.25, ..lit };
        assert_eq!(problems(&flicker, None).len(), 1);
        let on_purpose = ThumbnailRecipe { loop_jump_on_purpose: Some("a strobe".into()), ..Default::default() };
        assert!(problems(&flicker, Some(&on_purpose)).is_empty());
        // The reason for a jump does not lift the lit bar.
        let dim = TileScore { lit_avg: 0.1, ..flicker };
        assert_eq!(problems(&dim, Some(&on_purpose)).len(), 1);
    }

    /// Alternates a bright frame and a black one, so the loop's end meets its start with a jump.
    struct Strobe {
        on: bool,
    }

    impl Effect for Strobe {
        fn schema(&self) -> PropertySchema {
            PropertySchema {
                id: "strobe".into(),
                name: "Strobe".into(),
                description: String::new(),
                needs: Default::default(),
                properties: Vec::new(),
                presets: Vec::new(),
                // One update a frame (1/15 s each), so the strobe changes every frame.
                thumbnail: Some(ThumbnailRecipe { warm_up_s: 0.0, loop_s: 1.6, ..Default::default() }),
            }
        }
        fn init(&mut self, _props: &Props) {}
        fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
            self.on = !self.on;
            let v = if self.on { 1.0 } else { 0.0 };
            canvas.pixels.fill([v, v, v]);
        }
    }

    #[test]
    fn a_flickering_tile_fails_the_strict_seam() {
        let mut effect = Strobe { on: false };
        let body = render(&mut effect, &Props::new()).expect("renders");
        let s = score(&body).expect("well formed");
        assert!(s.seam > BAR_SEAM, "seam {}", s.seam);
        assert!(problems(&s, None).iter().any(|p| p.contains("loop jumps")));
    }

    #[test]
    fn one_lit_pixel_does_not_make_a_row_or_column_lit() {
        // 8x8, one frame. A row needs 25 % of its 8 pixels, which is 2.
        let header = |frames: u16| {
            let mut body = Vec::new();
            for v in [8u16, 8, frames] {
                body.extend_from_slice(&v.to_le_bytes());
            }
            body
        };
        let mut one = header(1);
        let mut frame = vec![0u8; 8 * 8 * 3];
        frame[0] = 255; // pixel (0, 0)
        one.extend_from_slice(&frame);
        let s = score(&one).expect("well formed");
        assert_eq!((s.rows_lit, s.cols_lit), (0, 0));

        // A second lit pixel in row 0 (x = 1) makes row 0 lit; the columns still hold one each.
        frame[3] = 255;
        let mut two = header(1);
        two.extend_from_slice(&frame);
        let s = score(&two).expect("well formed");
        assert_eq!((s.rows_lit, s.cols_lit), (1, 0));
    }

    #[test]
    fn a_render_that_runs_past_its_budget_is_stopped() {
        let updates = Arc::new(AtomicU32::new(0));
        let mut effect = Counter { updates: updates.clone(), recipe: None };
        let error = render_within(&mut effect, &Props::new(), std::time::Duration::ZERO)
            .expect_err("a zero budget stops the render");
        assert!(error.contains("budget"), "{error}");
        // It stopped at the first update, not after the whole recipe.
        assert_eq!(updates.load(Ordering::SeqCst), 1);
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

    #[tokio::test]
    async fn only_two_renders_run_at_once() {
        let cache = ThumbnailCache::default();
        let mut held = Vec::new();
        for _ in 0..2 {
            held.push(cache.render_slot().await);
        }
        // A third has to wait until a slot is given back.
        let waiting = tokio::time::timeout(std::time::Duration::from_millis(100), cache.render_slot()).await;
        assert!(waiting.is_err(), "a render slot was free beyond the limit");
        held.pop();
        let freed = tokio::time::timeout(std::time::Duration::from_millis(500), cache.render_slot()).await;
        assert!(freed.is_ok(), "a released slot is not handed on");
    }

    #[test]
    fn the_synthetic_inputs_move_once_round_per_loop() {
        let s = Synthetic { screen: true, audio: true, loop_s: 2.0 };
        let at = |t: f64| s.inputs(t);
        let (a, b, c) = (at(0.5), at(1.0), at(2.5));
        let spectrum = |i: &Inputs| i.audio.as_ref().expect("audio").magnitudes.clone();
        assert_ne!(spectrum(&a), spectrum(&b), "the spectrum moves");
        for (x, y) in spectrum(&a).iter().zip(spectrum(&c)) {
            assert!((x - y).abs() < 1e-4, "the spectrum is the same one loop later");
        }
        let screen = |i: &Inputs| i.screen.as_ref().expect("screen").pixels.clone();
        assert_ne!(screen(&a), screen(&b), "the screen moves");
        assert_eq!(screen(&a), screen(&c), "the screen is the same one loop later");
    }

    #[test]
    fn the_spectrum_slopes_down_and_spans_the_bands() {
        let spectrum = sloped_spectrum();
        assert_eq!(spectrum.centers_hz.len(), SPECTRUM_BANDS);
        assert!(spectrum.centers_hz.windows(2).all(|w| w[0] < w[1]));
        assert!(spectrum.magnitudes.windows(2).all(|w| w[0] > w[1]));
    }
}
