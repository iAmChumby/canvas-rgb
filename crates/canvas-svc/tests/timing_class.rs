//! The `cause:timing-test` class check (docs/bug-classes.md; #36 #39 #45 #47 #51 #55 #59 #66).
//!
//! A test that reads its verdict off a real clock fails when a busy machine delays it. This scan
//! runs over the repo's own test code on every `cargo test --workspace` (so in CI) and flags two
//! shapes:
//!
//! 1. A wall-clock upper bound: a measured duration compared with `<` or `<=` against a budget
//!    (`started.elapsed() < ...`, `took < ...`, `waited <= ...`, `.as_millis() < ...`).
//! 2. Sleep-then-assert: a `sleep(` followed within three lines by an `assert`, which asserts that
//!    something happened by the time a sleep ended.
//!
//! Every hit must be on the commented allow-list below, one entry per line, and every entry names
//! why that line is safe. The scan fails on an entry that matches no line, so the list cannot
//! collect dead entries.
//!
//! What it cannot catch (it reads text, one line at a time):
//! - product code with a clock in it (#36 was a flush limit, #66 a whole-second comparison);
//! - a bound with no literal in the test (#51 scaled one timed sample, with no `<` on a clock);
//! - a bound written the other way round (`budget > took`), through a helper, or on a clock
//!   variable with a name that is not listed in [`CLOCK_WORDS`];
//! - a sleep and its assert more than three lines apart, or a `recv_timeout` / poll loop that is
//!   too short for a busy machine (a `while t.elapsed() < limit {` line is the limit of such a
//!   loop, not a verdict, and is not flagged);
//! - a second identical line in a file that already has an allow-list entry for that text: the
//!   entry matches by text, so the new line is allowed with it;
//! - web tests (`findBy*` timeouts, #59): `app/web/vitest.setup.ts` and its test own that;
//! - a missing log line (#45/#55): the mechanism is unknown.
//!
//! Test code is: any file in a `tests/` folder, `tests.rs`, `*_tests.rs`, and the part of a
//! source file from its `#[cfg(test)]` line on. Comment lines never count.

use std::fs;
use std::path::{Path, PathBuf};

/// Names of a measured duration. A line that compares one of these with `<` or `<=` is a bound.
const CLOCK_WORDS: &[&str] = &[
    "elapsed()",
    "elapsed",
    "took",
    "waited",
    "taken",
    "as_millis()",
    "as_secs_f64()",
    "as_secs_f32()",
    "as_secs()",
    "as_micros()",
];

/// Accessors that may sit between the clock word and the comparison.
const ACCESSORS: &[&str] = &[".as_millis()", ".as_secs_f64()", ".as_secs_f32()", ".as_secs()", ".as_micros()"];

/// Allow-list for rule 1 (wall-clock bound): (path suffix, text the line must contain, why).
///
/// Three kinds of reason: "hang guard" (the bound is many times the work, so only a hang fails it),
/// "the property" (the bound is what the test is about), and "open" (a real sibling of the class,
/// listed in docs/bug-classes.md and not yet fixed; deleting its entry is the fix's last step).
const BOUND_ALLOWED: &[(&str, &str, &str)] = &[
    ("canvas-core/src/timer.rs", "elapsed < Duration::from_millis(120)", "open: 120 ms ceiling on a 33 ms sleep; a stalled runner overshoots it"),
    ("canvas-core/src/timer.rs", "start.elapsed() < Duration::from_millis(100)", "open: 100 ms ceiling on a catch-up that should cost nothing"),
    ("canvas-core/src/timer.rs", "start.elapsed() < Duration::from_millis(60)", "open: 60 ms ceiling after a reset; one 16.7 ms frame of slack"),
    ("canvas-daemon/src/cli.rs", "elapsed.as_millis() < 5_000", "hang guard: 5 s against a check that must stop at its first frame"),
    ("canvas-daemon/src/logging.rs", "waited < std::time::Duration::from_secs(5)", "hang guard: every step of that test is bounded by 250 ms; 5 s means a bound was ignored"),
    ("canvas-daemon/src/logging.rs", "took < std::time::Duration::from_millis(1000)", "the property: run_bounded gives up on a 5 s step after its 250 ms limit; 750 ms of slack"),
    ("canvas-daemon/src/user_effects.rs", "took < std::time::Duration::from_secs(5)", "hang guard: a 400 ms render budget against a 30 s frame budget; 5 s means the frame was not held to the budget"),
    ("canvas-daemon/src/user_effects.rs", "took < budget + std::time::Duration::from_secs_f64(frame_s)", "the property, calibrated: frames are sized to this machine, and the bound is one frame (#51)"),
    ("canvas-daemon/tests/effects_import.rs", "elapsed.as_millis() < 6_000", "open: 6 s ceiling on a 2 s import budget"),
    ("canvas-daemon/tests/quit.rs", "started.elapsed() < Duration::from_secs(15)", "open: a child process must serve its API within 15 s on a loaded runner"),
    ("canvas-daemon/tests/quit.rs", "took <= EXIT_DEADLINE", "the property: the daemon's own exit deadline"),
    ("canvas-html/tests/effects.rs", "started.elapsed().as_millis() <= 2_000", "hang guard: a frame after a runaway loop must not be stuck as well; 2 s"),
    ("canvas-html/tests/effects.rs", "elapsed.as_millis() < 30 * canvas_html::FRAME_BUDGET_MS as u128", "hang guard: 30 frames against 30 frame budgets of the engine"),
];

/// Allow-list for rule 2 (sleep-then-assert): (path suffix, text the sleep line must contain, why).
///
/// "safe" reasons: the sleep is a minimum (a slow runner only waits longer), or the assert is about
/// something that did NOT happen (slowness hides a failure, it cannot make one), or the sleep is
/// inside a loop that polls to a deadline. "open" means the sleep stands in for a state the code is
/// expected to reach in time, which a stalled runner can miss: a real sibling, not yet fixed.
const SLEEP_ASSERT_ALLOWED: &[(&str, &str, &str)] = &[
    ("attack_shark_x6/hid_cache.rs", "sleep(Duration::from_millis(350))", "safe: sleeps past a 300 ms TTL; a slower runner only ages the cache more"),
    ("canvas-adapters/tests/adapters_tests.rs", "sleep(Duration::from_millis(1700))", "safe: sleeps past the repeat interval; slowness only passes it further"),
    ("canvas-core/src/tests.rs", "sleep(Duration::from_millis(300))", "open: waits 300 ms for the engine to park, then asserts it parked (same family as #39)"),
    ("canvas-core/src/tests.rs", "sleep(Duration::from_secs(2))", "safe: asserts no frame was rendered or a count stayed at or under a ceiling; slowness lowers the count"),
    ("canvas-core/src/tests.rs", "sleep(Duration::from_secs(1))", "safe: asserts the frame counter did not move while paused"),
    ("canvas-core/src/tests.rs", "sleep(Duration::from_millis(150))", "safe: an upper ceiling on frames rendered; slowness lowers the count"),
    ("canvas-core/src/tests.rs", "sleep(Duration::from_millis(500))", "safe: asserts no more preview frames arrive"),
    ("canvas-daemon/src/logging.rs", "|| std::thread::sleep(std::time::Duration::from_secs(5))", "the property: the sleep is the stuck step run_bounded must give up on"),
    ("canvas-daemon/src/peripherals.rs", "sleep(Duration::from_millis(200))", "safe: asserts the gated sink has NOT written yet"),
    ("canvas-daemon/src/thumbnail.rs", "sleep(std::time::Duration::from_millis(100))", "safe: asserts a waiting task has NOT finished"),
    ("canvas-daemon/src/thumbnail.rs", "sleep(retry * 3)", "safe: sleeps past the retry interval; slowness only passes it further"),
    ("canvas-daemon/tests/api.rs", "sleep(retry * 2)", "safe: sleeps past the retry interval; slowness only passes it further"),
    ("canvas-daemon/tests/api.rs", "from_millis(60)", "open: onboarding timer on a compressed time scale (transient.rs); listed sibling"),
    ("canvas-daemon/tests/api.rs", "from_millis(70)", "open: onboarding timer on a compressed time scale (transient.rs); listed sibling"),
    ("canvas-daemon/tests/api.rs", "from_millis(250)", "open: onboarding timer on a compressed time scale (transient.rs); listed sibling"),
    ("canvas-daemon/tests/api.rs", "from_millis(350)", "open: onboarding timer on a compressed time scale (transient.rs); listed sibling"),
    ("canvas-daemon/tests/api.rs", "from_millis(300)", "open: onboarding timer on a compressed time scale (transient.rs); listed sibling"),
    ("canvas-daemon/tests/exit_lines.rs", "sleep(Duration::from_millis(100))", "safe: the poll loop of a child exit, with a 20 s deadline that panics"),
    ("canvas-daemon/tests/wave1.rs", "sleep(std::time::Duration::from_secs(2))", "open: 2 s for an engine to park, then asserts it parked; a state reached in about 100 ms"),
    ("canvas-svc/src/openrgb_tests.rs", "sleep(Duration::from_millis(200))", "safe: the poll loop breaks when the boot color is written"),
    ("canvas-svc/src/openrgb_tests.rs", "sleep(between)", "safe: sleeps past the watch window; slowness only passes it further"),
    ("canvas-svc/src/update_install_tests.rs", "sleep(Duration::from_millis(10))", "safe: the poll loop of a killed child, with a 10 s deadline that fails"),
    ("canvas-svc/src/update_install_tests.rs", "sleep(Duration::from_millis(300))", "safe: asserts a suspended setup has NOT run"),
];

#[derive(Debug, PartialEq, Eq)]
struct Finding {
    file: String,
    line: usize,
    rule: u8,
    text: String,
}

/// Which allow-list entries suppressed a finding: (list id, index). 1 = bound, 2 = sleep.
type Used = Vec<(u8, usize)>;

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

/// The code part of a line: a trailing `// comment` is cut off.
fn code_of(line: &str) -> &str {
    match line.find("//") {
        Some(at) => &line[..at],
        None => line,
    }
}

/// Whether `code` compares a measured duration with `<` or `<=`.
fn has_wall_clock_bound(code: &str) -> bool {
    for word in CLOCK_WORDS {
        let mut from = 0;
        while let Some(at) = code[from..].find(word) {
            let start = from + at;
            let mut rest = &code[start + word.len()..];
            from = start + word.len();
            // The word must be a whole name: `elapsed` in `time_elapsed_total` is another name.
            let before = code[..start].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            // `elapsed().as_millis()`: an accessor between the word and the comparison.
            loop {
                let trimmed = rest.trim_start();
                match ACCESSORS.iter().find(|a| trimmed.starts_with(*a)) {
                    Some(a) => rest = &trimmed[a.len()..],
                    None => break,
                }
            }
            let rest = rest.trim_start();
            // `<` or `<=`, but not `<<`, `<-`, a generic `<T>` or a closing `</`.
            if let Some(after) = rest.strip_prefix('<') {
                let after = after.strip_prefix('=').unwrap_or(after);
                if after.starts_with(' ')
                    || after.starts_with(|c: char| c.is_ascii_digit())
                    || after.starts_with("std::")
                    || after.starts_with("Duration")
                {
                    return true;
                }
            }
        }
    }
    false
}

fn allowed(list: &[(&str, &str, &str)], id: u8, rel: &str, line: &str, used: &mut Used) -> bool {
    let mut hit = false;
    for (i, (suffix, text, _why)) in list.iter().enumerate() {
        if rel.ends_with(suffix) && line.contains(text) {
            used.push((id, i));
            hit = true;
        }
    }
    hit
}

/// Whether the file is test code from its first line.
fn is_test_file(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    rel.contains("/tests/") || name == "tests.rs" || name.ends_with("_tests.rs")
}

/// Scans one file's text. `rel` is the repo-relative path with `/` separators.
fn scan_text_used(rel: &str, text: &str, used: &mut Used) -> Vec<Finding> {
    let lines: Vec<&str> = text.lines().collect();
    let first_test_line = if is_test_file(rel) {
        0
    } else {
        // The test module: the last `#[cfg(test)]` line whose next line opens a `mod`. A `#[cfg(test)]`
        // on a single item in the middle of product code does not start test code.
        let starts_module = |at: usize| {
            lines.get(at + 1).is_some_and(|next| {
                let next = next.trim_start();
                next.starts_with("mod ") || next.starts_with("pub mod ") || next.starts_with("pub(crate) mod ")
            })
        };
        match (0..lines.len()).rev().find(|&at| lines[at].trim_start().starts_with("#[cfg(test)]") && starts_module(at)) {
            Some(at) => at,
            None => return Vec::new(),
        }
    };
    let mut found = Vec::new();
    for (i, raw) in lines.iter().enumerate().skip(first_test_line) {
        if is_comment(raw) {
            continue;
        }
        let code = code_of(raw);
        // `while t.elapsed() < limit {`: the limit on a wait for a condition, not a verdict.
        let waits_for_a_condition = code.trim_start().starts_with("while ");
        if !waits_for_a_condition && has_wall_clock_bound(code) && !allowed(BOUND_ALLOWED, 1, rel, raw, used) {
            found.push(Finding { file: rel.to_string(), line: i + 1, rule: 1, text: raw.trim().to_string() });
        }
        if code.contains("sleep(") && !code.contains("fn ") {
            let asserts_soon = lines
                .iter()
                .skip(i + 1)
                .filter(|l| !l.trim().is_empty() && !is_comment(l))
                .take(3)
                .any(|l| l.trim_start().starts_with("assert"));
            if asserts_soon && !allowed(SLEEP_ASSERT_ALLOWED, 2, rel, raw, used) {
                found.push(Finding { file: rel.to_string(), line: i + 1, rule: 2, text: raw.trim().to_string() });
            }
        }
    }
    found
}

fn scan_text(rel: &str, text: &str) -> Vec<Finding> {
    scan_text_used(rel, text, &mut Vec::new())
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn skipped_dir(name: &str) -> bool {
    matches!(name, "target" | "node_modules" | "examples" | ".git" | "out" | ".next" | "fixtures")
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !skipped_dir(&name) {
                collect(&path, out);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn scan_repo() -> (usize, Vec<Finding>, Used) {
    let root = repo_root();
    let mut files = Vec::new();
    collect(&root.join("crates"), &mut files);
    let mut found = Vec::new();
    let mut used = Used::new();
    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let rel = rel.trim_start_matches("../").to_string();
        // This file names every pattern it scans for; it is not a test of a clock.
        if rel.ends_with("canvas-svc/tests/timing_class.rs") {
            continue;
        }
        // Lossy: a non-UTF-8 byte must not hide a file from the scan.
        let text = String::from_utf8_lossy(&fs::read(path).expect("read a source file")).into_owned();
        found.extend(scan_text_used(&rel, &text, &mut used));
    }
    (files.len(), found, used)
}

#[test]
fn no_test_reads_its_verdict_off_a_wall_clock_unless_the_line_says_why() {
    let (scanned, found, used) = scan_repo();
    assert!(scanned > 50, "the scan found only {scanned} files: the roots moved?");
    let report: Vec<String> = found
        .iter()
        .map(|f| {
            let what = match f.rule {
                1 => "a wall-clock upper bound in a test (count events, inject a clock, or allow-list the line with a reason)",
                _ => "sleeps, then asserts (wait for the condition, or allow-list the line with a reason)",
            };
            format!("{}:{}: {what}: {}", f.file, f.line, f.text)
        })
        .collect();
    assert!(report.is_empty(), "timing-test findings ({} files scanned):\n{}", scanned, report.join("\n"));
    let mut stale = Vec::new();
    for (id, list) in [(1u8, BOUND_ALLOWED), (2u8, SLEEP_ASSERT_ALLOWED)] {
        for (i, (suffix, text, why)) in list.iter().enumerate() {
            if !used.contains(&(id, i)) {
                stale.push(format!("{suffix}: `{text}` ({why})"));
            }
        }
    }
    assert!(stale.is_empty(), "allow-list entries that matched no line (delete them):\n{}", stale.join("\n"));
}

// ---- the scanner itself: each rule fires on the shape of the bug it exists for ----

fn rules(rel: &str, text: &str) -> Vec<u8> {
    scan_text(rel, text).iter().map(|f| f.rule).collect()
}

#[test]
fn rule_1_catches_a_bound_on_a_measured_duration() {
    for bad in [
        "assert!(started.elapsed() < Duration::from_millis(500));",
        "assert!(took < budget + Duration::from_millis(400), \"slow\");",
        "assert!(t.elapsed().as_millis() < 50);",
        "assert!(waited <= std::time::Duration::from_secs(1));",
        "assert!(start.elapsed().as_secs_f64() < 0.5);",
    ] {
        let src = format!("fn t() {{\n    {bad}\n}}\n");
        assert_eq!(rules("crates/x/tests/a.rs", &src), vec![1], "{bad}");
    }
}

#[test]
fn rule_1_leaves_other_comparisons_alone() {
    for fine in [
        "assert!(took > Duration::from_millis(100));",
        "assert!(count < 5);",
        "let v: Vec<Duration> = Vec::new();",
        "let x = elapsed_total < 3;",
        "assert!(finished_at >= started);",
        "// assert!(started.elapsed() < Duration::from_millis(500));",
        "let a = took << 2;",
    ] {
        let src = format!("fn t() {{\n    {fine}\n}}\n");
        assert!(rules("crates/x/tests/a.rs", &src).is_empty(), "{fine}");
    }
}

#[test]
fn rule_1_reads_only_test_code_of_a_source_file() {
    let product = "fn f() {\n    if t.elapsed() < limit { return; }\n}\n";
    assert!(rules("crates/x/src/a.rs", product).is_empty(), "product code is not scanned");
    let with_tests =
        "fn f() {}\n#[cfg(test)]\nmod tests {\n    fn t() {\n        assert!(t.elapsed() < Duration::from_secs(1));\n    }\n}\n";
    assert_eq!(rules("crates/x/src/a.rs", with_tests), vec![1]);
    assert_eq!(rules("crates/x/src/a_tests.rs", "fn t() {\n    assert!(took < budget);\n}\n"), vec![1]);
}

#[test]
fn rule_2_catches_a_sleep_followed_by_an_assert() {
    let bad = "fn t() {\n    std::thread::sleep(Duration::from_millis(50));\n    assert!(done());\n}\n";
    assert_eq!(rules("crates/x/tests/a.rs", bad), vec![2]);
    let far = "fn t() {\n    sleep(d);\n    a();\n    b();\n    c();\n    d();\n    assert!(done());\n}\n";
    assert!(rules("crates/x/tests/a.rs", far).is_empty(), "an assert four statements on is not the sleep's");
    let poll = "fn t() {\n    while !done() { std::thread::sleep(d); }\n    assert!(done());\n}\n";
    assert_eq!(
        rules("crates/x/tests/a.rs", poll),
        vec![2],
        "the text scan cannot tell a poll loop from a bare sleep: allow-list it"
    );
}

#[test]
fn an_allow_list_entry_applies_to_its_file_and_line_only() {
    let mut used = Used::new();
    let line = "assert!(x.elapsed() < Duration::from_secs(1));";
    assert!(!allowed(&[("crates/x/tests/a.rs", "x.elapsed()", "why")], 1, "crates/y/tests/a.rs", line, &mut used));
    assert!(used.is_empty());
    assert!(allowed(&[("crates/x/tests/a.rs", "x.elapsed()", "why")], 1, "crates/x/tests/a.rs", line, &mut used));
    assert_eq!(used, vec![(1, 0)]);
}
