//! Prints the Rust drive rule's verdicts for a fixed set of device ids as JSON.
//!
//! The editor copies the daemon's `is_driven` rule in `app/web/src/lib/drive.ts` so it offers
//! Remove only on lights the daemon would delete. The two copies are tied together by
//! `app/web/src/lib/__tests__/drive.vectors.json`: this printer writes the file, the editor's
//! test runs `drive.ts` over every row, and `crates/canvas-adapters/tests/drive_vectors.rs`
//! fails when the checked-in file is no longer what the Rust rule makes.
//!
//! Print it and write the file with:
//! `powershell -NoProfile -File tools/drive-vectors.ps1`
//!
//! The JSON is written by hand (no serde here), as in the arrangement vectors printer: it is a
//! small, fixed set of ids and their verdicts, and this keeps the printer's dependencies as they
//! are.

use canvas_proto::DeviceId;

/// One fixed device id, and what the case is meant to show.
struct Case {
    /// What the case shows.
    name: &'static str,
    /// The device id, as the daemon sees it.
    id: &'static str,
}

/// The fixed ids the editor's rule is checked against: both verdicts, and the edges of the
/// match — case, where the pattern sits in the id, the `&` separator, and an empty id.
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "the full keyboard HID path",
            id: r"openrgb:HID: \\?\HID#VID_3151&PID_4015&MI_02#9&298b2e71&0&0000#{4d1e55b2}",
        },
        Case {
            name: "the same path in lower case",
            id: r"openrgb:HID: \\?\hid#vid_3151&pid_4015&mi_02#9&298b2e71&0&0000#{4d1e55b2}",
        },
        Case {
            name: "mixed case vendor and product",
            id: r"OpenRGB:HID: \\?\Hid#Vid_3151&PiD_4015&MI_02",
        },
        Case {
            name: "the matched pattern on its own",
            id: "VID_3151&PID_4015",
        },
        Case {
            name: "the pattern at the start of the id",
            id: "VID_3151&PID_4015 and more",
        },
        Case {
            name: "the pattern at the end of the id",
            id: "prefix VID_3151&PID_4015",
        },
        Case {
            name: "the pattern twice in the id",
            id: "VID_3151&PID_4015 and VID_3151&PID_4015",
        },
        Case {
            name: "the pattern before a longer product code",
            id: "VID_3151&PID_40150",
        },
        Case {
            name: "an empty id",
            id: "",
        },
        Case {
            name: "the microphone HID path",
            id: r"openrgb:HID: \\?\HID#VID_03F0&PID_028C&MI_00#a&eb32b1c",
        },
        Case {
            name: "the native royuan id",
            id: "royuan:3151:4015",
        },
        Case {
            name: "the vendor part alone",
            id: "VID_3151",
        },
        Case {
            name: "the product part alone",
            id: "PID_4015",
        },
        Case {
            name: "a different vendor",
            id: "VID_3152&PID_4015",
        },
        Case {
            name: "a different product",
            id: "VID_3151&PID_4016",
        },
        Case {
            name: "a space instead of the separator",
            id: "VID_3151 PID_4015",
        },
        Case {
            name: "no separator",
            id: "VID_3151PID_4015",
        },
        Case {
            name: "a doubled separator",
            id: "VID_3151&&PID_4015",
        },
        Case {
            name: "the parts in the other order",
            id: "PID_4015&VID_3151",
        },
        Case {
            name: "a dotless i that Unicode uppercasing folds to I",
            id: "v\u{0131}d_3151&p\u{0131}d_4015",
        },
    ]
}

/// A JSON string. The ids carry backslashes and `&`, so the quotes and escapes are written out.
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

/// The whole file: every fixed id and the verdict the daemon's rule gives it.
///
/// Public so the parity test (`crates/canvas-adapters/tests/drive_vectors.rs`) can ask the printer
/// what it makes today and compare it with the checked-in file.
pub fn print_json() -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(
        "  \"note\": \"The Rust drive rule's verdicts for the fixed device ids. Printed by crates/canvas-adapters/examples/drive_vectors.rs; tools/drive-vectors.ps1 writes this file. The editor's TypeScript rule (app/web/src/lib/drive.ts) is checked against it.\",\n",
    );
    // The pattern list itself, so a pattern added on either side that no case below matches
    // still fails a test (#501 review).
    let patterns: Vec<String> =
        canvas_adapters::drive::NOT_DRIVEN.iter().map(|p| json_string(p)).collect();
    out.push_str(&format!("  \"not_driven\": [{}],\n", patterns.join(", ")));
    out.push_str("  \"cases\": [\n");
    let cases = cases();
    for (index, case) in cases.iter().enumerate() {
        let driven = canvas_adapters::drive::is_driven(&DeviceId(case.id.to_string()));
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": {},\n", json_string(case.name)));
        out.push_str(&format!("      \"id\": {},\n", json_string(case.id)));
        out.push_str(&format!("      \"driven\": {}\n", driven));
        out.push_str(if index + 1 == cases.len() { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

fn main() {
    let json = print_json();
    // With a path, the file is written there (tools/drive-vectors.ps1 does that);
    // without one, the JSON goes to the standard output.
    let mut args = std::env::args().skip(1);
    match args.next() {
        Some(path) => {
            if let Err(err) = std::fs::write(&path, &json) {
                eprintln!("drive_vectors: cannot write {path}: {err}");
                std::process::exit(1);
            }
            eprintln!("drive_vectors: wrote {path} ({} bytes)", json.len());
        }
        None => print!("{json}"),
    }
}
