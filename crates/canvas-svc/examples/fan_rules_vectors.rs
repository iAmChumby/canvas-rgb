//! Prints the Rust fan rule's constants and verdicts for fixed inputs as JSON.
//!
//! The editor copies the daemon's fan floors in `app/web/src/lib/fan-rules.ts` so it draws the
//! same floor and hot-floor line the service applies. The two copies are tied by
//! `app/web/src/lib/__tests__/fan-rules.vectors.json`: this printer writes the file, the editor's
//! test runs `fan-rules.ts` over every row, and `crates/canvas-svc/tests/fan_rules_vectors.rs`
//! fails when the checked-in file is no longer what the Rust rule makes.
//!
//! Print it and write the file with:
//! `powershell -NoProfile -File tools/fan-rules-vectors.ps1`
//!
//! This crate owns the printer and the freshness test rather than `canvas-proto`: the spine cannot
//! depend on a tool, and `canvas-svc` already has the fan tests that cover these rules
//! (`src/fans_tests.rs`, `src/gpu_fans_tests.rs`). Every constant and rule read below comes from
//! `canvas_proto::fans`; nothing in the spine is edited. The header ids are read from this crate's
//! own channel tables (`src/chip.rs`), so a header the chip gains appears as a new row.
//!
//! The JSON is written by hand (no serde here), as in the drive vectors printer: it is a fixed set
//! of rows, and this keeps the printer's dependencies as they are.

use canvas_proto::fans::{
    floor_for, runtime_floor_at, CPU_FAN_HEADER, CPU_FAN_MIN_PERCENT, DEFAULT_FLOOR_PERCENT,
    GPU_FAN_HEADER_PREFIX, GPU_FAN_MIN_PERCENT, HOT_CPU_FAN_FLOOR_PERCENT, HOT_FLOOR_PERCENT,
    HOT_TEMP_C,
};
use canvas_svc::chip::MAPS;

/// A constant the editor mirrors, with its value as a JSON literal (a quoted string or a number).
struct Constant {
    name: &'static str,
    value: String,
}

/// The fixed inputs to one rule, and what the case is meant to show.
struct Case {
    name: String,
    rule: Rule,
}

enum Rule {
    /// `rust` is `floor_for`, or the daemon's `runtime_floor_at` with no reading when `live`.
    BaseFloor { header: String, configured: u8, live: bool },
    /// The daemon's live hot-floor call: `runtime_floor_at` at the hot line (`fans.rs:1192`).
    HotFloor { header: String, configured: u8 },
}

/// A JSON string. Names and header ids carry quotes and backslashes, so escapes are written out.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON number. Whole floats are printed without a decimal point, as the integers are.
fn json_float(value: f32) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        (value as i64).to_string()
    } else {
        value.to_string()
    }
}

/// Every constant `fan-rules.ts` mirrors from the spine. The editor-only warning threshold is not
/// here: it is checked by the editor's own unit test, not by a Rust copy of an editor rule.
fn constants() -> Vec<Constant> {
    vec![
        Constant { name: "CPU_FAN_HEADER", value: json_string(CPU_FAN_HEADER) },
        Constant { name: "DEFAULT_FLOOR_PERCENT", value: DEFAULT_FLOOR_PERCENT.to_string() },
        Constant { name: "CPU_FAN_MIN_PERCENT", value: CPU_FAN_MIN_PERCENT.to_string() },
        Constant { name: "GPU_FAN_HEADER_PREFIX", value: json_string(GPU_FAN_HEADER_PREFIX) },
        Constant { name: "GPU_FAN_MIN_PERCENT", value: GPU_FAN_MIN_PERCENT.to_string() },
        Constant { name: "HOT_TEMP_C", value: json_float(HOT_TEMP_C) },
        Constant { name: "HOT_FLOOR_PERCENT", value: HOT_FLOOR_PERCENT.to_string() },
        Constant { name: "HOT_CPU_FAN_FLOOR_PERCENT", value: HOT_CPU_FAN_FLOOR_PERCENT.to_string() },
    ]
}

/// Every header the service can see: the ids in every channel table (`chip::MAPS`), with no hand
/// list, plus the NVML GPU headers the spine names by prefix (the tables document `gpu_fan1..4`).
/// A new channel id, or a new GPU index, makes a new row and a stale vector file.
fn headers() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for map in MAPS {
        for channel in map.channels {
            if !out.iter().any(|id| id == channel.id) {
                out.push(channel.id.to_string());
            }
        }
    }
    for index in 1..=4 {
        out.push(format!("{GPU_FAN_HEADER_PREFIX}{index}"));
    }
    out
}

/// Configured floors at each bound the rules have, with a value just under, at and just over it.
const BASE_CONFIGURED: [u8; 13] = [0, 19, 20, 21, 29, 30, 31, 35, 45, 49, 50, 51, 100];
const HOT_CONFIGURED: [u8; 8] = [0, 20, 34, 35, 36, 49, 50, 51];

/// The fixed rows. `baseFloor` covers `floor_for` and the daemon's second call with no reading;
/// `hotFloor` covers the daemon's first call at the hot line. Every header is included.
fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    for header in headers() {
        for configured in BASE_CONFIGURED {
            out.push(Case {
                name: format!("{header} base floor at configured {configured}"),
                rule: Rule::BaseFloor { header: header.clone(), configured, live: false },
            });
        }
        for configured in HOT_CONFIGURED {
            out.push(Case {
                name: format!("{header} hot floor at configured {configured}"),
                rule: Rule::HotFloor { header: header.clone(), configured },
            });
        }
        out.push(Case {
            name: format!("{header} base floor from runtime_floor_at with no reading"),
            rule: Rule::BaseFloor { header: header.clone(), configured: 20, live: true },
        });
    }
    out
}

/// The whole file: every mirrored constant, and every fixed row with the Rust verdict.
///
/// Public so the parity test (`crates/canvas-svc/tests/fan_rules_vectors.rs`) can ask the printer
/// what it makes today and compare it with the checked-in file.
pub fn print_json() -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(
        "  \"note\": \"The spine's fan constants and the daemon's fan-rule verdicts for fixed inputs. Printed by crates/canvas-svc/examples/fan_rules_vectors.rs; tools/fan-rules-vectors.ps1 writes this file. The editor's TypeScript mirror (app/web/src/lib/fan-rules.ts) is checked against it. Header ids come from crate::chip's channel tables plus the spine's GPU prefix. baseFloor rows use floor_for (or runtime_floor_at with no reading and the hot line argument); hotFloor rows use the daemon's live runtime_floor_at call at that hot line.\",\n",
    );

    out.push_str("  \"constants\": [\n");
    let constants = constants();
    for (index, constant) in constants.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": {},\n", json_string(constant.name)));
        out.push_str(&format!("      \"value\": {}\n", constant.value));
        out.push_str(if index + 1 == constants.len() { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ],\n");

    out.push_str("  \"cases\": [\n");
    let cases = cases();
    for (index, case) in cases.iter().enumerate() {
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": {},\n", json_string(&case.name)));
        match &case.rule {
            Rule::BaseFloor { header, configured, live } => {
                let expected = if *live {
                    runtime_floor_at(header, *configured, None, HOT_TEMP_C)
                } else {
                    floor_for(header, *configured)
                };
                out.push_str("      \"rule\": \"baseFloor\",\n");
                out.push_str(&format!(
                    "      \"rust\": {},\n",
                    json_string(if *live { "runtime_floor_at" } else { "floor_for" })
                ));
                out.push_str(&format!("      \"header\": {},\n", json_string(header)));
                out.push_str(&format!("      \"configured\": {configured},\n"));
                out.push_str(&format!("      \"expected\": {expected}\n"));
            }
            Rule::HotFloor { header, configured } => {
                let expected = runtime_floor_at(header, *configured, Some(HOT_TEMP_C), HOT_TEMP_C);
                out.push_str("      \"rule\": \"hotFloor\",\n");
                out.push_str("      \"rust\": \"runtime_floor_at\",\n");
                out.push_str(&format!("      \"header\": {},\n", json_string(header)));
                out.push_str(&format!("      \"configured\": {configured},\n"));
                out.push_str(&format!("      \"temp_c\": {},\n", json_float(HOT_TEMP_C)));
                out.push_str(&format!("      \"hot_c\": {},\n", json_float(HOT_TEMP_C)));
                out.push_str(&format!("      \"expected\": {expected}\n"));
            }
        }
        out.push_str(if index + 1 == cases.len() { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

fn main() {
    let json = print_json();
    // With a path, the file is written there (tools/fan-rules-vectors.ps1 does that);
    // without one, the JSON goes to the standard output.
    let mut args = std::env::args().skip(1);
    match args.next() {
        Some(path) => {
            if let Err(err) = std::fs::write(&path, &json) {
                eprintln!("fan_rules_vectors: cannot write {path}: {err}");
                std::process::exit(1);
            }
            eprintln!("fan_rules_vectors: wrote {path} ({} bytes)", json.len());
        }
        None => print!("{json}"),
    }
}
