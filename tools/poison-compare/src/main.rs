//! poison-compare: statistical comparison of the HTML runtime's rendering of Poison
//! against Chromium's rendering of the same file, both at 320x200 with the same
//! properties, N runs each.
//!
//! The file itself is not in this repository: Poison is SignalRGB's effect, and the owner
//! installs it as a user HTML effect in `%APPDATA%\canvas-rgb\effects\poison.html`. The
//! tool takes that path as an argument (the default is the installed file) and fails with
//! a clear message when the file is not there.
//!
//! Exit codes: 0 PASS, 1 FAIL or ERROR.
//!
//! Usage: cargo run -p poison-compare --release -- [--runs 20] [--frames 600]
//!        [--effect <path to effect.html>] [--keep]

mod stats;

use canvas_html::HtmlEffect;
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

/// The compare properties: the values stored for the owner's instance of the effect.
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

/// The file the comparison runs by default: the effect file the owner installs as a
/// user HTML effect.
fn default_effect_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("canvas-rgb").join("effects").join("poison.html")
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

/// The property values both sides run with.
fn compare_props() -> Props {
    let mut props = Props::new();
    props.insert(
        "bgColor".to_string(),
        PropValue::Color(COMPARE_BG.parse().expect("a hex color")),
    );
    for (key, hex) in [
        ("color1", COMPARE_COLORS[0]),
        ("color2", COMPARE_COLORS[1]),
        ("color3", COMPARE_COLORS[2]),
    ] {
        props.insert(
            key.to_string(),
            PropValue::Color(hex.parse().expect("a hex color")),
        );
    }
    props.insert("speedRaw".to_string(), PropValue::Number(COMPARE_SPEED));
    props
}

fn run_in(args: &Args, work: &Path) -> Result<ExitCode, String> {
    let palette = palette()?;

    // The effect file: the owner's own copy, never one from this repository.
    let effect_path = args.effect.clone().unwrap_or_else(default_effect_path);
    if !effect_path.is_file() {
        return Err(format!(
            "the effect file is not at {}.\n\
             Copy SignalRGB's Poison effect.html there (the file the user effect is\n\
             installed from), or point the tool at it with --effect <path>.",
            effect_path.display()
        ));
    }
    let source = std::fs::read_to_string(&effect_path)
        .map_err(|e| format!("cannot read {}: {e}", effect_path.display()))?;
    println!("Effect file: {}", effect_path.display());

    let original_frames = render_original(args, work, &palette, &effect_path)?;
    let original = summarize(&original_frames).ok_or("original side has no runs")?;

    // Rust side: each run gets a fresh effect, initialised with the compare values.
    let mut port_pixels = Vec::with_capacity(args.runs);
    let mut props_ok = false;
    for run in 0..args.runs {
        let mut effect = HtmlEffect::from_source("html:poison", &source, &Props::new())
            .map_err(|e| format!("the effect file did not load: {e}"))?;
        let props = compare_props();
        if run == 0 {
            props_ok = print_props_check(&effect, &props);
        }
        effect.init(&props);
        port_pixels.push(render_port(&mut effect, args.frames));
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
fn render_original(
    args: &Args,
    work: &Path,
    palette: &[[u8; 3]; 4],
    effect_path: &Path,
) -> Result<Vec<FrameStats>, String> {
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
        .arg(COMPARE_SPEED.to_string())
        .arg("--effect")
        .arg(effect_path);
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

/// Confirms the effect file declares the properties the comparison drives, and that the
/// values pushed into it are the compare values. Returns false on a mismatch.
///
/// The keys are the original effect.html's own names (also the JS globals render.mjs
/// sets).
const PROP_KEYS: [(&str, &str); 4] = [
    ("bgColor", COMPARE_BG),
    ("color1", COMPARE_COLORS[0]),
    ("color2", COMPARE_COLORS[1]),
    ("color3", COMPARE_COLORS[2]),
];
const SPEED_KEY: &str = "speedRaw";

fn print_props_check(effect: &HtmlEffect, props: &Props) -> bool {
    let schema = effect.schema();
    let keys: Vec<&str> = schema.properties.iter().map(|p| p.key.as_str()).collect();

    // The file has to declare the properties the comparison drives, or the two sides
    // are not running the same effect.
    let mut ok = true;
    for (key, _) in PROP_KEYS {
        ok &= keys.contains(&key);
    }
    ok &= keys.contains(&SPEED_KEY);
    println!(
        "Effect properties: {}: {}",
        keys.join(", "),
        if ok { "PASS" } else { "FAIL" }
    );

    // The values both sides run with.
    let mut values_ok = true;
    for (key, hex_value) in PROP_KEYS {
        let want: Option<Rgb8> = hex_value.parse().ok();
        let got = match props.get(key) {
            Some(PropValue::Color(c)) => Some(*c),
            _ => None,
        };
        values_ok &= want.is_some() && got == want;
    }
    values_ok &= matches!(props.get(SPEED_KEY), Some(PropValue::Number(n)) if *n == COMPARE_SPEED);
    println!(
        "Compare values: bgColor {COMPARE_BG}, color1 {}, color2 {}, color3 {}, speedRaw {COMPARE_SPEED}: {}",
        COMPARE_COLORS[0],
        COMPARE_COLORS[1],
        COMPARE_COLORS[2],
        if values_ok { "PASS" } else { "FAIL" }
    );
    ok && values_ok
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
