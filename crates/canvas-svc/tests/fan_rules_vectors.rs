//! The checked-in vectors must be what the Rust fan rule makes today (#507).
//!
//! `app/web/src/lib/__tests__/fan-rules.vectors.json` is the agreement between the daemon's fan
//! rules and the editor's TypeScript copy: the editor's test runs `fan-rules.ts` over the file's
//! rows, and `tools/fan-rules-vectors.ps1` writes the file from the Rust printer.
//!
//! Nothing checked that the file still came from Rust. A hand edit, or a fan-rule change without
//! the tool, would leave the editor checked against verdicts the daemon never gives. This test
//! reads the file and compares it with the Rust printer's own output.
//!
//! Run `powershell -NoProfile -File tools/fan-rules-vectors.ps1` after a fan-rule change; this
//! test fails until the file is regenerated.

use std::path::PathBuf;

/// The Rust printer, as a module: the example and this test must agree on the constants and rows.
#[allow(dead_code)]
#[path = "../examples/fan_rules_vectors.rs"]
mod printer;

/// The checked-in vector file, next to the editor's test that reads it.
fn vectors_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../app/web/src/lib/__tests__/fan-rules.vectors.json")
}

#[test]
fn the_checked_in_vectors_are_what_the_fan_rule_makes() {
    let path = vectors_path();
    let checked_in = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the vector file is read: {} ({e})", path.display()));
    let made = printer::print_json();
    // Git may check the file out with CRLF; the printer always writes LF.
    let normalise = |text: String| {
        text.lines().map(|line| line.trim_end_matches('\r')).collect::<Vec<&str>>().join("\n")
    };
    assert_eq!(
        normalise(checked_in),
        normalise(made),
        "the vector file is not what the fan rule makes: run tools/fan-rules-vectors.ps1"
    );
}
