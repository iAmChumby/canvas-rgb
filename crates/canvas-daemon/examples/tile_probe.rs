//! Throwaway: renders one effect with a recipe given on the command line and prints its score.
//! cargo run -p canvas-daemon --example tile_probe -- <id> <warm> <loop> <fade> [key=value ...] [--sheet dir]
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
    let mut recipe = ThumbnailRecipe {
        warm_up_s: args[1].parse().unwrap(),
        loop_s: args[2].parse().unwrap(),
        crossfade_frames: args[3].parse().unwrap(),
        ..Default::default()
    };
    let mut sheet = None;
    let mut budget = std::time::Duration::from_secs(900);
    let mut i = 4;
    while i < args.len() {
        if args[i] == "--sheet" {
            sheet = Some(args[i + 1].clone());
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
    let inner: Box<dyn Effect> = if let Some(stem) = id.strip_prefix("html:") {
        let (_, src) = canvas_daemon::user_effects::STOCK_EFFECTS.iter().find(|(s, _)| *s == stem).unwrap();
        canvas_daemon::user_effects::html_builder()(id, src).unwrap()
    } else {
        canvas_effects::create(id).unwrap()
    };
    let mut effect = Wrap(inner, recipe);
    let started = std::time::Instant::now();
    let body = canvas_daemon::thumbnail::render_within(&mut effect, &Props::new(), budget)
        .unwrap_or_else(|why| panic!("{why}"));
    let s = canvas_daemon::thumbnail::score(&body).unwrap();
    println!(
        "{id}: lit {:.0}% ({:.0}>{:.0}) rows {} cols {} seam {:.1}% step {:.1}% max {:.1}%  [{:.1?}]",
        s.lit_avg * 100.0, s.lit_first * 100.0, s.lit_last * 100.0, s.rows_lit, s.cols_lit,
        s.seam * 100.0, s.step * 100.0, s.max_step * 100.0, started.elapsed()
    );
    if let Some(dir) = sheet {
        std::fs::create_dir_all(&dir).unwrap();
        let name = id.replace(':', "-");
        canvas_daemon::cli::write_tile_sheet(&std::path::Path::new(&dir).join(format!("{name}-sheet.png")), &body).unwrap();
    }
}
