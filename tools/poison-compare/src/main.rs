//! poison-compare: statistical comparison of the Rust Poison port against SignalRGB's original
//! `effect.html`, both at 320x200 with the same properties, N runs each.
//!
//! Exit codes: 0 PASS, 1 FAIL or ERROR, 2 UNKNOWN (the port does not exist yet).
//!
//! Usage: cargo run -p poison-compare --release -- [--runs 20] [--frames 600]
//!        [--effect <path to effect.html>] [--keep]

mod stats;

use canvas_proto::{Canvas, Effect, Inputs, PropValue, Props, Rgb8};
use stats::{
    check_mean, check_share, frame_stats, summarize, Agg, Check, FrameStats, SideSummary,
    PALETTE_DELTA_E,
};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 200;
/// One step per frame at 60 fps, the same step the original gets from requestAnimationFrame.
const DT: f32 = 1.0 / 60.0;

/// The compare properties (brief): Poison's declared defaults.
const COMPARE_BG: &str = "#2d0075";
const COMPARE_COLORS: [&str; 3] = ["#6000fc", "#b300ff", "#8a42ff"];
const COMPARE_SPEED: f64 = 35.0;

const PALETTE_LABELS: [&str; 4] = ["bg", "color1", "color2", "color3"];

struct Args {
    runs: usize,
    frames: usize,
    effect: Option<PathBuf>,
    keep: bool,
}

fn usage() -> &'static str {
    "usage: poison-compare [--runs N] [--frames N] [--effect <effect.html>] [--keep]"
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { runs: 20, frames: 600, effect: None, keep: false };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--runs" => args.runs = positive(&flag, it.next())?,
            "--frames" => args.frames = positive(&flag, it.next())?,
            "--effect" => {
                args.effect = Some(PathBuf::from(it.next().ok_or("--effect needs a path")?))
            }
            "--keep" => args.keep = true,
            "-h" | "--help" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}; {}", usage())),
        }
    }
    Ok(args)
}

fn positive(flag: &str, value: Option<String>) -> Result<usize, String> {
    let v = value.ok_or(format!("{flag} needs a value"))?;
    match v.parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(format!("{flag} must be a positive integer, got {v:?}")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ERROR: {msg}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let args = parse_args()?;
    let work = std::env::temp_dir().join(format!("poison-compare-{}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|e| format!("cannot create {}: {e}", work.display()))?;
    let result = run_in(&args, &work);
    if !args.keep {
        let _ = std::fs::remove_dir_all(&work);
    }
    result
}

fn palette() -> Result<[[u8; 3]; 4], String> {
    let parse = |hex: &str| -> Result<[u8; 3], String> {
        let c: Rgb8 = hex.parse().map_err(|e| format!("{e}"))?;
        Ok([c.r, c.g, c.b])
    };
    Ok([
        parse(COMPARE_BG)?,
        parse(COMPARE_COLORS[0])?,
        parse(COMPARE_COLORS[1])?,
        parse(COMPARE_COLORS[2])?,
    ])
}

fn run_in(args: &Args, work: &Path) -> Result<ExitCode, String> {
    let palette = palette()?;
    let original_frames = render_original(args, work, &palette)?;
    let original = summarize(&original_frames).ok_or("original side has no runs")?;

    if canvas_effects::create("poison").is_none() {
        print_side("Original (SignalRGB effect.html)", &original, args);
        println!("UNKNOWN: canvas-effects has no poison effect yet");
        return Ok(ExitCode::from(2));
    }

    // Port side: each run gets a fresh effect, initialised with its own schema defaults.
    let mut port_pixels = Vec::with_capacity(args.runs);
    let mut props_ok = true;
    for run in 0..args.runs {
        let mut effect = canvas_effects::create("poison").ok_or("poison vanished between calls")?;
        let props = effect.schema().defaults();
        if run == 0 {
            props_ok = print_props_check(&props);
        }
        effect.init(&props);
        port_pixels.push(render_port(effect.as_mut(), args.frames));
    }
    let port_stats: Vec<FrameStats> = port_pixels
        .iter()
        .map(|px| frame_stats(px, &palette).ok_or("port frame is empty"))
        .collect::<Result<_, _>>()?;
    let port = summarize(&port_stats).ok_or("port side has no runs")?;

    let mut checks = Vec::new();
    for (i, name) in ["mean R", "mean G", "mean B"].iter().enumerate() {
        checks.push(check_mean(name, original.mean[i].mean, port.mean[i].mean));
    }
    for (i, label) in PALETTE_LABELS.iter().enumerate() {
        let name = format!("share {label} {}", hex(palette[i]));
        checks.push(check_share(&name, original.share[i].mean, port.share[i].mean));
    }

    print_side_pair(&original, &port, args);
    print_checks(&checks);
    println!(
        "Palette: nearest palette color by CIE76 dE, counted when within dE {PALETTE_DELTA_E}. \
         Other (not within dE {PALETTE_DELTA_E} of any palette color): original {}, port {}.",
        fmt_agg(Some(original.other), true),
        fmt_agg(Some(port.other), true)
    );

    let all_pass = props_ok && checks.iter().all(|c| c.pass);
    println!("RESULT: {}", if all_pass { "PASS" } else { "FAIL" });
    Ok(if all_pass { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

/// Runs the original through `tools/poison-compare/js/render.mjs` and splits its output per run.
fn render_original(args: &Args, work: &Path, palette: &[[u8; 3]; 4]) -> Result<Vec<FrameStats>, String> {
    let out = work.join("original.rgb");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("js").join("render.mjs");
    let mut cmd = Command::new("node");
    cmd.arg(&script)
        .arg("--out")
        .arg(&out)
        .arg("--runs")
        .arg(args.runs.to_string())
        .arg("--frames")
        .arg(args.frames.to_string())
        .arg("--bg")
        .arg(COMPARE_BG)
        .arg("--color1")
        .arg(COMPARE_COLORS[0])
        .arg("--color2")
        .arg(COMPARE_COLORS[1])
        .arg("--color3")
        .arg(COMPARE_COLORS[2])
        .arg("--speed")
        .arg(COMPARE_SPEED.to_string());
    if let Some(effect) = &args.effect {
        cmd.arg("--effect").arg(effect);
    }
    let output = cmd.output().map_err(|e| format!("cannot start node (render.mjs): {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "render.mjs failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    println!("render.mjs: {}", String::from_utf8_lossy(&output.stdout).trim());

    let bytes = std::fs::read(&out).map_err(|e| format!("cannot read {}: {e}", out.display()))?;
    let per_run = (WIDTH * HEIGHT * 3) as usize;
    if bytes.len() != per_run * args.runs {
        return Err(format!(
            "render.mjs wrote {} bytes, expected {} runs x {per_run}",
            bytes.len(),
            args.runs
        ));
    }
    bytes
        .chunks_exact(per_run)
        .map(|px| frame_stats(px, palette).ok_or_else(|| "original frame is empty".to_string()))
        .collect()
}

/// Steps the port's effect `frames` times on a fresh canvas and returns packed 8-bit RGB.
fn render_port(effect: &mut dyn Effect, frames: usize) -> Vec<u8> {
    let mut canvas = Canvas::new(WIDTH, HEIGHT);
    let inputs = Inputs::default();
    for _ in 0..frames {
        effect.update(DT, &mut canvas, &inputs);
    }
    let mut out = Vec::with_capacity(canvas.pixels.len() * 3);
    for px in &canvas.pixels {
        let c = Rgb8::from_f32(*px);
        out.extend_from_slice(&[c.r, c.g, c.b]);
    }
    out
}

/// Confirms the port's declared defaults are the compare values. Returns false on mismatch.
/// Keys are the original effect.html's own names (also the JS globals render.mjs sets).
const PROP_KEYS: [(&str, &str); 4] = [
    ("bgColor", COMPARE_BG),
    ("color1", COMPARE_COLORS[0]),
    ("color2", COMPARE_COLORS[1]),
    ("color3", COMPARE_COLORS[2]),
];
const SPEED_KEY: &str = "speedRaw";

fn print_props_check(props: &Props) -> bool {
    let mut ok = true;
    for (key, hex_value) in PROP_KEYS {
        let want: Option<Rgb8> = hex_value.parse().ok();
        let got = match props.get(key) {
            Some(PropValue::Color(c)) => Some(*c),
            _ => None,
        };
        ok &= want.is_some() && got == want;
    }
    ok &= matches!(props.get(SPEED_KEY), Some(PropValue::Number(n)) if *n == COMPARE_SPEED);
    println!(
        "Port schema defaults: bgColor {COMPARE_BG}, color1 {}, color2 {}, color3 {}, speedRaw {COMPARE_SPEED}: {}",
        COMPARE_COLORS[0],
        COMPARE_COLORS[1],
        COMPARE_COLORS[2],
        if ok { "PASS" } else { "FAIL" }
    );
    ok
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn fmt_agg(a: Option<Agg>, pct: bool) -> String {
    match a {
        None => "not run".to_string(),
        Some(a) => {
            let v = if pct { a.mean * 100.0 } else { a.mean };
            let unit = if pct { "%" } else { "" };
            match a.sd {
                Some(sd) => format!("{v:.2}{unit} ± {:.2}", if pct { sd * 100.0 } else { sd }),
                None => format!("{v:.2}{unit} (1 run)"),
            }
        }
    }
}

fn print_side(title: &str, side: &SideSummary, args: &Args) {
    println!("\n## {title}: {} runs x {} frames at {WIDTH}x{HEIGHT}\n", side.runs, args.frames);
    println!("| statistic | mean ± sd across runs |");
    println!("|---|---|");
    for (i, c) in ["R", "G", "B"].iter().enumerate() {
        println!("| mean {c} (0..255) | {} |", fmt_agg(Some(side.mean[i]), false));
    }
    for (i, label) in PALETTE_LABELS.iter().enumerate() {
        println!("| share {label} (%) | {} |", fmt_agg(Some(side.share[i]), true));
    }
    println!("| other (%) | {} |", fmt_agg(Some(side.other), true));
}

fn print_side_pair(original: &SideSummary, port: &SideSummary, args: &Args) {
    println!("\n## Statistics: {} runs x {} frames at {WIDTH}x{HEIGHT}\n", original.runs, args.frames);
    println!("| statistic | original mean ± sd | port mean ± sd |");
    println!("|---|---|---|");
    for (i, c) in ["R", "G", "B"].iter().enumerate() {
        println!(
            "| mean {c} (0..255) | {} | {} |",
            fmt_agg(Some(original.mean[i]), false),
            fmt_agg(Some(port.mean[i]), false)
        );
    }
    for (i, label) in PALETTE_LABELS.iter().enumerate() {
        println!(
            "| share {label} (%) | {} | {} |",
            fmt_agg(Some(original.share[i]), true),
            fmt_agg(Some(port.share[i]), true)
        );
    }
}

fn print_checks(checks: &[Check]) {
    println!("\n## Checks\n");
    println!("| check | original | port | tolerance | delta | result |");
    println!("|---|---|---|---|---|---|");
    for c in checks {
        let is_share = c.name.starts_with("share");
        let unit = if is_share { "%" } else { "" };
        let tol = if is_share { format!("±{:.1} pp", c.tolerance) } else { format!("±{:.2}", c.tolerance) };
        let delta_unit = if is_share { " pp" } else { "" };
        println!(
            "| {} | {:.2}{unit} | {:.2}{unit} | {tol} | {:+.2}{delta_unit} | {} |",
            c.name,
            c.original,
            c.port,
            c.delta,
            if c.pass { "PASS" } else { "FAIL" }
        );
    }
}
