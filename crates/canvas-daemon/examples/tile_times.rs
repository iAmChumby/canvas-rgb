//! Times every shipped tile at its shipped recipe, the way the daemon renders it.
//! cargo run --release -p canvas-daemon --example tile_times -- [reps] [id ...]
//!
//! Prints one line per effect: the seconds of each repetition, the slowest, and the margin
//! against `RENDER_BUDGET` (budget / slowest). Run it idle and again with the PC busy.
use canvas_daemon::thumbnail::{self, RENDER_BUDGET};
use canvas_daemon::user_effects::{html_builder_seeded, STOCK_EFFECTS};
use canvas_proto::props::Props;
use canvas_proto::traits::Effect;
use canvas_proto::EFFECT_IDS;

/// Builds a fresh effect each time it is called.
type Make = Box<dyn Fn() -> Box<dyn Effect>>;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let reps: usize = if args.first().is_some_and(|a| a.parse::<usize>().is_ok()) { args.remove(0).parse().unwrap() } else { 3 };
    let build = html_builder_seeded(thumbnail::TILE_SEED);
    let mut all: Vec<(String, Make)> = Vec::new();
    for id in EFFECT_IDS {
        let id = id.to_string();
        all.push((id.clone(), Box::new(move || canvas_effects::create(&id).expect("a built-in effect builds"))));
    }
    for (stem, source) in STOCK_EFFECTS {
        let id = format!("html:{stem}");
        let build = build.clone();
        let source = source.to_string();
        let id2 = id.clone();
        all.push((id, Box::new(move || build(&id2, &source).expect("a stock effect builds"))));
    }
    println!("RENDER_BUDGET {} s, {reps} repetitions each", RENDER_BUDGET.as_secs());
    println!("{:<20} {:>34} {:>9} {:>8}", "tile", "seconds per repetition", "slowest", "margin");
    for (id, make) in &all {
        if !args.is_empty() && !args.contains(id) {
            continue;
        }
        let mut times = Vec::new();
        for _ in 0..reps {
            let mut effect = make();
            let started = std::time::Instant::now();
            // The daemon's own call, with a budget long enough to time a render that would be given up.
            let result = thumbnail::render_within(effect.as_mut(), &Props::new(), std::time::Duration::from_secs(600));
            assert!(result.is_ok(), "{id}: {result:?}");
            times.push(started.elapsed().as_secs_f64());
        }
        let slowest = times.iter().cloned().fold(0.0, f64::max);
        let list: Vec<String> = times.iter().map(|t| format!("{t:.2}")).collect();
        println!("{id:<20} {:>34} {slowest:>9.2} {:>7.1}x", list.join(" "), RENDER_BUDGET.as_secs_f64() / slowest);
    }
}
