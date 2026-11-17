//! The OKLab blend and Spin against the vectors the editor's TypeScript shares (#468).
//!
//! `app/web/src/lib/__tests__/spin.vectors.json` is the agreement between this crate
//! (`pattern_spin.rs`) and the editor (`lib/oklab.ts`, `lib/spin.ts`): the editor's tests compare
//! their own output with the same file, so the preview and the lights cannot drift apart.
//!
//! The `oklab` section is Björn Ottosson's published values (https://bottosson.github.io/posts/oklab/),
//! independent of both implementations. The `mix` and `spin` sections' `expect` fields are what
//! this crate produces; set `CANVAS_WRITE_VECTORS=1` to rewrite them after a deliberate change.

use canvas_core::pattern_spin::{mix, to_oklab, SpinSource};
use canvas_proto::color::Rgb8;
use serde_json::Value;
use std::path::PathBuf;

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/web/src/lib/__tests__/spin.vectors.json")
}

fn hex(c: Rgb8) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

fn parse(text: &str) -> Rgb8 {
    text.parse().unwrap_or_else(|e| panic!("{text} is a colour: {e:?}"))
}

fn made(vectors: &Value) -> (Vec<String>, Value) {
    let mut out = vectors.clone();
    let mut tally = Vec::new();
    for case in out["mix"].as_array_mut().expect("a mix list") {
        let (from, to) = (parse(case["from"].as_str().unwrap()), parse(case["to"].as_str().unwrap()));
        let got = hex(mix(from, to, case["t"].as_f64().unwrap()));
        tally.push(format!("mix {} {} {} -> {got}", hex(from), hex(to), case["t"]));
        case["expect"] = Value::String(got);
    }
    for case in out["spin"].as_array_mut().expect("a spin list") {
        let colours: Vec<Rgb8> =
            case["colours"].as_array().unwrap().iter().map(|c| parse(c.as_str().unwrap())).collect();
        let mut frame = Vec::new();
        SpinSource::new(colours).at(
            case["turns_per_s"].as_f64().unwrap(),
            case["t"].as_f64().unwrap(),
            &mut frame,
        );
        let got: Vec<Value> = frame.iter().map(|c| Value::String(hex(*c))).collect();
        tally.push(format!("spin {} -> {} LEDs", case["name"], got.len()));
        case["expect"] = Value::Array(got);
    }
    (tally, out)
}

#[test]
fn oklab_matches_ottossons_published_values() {
    let text = std::fs::read_to_string(path()).expect("the vector file is read");
    let vectors: Value = serde_json::from_str(&text).expect("the vector file is JSON");
    for case in vectors["oklab"].as_array().unwrap() {
        let c = parse(case["hex"].as_str().unwrap());
        let lab = to_oklab(c);
        for (i, want) in case["lab"].as_array().unwrap().iter().enumerate() {
            let want = want.as_f64().unwrap();
            assert!((lab[i] - want).abs() < 2e-5, "{}: lab[{i}] is {} (published {want})", hex(c), lab[i]);
        }
    }
}

#[test]
fn the_checked_in_vectors_are_what_the_rust_blend_and_spin_make() {
    let text = std::fs::read_to_string(path()).expect("the vector file is read");
    let vectors: Value = serde_json::from_str(&text).expect("the vector file is JSON");
    let (tally, now) = made(&vectors);
    if std::env::var_os("CANVAS_WRITE_VECTORS").is_some() {
        let mut pretty = serde_json::to_string_pretty(&now).unwrap();
        pretty.push('\n');
        std::fs::write(path(), pretty).expect("the vector file is written");
        for line in &tally {
            println!("{line}");
        }
        return;
    }
    for kind in ["mix", "spin"] {
        let (a, b) = (vectors[kind].as_array().unwrap(), now[kind].as_array().unwrap());
        assert_eq!(a.len(), b.len());
        for (checked_in, made) in a.iter().zip(b) {
            assert_eq!(checked_in["expect"], made["expect"], "{kind}: {checked_in}");
        }
    }
}
