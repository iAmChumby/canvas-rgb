//! Times and loop-checks one stock HTML effect in the real engine (canvas-html + QuickJS),
//! with no daemon, no device and no window.
//!
//! Usage: cargo run --release -p canvas-daemon --example measure_stock_effects -- <file.html> <period_seconds>
//!
//! Prints, for the 30 fps default tick and for a 60 fps tick: the per-update time (mean,
//! p95, max, in ms) over one FULL period, first cycle and second cycle separately, plus
//! the loop-closure differences (mean absolute difference per 8-bit channel) and the
//! process's peak memory at the end.

use canvas_proto::{Canvas, Effect, Inputs, Props};

const W: u32 = 320;
const H: u32 = 200;

fn load(src: &str) -> canvas_html::HtmlEffect {
    let mut e = canvas_html::HtmlEffect::from_source_seeded("measure", src, &Props::new(), 0x9e37_79b9)
        .expect("the effect loads");
    let schema = e.schema();
    e.init(&schema.defaults());
    e
}

fn stats(mut v: Vec<f64>) -> (f64, f64, f64) {
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = v[((v.len() as f64) * 0.95) as usize - 1];
    (mean, p95, *v.last().unwrap())
}

fn bytes(c: &Canvas) -> Vec<u8> {
    c.pixels
        .iter()
        .flat_map(|p| p.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8))
        .collect()
}

fn mad(a: &[u8], b: &[u8]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).abs() as f64).sum::<f64>() / a.len() as f64
}

fn timing(src: &str, period: f64, fps: u32, cycles: usize) {
    let mut e = load(src);
    let mut canvas = Canvas::new(W, H);
    let inputs = Inputs::default();
    let dt = 1.0 / fps as f32;
    let n = (period * fps as f64).round() as usize;
    for cycle in 1..=cycles {
        let mut t = Vec::with_capacity(n);
        for _ in 0..n {
            let s = std::time::Instant::now();
            e.update(dt, &mut canvas, &inputs);
            t.push(s.elapsed().as_secs_f64() * 1000.0);
        }
        let (mean, p95, max) = stats(t);
        println!(
            "timing fps={fps} cycle={cycle} updates={n} mean_ms={mean:.2} p95_ms={p95:.2} max_ms={max:.2} errors={} {:?}",
            e.errors().len(),
            e.errors()
        );
    }
}

fn closure(src: &str, period: f64) {
    // 60 fps ticks of exactly 1/60 s: one update is one script frame, so a period is an
    // exact whole number of script frames.
    let n = (period * 60.0).round() as usize;
    let mut e = load(src);
    let mut canvas = Canvas::new(W, H);
    let inputs = Inputs::default();
    let dt = 1.0f32 / 60.0;
    let mut frames: std::collections::BTreeMap<usize, Vec<u8>> = Default::default();
    let sample = |k: usize| k.is_multiple_of(24) || k == 1 || k == n + 1 || k == n / 2 || k == n + n / 2 || k == n || k == 2 * n;
    let mut third: Vec<(usize, f64)> = Vec::new();
    for k in 1..=3 * n {
        e.update(dt, &mut canvas, &inputs);
        if sample(k) || (k > 2 * n && k <= 3 * n && (k - n).is_multiple_of(24)) {
            let b = bytes(&canvas);
            if k > 2 * n {
                if let Some(prev) = frames.get(&(k - n)) {
                    third.push((k, mad(prev, &b)));
                }
            }
            if k <= 2 * n {
                frames.insert(k, b);
            }
        }
    }
    let g = |k: usize| frames.get(&k).unwrap();
    println!("closure period_frames={n}");
    println!("closure first_cycle  t=1f vs t=T+1f   mad={:.4}", mad(g(1), g(n + 1)));
    println!("closure steady       t=T  vs t=2T     mad={:.4}", mad(g(n), g(2 * n)));
    println!("closure steady       t=T/2 vs t=T+T/2 mad={:.4}", mad(g(n / 2), g(n + n / 2)));
    println!("closure control      t=T  vs t=T+T/2  mad={:.4} (must be large)", mad(g(n), g(n + n / 2)));
    // all sampled phases, cycle 2 vs cycle 3
    let max = third.iter().map(|x| x.1).fold(0.0, f64::max);
    println!("closure all-phase cycle2 vs cycle3 samples={} max_mad={max:.4}", third.len());
    // the first cycle against the second, same sampled phases
    let mut worst = 0.0f64;
    let mut cnt = 0;
    for k in (24..=n).step_by(24) {
        if let (Some(a), Some(b)) = (frames.get(&k), frames.get(&(k + n))) {
            worst = worst.max(mad(a, b));
            cnt += 1;
        }
    }
    println!("closure all-phase cycle1 vs cycle2 samples={cnt} max_mad={worst:.4}");
}

/// Runs two files side by side for one period and prints how far apart their pictures are.
fn compare(a_src: &str, b_src: &str, period: f64) {
    let n = (period * 60.0).round() as usize;
    let (mut a, mut b) = (load(a_src), load(b_src));
    let (mut ca, mut cb) = (Canvas::new(W, H), Canvas::new(W, H));
    let inputs = Inputs::default();
    let dt = 1.0f32 / 60.0;
    let (mut worst, mut sum, mut cnt) = (0.0f64, 0.0f64, 0usize);
    let mut worst_px = 0i32;
    for k in 1..=n {
        a.update(dt, &mut ca, &inputs);
        b.update(dt, &mut cb, &inputs);
        if k % 12 == 0 {
            let (x, y) = (bytes(&ca), bytes(&cb));
            let d = mad(&x, &y);
            worst = worst.max(d);
            sum += d;
            cnt += 1;
            worst_px = worst_px.max(x.iter().zip(&y).map(|(p, q)| (*p as i32 - *q as i32).abs()).max().unwrap());
        }
    }
    println!(
        "compare frames_sampled={cnt} mean_mad={:.4} max_mad={worst:.4} max_single_channel_diff={worst_px} errors_a={} errors_b={}",
        sum / cnt as f64,
        a.errors().len(),
        b.errors().len()
    );
}

/// Writes the picture at 60 fps frame `frame` as a binary PPM (for looking at with an image viewer).
fn dump(src: &str, frame: usize, out: &str) {
    let mut e = load(src);
    let mut canvas = Canvas::new(W, H);
    let inputs = Inputs::default();
    for _ in 0..frame {
        e.update(1.0 / 60.0, &mut canvas, &inputs);
    }
    let mut data = format!("P6
{W} {H}
255
").into_bytes();
    data.extend(bytes(&canvas));
    std::fs::write(out, data).expect("write the ppm");
}

fn memory() {
    let pid = std::process::id();
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("$p=Get-Process -Id {pid}; 'memory peak_working_set_mb={{0:N1}} working_set_mb={{1:N1}} private_mb={{2:N1}}' -f ($p.PeakWorkingSet64/1MB),($p.WorkingSet64/1MB),($p.PrivateMemorySize64/1MB)"),
        ])
        .output();
    if let Ok(o) = out {
        print!("{}", String::from_utf8_lossy(&o.stdout));
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("file.html");
    let period: f64 = args.next().expect("period seconds").parse().expect("a number");
    let mode = args.next().unwrap_or_else(|| "all".to_string());
    let src = std::fs::read_to_string(&path).expect("read the file");
    println!("effect {path} period={period}s");
    if mode == "all" || mode == "timing" {
        timing(&src, period, 30, 2);
        timing(&src, period, 60, 2);
    }
    if mode == "quick" {
        timing(&src, period, 60, 1);
    }
    if mode == "all" || mode == "closure" {
        closure(&src, period);
    }
    if mode == "compare" {
        let other = std::fs::read_to_string(args.next().expect("other file")).expect("read");
        compare(&src, &other, period);
    }
    if mode == "dump" {
        let frame: usize = args.next().expect("frame").parse().expect("a number");
        dump(&src, frame, &args.next().expect("out.ppm"));
    }
    if mode == "baseline" {
        // nothing loaded
    }
    memory();
}
