//! The tile quality bar, over every built-in and stock effect (issue #16).
//!
//! Headless: no daemon, no device, no hardware. Each effect is rendered with the same
//! `thumbnail::render` the API serves, scored with `thumbnail::score` and held to
//! `thumbnail::problems`. A tile under the bar fails here unless its recipe says it is
//! dark on purpose (and that exemption covers the lit share only).
//!
//! `cargo test -p canvas-daemon --test tile_quality -- --nocapture` prints the table.

use canvas_daemon::thumbnail::{self, TileScore};
use canvas_daemon::user_effects::{html_builder, STOCK_EFFECTS};
use canvas_proto::props::{Props, ThumbnailRecipe};
use canvas_proto::traits::Effect;
use canvas_proto::EFFECT_IDS;

/// Every effect the library ships: the built-ins, then the stock HTML effects.
fn shipped() -> Vec<(String, Box<dyn Effect>)> {
    let mut out: Vec<(String, Box<dyn Effect>)> = Vec::new();
    for id in EFFECT_IDS {
        out.push((id.to_string(), canvas_effects::create(id).expect("a built-in effect builds")));
    }
    let build = html_builder();
    for (stem, source) in STOCK_EFFECTS {
        let id = format!("html:{stem}");
        out.push((id.clone(), build(&id, source).expect("a stock effect builds")));
    }
    out
}

fn measure(effect: &mut dyn Effect) -> (TileScore, Option<ThumbnailRecipe>) {
    let schema = effect.schema();
    // The budget the daemon uses is for release builds; a debug build of a paint effect is slower.
    let body = thumbnail::render_within(effect, &Props::new(), std::time::Duration::from_secs(600)).expect("the render fits the test budget");
    (thumbnail::score(&body).expect("the render is well formed"), schema.thumbnail)
}

fn print_table(rows: &[(String, TileScore, Option<ThumbnailRecipe>)]) {
    println!(
        "{:<18} {:>14} {:>8} {:>10} {:>10} {:>7} {:>7} {:>7}  recipe",
        "effect", "lit first>last", "lit avg", "rows lit", "cols lit", "seam", "step", "max"
    );
    for (id, s, recipe) in rows {
        let note = match recipe {
            None => "default".to_string(),
            Some(r) => format!(
                "warm {} s, loop {} s, fade {}{}",
                r.warm_up_s,
                r.loop_s,
                r.crossfade_frames,
                r.dark_on_purpose.as_deref().map(|d| format!(", dark: {d}")).unwrap_or_default()
            ),
        };
        println!(
            "{:<18} {:>5.0} > {:>4.0} % {:>7.0} % {:>5} / {} {:>5} / {} {:>6.1}% {:>6.1}% {:>6.1}%  {}",
            id,
            s.lit_first * 100.0,
            s.lit_last * 100.0,
            s.lit_avg * 100.0,
            s.rows_lit,
            thumbnail::HEIGHT,
            s.cols_lit,
            thumbnail::WIDTH,
            s.seam * 100.0,
            s.step * 100.0,
            s.max_step * 100.0,
            note
        );
    }
}

/// Holds the built-in effects (`stock` false) or the stock HTML effects (`stock` true) to the bar.
fn hold_to_the_bar(stock: bool) {
    let mut rows = Vec::new();
    let mut failures = Vec::new();
    for (id, mut effect) in shipped().into_iter().filter(|(id, _)| id.starts_with("html:") == stock) {
        let (score, recipe) = measure(effect.as_mut());
        for problem in thumbnail::problems(&score, recipe.as_ref()) {
            failures.push(format!("{id}: {problem}"));
        }
        rows.push((id, score, recipe));
    }
    assert!(!rows.is_empty(), "no effects were measured");
    print_table(&rows);
    assert!(failures.is_empty(), "tiles under the bar:\n{}", failures.join("\n"));
}

#[test]
fn every_built_in_tile_passes_the_bar_or_says_why_not() {
    hold_to_the_bar(false);
}

/// The paint effects fit their 100 ms frame budget in release; CI runs this one with
/// `--release --include-ignored`.
#[test]
#[cfg_attr(debug_assertions, ignore = "the paint effects fit the 100 ms frame budget in release; CI runs these with --release")]
fn every_stock_tile_passes_the_bar_or_says_why_not() {
    hold_to_the_bar(true);
}

#[test]
fn the_test_covers_the_fourteen_shipped_effects_by_name() {
    let ids: Vec<String> = shipped().into_iter().map(|(id, _)| id).collect();
    let expected = [
        "solid", "gradient", "rainbow_wave", "spiral", "breathing", "color_cycle", "aurora", "rain", "fire",
        "screen_ambience", "audio_spectrum", "html:ink-drop", "html:wet-smear", "html:palette-knife",
    ];
    // A new effect that is not in this list is not yet held to the bar: add it here.
    assert_eq!(ids, expected);
}

/// A tile that is a still picture reads as a broken tile for an effect that follows motion
/// (a spectrum, a screen, a flame): each of these has to move, and none may cut hard.
#[test]
fn the_tiles_of_moving_effects_move_and_do_not_cut() {
    let mut failures = Vec::new();
    for (id, mut effect) in shipped() {
        let wants_motion = matches!(id.as_str(), "audio_spectrum" | "screen_ambience" | "fire" | "breathing");
        if !wants_motion && !id.starts_with("html:") {
            continue;
        }
        if id.starts_with("html:") && !cfg!(not(debug_assertions)) {
            continue; // the paint effects are measured in release
        }
        let (score, _) = measure(effect.as_mut());
        if wants_motion && score.step < 0.01 {
            failures.push(format!("{id}: the tile barely moves (step {:.2} %)", score.step * 100.0));
        }
        if score.max_step > 0.25 {
            failures.push(format!("{id}: the tile cuts hard between two frames ({:.0} %)", score.max_step * 100.0));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_dark_tile_without_a_stated_reason_is_a_problem_and_with_one_it_is_not() {
    let dark = TileScore {
        lit_first: 0.1,
        lit_last: 0.1,
        lit_avg: 0.1,
        rows_lit: thumbnail::HEIGHT as usize,
        cols_lit: thumbnail::WIDTH as usize,
        seam: 0.0,
        step: 0.0,
        max_step: 0.0,
    };
    assert_eq!(thumbnail::problems(&dark, None).len(), 1);
    let on_purpose = ThumbnailRecipe { dark_on_purpose: Some("a spark on black".into()), ..Default::default() };
    assert!(thumbnail::problems(&dark, Some(&on_purpose)).is_empty());

    // The exemption never covers the seam or the coverage.
    let popping = TileScore { seam: 0.5, rows_lit: 8, ..dark };
    assert_eq!(thumbnail::problems(&popping, Some(&on_purpose)).len(), 2);
}
