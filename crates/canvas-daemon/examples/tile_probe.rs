//! Throwaway: renders one effect with a recipe given on the command line and prints its score.
//! cargo run -p canvas-daemon --example tile_probe -- <id> <warm> <loop> <fade> [key=value ...] [--sheet dir]
//! cargo run -p canvas-daemon --example tile_probe -- <id> own [--file effect.html] [--repeat n] [--sheet dir]
//!
//! `own` renders the tile with the effect's own recipe (what the library serves) instead of one given
//! on the command line. `--file` loads an `html:` effect from a file on disk instead of the stock copy.
use canvas_proto::props::{PropValue, PropertySchema, Props, ThumbnailRecipe};
use canvas_proto::traits::{Canvas, Effect, Inputs};

struct Wrap(Box<dyn Effect>, ThumbnailRecipe);

impl Effect for Wrap {
    fn schema(&self) -> PropertySchema {
        let mut s = self.0.schema();
        s.thumbnail = Some(self.1.clone());
        s
    }
    fn init(&mut self, props: &Props) {
        self.0.init(props)
    }
    fn update(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs) {
        self.0.update(dt, canvas, inputs)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let id = &args[0];
    let own = args[1] == "own";
    let mut recipe = if own {
        ThumbnailRecipe::default()
    } else {
        ThumbnailRecipe {
            warm_up_s: args[1].parse().unwrap(),
            loop_s: args[2].parse().unwrap(),
            crossfade_frames: args[3].parse().unwrap(),
            ..Default::default()
        }
    };
    let mut sheet = None;
    let mut file: Option<String> = None;
    let mut budget = std::time::Duration::from_secs(900);
    let mut repeat = 1usize;
    let mut steps = false;
    let mut i = if own { 2 } else { 4 };
    while i < args.len() {
        if args[i] == "--steps" {
            steps = true;
            i += 1;
            continue;
        }
        if args[i] == "--file" {
            file = Some(args[i + 1].clone());
            i += 2;
            continue;
        }
        if args[i] == "--sheet" {
            sheet = Some(args[i + 1].clone());
            i += 2;
            continue;
        }
        if args[i] == "--repeat" {
            repeat = args[i + 1].parse().expect("a number of renders");
            i += 2;
            continue;
        }
        if args[i] == "--budget" {
            budget = std::time::Duration::from_secs(args[i + 1].parse().expect("a budget in seconds"));
            i += 2;
            continue;
        }
        let (k, v) = args[i].split_once('=').unwrap();
        let value = if let Ok(n) = v.parse::<f64>() {
            PropValue::Number(n)
        } else if v.starts_with('#') {
            PropValue::Color(v.parse().unwrap())
        } else {
            PropValue::Choice(v.to_string())
        };
        recipe.props.insert(k.to_string(), value);
        i += 1;
    }
    let mut first: Option<Vec<u8>> = None;
    let mut identical = true;
    let started = std::time::Instant::now();
    for round in 0..repeat {
        let inner: Box<dyn Effect> = if let Some(stem) = id.strip_prefix("html:") {
            let src = match &file {
                Some(path) => std::fs::read_to_string(path).expect("the effect file"),
                None => canvas_daemon::user_effects::STOCK_EFFECTS.iter().find(|(s, _)| *s == stem).unwrap().1.to_string(),
            };
            canvas_daemon::user_effects::html_builder_seeded(canvas_daemon::thumbnail::TILE_SEED)(id, &src).unwrap()
        } else {
            canvas_effects::create(id).unwrap()
        };
        let mut effect: Box<dyn Effect> = if own { inner } else { Box::new(Wrap(inner, recipe.clone())) };
        let body = canvas_daemon::thumbnail::render_within(effect.as_mut(), &Props::new(), budget)
            .unwrap_or_else(|why| panic!("{why}"));
        let s = canvas_daemon::thumbnail::score(&body).unwrap();
        println!(
            "{id}: lit {:.0}% ({:.0}>{:.0}) rows {} cols {} seam {:.1}% step {:.1}% max {:.1}% band {}  [{:.1?}]",
            s.lit_avg * 100.0, s.lit_first * 100.0, s.lit_last * 100.0, s.rows_lit, s.cols_lit,
            s.seam * 100.0, s.step * 100.0, s.max_step * 100.0, s.band_rows, started.elapsed()
        );
        if steps && round == 0 {
            // The step between each pair of frames, as a share of full scale: where the tile moves.
            let frame_len = 64 * 40 * 3;
            let frames: Vec<&[u8]> = body[6..].chunks_exact(frame_len).collect();
            let list: Vec<String> = frames
                .windows(2)
                .map(|p| {
                    let sum: u64 = p[0].iter().zip(p[1]).map(|(a, b)| (*a as i32 - *b as i32).unsigned_abs() as u64).sum();
                    format!("{:.1}", sum as f64 / (frame_len as f64 * 255.0) * 100.0)
                })
                .collect();
            println!("   steps %: {}", list.join(" "));
        }
        let problems = canvas_daemon::thumbnail::problems(&s, effect.schema().thumbnail.as_ref());
        for problem in &problems {
            println!("   problem: {problem}");
        }
        match &first {
            None => first = Some(body),
            Some(before) => {
                if before != &body {
                    identical = false;
                    println!("   render {round} differs from the first");
                }
            }
        }
    }
    if repeat > 1 {
        println!("{id}: {repeat} renders, identical: {identical}");
    }
    if let Some(dir) = sheet {
        std::fs::create_dir_all(&dir).unwrap();
        let name = id.replace(':', "-");
        let body = first.expect("a render");
        canvas_daemon::cli::write_tile_sheet(&std::path::Path::new(&dir).join(format!("{name}-sheet.png")), &body).unwrap();
    }
}
