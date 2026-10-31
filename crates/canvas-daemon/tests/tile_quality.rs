//! The tile quality bar, over every built-in and stock effect (issue #16).
//!
//! Headless: no daemon, no device, no hardware. Each effect is rendered with the same
//! `thumbnail::render` the API serves, scored with `thumbnail::score` and held to
//! `thumbnail::problems`. A tile under the bar fails here unless its recipe says it is
//! dark on purpose (and that exemption covers the lit share only).
//!
//! `cargo test -p canvas-daemon --test tile_quality -- --nocapture` prints the table.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::thumbnail::{self, TileScore};
use std::sync::Arc;
use tower::ServiceExt;
use canvas_daemon::user_effects::{html_builder_seeded, STOCK_EFFECTS};
use canvas_proto::props::{Props, ThumbnailRecipe};
use canvas_proto::traits::{Canvas, Effect, Inputs};
use canvas_proto::EFFECT_IDS;

/// Every effect the library ships: the built-ins, then the stock HTML effects.
fn shipped() -> Vec<(String, Box<dyn Effect>)> {
    let mut out: Vec<(String, Box<dyn Effect>)> = Vec::new();
    for id in EFFECT_IDS {
        out.push((id.to_string(), canvas_effects::create(id).expect("a built-in effect builds")));
    }
    // The stock HTML effects are built with the tile seed, so a tile is the same picture
    // every time it is rendered.
    let build = html_builder_seeded(thumbnail::TILE_SEED);
    for (stem, source) in STOCK_EFFECTS {
        let id = format!("html:{stem}");
        out.push((id.clone(), build(&id, source).expect("a stock effect builds")));
    }
    out
}

fn measure(effect: &mut dyn Effect) -> (TileScore, Option<ThumbnailRecipe>) {
    let schema = effect.schema();
    // The budget the daemon uses is for release builds; a debug build of a paint effect is slower.
    let body = thumbnail::render_within(effect, &Props::new(), std::time::Duration::from_secs(600))
        .unwrap_or_else(|why| panic!("the tile of {} was given up: {why}", schema.id));
    (thumbnail::score(&body).expect("the render is well formed"), schema.thumbnail)
}

fn print_table(rows: &[(String, TileScore, Option<ThumbnailRecipe>)]) {
    println!(
        "{:<18} {:>14} {:>8} {:>10} {:>10} {:>7} {:>7} {:>7} {:>5}  recipe",
        "effect", "lit first>last", "lit avg", "rows lit", "cols lit", "seam", "step", "max", "band"
    );
    for (id, s, recipe) in rows {
        let note = match recipe {
            None => "default".to_string(),
            Some(r) => format!(
                "warm {} s, loop {} s, fade {}{}",
                r.warm_up_s,
                r.loop_s,
                r.crossfade_frames,
                r.dark_on_purpose.as_deref().map(|d| format!(", dark: {d}")).unwrap_or_default()
            ),
        };
        println!(
            "{:<18} {:>5.0} > {:>4.0} % {:>7.0} % {:>5} / {} {:>5} / {} {:>6.1}% {:>6.1}% {:>6.1}% {:>5}  {}",
            id,
            s.lit_first * 100.0,
            s.lit_last * 100.0,
            s.lit_avg * 100.0,
            s.rows_lit,
            thumbnail::HEIGHT,
            s.cols_lit,
            thumbnail::WIDTH,
            s.seam * 100.0,
            s.step * 100.0,
            s.max_step * 100.0,
            s.band_rows,
            note
        );
    }
}

/// Holds the built-in effects (`stock` false) or the stock HTML effects (`stock` true) to the bar.
fn hold_to_the_bar(stock: bool) {
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (id, mut effect) in shipped().into_iter().filter(|(id, _)| id.starts_with("html:") == stock) {
        let (score, recipe) = measure(effect.as_mut());
        for problem in thumbnail::problems(&score, recipe.as_ref()) {
            failures.push(format!("{id}: {problem}"));
        }
        rows.push((id, score, recipe));
    }
    assert!(!rows.is_empty(), "no effects were measured");
    print_table(&rows);
    assert!(failures.is_empty(), "tiles under the bar:\n{}", failures.join("\n"));
}

#[test]
fn every_built_in_tile_passes_the_bar_or_says_why_not() {
    hold_to_the_bar(false);
}

/// The paint effects take minutes to render in a debug build; CI runs this one with
/// `--release --include-ignored`.
#[test]
#[cfg_attr(debug_assertions, ignore = "the paint effects take minutes to render in a debug build; CI runs these with --release")]
fn every_stock_tile_passes_the_bar_or_says_why_not() {
    hold_to_the_bar(true);
}

#[test]
fn the_test_covers_the_fourteen_shipped_effects_by_name() {
    let ids: Vec<String> = shipped().into_iter().map(|(id, _)| id).collect();
    let expected = [
        "solid", "gradient", "rainbow_wave", "spiral", "breathing", "color_cycle", "aurora", "rain", "fire",
        "screen_ambience", "audio_spectrum", "html:ink-drop", "html:wet-smear", "html:palette-knife",
    ];
    // A new effect that is not in this list is not yet held to the bar: add it here.
    assert_eq!(ids, expected);
}

/// A tile that is a still picture reads as a broken tile for an effect that follows motion
/// (a spectrum, a screen, a flame): each of these has to move, and none may cut hard.
#[test]
fn the_tiles_of_moving_effects_move_and_do_not_cut() {
    let mut failures = Vec::new();
    for (id, mut effect) in shipped() {
        let wants_motion = matches!(id.as_str(), "audio_spectrum" | "screen_ambience" | "fire" | "breathing");
        if id.starts_with("html:") && !cfg!(not(debug_assertions)) {
            continue; // the paint effects are measured in release
        }
        let (score, _) = measure(effect.as_mut());
        if wants_motion && score.step < 0.01 {
            failures.push(format!("{id}: the tile barely moves (step {:.2} %)", score.step * 100.0));
        }
        if score.max_step > 0.25 {
            failures.push(format!("{id}: the tile cuts hard between two frames ({:.0} %)", score.max_step * 100.0));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every tile may not cut hard inside its loop, the ones the motion test skips included.
///
/// The motion test above only reads the tiles that follow motion. The bar's hard-cut term
/// reads every tile, so the seven built-ins it skips are held to it here too.
#[test]
fn no_tile_cuts_hard_inside_its_loop() {
    let mut failures = Vec::new();
    for (id, mut effect) in shipped() {
        if id.starts_with("html:") && !cfg!(not(debug_assertions)) {
            continue; // the paint effects are measured in release
        }
        let (score, recipe) = measure(effect.as_mut());
        for problem in thumbnail::problems(&score, recipe.as_ref()) {
            if problem.contains("must not cut hard") {
                failures.push(format!("{id}: {problem}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_breathing_tile_is_not_the_solid_tile() {
    // Breathing's tile must show the pulse, not stand still like Solid. Its first frame is
    // half way through the swell, so it is dimmer than Solid's flat colour.
    let mut breathing = canvas_effects::create("breathing").expect("Breathing builds");
    let mut solid = canvas_effects::create("solid").expect("Solid builds");
    let body = |effect: &mut Box<dyn Effect>| {
        thumbnail::render_within(effect.as_mut(), &Props::new(), std::time::Duration::from_secs(60))
            .expect("the render fits the test budget")
    };
    let first_mean = |body: &[u8]| -> f64 {
        let w = u16::from_le_bytes([body[0], body[1]]) as usize;
        let h = u16::from_le_bytes([body[2], body[3]]) as usize;
        let frame = &body[6..6 + w * h * 3];
        frame.iter().map(|v| *v as f64).sum::<f64>() / frame.len() as f64
    };
    let breathing_mean = first_mean(&body(&mut breathing));
    let solid_mean = first_mean(&body(&mut solid));
    assert!(
        breathing_mean < solid_mean - 5.0,
        "the breathing tile's first frame is as bright as Solid's ({breathing_mean} against {solid_mean})"
    );
}

/// The breathing tile must show the pulse, not just be a colour: the trough has to be well
/// below the peak, or the tile is a flat block that happens to be dim.
#[test]
fn the_breathing_tile_pulses() {
    let mut effect = canvas_effects::create("breathing").expect("Breathing builds");
    let body = thumbnail::render_within(effect.as_mut(), &Props::new(), std::time::Duration::from_secs(60))
        .expect("the render fits the test budget");
    let w = u16::from_le_bytes([body[0], body[1]]) as usize;
    let h = u16::from_le_bytes([body[2], body[3]]) as usize;
    let frames: Vec<&[u8]> = body[6..].chunks_exact(w * h * 3).collect();
    let brightness = |frame: &[u8]| -> f64 {
        frame.iter().map(|v| *v as f64).sum::<f64>() / frame.len() as f64
    };
    let peak = frames.iter().map(|f| brightness(f)).fold(f64::MIN, f64::max);
    let trough = frames.iter().map(|f| brightness(f)).fold(f64::MAX, f64::min);
    assert!(
        peak > 0.0 && trough < peak * 0.6,
        "the breathing tile does not pulse: the peak is {peak} and the trough is {trough}"
    );
    // The trough is well below the peak: a fifth of it or less.
    assert!(
        trough <= peak * 0.4,
        "the breathing tile's trough ({trough}) is not well below its peak ({peak})"
    );
}

#[test]
fn a_dark_tile_without_a_stated_reason_is_a_problem_and_with_one_it_is_not() {
    let dark = TileScore {
        lit_first: 0.1,
        lit_last: 0.1,
        lit_avg: 0.1,
        rows_lit: thumbnail::HEIGHT as usize,
        cols_lit: thumbnail::WIDTH as usize,
        seam: 0.0,
        step: 0.0,
        max_step: 0.0,
        band_rows: 0,
    };
    assert_eq!(thumbnail::problems(&dark, None).len(), 1);
    let on_purpose = ThumbnailRecipe { dark_on_purpose: Some("a spark on black".into()), ..Default::default() };
    assert!(thumbnail::problems(&dark, Some(&on_purpose)).is_empty());

    // The exemption never covers the seam or the coverage.
    let popping = TileScore { seam: 0.5, rows_lit: 8, ..dark };
    assert_eq!(thumbnail::problems(&popping, Some(&on_purpose)).len(), 2);
}

/// The tile of an effect that draws from the clock is the same picture every time.
///
/// Fire and Rain seed themselves from the entropy pool, so without the tile's fixed seed
/// every render is a different fire and a different rain, and the bar is a lottery.
#[test]
fn the_tile_of_a_seeded_effect_is_the_same_every_time() {
    for id in ["fire", "rain"] {
        let first = {
            let mut effect = canvas_effects::create(id).expect("the effect builds");
            measure(effect.as_mut()).0
        };
        for round in 1..10 {
            let mut effect = canvas_effects::create(id).expect("the effect builds");
            let score = measure(effect.as_mut()).0;
            assert_eq!(score, first, "{id}: render {round} differs from the first");
        }
    }
}

/// The same, for the stock HTML effects. They are slower, so this runs in release.
#[test]
#[cfg_attr(debug_assertions, ignore = "the paint effects take minutes to render in a debug build; CI runs these with --release")]
fn the_tiles_of_the_stock_effects_are_the_same_every_time() {
    for id in ["html:ink-drop", "html:wet-smear", "html:palette-knife"] {
        let build = html_builder_seeded(thumbnail::TILE_SEED);
        let source = STOCK_EFFECTS
            .iter()
            .find(|(stem, _)| format!("html:{stem}") == id)
            .map(|(_, source)| *source)
            .expect("a stock effect");
        let first = {
            let mut effect = build(id, source).expect("the effect builds");
            measure(effect.as_mut()).0
        };
        for round in 1..3 {
            let mut effect = build(id, source).expect("the effect builds");
            let score = measure(effect.as_mut()).0;
            assert_eq!(score, first, "{id}: render {round} differs from the first");
        }
    }
}

/// The Aurora's flow has no period a tile can close on, so the tile's end is crossfaded into
/// its start. The two pictures the crossfade blends are a whole drift apart, and the blend
/// shows both: a doubled ring beside the eye.
///
/// The drift has to stay small for that: the bar's 10 % is for a tile that cuts hard, and
/// the aurora is not that, but its loop is a crossfade of two pictures, and a fast flow
/// shows the pattern twice. The old recipe (speed 8 over a two second loop) drifted 1.6 rad
/// per loop and doubled the ring in frames 1 to 4.
#[test]
fn the_aurora_tile_drifts_slowly_enough_to_close_its_loop() {
    let mut effect = canvas_effects::create("aurora").expect("Aurora builds");
    let (score, _) = measure(effect.as_mut());
    assert!(
        score.max_step <= 0.04,
        "the aurora tile moves {:.1} % between two frames; its loop is a crossfade of two pictures, so a fast flow shows the pattern twice",
        score.max_step * 100.0
    );
    assert!(
        score.seam <= 0.04,
        "the aurora tile jumps {:.1} % where the loop restarts",
        score.seam * 100.0
    );
}

/// The Rain tile must be bright: a dim rain reads as a broken tile, even when it passes the
/// general lit bar. The old recipe (density 300, trail 24) was 63 % lit.
#[test]
fn the_rain_tile_is_bright() {
    let mut effect = canvas_effects::create("rain").expect("Rain builds");
    let (score, _) = measure(effect.as_mut());
    assert!(
        score.lit_avg >= 0.75,
        "the rain tile is only {:.0} % lit; a dim rain reads as a broken tile",
        score.lit_avg * 100.0
    );
}

/// The stock wet-smear file.
fn wet_smear_source() -> &'static str {
    STOCK_EFFECTS
        .iter()
        .find(|(stem, _)| *stem == "wet-smear")
        .map(|(_, source)| *source)
        .expect("a stock effect")
}

/// The Wet Smear tile must be bright: the smear setting that blends the paint into mud
/// takes the tile from 94 % lit to 70 %, and a muddy tile reads as a broken one.
///
/// The effect is built directly, in tile mode, so a failure can print what the effect
/// reported (`errors()`): a frame that was dropped or that threw shows there, and the number
/// alone would not say why the tile is dim.
#[test]
fn the_wet_smear_tile_is_bright() {
    let mut effect = canvas_html::HtmlEffect::from_source_seeded(
        "html:wet-smear",
        wet_smear_source(),
        &Props::new(),
        thumbnail::TILE_SEED,
    )
    .expect("the effect builds")
    .tile_mode();
    let (score, _) = measure(&mut effect);
    assert!(
        score.lit_avg >= 0.85,
        "the wet smear tile is only {:.0} % lit; the effect reported {:?}",
        score.lit_avg * 100.0,
        effect.errors()
    );
    assert!(effect.errors().is_empty(), "the wet smear tile reported errors: {:?}", effect.errors());
}

/// The Wet Smear tile must not carry a half-pixel checkerboard.
///
/// The effect's picture is a grid that is upsampled onto its canvas. A plain bilinear
/// upsample leaves the weight between two cells alternating from pixel to pixel, so every
/// soft edge comes out as a row of alternating pixels: a checkerboard over the whole tile.
/// The tile is 64x40 and the canvas 80x50, so the checkerboard is measured on the canvas,
/// where it is a pixel wide.
#[test]
fn the_wet_smear_tile_has_no_checkerboard() {
    let mut effect = canvas_html::HtmlEffect::from_source_seeded(
        "html:wet-smear",
        wet_smear_source(),
        &Props::new(),
        thumbnail::TILE_SEED,
    )
    .expect("the effect builds")
    .tile_mode();
    let schema = effect.schema();
    // The effect's own canvas, so the picture is not scaled on the way in.
    let (w, h) = (80u32, 50u32);
    let mut canvas = Canvas::new(w, h);
    effect.init(&schema.defaults());
    for _ in 0..120 {
        effect.update(1.0 / 60.0, &mut canvas, &Inputs::default());
    }
    // The share of the pixels whose colour alternates from one pixel to the next: on a
    // smooth picture the step to the next pixel is half the step over two, and on a
    // checkerboard it is the whole step. The old bilinear upsample left 2 % of the canvas
    // alternating; the B-spline leaves 1 %.
    let at = |x: u32, y: u32| canvas.get(x, y).iter().map(|v| *v as f64).sum::<f64>();
    let mut alternating = 0usize;
    let mut total = 0usize;
    for y in 0..h {
        for x in 0..w - 2 {
            let near = (at(x + 1, y) - at(x, y)).abs();
            let far = (at(x, y) - at(x + 2, y)).abs();
            if near > far {
                alternating += 1;
            }
            total += 1;
        }
    }
    assert!(effect.errors().is_empty(), "the wet smear reported errors: {:?}", effect.errors());
    let fraction = alternating as f64 / total as f64;
    assert!(
        fraction < 0.015,
        "{} % of the tile's pixels alternate from one to the next: the picture carries a half-pixel checkerboard",
        (fraction * 100.0 * 10.0).round() / 10.0
    );
}

// ---------------------------------------------------------------- what ships

/// The token the API test uses.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Asks the daemon's own API (the router, the real effects and the real stock effects, the
/// fakes only under the engine and the device) for the tile of every id, the way the library does.
async fn tiles_served_by_the_api(ids: &[String]) -> Vec<(String, Vec<u8>)> {
    let home = std::env::temp_dir().join(format!("canvas-rgb-tiles-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("a temp folder");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let app = Arc::new(canvas_daemon::state::App::new(
        canvas_daemon::storage::Store::at(&home),
        Arc::new(common::FakeEngine::default()),
        Box::new(common::FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    ));
    let router = canvas_daemon::api::router(app);
    let mut out = Vec::new();
    for id in ids {
        let request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/effects/{id}/thumbnail"))
            .header("host", "127.0.0.1:9430")
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .expect("the request builds");
        let response = router.clone().oneshot(request).await.expect("the router answers");
        assert_eq!(response.status(), StatusCode::OK, "the API did not serve the tile of {id}");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("the body");
        out.push((id.clone(), bytes.to_vec()));
    }
    let _ = std::fs::remove_dir_all(&home);
    out
}

/// The recipe an effect ships with, by id.
fn recipe_of(id: &str) -> Option<ThumbnailRecipe> {
    match STOCK_EFFECTS.iter().find(|(stem, _)| format!("html:{stem}") == id) {
        Some((_, source)) => {
            let build = html_builder_seeded(thumbnail::TILE_SEED);
            build(id, source).expect("a stock effect builds").schema().thumbnail
        }
        None => canvas_effects::create(id).expect("a built-in effect builds").schema().thumbnail,
    }
}

/// Holds the tiles the API serves to the bar. These are the bytes the library shows: the
/// daemon's tile for each effect at its defaults, scored the same way as everything else.
async fn served_tiles_hold_to_the_bar(stock: bool) {
    let ids: Vec<String> = shipped()
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| id.starts_with("html:") == stock)
        .collect();
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (id, body) in tiles_served_by_the_api(&ids).await {
        let score = thumbnail::score(&body).expect("the served tile is well formed");
        let recipe = recipe_of(&id);
        for problem in thumbnail::problems(&score, recipe.as_ref()) {
            failures.push(format!("{id}: {problem}"));
        }
        rows.push((id, score, recipe));
    }
    println!("what the daemon serves:");
    print_table(&rows);
    assert!(failures.is_empty(), "tiles the API serves, under the bar:
{}", failures.join("
"));
}

#[tokio::test]
async fn the_tiles_the_daemon_serves_for_the_built_in_effects_pass_the_bar() {
    served_tiles_hold_to_the_bar(false).await;
}

#[tokio::test]
#[cfg_attr(debug_assertions, ignore = "the paint effects take minutes to render in a debug build; CI runs these with --release")]
async fn the_tiles_the_daemon_serves_for_the_stock_effects_pass_the_bar() {
    served_tiles_hold_to_the_bar(true).await;
}

/// The frame strips the editor's demo serves (`app/web/public/demo-thumbs`) are the tiles the
/// static demo shows, so they are held to the same bar, measured on the PNG files themselves
/// (what ships), not on a separate preview render.
#[test]
fn the_demo_strips_the_editor_ships_pass_the_bar() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/web/public/demo-thumbs");
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for id in canvas_proto::EFFECT_IDS {
        let path = dir.join(format!("{id}.png"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("the demo strip {} is missing: {e}", path.display()));
        let body = thumbnail::body_from_strip_png(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let score = thumbnail::score(&body).expect("the strip is well formed");
        let recipe = recipe_of(id);
        for problem in thumbnail::problems(&score, recipe.as_ref()) {
            failures.push(format!("{id}: {problem}"));
        }
        rows.push((id.to_string(), score, recipe));
    }
    println!("the demo strips, measured on the PNG files:");
    print_table(&rows);
    assert!(failures.is_empty(), "demo strips under the bar:
{}", failures.join("
"));
}

/// How much one set of rows of a picture disagrees with its neighbours, as a ratio.
///
/// The Wet Smear draws a quarter of its rows per frame (rows Y with Y % 4 = the frame's part).
/// The library tile reads the picture after four frames, a whole cycle of parts. If the four parts
/// read the live grid, row Y % 4 = 0 comes from the first frame and row Y % 4 = 3 from the last,
/// so a moving edge is displaced by three frames between a row 0 and the row above it (a row 3),
/// and by one frame between every other pair of neighbours. That row pair differs more than the
/// others, and this is how much more. If the four parts read one snapshot, every pair of rows is the
/// same moment and the ratio is about 1.
fn row_phase_ratio(canvas: &Canvas) -> f64 {
    let (w, h) = (canvas.width, canvas.height);
    let at = |x: u32, y: u32| canvas.get(x, y).iter().map(|v| *v as f64).sum::<f64>();
    let mut by_phase = [0.0f64; 4];
    for y in 1..h {
        for x in 0..w {
            by_phase[(y % 4) as usize] += (at(x, y) - at(x, y - 1)).abs();
        }
    }
    by_phase[0] / ((by_phase[1] + by_phase[2] + by_phase[3]) / 3.0)
}

/// The Wet Smear at the speed the library tile runs it, on the effect's own canvas, read the way the
/// tile reads it (after steps of four 60 Hz frames): the row-phase ratio of each of 40 pictures.
fn wet_smear_row_phase_ratios(source: &str) -> Vec<f64> {
    let mut effect =
        canvas_html::HtmlEffect::from_source_seeded("html:wet-smear", source, &Props::new(), thumbnail::TILE_SEED)
            .expect("the effect builds")
            .tile_mode();
    let mut props = effect.schema().defaults();
    // The tile's own showcase speed.
    props.insert("speed".to_string(), canvas_proto::PropValue::Number(100.0));
    effect.init(&props);
    let mut canvas = Canvas::new(80, 50);
    let step = 1.0 / 15.0;
    for _ in 0..150 {
        effect.update(step, &mut canvas, &Inputs::default());
    }
    let mut ratios = Vec::new();
    for _ in 0..40 {
        effect.update(step, &mut canvas, &Inputs::default());
        ratios.push(row_phase_ratio(&canvas));
    }
    assert!(effect.errors().is_empty(), "the wet smear reported errors: {:?}", effect.errors());
    ratios
}

/// The Wet Smear's tile has no comb along its stroke edges: the four parts of its picture all read
/// one snapshot of the sim, so every row of a picture is the same moment. Reading the live grid
/// made the tile (which runs the sim at twice the default rate) a row of teeth on every moving edge:
/// the row-phase ratio was 1.6 on average and up to 2.4; with one snapshot it is 0.99 and at most 1.01.
#[test]
#[cfg_attr(debug_assertions, ignore = "the paint effects take minutes to render in a debug build; CI runs these with --release")]
fn the_wet_smear_tile_has_no_comb_along_its_edges() {
    let ratios = wet_smear_row_phase_ratios(wet_smear_source());
    let worst = ratios.iter().cloned().fold(0.0, f64::max);
    let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
    println!("row-phase ratio: mean {mean:.3}, worst {worst:.3} over {} pictures", ratios.len());
    assert!(
        worst < 1.1,
        "the wet smear's rows are different moments: the row-phase ratio reached {worst:.2} (mean {mean:.2}); one snapshot gives about 1.0"
    );
}

/// The step between each pair of consecutive frames of a tile body, as a share of full scale.
fn steps_of(body: &[u8]) -> Vec<f64> {
    let frame_len = thumbnail::WIDTH as usize * thumbnail::HEIGHT as usize * 3;
    let frames: Vec<&[u8]> = body[6..].chunks_exact(frame_len).collect();
    frames
        .windows(2)
        .map(|pair| {
            let sum: u64 = pair[0].iter().zip(pair[1]).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as u64).sum();
            sum as f64 / (frame_len as f64 * 255.0)
        })
        .collect()
}

/// A paint effect's tile moves through its whole loop. Palette Knife's old recipe held one
/// painting for two thirds of the loop (frame steps of 0.2 to 0.4 % from frame 8 on, 15 of 23
/// steps under half a percent); a tile that stands still that long reads as a frozen picture.
#[test]
#[cfg_attr(debug_assertions, ignore = "the paint effects take minutes to render in a debug build; CI runs these with --release")]
fn no_stock_tile_stands_still_for_a_third_of_its_loop() {
    let mut failures = Vec::new();
    for (stem, source) in STOCK_EFFECTS {
        let id = format!("html:{stem}");
        let mut effect = html_builder_seeded(thumbnail::TILE_SEED)(&id, source).expect("a stock effect builds");
        let body = thumbnail::render_within(effect.as_mut(), &Props::new(), std::time::Duration::from_secs(600))
            .unwrap_or_else(|why| panic!("the tile of {id} was given up: {why}"));
        let steps = steps_of(&body);
        let still = steps.iter().filter(|s| **s < 0.005).count();
        println!("{id}: {still} of {} steps under 0.5 %", steps.len());
        if still * 3 > steps.len() {
            failures.push(format!("{id}: {still} of {} frame steps are under 0.5 %: the tile stands still for most of its loop", steps.len()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("
"));
}
