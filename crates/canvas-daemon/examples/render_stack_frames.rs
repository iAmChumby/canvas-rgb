//! Renders the "Warm flame" stack through the daemon's own compositor (#256): a White 2700 K
//! layer, whole canvas, masked by the real Fire effect as a luma mask. It runs 90 frames at 30 fps
//! on the 320 x 200 canvas and writes frames 30, 45, 60, 75 and 90 as PNGs, twice: Fire's Mono
//! palette (the demo's seed) and its Classic palette.
//!
//! The demo backend's Fire is a JS stand-in; this is the effect the lights run. A fixed tile seed
//! goes to every layer, so the two palettes see the same flame and a rerun is the same frames.
//!
//! Usage: cargo run --release -p canvas-daemon --example render_stack_frames [out_dir]
//! Default out_dir: .build/screens/256/real (relative to where it is run).

use canvas_daemon::stack::{stack_props, AreaMap, LeafBuilder, LiveMap, StackEffect};
use canvas_proto::props::{Props, PropValue};
use canvas_proto::stack::{BlendMode, LayerArea, LayerMask, MaskSource, Stack, StackLayer};
use canvas_proto::traits::{Canvas, Effect, Inputs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// The frames the example keeps, after that many 1/30 s updates.
const KEEP: [usize; 5] = [30, 45, 60, 75, 90];
const FRAMES: usize = 90;
const DT: f32 = 1.0 / 30.0;

fn main() {
    let out = PathBuf::from(
        std::env::args().nth(1).unwrap_or_else(|| ".build/screens/256/real".to_string()),
    );
    std::fs::create_dir_all(&out).expect("create the output folder");
    for (tag, palette) in [("mono", "Mono"), ("classic", "Classic")] {
        let stack = warm_flame(palette);
        render(&stack, &format!("warm-flame-{tag}"), &out);
    }
    println!("frames written to {}", out.display());
}

/// One White 2700 K layer, whole canvas, with Fire as its luma mask at full amount.
fn warm_flame(palette: &str) -> Stack {
    let mut fire = Props::new();
    fire.insert("intensity".into(), PropValue::Number(70.0));
    fire.insert("palette".into(), PropValue::Choice(palette.to_string()));
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
    let areas = Arc::new(RwLock::new(AreaMap::default()));
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
            let (mean, lit) = stats(&canvas);
            println!("{name:<26} mean luma {mean:.4}  luma > 0.5: {:>5.2} %", lit * 100.0);
        }
    }
}

/// Mean Rec. 709 luma of the encoded canvas values, and the share of pixels over 0.5.
fn stats(canvas: &Canvas) -> (f32, f32) {
    let mut sum = 0.0f32;
    let mut lit = 0usize;
    for pixel in &canvas.pixels {
        let luma = 0.2126 * pixel[0].clamp(0.0, 1.0)
            + 0.7152 * pixel[1].clamp(0.0, 1.0)
            + 0.0722 * pixel[2].clamp(0.0, 1.0);
        sum += luma;
        if luma > 0.5 {
            lit += 1;
        }
    }
    let count = canvas.pixels.len().max(1) as f32;
    (sum / count, lit as f32 / count)
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
