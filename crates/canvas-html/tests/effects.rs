//! Integration tests for the HTML effect runtime.
//!
//! The fixtures are written for this repo: `lumen.html` is an effect in the SignalRGB
//! file format that exercises every engine feature the format needs (see
//! `fixtures/README.md` for the list), and `generic.html` is a small effect with a
//! gradient, a rectangle and an arc.

use canvas_html::HtmlEffect;
use canvas_proto::{Canvas, Effect, Inputs, PropKind, PropValue, Props};

const LUMEN: &str = include_str!("fixtures/lumen.html");
const GENERIC: &str = include_str!("fixtures/generic.html");

/// One frame at 60 fps.
const DT: f32 = 1.0 / 60.0;

/// Lumen's declared defaults, as its file states them.
const LUMEN_BG: &str = "#0b1b3a";
const LUMEN_TINTS: [&str; 3] = ["#19e6ff", "#7a5cff", "#ff5cc8"];
const LUMEN_SPEED: f64 = 12.0;

/// The background the fixture is compared with: the file's own default is overridden,
/// the tints are not.
const COMPARE_BG: &str = "#141a3d";
/// The speed the fixture is compared with.
const COMPARE_SPEED: f64 = 35.0;

/// The property values the comparison runs with.
fn compare_props() -> Props {
    let mut props = Props::new();
    props.insert(
        "background".to_string(),
        PropValue::Color(COMPARE_BG.parse().expect("a hex color")),
    );
    for (key, hex) in [
        ("tint1", LUMEN_TINTS[0]),
        ("tint2", LUMEN_TINTS[1]),
        ("tint3", LUMEN_TINTS[2]),
    ] {
        props.insert(
            key.to_string(),
            PropValue::Color(hex.parse().expect("a hex color")),
        );
    }
    props.insert("speed".to_string(), PropValue::Number(COMPARE_SPEED));
    props
}

/// The seed the thresholded tests run with, so their numbers reproduce.
const LUMEN_SEED: u32 = 0x9e37_79b9;

fn lumen() -> HtmlEffect {
    HtmlEffect::from_source_seeded("lumen", LUMEN, &Props::new(), LUMEN_SEED).expect("lumen loads")
}

#[test]
fn two_production_instances_of_lumen_draw_different_pictures() {
    // `from_source` seeds from entropy, so each activation gets its own sequence.
    let mut first = HtmlEffect::from_source("lumen", LUMEN, &Props::new()).expect("loads");
    let mut second = HtmlEffect::from_source("lumen", LUMEN, &Props::new()).expect("loads");
    let a = run(&mut first, 120);
    let b = run(&mut second, 120);
    assert!(a.pixels != b.pixels, "two instances drew the same random sequence");
}

#[test]
fn two_instances_with_the_same_seed_draw_identical_pictures() {
    let mut first = HtmlEffect::from_source_seeded("lumen", LUMEN, &Props::new(), 7).expect("loads");
    let mut second = HtmlEffect::from_source_seeded("lumen", LUMEN, &Props::new(), 7).expect("loads");
    let a = run(&mut first, 120);
    let b = run(&mut second, 120);
    assert!(a.pixels == b.pixels, "the same seed must reproduce the picture");
}

/// Runs the effect for `frames` frames on a fresh canvas and returns the canvas.
fn run(effect: &mut HtmlEffect, frames: usize) -> Canvas {
    let mut canvas = Canvas::new(320, 200);
    let inputs = Inputs::default();
    for _ in 0..frames {
        effect.update(DT, &mut canvas, &inputs);
    }
    canvas
}

/// The count of pixels that differ from the first pixel.
fn distinct_pixel_count(canvas: &Canvas) -> usize {
    let mut seen = std::collections::HashSet::new();
    for px in &canvas.pixels {
        seen.insert((
            (px[0] * 255.0).round() as u8,
            (px[1] * 255.0).round() as u8,
            (px[2] * 255.0).round() as u8,
        ));
    }
    seen.len()
}

/// The count of pixels within a small distance of a color.
fn pixels_near(canvas: &Canvas, target: [u8; 3], tolerance: i32) -> usize {
    canvas
        .pixels
        .iter()
        .filter(|px| {
            let r = (px[0] * 255.0).round() as i32;
            let g = (px[1] * 255.0).round() as i32;
            let b = (px[2] * 255.0).round() as i32;
            (r - target[0] as i32).abs() <= tolerance
                && (g - target[1] as i32).abs() <= tolerance
                && (b - target[2] as i32).abs() <= tolerance
        })
        .count()
}

// ---------------------------------------------------------------- schema

#[test]
fn lumen_schema_has_five_properties_with_the_declared_defaults() {
    let effect = lumen();
    let schema = effect.schema();

    assert_eq!(schema.id, "lumen");
    assert_eq!(schema.name, "Lumen");
    assert_eq!(schema.description, "Slow rings of light drifting across a dark canvas.");
    assert_eq!(schema.properties.len(), 5, "lumen declares five properties");

    let keys: Vec<&str> = schema.properties.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, vec!["background", "tint1", "tint2", "tint3", "speed"]);

    // The labels come from the file.
    assert_eq!(schema.properties[0].label, "Background");
    assert_eq!(schema.properties[4].label, "Speed");

    // The defaults come from the file too.
    let defaults = schema.defaults();
    assert_eq!(
        defaults.get("background"),
        Some(&PropValue::Color(LUMEN_BG.parse().expect("a hex color")))
    );
    for (key, hex) in [
        ("tint1", LUMEN_TINTS[0]),
        ("tint2", LUMEN_TINTS[1]),
        ("tint3", LUMEN_TINTS[2]),
    ] {
        assert_eq!(
            defaults.get(key),
            Some(&PropValue::Color(hex.parse().expect("a hex color"))),
            "{key} default"
        );
    }
    assert_eq!(defaults.get("speed"), Some(&PropValue::Number(LUMEN_SPEED)));

    // Every color property is a color; the speed is a number.
    assert!(matches!(
        schema.properties[0].kind,
        PropKind::Color { .. }
    ));
    assert!(matches!(
        schema.properties[4].kind,
        PropKind::Number { min, max, step, .. } if min == 0.0 && max == 100.0 && step == 1.0
    ));

    // HTML effects consume no screen or audio input in v1.
    assert!(!schema.needs.screen);
    assert!(!schema.needs.audio);
}

#[test]
fn default_overrides_replace_the_declared_defaults() {
    let effect = HtmlEffect::from_source("lumen", LUMEN, &compare_props()).expect("lumen loads");
    let defaults = effect.schema().defaults();
    assert_eq!(
        defaults.get("background"),
        Some(&PropValue::Color(COMPARE_BG.parse().expect("a hex color")))
    );
    assert_eq!(defaults.get("speed"), Some(&PropValue::Number(COMPARE_SPEED)));
    // The colors that were not overridden keep the file's defaults.
    assert_eq!(
        defaults.get("tint1"),
        Some(&PropValue::Color(LUMEN_TINTS[0].parse().expect("a hex color")))
    );
}

#[test]
fn a_file_without_a_script_is_a_parse_error() {
    let html = "<head><title>Empty</title></head><body></body>";
    match HtmlEffect::from_source("empty", html, &Props::new()) {
        Err(canvas_html::HtmlError::Parse(_)) => {}
        Err(other) => panic!("expected a parse error, got {other}"),
        Ok(_) => panic!("a file without a script must not load"),
    }
}

#[test]
fn a_script_that_throws_is_a_script_error() {
    let html = r#"<canvas id="c" width="10" height="10"></canvas><script>throw new Error("boom");</script>"#;
    match HtmlEffect::from_source("bad", html, &Props::new()) {
        Err(canvas_html::HtmlError::Script(_)) => {}
        Err(other) => panic!("expected a script error, got {other}"),
        Ok(_) => panic!("a script that throws must not load"),
    }
}

// ---------------------------------------------------------------- the deadline

/// An effect whose top-level script never ends.
const RUNAWAY_LOAD: &str = r##"<head><title>Loop</title></head>
<body><canvas id="c" width="16" height="16"></canvas></body>
<script>
  while (true) {}
</script>"##;

/// An effect that draws one good frame and then loops for ever.
const RUNAWAY_FRAME: &str = r##"<head><title>Loop</title></head>
<body><canvas id="c" width="16" height="16"></canvas></body>
<script>
  var ctx = document.getElementById("c").getContext("2d");
  ctx.fillStyle = "#00ff00";
  var frame = 0;
  function draw() {
    frame++;
    if (frame === 1) { ctx.fillRect(0, 0, 16, 16); }
    while (true) {}
  }
  window.requestAnimationFrame(draw);
</script>"##;

#[test]
fn a_runaway_script_at_the_top_level_is_stopped_at_the_budget() {
    let started = std::time::Instant::now();
    let result = HtmlEffect::from_source("loop", RUNAWAY_LOAD, &Props::new());
    let elapsed = started.elapsed();

    match result {
        Err(canvas_html::HtmlError::Timeout(message)) => {
            assert!(
                message.contains("ran longer than"),
                "the message names the budget: {message}"
            );
            assert!(
                message.contains(&canvas_html::FRAME_BUDGET_MS.to_string())
                    || message.contains(&canvas_html::LOAD_BUDGET_MS.to_string()),
                "the message names the budget that was hit: {message}"
            );
        }
        Err(other) => panic!("expected a timeout, got {other}"),
        Ok(_) => panic!("a runaway script must not load"),
    }

    // It was stopped at the budget: not before it, and not long after it.
    let millis = elapsed.as_millis() as u64;
    assert!(
        millis >= canvas_html::LOAD_BUDGET_MS - 200,
        "it ran for {elapsed:?}, the budget is {} ms",
        canvas_html::LOAD_BUDGET_MS
    );
    assert!(
        millis <= canvas_html::LOAD_BUDGET_MS + 3_000,
        "it ran for {elapsed:?}, the budget is {} ms",
        canvas_html::LOAD_BUDGET_MS
    );
}

#[test]
fn a_runaway_frame_callback_is_stopped_and_the_effect_keeps_running() {
    let mut effect = HtmlEffect::from_source("loop", RUNAWAY_FRAME, &Props::new()).expect("loads");
    let mut canvas = Canvas::new(320, 200);

    let started = std::time::Instant::now();
    effect.update(DT, &mut canvas, &Inputs::default());
    let elapsed = started.elapsed();

    // The frame was cut off at the budget, not later.
    let millis = elapsed.as_millis() as u64;
    assert!(
        millis >= canvas_html::FRAME_BUDGET_MS - 20,
        "the frame ran for {elapsed:?}, the budget is {} ms",
        canvas_html::FRAME_BUDGET_MS
    );
    assert!(
        millis <= canvas_html::FRAME_BUDGET_MS + 2_000,
        "the frame ran for {elapsed:?}, the budget is {} ms",
        canvas_html::FRAME_BUDGET_MS
    );

    // The error is reported once, and it says what happened.
    assert_eq!(effect.errors().len(), 1, "one distinct error: {:?}", effect.errors());
    assert!(
        effect.errors()[0].contains("ran longer than"),
        "the message names the budget: {:?}",
        effect.errors()
    );

    // The update returned, so the engine's loop is not stuck: the next frame runs too.
    let started = std::time::Instant::now();
    effect.update(DT, &mut canvas, &Inputs::default());
    assert!(
        started.elapsed().as_millis() <= 2_000,
        "the next frame was stuck too"
    );
    // The good frame from before the loop is still on the canvas.
    assert_eq!(pixels_near(&canvas, [0, 255, 0], 2), 320 * 200, "the good frame was lost");
}

#[test]
fn the_fixtures_still_run_inside_the_budget() {
    // Lumen draws 26 stroked rings over 320x200; generic.html fills the canvas with a
    // gradient. Neither may come near the frame budget.
    for (name, html) in [("lumen", LUMEN), ("generic", GENERIC)] {
        let mut effect = HtmlEffect::from_source(name, html, &Props::new()).expect("loads");
        let mut canvas = Canvas::new(320, 200);
        let started = std::time::Instant::now();
        for _ in 0..30 {
            effect.update(DT, &mut canvas, &Inputs::default());
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed.as_millis() < 30 * canvas_html::FRAME_BUDGET_MS as u128,
            "{name} took {elapsed:?} for 30 frames"
        );
        assert!(effect.errors().is_empty(), "{name} reported: {:?}", effect.errors());
    }
}

// ---------------------------------------------------------------- running

#[test]
fn lumen_runs_600_frames_without_a_js_error() {
    let mut effect = lumen();
    let canvas = run(&mut effect, 600);

    // No script error was reported in 600 frames.
    assert!(
        effect.errors().is_empty(),
        "lumen reported errors: {:?}",
        effect.errors()
    );
    // The canvas is not empty: the effect drew something.
    assert!(
        canvas.pixels.iter().any(|px| px.iter().any(|c| *c > 0.0)),
        "lumen drew nothing"
    );
}

#[test]
fn a_runtime_error_is_reported_and_the_last_good_frame_is_kept() {
    // The callback throws on its second frame; the first frame's picture must survive.
    let html = r##"<head><title>T</title></head>
<body><canvas id="c" width="16" height="16"></canvas></body>
<script>
  var ctx = document.getElementById("c").getContext("2d");
  var frame = 0;
  ctx.fillStyle = "#00ff00";
  function draw() {
    frame++;
    if (frame === 1) { ctx.fillRect(0, 0, 16, 16); }
    if (frame >= 2) { throw new Error("late failure"); }
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;
    let mut effect = HtmlEffect::from_source("t", html, &Props::new()).expect("loads");
    let canvas = run(&mut effect, 30);
    assert_eq!(effect.errors().len(), 1, "one distinct error: {:?}", effect.errors());
    assert!(effect.errors()[0].contains("late failure"));
    // The effect's 16x16 picture is a full green fill, so every one of the 320x200 canvas
    // pixels (nearest-sampled from it) is green.
    assert_eq!(pixels_near(&canvas, [0, 255, 0], 2), 320 * 200, "the good frame was lost");
}

#[test]
fn a_timer_queued_by_a_timer_runs_on_the_next_frame() {
    let html = r##"<head><title>T</title></head>
<body><canvas id="c" width="4" height="4"></canvas></body>
<script>
  var ctx = document.getElementById("c").getContext("2d");
  ctx.fillStyle = "#ff0000";
  setTimeout(function () {
    setTimeout(function () { ctx.fillRect(0, 0, 4, 4); }, 0);
  }, 0);
</script>"##;
    let mut effect = HtmlEffect::from_source("t", html, &Props::new()).expect("loads");
    // The first frame runs the first timer, which queues the second.
    let after_one = run(&mut effect, 1);
    assert_eq!(pixels_near(&after_one, [255, 0, 0], 2), 0, "the second timer ran too early");
    // The second frame runs the queued timer, which fills the 4x4 picture: every canvas
    // pixel is red.
    let after_two = run(&mut effect, 1);
    assert_eq!(pixels_near(&after_two, [255, 0, 0], 2), 320 * 200, "the queued timer was dropped");
}

#[test]
fn a_bad_canvas_size_is_a_parse_error() {
    let html = r#"<canvas id="c" width="99999" height="10"></canvas><script>var x = 1;</script>"#;
    match HtmlEffect::from_source("big", html, &Props::new()) {
        Err(canvas_html::HtmlError::Parse(_)) => {}
        Err(other) => panic!("expected a parse error, got {other}"),
        Ok(_) => panic!("a canvas over the size cap must not load"),
    }
}

#[test]
fn lumen_canvas_is_not_uniform_and_holds_the_palette_colors() {
    let mut effect = lumen();
    let canvas = run(&mut effect, 600);

    // Not uniform: many different colors on the canvas.
    let distinct = distinct_pixel_count(&canvas);
    assert!(distinct > 10, "the canvas has only {distinct} distinct colors");

    // The palette colors show up. Lumen strokes with the tints at 80% alpha over a
    // translucent background, so the exact colors are blended; a tolerance of 40 per
    // channel covers the blend.
    for hex in LUMEN_TINTS {
        let target: [u8; 3] = {
            let c: canvas_proto::Rgb8 = hex.parse().expect("a hex color");
            [c.r, c.g, c.b]
        };
        let near = pixels_near(&canvas, target, 40);
        assert!(near > 100, "the color {hex} appears on only {near} pixels");
    }
}

#[test]
fn lumen_with_speed_zero_draws_the_same_frame_every_time() {
    let mut effect = lumen();
    // Speed 0 freezes the animation: the rings stop moving and stop growing.
    let mut props = effect.schema().defaults();
    props.insert("speed".to_string(), PropValue::Number(0.0));
    effect.init(&props);

    // Lumen never moves at speed 0, but each frame strokes the same rings again, and the
    // anti-aliased edge pixels take a few hundred frames to saturate (the rings are
    // re-stroked, so the picture settles). The test waits for the picture to settle.
    let mut canvas = run(&mut effect, 500);
    let first = canvas.pixels.clone();

    // The next 60 frames must not change the picture.
    for _ in 0..60 {
        effect.update(DT, &mut canvas, &Inputs::default());
    }

    assert_eq!(
        first.len(),
        canvas.pixels.len(),
        "the canvas changed size"
    );
    assert!(
        first == canvas.pixels,
        "a speed-0 effect must draw identical frames"
    );
}

#[test]
fn a_props_change_on_a_live_instance_updates_the_globals() {
    let mut effect = lumen();
    // Lumen paints its background every frame, so a new background color shows up in
    // the picture once the frames have replaced the old one.
    let mut props = effect.schema().defaults();
    props.insert(
        "background".to_string(),
        PropValue::Color("#00ff00".parse().expect("a hex color")),
    );
    effect.init(&props);
    let canvas = run(&mut effect, 120);

    let green = pixels_near(&canvas, [0, 255, 0], 12);
    assert!(green > 1000, "the new background color did not take effect ({green} green pixels)");
}

// ---------------------------------------------------------------- generic

#[test]
fn a_generic_html_effect_works() {
    let effect = HtmlEffect::from_source("generic", GENERIC, &Props::new()).expect("generic loads");
    let schema = effect.schema();

    assert_eq!(schema.name, "Generic Test Effect");
    assert_eq!(schema.properties.len(), 4);
    assert_eq!(
        schema.defaults().get("speed"),
        Some(&PropValue::Number(20.0))
    );
    assert_eq!(
        schema.defaults().get("showArc"),
        Some(&PropValue::Bool(true))
    );
    assert!(matches!(
        schema.properties[1].kind,
        PropKind::Bool { default: true }
    ));

    let mut effect = effect;
    let canvas = run(&mut effect, 120);
    assert!(distinct_pixel_count(&canvas) > 10, "the generic effect drew nothing");
}

#[test]
fn a_boolean_property_hides_the_arc_when_it_is_false() {
    let mut effect = HtmlEffect::from_source("generic", GENERIC, &Props::new()).expect("loads");
    let mut props = effect.schema().defaults();
    props.insert("showArc".to_string(), PropValue::Bool(false));
    effect.init(&props);

    let without_arc = run(&mut effect, 60);

    // With the arc on, the picture differs.
    let mut effect = HtmlEffect::from_source("generic", GENERIC, &Props::new()).expect("loads");
    let mut props = effect.schema().defaults();
    props.insert("showArc".to_string(), PropValue::Bool(true));
    effect.init(&props);
    let with_arc = run(&mut effect, 60);

    assert!(
        without_arc.pixels != with_arc.pixels,
        "the boolean property had no effect on the drawing"
    );
}

// ---------------------------------------------------------------- properties and stepping

#[test]
fn every_property_type_parses_from_its_meta_tag() {
    let html = r##"<head><title>K</title>
<meta property="mode" label="Mode" type="combobox" values="a,b,c" default="b" />
<meta property="tint" label="Tint" type="hue" default="120" />
<meta property="label" label="Label" type="textfield" default="hi" />
<meta property="flag" label="Flag" type="boolean" default="1" />
</head><script>var x = 1;</script>"##;
    let effect = HtmlEffect::from_source("k", html, &Props::new()).expect("loads");
    let schema = effect.schema();
    assert_eq!(schema.properties.len(), 4);
    assert!(matches!(
        &schema.properties[0].kind,
        PropKind::Choice { options, default } if options.len() == 3 && default.as_str() == "b"
    ));
    assert!(matches!(
        &schema.properties[1].kind,
        PropKind::Hue { default } if *default == 120.0
    ));
    assert!(matches!(
        &schema.properties[2].kind,
        PropKind::Choice { options, default } if options.len() == 1 && default.as_str() == "hi"
    ));
    assert!(matches!(
        &schema.properties[3].kind,
        PropKind::Bool { default: true }
    ));
    // No canvas element: the default 320x200 applies.
    assert_eq!(effect.size(), (320, 200));
}

#[test]
fn an_update_runs_whole_60hz_frames_capped_at_four() {
    // Each frame paints the frame count into the red channel.
    let html = r##"<head><title>F</title></head><body><canvas id="c" width="4" height="4"></canvas></body>
<script>
  var ctx = document.getElementById("c").getContext("2d");
  var n = 0;
  function tick() {
    n++;
    ctx.fillStyle = "rgb(" + n + ",0,0)";
    ctx.fillRect(0, 0, 4, 4);
    window.requestAnimationFrame(tick);
  }
  window.requestAnimationFrame(tick);
</script>"##;
    let mut effect = HtmlEffect::from_source("f", html, &Props::new()).expect("loads");
    let mut canvas = Canvas::new(4, 4);
    let inputs = Inputs::default();

    // One whole frame at 60 Hz.
    effect.update(0.0167, &mut canvas, &inputs);
    assert_eq!((canvas.pixels[0][0] * 255.0).round() as u32, 1);

    // A 0.1 s step asks for six frames; at most four run, so the counter reaches 5.
    effect.update(0.1, &mut canvas, &inputs);
    assert_eq!((canvas.pixels[0][0] * 255.0).round() as u32, 5);
}

#[test]
fn every_canvas_method_the_brief_lists_runs_without_error() {
    let html = r##"<head><title>M</title></head><body><canvas id="c" width="8" height="8"></canvas></body>
<script>
  var c = document.getElementById("c");
  var ctx = c.getContext("2d");
  ctx.globalAlpha = 0.5;
  ctx.lineWidth = 2;
  ctx.strokeStyle = "rgb(255, 0, 0)";
  ctx.fillStyle = "hsl(120, 100%, 50%)";
  ctx.globalCompositeOperation = "lighter";
  ctx.clearRect(0, 0, 8, 8);
  ctx.fillRect(0, 0, 8, 8);
  ctx.strokeRect(1, 1, 6, 6);
  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.lineTo(8, 0);
  ctx.quadraticCurveTo(4, 4, 0, 8);
  ctx.bezierCurveTo(1, 1, 2, 2, 3, 3);
  ctx.closePath();
  ctx.fill();
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(4, 4, 2, 0, Math.PI, false);
  ctx.ellipse(4, 4, 3, 2, 0, 0, Math.PI * 2, false);
  ctx.rect(1, 1, 2, 2);
  ctx.stroke();
  ctx.save();
  ctx.translate(1, 1);
  ctx.rotate(0.1);
  ctx.scale(1, 1);
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.resetTransform();
  ctx.restore();
  var g = ctx.createLinearGradient(0, 0, 8, 8);
  g.addColorStop(0, "#ff0000");
  g.addColorStop(1, "#0000ff");
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, 8, 8);
  var r = ctx.createRadialGradient(4, 4, 1, 4, 4, 4);
  r.addColorStop(0, "#ffffff");
  r.addColorStop(1, "#000000");
  ctx.fillStyle = r;
  ctx.fillRect(0, 0, 8, 8);
  var img = ctx.getImageData(0, 0, 8, 8);
  ctx.putImageData(img, 0, 0);
  ctx.putImageData(ctx.createImageData(2, 2), 0, 0);
  ctx.fillText("hello", 1, 1);
  console.log("done");
</script>"##;
    let mut effect = HtmlEffect::from_source("m", html, &Props::new()).expect("loads");
    let canvas = run(&mut effect, 2);
    assert!(effect.errors().is_empty(), "canvas methods reported: {:?}", effect.errors());
    assert!(
        canvas.pixels.iter().any(|px| px.iter().any(|c| *c > 0.0)),
        "the canvas methods drew nothing"
    );
}

// ---------------------------------------------------------------- compute

/// Times Lumen at a canvas of `width` x `height` (the script still sees 320x200; the
/// canvas size is the render scale), and returns the mean microseconds per 60 Hz frame.
fn lumen_us_per_frame(width: u32, height: u32) -> f64 {
    let mut effect = lumen();
    let mut canvas = Canvas::new(width, height);
    let inputs = Inputs::default();

    // Warm up: the first frames fill the caches and the first strokes size the pixmap.
    for _ in 0..60 {
        effect.update(DT, &mut canvas, &inputs);
    }

    let frames = 1800;
    let start = std::time::Instant::now();
    for _ in 0..frames {
        effect.update(DT, &mut canvas, &inputs);
    }
    let elapsed = start.elapsed();
    let per_frame = elapsed.as_secs_f64() * 1e6 / frames as f64;
    println!("lumen at {width}x{height}: {frames} frames in {elapsed:?}");
    println!("lumen at {width}x{height}: mean {per_frame:.1} us/frame");
    assert!(effect.errors().is_empty(), "lumen reported errors: {:?}", effect.errors());
    per_frame
}

/// The compute budget: Lumen at 50% render scale (160x100) must cost at most 1 ms per
/// 60 Hz frame in release. Run with
/// `cargo test -p canvas-html --release --test effects -- --ignored --nocapture`.
#[test]
#[ignore = "a release-mode benchmark; run it explicitly"]
fn lumen_frame_cost_at_50_percent() {
    let per_frame = lumen_us_per_frame(160, 100);
    assert!(
        per_frame <= 1000.0,
        "lumen costs {per_frame:.1} us/frame at 50%, the budget is 1000 us/frame"
    );
}

/// Reference timings: Lumen at 100% (320x200) and 25% (80x50). Printed, not asserted.
#[test]
#[ignore = "a release-mode benchmark; run it explicitly"]
fn lumen_frame_cost_reference_scales() {
    lumen_us_per_frame(320, 200);
    lumen_us_per_frame(80, 50);
}

/// The CIE L*a*b* colour of an sRGB pixel, for ΔE comparisons.
fn lab(rgb: [f32; 3]) -> [f32; 3] {
    let lin = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(rgb[0]), lin(rgb[1]), lin(rgb[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn delta_e76(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The share of pixels within ΔE 15 of each palette colour.
fn palette_shares(pixels: &[[f32; 3]], palette: &[[f32; 3]]) -> Vec<f32> {
    let labs: Vec<[f32; 3]> = palette.iter().map(|&p| lab(p)).collect();
    palette
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let hits = pixels
                .iter()
                .filter(|&&px| delta_e76(lab(px), labs[i]) <= 15.0)
                .count();
            hits as f32 / pixels.len() as f32
        })
        .collect()
}

/// Lumen at 100% (320x200) against Lumen at 50% (160x100), both seeded the same and run
/// for 600 frames. The 100% picture is box-downsampled to 160x100. Mean RGB per channel
/// must agree within 3 levels, and the palette shares within 3 percentage points.
#[test]
fn lumen_at_half_scale_matches_full_scale_statistically() {
    let props = compare_props();
    let mut full = HtmlEffect::from_source_seeded("lumen", LUMEN, &props, LUMEN_SEED).expect("loads");
    let mut half = HtmlEffect::from_source_seeded("lumen", LUMEN, &props, LUMEN_SEED).expect("loads");

    let mut full_canvas = Canvas::new(320, 200);
    let mut half_canvas = Canvas::new(160, 100);
    let inputs = Inputs::default();
    for _ in 0..600 {
        full.update(DT, &mut full_canvas, &inputs);
        half.update(DT, &mut half_canvas, &inputs);
    }
    assert!(full.errors().is_empty() && half.errors().is_empty());

    // Box-downsample the 100% picture to 160x100 (2x2 mean).
    let mut down: Vec<[f32; 3]> = Vec::with_capacity(160 * 100);
    for y in 0..100u32 {
        for x in 0..160u32 {
            let mut acc = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = full_canvas.get(x * 2 + dx, y * 2 + dy);
                for k in 0..3 {
                    acc[k] += p[k] * 0.25;
                }
            }
            down.push(acc);
        }
    }

    let mean = |px: &[[f32; 3]]| {
        let mut m = [0.0f64; 3];
        for p in px {
            for k in 0..3 {
                m[k] += p[k] as f64 * 255.0;
            }
        }
        m.map(|v| v / px.len() as f64)
    };
    let (mean_full, mean_half) = (mean(&down), mean(&half_canvas.pixels));
    for k in 0..3 {
        let diff = (mean_full[k] - mean_half[k]).abs();
        println!("mean channel {k}: 100% {:.2}, 50% {:.2}, diff {diff:.2}", mean_full[k], mean_half[k]);
        assert!(diff <= 3.0, "mean channel {k} differs by {diff:.2} levels");
    }

    let palette: Vec<[f32; 3]> = [
        props.get("background"),
        props.get("tint1"),
        props.get("tint2"),
        props.get("tint3"),
    ]
    .iter()
    .map(|v| match v {
        Some(PropValue::Color(c)) => [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0],
        _ => panic!("the compare props hold four colours"),
    })
    .collect();
    let shares_full = palette_shares(&down, &palette);
    let shares_half = palette_shares(&half_canvas.pixels, &palette);
    for i in 0..palette.len() {
        let diff = (shares_full[i] - shares_half[i]).abs() * 100.0;
        println!(
            "palette {i}: 100% {:.1}%, 50% {:.1}%, diff {diff:.2} pp",
            shares_full[i] * 100.0,
            shares_half[i] * 100.0
        );
        assert!(diff <= 3.0, "palette colour {i} share differs by {diff:.2} pp");
    }
}

/// Writes Lumen's picture after `LUMEN_DUMP_FRAMES` frames (default 600), seed 1000 (the
/// seed `tools/poison-compare/js/render.mjs` gives its first run), as raw RGB bytes to the
/// path in `LUMEN_DUMP`. The Chromium-comparison dump; ignored by default.
#[test]
#[ignore = "writes a comparison dump; set LUMEN_DUMP"]
fn dump_lumen_seed_1000() {
    let path = std::env::var("LUMEN_DUMP").expect("set LUMEN_DUMP");
    let frames: usize = std::env::var("LUMEN_DUMP_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let mut effect =
        HtmlEffect::from_source_seeded("lumen", LUMEN, &compare_props(), 1000).expect("loads");
    let canvas = run(&mut effect, frames);
    let mut bytes = Vec::with_capacity(canvas.pixels.len() * 3);
    for px in &canvas.pixels {
        let c = canvas_proto::Rgb8::from_f32(*px);
        bytes.extend_from_slice(&[c.r, c.g, c.b]);
    }
    std::fs::write(&path, bytes).expect("write the dump");
}

#[test]
fn set_timeout_honours_its_delay() {
    // A browser-style loop: step() re-arms itself every 1000 ms. After one second of frames
    // it has run once or twice, never once per frame. The canvas turns red once it has run
    // three times.
    let html = r##"<head><title>T</title></head>
<body><canvas id="c" width="4" height="4"></canvas></body>
<script>
  var ctx = document.getElementById("c").getContext("2d");
  var n = 0;
  function step() { n++; setTimeout(step, 1000); }
  step();
  function draw() {
    ctx.fillStyle = n >= 3 ? "#ff0000" : "#00ff00";
    ctx.fillRect(0, 0, 4, 4);
    requestAnimationFrame(draw);
  }
  requestAnimationFrame(draw);
</script>"##;
    let mut effect = HtmlEffect::from_source("t", html, &Props::new()).expect("loads");
    let canvas = run(&mut effect, 60);
    assert_eq!(pixels_near(&canvas, [255, 0, 0], 2), 0, "step() ran about once per frame");
}

#[test]
fn the_load_budget_covers_all_scripts_together() {
    // Time how long a counting loop takes here, then give the page four scripts that each
    // take about 0.9 s: 3.6 s together, over the 2 s load budget.
    fn page(iterations: u64, scripts: usize) -> String {
        let one = format!("<script>for (var i = 0; i < {iterations}; i++) {{}}</script>");
        format!("<head><title>T</title></head><body><canvas id=\"c\" width=\"4\" height=\"4\"></canvas></body>{}", one.repeat(scripts))
    }
    // Calibrate with a loop that grows until it takes about 50 ms, so a slow machine (a debug
    // build on a CI runner) never spends the 2 s budget on the calibration itself.
    let mut probe = 10_000u64;
    let per_iter = loop {
        let start = std::time::Instant::now();
        HtmlEffect::from_source("probe", &page(probe, 1), &Props::new()).expect("probe loads");
        let took = start.elapsed().as_secs_f64();
        if took >= 0.05 {
            break took / probe as f64;
        }
        probe *= 2;
    };
    let iterations = (0.9 / per_iter) as u64;
    let result = HtmlEffect::from_source("t", &page(iterations, 4), &Props::new());
    assert!(
        matches!(result, Err(canvas_html::HtmlError::Timeout(_))),
        "four 0.9 s scripts should exceed one shared 2 s budget"
    );
}

#[test]
fn a_script_that_allocates_without_limit_fails_cleanly() {
    // About 640 MB of arrays: far over the 256 MB heap cap, small enough to run safely if
    // there is no cap.
    let html = r##"<head><title>T</title></head>
<body><canvas id="c" width="4" height="4"></canvas></body>
<script>
  var keep = [];
  for (var i = 0; i < 400; i++) { keep.push(new Array(100000).fill(0)); }
</script>"##;
    let result = HtmlEffect::from_source("t", html, &Props::new());
    assert!(matches!(result, Err(canvas_html::HtmlError::Script(_))), "the heap cap should raise an error");
}

#[test]
fn the_largest_allowed_canvas_can_be_read_back() {
    // 4096 x 4096 is the biggest canvas the loader accepts; its getImageData buffer is 64 MB
    // and must fit under the JS heap cap with headroom.
    let html = r##"<head><title>T</title></head>
<body><canvas id="c" width="4096" height="4096"></canvas></body>
<script>
  var ctx = document.getElementById("c").getContext("2d");
  var img = ctx.getImageData(0, 0, 4096, 4096);
  if (img.data.length !== 4096 * 4096 * 4) { throw new Error("short read " + img.data.length); }
</script>"##;
    let result = HtmlEffect::from_source("t", html, &Props::new());
    assert!(result.is_ok(), "a max-size getImageData failed: {:?}", result.err());
}

#[test]
fn a_throwing_timer_does_not_drop_the_timers_queued_behind_it() {
    let html = r##"<head><title>T</title></head>
<body><canvas id="c" width="4" height="4"></canvas></body>
<script>
  var ctx = document.getElementById("c").getContext("2d");
  ctx.fillStyle = "#ff0000";
  setTimeout(function () { throw new Error("first timer fails"); }, 0);
  setTimeout(function () { ctx.fillRect(0, 0, 4, 4); }, 0);
</script>"##;
    let mut effect = HtmlEffect::from_source("t", html, &Props::new()).expect("loads");
    let canvas = run(&mut effect, 4);
    assert_eq!(effect.errors().len(), 1, "{:?}", effect.errors());
    assert_eq!(pixels_near(&canvas, [255, 0, 0], 2), 320 * 200, "the second timer was dropped");
}

// ---------------------------------------------------------------- stock effects
//
// The effects that ship inside the daemon (`crates/canvas-daemon/stock-effects/`). Each one
// is a seamless loop: the picture a full period later is the same picture.

const INK_DROP: &str = include_str!("../../canvas-daemon/stock-effects/ink-drop.html");
const WET_SMEAR: &str = include_str!("../../canvas-daemon/stock-effects/wet-smear.html");
const PALETTE_KNIFE: &str = include_str!("../../canvas-daemon/stock-effects/palette-knife.html");

/// (id, source, period in seconds at the default Speed).
const STOCK: [(&str, &str, usize); 3] = [
    ("ink-drop", INK_DROP, 12),
    ("wet-smear", WET_SMEAR, 16),
    ("palette-knife", PALETTE_KNIFE, 24),
];

fn stock(id: &str, source: &str) -> HtmlEffect {
    let mut effect =
        HtmlEffect::from_source_seeded(id, source, &Props::new(), LUMEN_SEED).expect("loads");
    let defaults = effect.schema().defaults();
    effect.init(&defaults);
    effect
}

fn rgb8(canvas: &Canvas) -> Vec<u8> {
    canvas
        .pixels
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8))
        .collect()
}

fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).abs() as f64).sum::<f64>() / a.len() as f64
}

#[test]
#[cfg_attr(debug_assertions, ignore = "the paint effects fit the 100 ms frame budget in release; CI runs these with --release")]
fn the_stock_effects_load_move_and_report_no_errors() {
    for (id, source, _) in STOCK {
        let mut effect = stock(id, source);
        let schema = effect.schema();
        assert!(!schema.name.is_empty(), "{id} has a name");
        assert!(!schema.description.is_empty(), "{id} has a description");
        assert!(schema.properties.iter().any(|p| p.key == "speed"), "{id} has a speed property");
        let mut canvas = Canvas::new(320, 200);
        let inputs = Inputs::default();
        let mut first = None;
        let mut moved = false;
        for frame in 0..120 {
            effect.update(DT, &mut canvas, &inputs);
            let now = rgb8(&canvas);
            if frame == 0 {
                first = Some(now);
            } else if first.as_ref() != Some(&now) {
                moved = true;
            }
        }
        assert!(effect.errors().is_empty(), "{id} reported errors: {:?}", effect.errors());
        assert!(moved, "{id} does not move");
        assert!(distinct_pixel_count(&canvas) > 16, "{id} is nearly flat");
    }
}

/// Runs `period` seconds of 60 Hz frames twice and compares the picture at the end of the
/// first period with the end of the second, and one mid-period pair.
fn assert_loop_closes(id: &str, source: &str, period: usize) {
    let n = period * 60;
    let mut effect = stock(id, source);
    let mut canvas = Canvas::new(320, 200);
    let inputs = Inputs::default();
    let (mut at_half, mut at_t, mut at_t_half, mut at_2t) = (vec![], vec![], vec![], vec![]);
    for k in 1..=2 * n {
        effect.update(DT, &mut canvas, &inputs);
        if k == n / 2 {
            at_half = rgb8(&canvas);
        } else if k == n {
            at_t = rgb8(&canvas);
        } else if k == n + n / 2 {
            at_t_half = rgb8(&canvas);
        } else if k == 2 * n {
            at_2t = rgb8(&canvas);
        }
    }
    assert!(effect.errors().is_empty(), "{id} reported errors: {:?}", effect.errors());
    let closed = mean_abs_diff(&at_t, &at_2t);
    let closed_half = mean_abs_diff(&at_half, &at_t_half);
    let control = mean_abs_diff(&at_t, &at_t_half);
    assert!(closed < 0.05, "{id}: t=T and t=2T differ by {closed:.4} levels");
    assert!(closed_half < 0.05, "{id}: T/2 and 3T/2 differ by {closed_half:.4} levels");
    assert!(control > 5.0, "{id}: the control pair differs by only {control:.4} levels");
}

#[test]
#[cfg_attr(debug_assertions, ignore = "the paint effects fit the 100 ms frame budget in release; CI runs these with --release")]
fn ink_drop_loops_every_12_seconds() {
    assert_loop_closes("ink-drop", INK_DROP, 12);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "the paint effects fit the 100 ms frame budget in release; CI runs these with --release")]
fn wet_smear_loops_every_16_seconds() {
    assert_loop_closes("wet-smear", WET_SMEAR, 16);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "the paint effects fit the 100 ms frame budget in release; CI runs these with --release")]
fn palette_knife_loops_every_24_seconds() {
    assert_loop_closes("palette-knife", PALETTE_KNIFE, 24);
}

/// A small effect with a `<meta thumbnail ...>` tag. The script paints a colour, so a test
/// can tell a picture from a blank canvas.
fn with_recipe(tag: &str) -> String {
    format!(
        r##"<head><title>T</title>{tag}
  <meta property="speed" label="Speed" type="number" min="0" max="100" default="30" />
  <meta property="tint" label="Tint" type="color" default="#ff0000" />
  <meta property="style" label="Style" type="combobox" values="a,b" default="a" />
  <meta property="glow" label="Glow" type="boolean" default="true" />
</head><body><canvas id="c" width="8" height="8"></canvas></body>
<script>var ctx = document.getElementById("c").getContext("2d");
function draw() {{ ctx.fillStyle = "#ff8800"; ctx.fillRect(0, 0, 8, 8); window.requestAnimationFrame(draw); }}
window.requestAnimationFrame(draw);</script>"##
    )
}

#[test]
fn a_thumbnail_meta_becomes_the_effects_recipe() {
    let html = with_recipe(
        r##"<meta thumbnail warm_up="3" loop="4.5" crossfade="6" props="speed=90; tint=#00ff80; style=b" dark_on_purpose="a spark on black" />"##,
    );
    let effect = HtmlEffect::from_source("html:t", &html, &Props::new()).expect("the file loads");
    assert!(effect.warnings().is_empty());
    let recipe = effect.schema().thumbnail.expect("a recipe");
    assert_eq!((recipe.warm_up_s, recipe.loop_s, recipe.crossfade_frames), (3.0, 4.5, 6));
    assert_eq!(recipe.dark_on_purpose.as_deref(), Some("a spark on black"));
    assert_eq!(recipe.props.get("speed"), Some(&PropValue::Number(90.0)));
    assert_eq!(recipe.props.get("style"), Some(&PropValue::Choice("b".into())));
    assert_eq!(
        recipe.props.get("tint"),
        Some(&PropValue::Color("#00ff80".parse().expect("a hex color")))
    );
}

#[test]
fn a_file_without_a_thumbnail_meta_has_no_recipe_and_a_bare_one_takes_the_defaults() {
    let none = HtmlEffect::from_source("html:t", &with_recipe(""), &Props::new()).expect("loads");
    assert!(none.schema().thumbnail.is_none());
    let bare = HtmlEffect::from_source("html:t", &with_recipe(r#"<meta thumbnail />"#), &Props::new()).expect("loads");
    assert_eq!(bare.schema().thumbnail, Some(canvas_proto::props::ThumbnailRecipe::default()));
}

#[test]
fn a_bad_thumbnail_meta_is_ignored_with_a_warning_and_the_effect_still_runs() {
    for (tag, wants) in [
        (r#"<meta thumbnail loop="4.5s" />"#, "loop"),
        (r#"<meta thumbnail props="nope=1" />"#, "no property"),
        (r#"<meta thumbnail props="speed=fast" />"#, "not a valid speed"),
        (r#"<meta thumbnail props="speed=inf" />"#, "not a valid speed"),
        (r#"<meta thumbnail props="speed=NaN" />"#, "not a valid speed"),
        (r#"<meta thumbnail props="speed" />"#, "key=value"),
        (r#"<meta thumbnail props="glow=yes" />"#, "not a valid glow"),
        (r#"<meta thumbnail props="style=z" />"#, "not a valid style"),
    ] {
        let mut effect = HtmlEffect::from_source("html:t", &with_recipe(tag), &Props::new())
            .unwrap_or_else(|e| panic!("{tag} must not stop the effect loading: {e}"));
        assert!(effect.schema().thumbnail.is_none(), "{tag}: no recipe");
        let warnings = effect.warnings().join(" ");
        assert!(warnings.contains(wants), "{tag}: {warnings}");
        // The effect runs, and it paints.
        let mut canvas = Canvas::new(8, 8);
        effect.init(&effect.schema().defaults());
        effect.update(DT, &mut canvas, &Inputs::default());
        assert!(effect.errors().is_empty(), "{tag}: {:?}", effect.errors());
        assert!(
            canvas.pixels.iter().any(|p| p.iter().any(|v| *v > 0.1)),
            "{tag}: the effect ran but painted nothing"
        );
    }
}

#[test]
fn a_bad_property_meta_is_ignored_with_a_warning_and_the_effect_still_runs() {
    // A property the file describes badly is one property the editor does not show: the
    // effect loads, runs and keeps every other property. A bad property never takes the
    // whole effect away (bug class `cause:bad-file-part`).
    for (tag, key, wants) in [
        (r#"<meta property="extra1" label="Extra 1" type="color" />"#, "extra1", "bad color default"),
        (
            r#"<meta property="extra2" label="Extra 2" type="color" default="not-a-colour" />"#,
            "extra2",
            "bad color default",
        ),
        (
            r#"<meta property="extra3" label="Extra 3" type="combobox" />"#,
            "extra3",
            "no values",
        ),
    ] {
        let effect = HtmlEffect::from_source("html:t", &with_recipe(tag), &Props::new())
            .unwrap_or_else(|e| panic!("{tag} must not stop the effect loading: {e}"));
        // The property is not in the schema.
        assert!(
            !effect.schema().properties.iter().any(|p| p.key == key),
            "{tag}: the property {key} is still in the schema"
        );
        // The other properties are still there.
        for keep in ["speed", "glow", "tint", "style"] {
            assert!(
                effect.schema().properties.iter().any(|p| p.key == keep),
                "{tag}: the property {keep} is gone"
            );
        }
        let warnings = effect.warnings().join(" ");
        assert!(warnings.contains(wants), "{tag}: {warnings}");
        assert!(warnings.contains(key), "{tag}: the warning does not name the property: {warnings}");
        // The effect runs.
        let mut effect = effect;
        let mut canvas = Canvas::new(8, 8);
        effect.init(&effect.schema().defaults());
        effect.update(DT, &mut canvas, &Inputs::default());
        assert!(effect.errors().is_empty(), "{tag}: {:?}", effect.errors());
    }
}

#[test]
fn a_boolean_in_a_recipe_takes_true_false_1_or_0_and_a_number_is_held_to_its_range() {
    let html = with_recipe(r#"<meta thumbnail props="glow=0;speed=500" loop_jump_on_purpose="a strobe" />"#);
    let effect = HtmlEffect::from_source("html:t", &html, &Props::new()).expect("loads");
    let recipe = effect.schema().thumbnail.expect("a recipe");
    assert_eq!(recipe.props.get("glow"), Some(&PropValue::Bool(false)));
    assert_eq!(recipe.props.get("speed"), Some(&PropValue::Number(100.0)));
    assert_eq!(recipe.loop_jump_on_purpose.as_deref(), Some("a strobe"));
}

#[test]
fn a_number_prop_that_is_not_finite_or_out_of_range_never_reaches_the_script() {
    let mut effect = HtmlEffect::from_source("html:t", &with_recipe(""), &Props::new()).expect("loads");
    let mut props = Props::new();
    props.insert("speed".into(), PropValue::Number(f64::NAN));
    effect.init(&props);
    assert_eq!(effect.values().get("speed"), Some(&PropValue::Number(30.0)));
    props.insert("speed".into(), PropValue::Number(1e9));
    effect.init(&props);
    assert_eq!(effect.values().get("speed"), Some(&PropValue::Number(100.0)));
}
