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
//! `canvas_proto::fans`; nothing in the spine is edited.
//!
//! The JSON is written by hand (no serde here), as in the drive vectors printer: it is a small,
//! fixed set of rows, and this keeps the printer's dependencies as they are.

use canvas_proto::fans::{
    floor_for, runtime_floor, CPU_FAN_HEADER, CPU_FAN_MIN_PERCENT, DEFAULT_FLOOR_PERCENT,
    GPU_FAN_HEADER_PREFIX, GPU_FAN_MIN_PERCENT, HOT_CPU_FAN_FLOOR_PERCENT, HOT_FLOOR_PERCENT,
    HOT_TEMP_C,
};

/// The editor's idle-range warning sits ten degrees over the spine's hot line (issue #297). Only
/// the base comes from the spine; the extra ten is the editor's own fixed rule.
const CPU_IDLE_EXTRA_C: f32 = 10.0;

/// A constant the editor mirrors, with its value as a JSON literal (a quoted string or a number).
struct Constant {
    name: &'static str,
    value: String,
}

/// The fixed inputs to one rule, and what the case is meant to show.
struct Case {
    name: &'static str,
    rule: Rule,
}

enum Rule {
    BaseFloor { header: &'static str, configured: u8 },
    HotFloor { header: &'static str, configured: u8 },
    CpuLimitInIdleRange { kind: &'static str, limit_c: f32 },
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

/// Every constant `fan-rules.ts` mirrors from the spine, plus the CPU warning threshold the
/// editor derives from the spine's hot line.
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
        Constant { name: "CPU_IDLE_LIMIT_C", value: json_float(HOT_TEMP_C + CPU_IDLE_EXTRA_C) },
    ]
}

fn base(name: &'static str, header: &'static str, configured: u8) -> Case {
    Case { name, rule: Rule::BaseFloor { header, configured } }
}

fn hot(name: &'static str, header: &'static str, configured: u8) -> Case {
    Case { name, rule: Rule::HotFloor { header, configured } }
}

fn idle(name: &'static str, kind: &'static str, limit_c: f32) -> Case {
    Case { name, rule: Rule::CpuLimitInIdleRange { kind, limit_c } }
}

/// The fixed rows: every bound the rules have, with one value just under, at and just over it,
/// plus the ordinary case each rule is for. `hotFloor` is the floor while a header is hot, so its
/// Rust verdict is `runtime_floor` at the hot line itself.
fn cases() -> Vec<Case> {
    vec![
        // floor_for / baseFloor: the configured floor's default bound, then each header minimum.
        base("a system header with no configured floor", "sys_fan1", 0),
        base("a system header just under the default", "sys_fan1", 19),
        base("a system header at the default", "sys_fan1", 20),
        base("a system header just over the default", "sys_fan1", 21),
        base("a system header at the top", "sys_fan1", 100),
        base("the CPU header with no configured floor", CPU_FAN_HEADER, 0),
        base("the CPU header with the default floor", CPU_FAN_HEADER, 20),
        base("the CPU header just under its minimum", CPU_FAN_HEADER, 29),
        base("the CPU header at its minimum", CPU_FAN_HEADER, 30),
        base("the CPU header just over its minimum", CPU_FAN_HEADER, 31),
        base("a GPU header with no configured floor", "gpu_fan1", 0),
        base("a GPU header with the default floor", "gpu_fan1", 20),
        base("a GPU header just under its minimum", "gpu_fan1", 29),
        base("a GPU header at its minimum", "gpu_fan1", 30),
        base("a GPU header just over its minimum", "gpu_fan1", 31),
        base("a second GPU header above its minimum", "gpu_fan2", 45),
        // runtime_floor at HOT_TEMP_C / hotFloor: the ordinary floor, then the two hot floors.
        hot("a system header in the hot state", "sys_fan1", 20),
        hot("a system header at its hot floor", "sys_fan1", 35),
        hot("a system header just over its hot floor", "sys_fan1", 36),
        hot("the CPU header in the hot state", CPU_FAN_HEADER, 20),
        hot("the CPU header just under its hot floor", CPU_FAN_HEADER, 49),
        hot("the CPU header at its hot floor", CPU_FAN_HEADER, 50),
        hot("the CPU header just over its hot floor", CPU_FAN_HEADER, 51),
        hot("a GPU header in the hot state", "gpu_fan1", 20),
        hot("a GPU header just under its hot floor", "gpu_fan1", 34),
        hot("a GPU header at its hot floor", "gpu_fan1", 35),
        hot("a GPU header just over its hot floor", "gpu_fan1", 36),
        // The editor's warning threshold, derived from the spine's hot line.
        idle("a CPU line at the issue's example value", "cpu", 50.0),
        idle("a CPU line just under the threshold", "cpu", 59.0),
        idle("a CPU line at the threshold", "cpu", 60.0),
        idle("a CPU line just over the threshold", "cpu", 61.0),
        idle("a GPU line is not the CPU warning", "gpu", 50.0),
        idle("a board line is not the CPU warning", "board", 50.0),
    ]
}

/// The whole file: every mirrored constant, and every fixed row with the Rust verdict.
///
/// Public so the parity test (`crates/canvas-svc/tests/fan_rules_vectors.rs`) can ask the printer
/// what it makes today and compare it with the checked-in file.
pub fn print_json() -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(
        "  \"note\": \"The Rust fan rule's constants and verdicts for fixed inputs. Printed by crates/canvas-svc/examples/fan_rules_vectors.rs; tools/fan-rules-vectors.ps1 writes this file. The editor's TypeScript mirror (app/web/src/lib/fan-rules.ts) is checked against it. The hotFloor rows use runtime_floor at HOT_TEMP_C: the floor while a header is hot.\",\n",
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
        out.push_str(&format!("      \"name\": {},\n", json_string(case.name)));
        match &case.rule {
            Rule::BaseFloor { header, configured } => {
                let expected = floor_for(header, *configured);
                out.push_str("      \"rule\": \"baseFloor\",\n");
                out.push_str(&format!("      \"header\": {},\n", json_string(header)));
                out.push_str(&format!("      \"configured\": {configured},\n"));
                out.push_str(&format!("      \"expected\": {expected}\n"));
            }
            Rule::HotFloor { header, configured } => {
                let expected = runtime_floor(header, *configured, Some(HOT_TEMP_C));
                out.push_str("      \"rule\": \"hotFloor\",\n");
                out.push_str(&format!("      \"header\": {},\n", json_string(header)));
                out.push_str(&format!("      \"configured\": {configured},\n"));
                out.push_str(&format!("      \"expected\": {expected}\n"));
            }
            Rule::CpuLimitInIdleRange { kind, limit_c } => {
                let expected = *kind == "cpu" && *limit_c < HOT_TEMP_C + CPU_IDLE_EXTRA_C;
                out.push_str("      \"rule\": \"cpuLimitInIdleRange\",\n");
                out.push_str(&format!("      \"kind\": {},\n", json_string(kind)));
                out.push_str(&format!("      \"limit_c\": {},\n", json_float(*limit_c)));
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
