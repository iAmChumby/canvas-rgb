//! A scratch timing harness for the effects. Not part of the crate's public API.
//!
//! Times `update` at 320x200 for every effect in `EFFECT_IDS` order, after a warm-up,
//! over a fixed frame count, and prints microseconds per update. Run in release only:
//!
//! `powershell -NoProfile -File tools/with-build-lock.ps1 cargo run --release -p canvas-effects --example timing`

use canvas_proto::{Canvas, Inputs};
use std::time::Instant;

const FRAMES: u32 = 600;

fn main() {
    println!("{:>16} {:>12}", "effect", "us/update");
    for id in canvas_proto::EFFECT_IDS {
        let mut e = canvas_effects::create(id).expect(id);
        let s = e.schema();
        e.init(&s.defaults());
        let mut canvas = Canvas::new(320, 200);
        let inputs = Inputs::default();
        for _ in 0..30 {
            e.update(1.0 / 30.0, &mut canvas, &inputs);
        }
        let start = Instant::now();
        for _ in 0..FRAMES {
            e.update(1.0 / 30.0, &mut canvas, &inputs);
        }
        let us = start.elapsed().as_secs_f64() * 1_000_000.0 / f64::from(FRAMES);
        println!("{id:>16} {us:>12.1}");
    }
}
