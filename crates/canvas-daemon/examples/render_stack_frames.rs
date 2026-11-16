//! Renders the "Warm flame" stack through the daemon's own compositor (#256): a White 2700 K
//! layer, whole canvas, masked by the real Fire effect as a luma mask. It runs 90 frames at 30 fps
//! on the 320 x 200 canvas and writes frames 30, 45, 60, 75 and 90 as PNGs, twice: Fire's Mono
//! palette (the demo's seed) and its Classic palette.
//!
//! The demo backend's Fire is a JS stand-in; this is the effect the lights run. The reserved tile
//! seed goes into the mask's own props (the compositor hands it to a layer's effect, not its
//! mask's), so every run and every palette sees the same heat field.
//!
//! Usage: cargo run --release -p canvas-daemon --example render_stack_frames [out_dir]
//!        cargo run --release -p canvas-daemon --example render_stack_frames -- --tune [out_dir]
//! Default out_dir: .build/screens/256/real (relative to where it is run). `--tune` writes each
//! compared Fire setting into `<out_dir>/tune-<name>/`.

use canvas_daemon::stack::{stack_props, AreaMaps, LeafBuilder, LiveMap, StackEffect};
use canvas_proto::props::{Props, PropValue};
use canvas_proto::stack::{BlendMode, LayerArea, LayerMask, MaskSource, Stack, StackLayer};
use canvas_proto::traits::{Canvas, Effect, Inputs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// The frames the example keeps, after that many 1/30 s updates.
const KEEP: [usize; 5] = [30, 45, 60, 75, 90];
const FRAMES: usize = 90;
const DT: f32 = 1.0 / 30.0;
/// Warm flame's Fire values, as the demo seed ships them (#256): measured on the real compositor,
/// the flame body reaches about half the canvas with clear tongues.
const INTENSITY: f64 = 100.0;
const COOLING: f64 = 16.0;

/// The settings compared for "the flame body reaches about half the canvas" (#256): intensity,
/// cooling, and the name of the folder each one's frames go into.
const TUNES: [(&str, f64, f64); 3] = [
    ("strong", 100.0, 6.0),
    ("balanced", 100.0, 12.0),
    ("half", 100.0, 16.0),
];

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some("--tune") {
        let out = PathBuf::from(args.next().unwrap_or_else(|| ".build/screens/256/real".to_string()));
        std::fs::create_dir_all(&out).expect("create the output folder");
        for (name, intensity, cooling) in TUNES {
            let dir = out.join(format!("tune-{name}"));
            std::fs::create_dir_all(&dir).expect("create the tuning folder");
            println!("--- tune-{name}: intensity {intensity}, cooling {cooling}, palette Mono");
            render(&warm_flame("Mono", intensity, cooling), "warm-flame", &dir);
        }
        println!("tuning frames written to {}", out.display());
        return;
    }
    let out = PathBuf::from(first.unwrap_or_else(|| ".build/screens/256/real".to_string()));
    std::fs::create_dir_all(&out).expect("create the output folder");
    for (tag, palette) in [("mono", "Mono"), ("classic", "Classic")] {
        let stack = warm_flame(palette, INTENSITY, COOLING);
        render(&stack, &format!("warm-flame-{tag}"), &out);
    }
    println!("frames written to {}", out.display());
}

/// One White 2700 K layer, whole canvas, with Fire as its luma mask at full amount.
fn warm_flame(palette: &str, intensity: f64, cooling: f64) -> Stack {
    let mut fire = Props::new();
    fire.insert("intensity".into(), PropValue::Number(intensity));
    fire.insert("cooling".into(), PropValue::Number(cooling));
    fire.insert("palette".into(), PropValue::Choice(palette.to_string()));
    // The compositor hands the reserved tile seed to a layer's effect but not to its mask's, so the
    // example puts it in the mask's own props: every run and every setting sees the same heat field.
    fire.insert(
        canvas_effects::TILE_SEED_PROP.to_string(),
        PropValue::Number(256.0),
    );
    let mut white = Props::new();
    white.insert("temperature".into(), PropValue::Number(2700.0));
    Stack {
        id: "stack:Warm flame".into(),
        layers: vec![StackLayer {
            id: "l1".into(),
            name: "Warm white".into(),
            effect: "white".into(),
            props: white,
            area: LayerArea::Canvas,
            blend: BlendMode::Normal,
            opacity: 100,
            visible: true,
            mask: Some(LayerMask {
                effect: "fire".into(),
                props: fire,
                source: MaskSource::Luma,
                invert: false,
                amount: 100,
            }),
        }],
    }
}

fn render(stack: &Stack, tag: &str, out: &Path) {
    let leaf: LeafBuilder = Arc::new(|id: &str| {
        canvas_effects::create(id).ok_or_else(|| format!("no effect with the id {id}"))
    });
    let live = Arc::new(RwLock::new(LiveMap::from_stack(stack, 1)));
    let areas = Arc::new(RwLock::new(AreaMaps::default()));
    let mut effect = StackEffect::new(stack, leaf, live, areas);
    // The layer's own values, plus the reserved tile seed so the Fire is the same every run and
    // both palettes see the same heat field.
    let resolve = |id: &str| canvas_effects::create(id).map(|effect| effect.schema());
    let mut props = stack_props(stack, &resolve);
    props.insert(
        canvas_effects::TILE_SEED_PROP.to_string(),
        PropValue::Number(256.0),
    );
    effect.init(&props);

    let mut canvas = Canvas::with_scale(1.0);
    let inputs = Inputs::default();
    for frame in 1..=FRAMES {
        effect.update(DT, &mut canvas, &inputs);
        if KEEP.contains(&frame) {
            let name = format!("{tag}-{frame:03}.png");
            let path = out.join(&name);
            write_png(&path, &canvas);
            let stats = stats(&canvas);
            println!(
                "{name:<22} mean luma {:.4}  coverage > 0.5: {:>5.2} %  top row {:>3} ({:>3.0} % up)",
                stats.mean,
                stats.coverage_over_half * 100.0,
                stats.top_row,
                stats.up * 100.0,
            );
        }
    }
}

/// One frame's numbers: its mean Rec. 709 luma, the share of pixels whose **mask coverage** is over
/// 0.5, and the topmost row the flame reaches (coverage over 0.1).
///
/// The layer is White 2700 K, whose red channel is 1.0, and it is composited Normal over black, so
/// a pixel's red channel *is* the mask's coverage. `up` is how far that row is from the bottom, as
/// a share of the canvas height.
struct Stats {
    mean: f32,
    coverage_over_half: f32,
    top_row: usize,
    up: f32,
}

fn stats(canvas: &Canvas) -> Stats {
    let width = canvas.width as usize;
    let height = canvas.height as usize;
    let mut sum = 0.0f32;
    let mut over_half = 0usize;
    let mut top_row = height;
    for (index, pixel) in canvas.pixels.iter().enumerate() {
        let luma = 0.2126 * pixel[0].clamp(0.0, 1.0)
            + 0.7152 * pixel[1].clamp(0.0, 1.0)
            + 0.0722 * pixel[2].clamp(0.0, 1.0);
        sum += luma;
        let coverage = pixel[0].clamp(0.0, 1.0);
        if coverage > 0.5 {
            over_half += 1;
        }
        if coverage > 0.1 {
            top_row = top_row.min(index / width);
        }
    }
    let count = canvas.pixels.len().max(1) as f32;
    Stats {
        mean: sum / count,
        coverage_over_half: over_half as f32 / count,
        top_row,
        up: (height.saturating_sub(top_row)) as f32 / height.max(1) as f32,
    }
}

fn write_png(path: &Path, canvas: &Canvas) {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let rgb: Vec<u8> = canvas
        .pixels
        .iter()
        .flat_map(|pixel| pixel.map(byte))
        .collect();
    let file = std::fs::File::create(path).expect("create the png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), canvas.width, canvas.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::High);
    encoder
        .write_header()
        .expect("png header")
        .write_image_data(&rgb)
        .expect("png data");
}
