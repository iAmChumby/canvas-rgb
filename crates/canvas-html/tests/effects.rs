//! Integration tests for the HTML effect runtime.
//!
//! The fixtures are SignalRGB's own Poison effect (copied byte for byte at Luke's
//! request; see `fixtures/README.md`) and a small effect written for this repo.

use canvas_html::HtmlEffect;
use canvas_proto::{Canvas, Effect, Inputs, PropKind, PropValue, Props};

const POISON: &str = include_str!("fixtures/poison.html");
const GENERIC: &str = include_str!("fixtures/generic.html");

/// One frame at 60 fps.
const DT: f32 = 1.0 / 60.0;

/// Poison's declared defaults, as its file states them.
const POISON_BG: &str = "#130032";
const POISON_COLORS: [&str; 3] = ["#6000fc", "#b300ff", "#8a42ff"];
const POISON_SPEED: f64 = 10.0;

/// The properties `tools/poison-compare` compares with.
fn compare_props() -> Props {
    let mut props = Props::new();
    props.insert(
        "bgColor".to_string(),
        PropValue::Color("#2d0075".parse().expect("a hex color")),
    );
    for (key, hex) in [
        ("color1", POISON_COLORS[0]),
        ("color2", POISON_COLORS[1]),
        ("color3", POISON_COLORS[2]),
    ] {
        props.insert(
            key.to_string(),
            PropValue::Color(hex.parse().expect("a hex color")),
        );
    }
    props.insert("speedRaw".to_string(), PropValue::Number(35.0));
    props
}

fn poison() -> HtmlEffect {
    HtmlEffect::from_source("poison", POISON, &Props::new()).expect("poison loads")
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
fn poison_schema_has_five_properties_with_the_declared_defaults() {
    let effect = poison();
    let schema = effect.schema();

    assert_eq!(schema.id, "poison");
    assert_eq!(schema.name, "Poison");
    assert_eq!(schema.description, "A test effect.");
    assert_eq!(schema.properties.len(), 5, "poison declares five properties");

    let keys: Vec<&str> = schema.properties.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, vec!["bgColor", "color1", "color2", "color3", "speedRaw"]);

    // The labels come from the file.
    assert_eq!(schema.properties[0].label, "Background Color");
    assert_eq!(schema.properties[4].label, "Speed");

    // The defaults come from the file too.
    let defaults = schema.defaults();
    assert_eq!(
        defaults.get("bgColor"),
        Some(&PropValue::Color(POISON_BG.parse().expect("a hex color")))
    );
    for (key, hex) in [
        ("color1", POISON_COLORS[0]),
        ("color2", POISON_COLORS[1]),
        ("color3", POISON_COLORS[2]),
    ] {
        assert_eq!(
            defaults.get(key),
            Some(&PropValue::Color(hex.parse().expect("a hex color"))),
            "{key} default"
        );
    }
    assert_eq!(defaults.get("speedRaw"), Some(&PropValue::Number(POISON_SPEED)));

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
    let effect = HtmlEffect::from_source("poison", POISON, &compare_props()).expect("poison loads");
    let defaults = effect.schema().defaults();
    assert_eq!(
        defaults.get("bgColor"),
        Some(&PropValue::Color("#2d0075".parse().expect("a hex color")))
    );
    assert_eq!(defaults.get("speedRaw"), Some(&PropValue::Number(35.0)));
    // The colors that were not overridden keep the file's defaults.
    assert_eq!(
        defaults.get("color1"),
        Some(&PropValue::Color(POISON_COLORS[0].parse().expect("a hex color")))
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

// ---------------------------------------------------------------- running

#[test]
fn poison_runs_600_frames_without_a_js_error() {
    let mut effect = poison();
    let canvas = run(&mut effect, 600);

    // No script error was reported in 600 frames.
    assert!(
        effect.errors().is_empty(),
        "poison reported errors: {:?}",
        effect.errors()
    );
    // The canvas is not empty: the effect drew something.
    assert!(
        canvas.pixels.iter().any(|px| px.iter().any(|c| *c > 0.0)),
        "poison drew nothing"
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
fn poison_canvas_is_not_uniform_and_holds_the_palette_colors() {
    let mut effect = poison();
    let canvas = run(&mut effect, 600);

    // Not uniform: many different colors on the canvas.
    let distinct = distinct_pixel_count(&canvas);
    assert!(distinct > 10, "the canvas has only {distinct} distinct colors");

    // The palette colors show up. Poison strokes with the colors at 53% alpha over a
    // translucent background, so the exact colors are blended; a tolerance of 40 per
    // channel covers the blend.
    for hex in POISON_COLORS {
        let target: [u8; 3] = {
            let c: canvas_proto::Rgb8 = hex.parse().expect("a hex color");
            [c.r, c.g, c.b]
        };
        let near = pixels_near(&canvas, target, 40);
        assert!(near > 100, "the color {hex} appears on only {near} pixels");
    }
}

#[test]
fn poison_with_speed_zero_draws_the_same_frame_every_time() {
    let mut effect = poison();
    // Speed 0 freezes the animation: the squares stop moving.
    let mut props = effect.schema().defaults();
    props.insert("speedRaw".to_string(), PropValue::Number(0.0));
    effect.init(&props);

    // Poison never moves at speed 0, but each frame strokes the same arcs again, and the
    // anti-aliased edge pixels take a few hundred frames to saturate (measured: frames 200
    // to 300 differ by 0 pixels; frames 62 to 100 still differ by up to 38 levels). This is
    // the original effect's compositing, so the test waits for the picture to settle.
    let mut canvas = run(&mut effect, 400);
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
    let mut effect = poison();
    // Poison paints its background every frame while speed is above 0, so a new background
    // color shows up in the picture once the frames have replaced the old one.
    let mut props = effect.schema().defaults();
    props.insert(
        "bgColor".to_string(),
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
        schema.defaults().get("speedRaw"),
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

/// The compute budget: Poison at 320x200 must cost at most 1 ms per frame in release.
/// Run with `cargo test -p canvas-html --release -- --ignored --nocapture`.
#[test]
#[ignore = "a release-mode benchmark; run it explicitly"]
fn poison_frame_cost() {
    let mut effect = poison();
    let mut canvas = Canvas::new(320, 200);
    let inputs = Inputs::default();

    // Warm up: the first frames fill the JIT caches.
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

    println!("poison: {frames} frames at 320x200 in {elapsed:?}");
    println!("poison: mean {per_frame:.1} us/frame");
    assert!(
        per_frame <= 1000.0,
        "poison costs {per_frame:.1} us/frame, the budget is 1000 us/frame"
    );
}
