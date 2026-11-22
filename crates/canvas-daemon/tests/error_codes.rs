//! #541: every `error` string the API answers with is one of the spine's `error_codes`, so a
//! client that switches on it sees every case. The daemon once answered 410 with a bare
//! `"gone"` literal the spine did not list.

use std::path::Path;

/// The handler source of `src/api`, without its test modules.
fn api_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("api");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("src/api") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read");
        let code = text.split("#[cfg(test)]").next().unwrap_or("").to_string();
        out.push((path.file_name().unwrap().to_string_lossy().into_owned(), code));
    }
    assert!(out.len() > 5, "src/api should hold the handlers");
    out
}

#[test]
fn every_api_failure_names_a_spine_error_code() {
    let mut bad = Vec::new();
    let mut seen = 0;
    for (file, code) in api_sources() {
        for (i, _) in code.match_indices("ApiFailure::new(") {
            seen += 1;
            // The second argument is the code: the text after the first top-level comma.
            let rest = &code[i + "ApiFailure::new(".len()..];
            let second = rest.split(',').nth(1).unwrap_or("").trim();
            if !second.contains("error_codes::") {
                bad.push(format!("{file}: {second}"));
            }
        }
    }
    assert!(seen > 0, "no ApiFailure::new call found; the scan is broken");
    assert!(bad.is_empty(), "ApiFailure codes outside the spine's error_codes: {bad:?}");
}

#[test]
fn no_json_error_body_spells_its_code_as_a_literal() {
    let mut bad = Vec::new();
    for (file, code) in api_sources() {
        for line in code.lines() {
            let t = line.trim();
            if t.starts_with("\"error\": \"") || t.contains("\"error\": \"") {
                bad.push(format!("{file}: {t}"));
            }
        }
    }
    assert!(bad.is_empty(), "JSON error bodies with a literal code: {bad:?}");
}

#[test]
fn gone_is_a_spine_code() {
    assert_eq!(canvas_proto::api::error_codes::GONE, "gone");
}
