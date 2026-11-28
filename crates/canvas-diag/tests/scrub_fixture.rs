//! The shared scrub fixture: the TypeScript test reads the same JSON file (G3).

use canvas_diag::scrub::scrub_device_ids;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Case {
    input: String,
    expected: String,
}

#[test]
fn the_shared_scrub_cases_match_rust() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/scrub-cases.json");
    let text = std::fs::read_to_string(path).expect("the shared fixture exists");
    let cases: Vec<Case> = serde_json::from_str(&text).expect("the fixture parses");
    assert!(cases.len() >= 8, "the fixture holds every named case");
    for case in cases {
        let actual = scrub_device_ids(&case.input);
        if actual != case.expected {
            println!(
                "MISMATCH input={:?} expected={:?} actual={:?} manifest={:?} cwd={:?} prefixes={:?}",
                case.input,
                case.expected,
                actual,
                env!("CARGO_MANIFEST_DIR"),
                std::env::current_dir(),
                canvas_diag::scrub::ADAPTER_PREFIXES,
            );
        }
        assert_eq!(actual, case.expected, "case {:?}", case.input);
    }
}
