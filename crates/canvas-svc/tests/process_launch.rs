//! The `cause:process-launch` class check (docs/bug-classes.md; #6 and #28).
//!
//! Four rules, run over the repo's own source on every `cargo test --workspace` (so in CI):
//!
//! 1. Nothing outside `crates/canvas-svc` starts `canvas-rgbd.exe`. The service launches the daemon
//!    in the user's session with no window; any other path skips those rules (#6). A line that names
//!    the daemon (`canvas-rgbd`, `DaemonExe`, `RunKeyDaemon`) beside a process-start or Run-key
//!    write is a finding, and so is any `[Run]` entry in the Inno script that names it.
//! 2. No code asks Windows to start a process elevated with an elevating `ShellExecute` verb or
//!    `Start-Process -Verb RunAs`, except the commented allow-list below (#28: setup started
//!    elevated relaunched the tray elevated).
//! 3. Every place that starts a process says which token it runs with: a comment (not code: a
//!    `let token: u32` does not count) containing `token:` on the same line or in the 8 lines above it.
//! 4. A file outside `canvas-svc` that names the daemon (`canvas-rgbd`, `DaemonExe`, `daemon_path`,
//!    in code, or in the `[Code]` section of the Inno script) may not start any process at all unless a
//!    line-level allow-list entry says why. This catches `Command::new(&daemon_path)`, where rule 1's
//!    "daemon name and launch on one line" cannot see the daemon. Residual gap, accepted: a launch of
//!    the daemon through a path read from a config file or built in a file that never names the daemon
//!    is invisible to a text scan; rule 3 still forces the author to write which token it runs with,
//!    and review of that line is where it would be caught.
//!
//! The scan reads `*.rs`, `*.ps1`, `*.iss`, `*.cmd` and `*.bat` under `crates/`, `app/src-tauri/`,
//! `installer/` and `tools/`. Comment lines never count as a launch. Test files are skipped (a
//! `tests/` folder, `tests.rs`, `*_tests.rs`), because the scan lives in one and tests may start
//! anything; so are `target/`, `node_modules/` and `examples/` (hand-run probes, never shipped).

use std::fs;
use std::path::{Path, PathBuf};

/// Names that mean "the daemon".
const DAEMON_NAMES: &[&str] = &[
    "canvas-rgbd",
    "canvas_rgbd",
    "DaemonExe",
    "DAEMON_EXE",
    "RunKeyDaemon",
    "daemon_path",
];

/// Whether the line names the daemon, in any letter case.
fn names_daemon_in(line: &str) -> bool {
    let lower = line.to_lowercase();
    DAEMON_NAMES.iter().any(|n| lower.contains(&n.to_lowercase()))
}

/// Things that start a process or register one to start at logon. `Exec(` does not match
/// `ExecAsOriginalUser(`; both are listed.
const LAUNCHES: &[&str] = &[
    "Start-Process",
    "Exec(",
    "ExecAsOriginalUser(",
    "Command::new(",
    "CreateProcessAsUserW(",
    "CreateProcessW(",
    "CreateProcessWithTokenW(",
    "ShellExecuteExW(",
    "ShellExecuteW(",
    "Set-ItemProperty",
    "New-ItemProperty",
    "reg add",
    "reg.exe add",
    "Filename:",
];

/// Things that start a process (the rule-3 subset: a Run-key write starts nothing by itself).
const PROCESS_STARTS: &[&str] = &[
    "CreateProcessA(",
    "ShellExecuteA(",
    "ShellExecuteExA(",
    "WinExec(",
    "saps ",
    "Invoke-Item",
    "[System.Diagnostics.Process]::Start(",
    "Start-Process",
    "Exec(",
    "ExecAsOriginalUser(",
    "Command::new(",
    "CreateProcessAsUserW(",
    "CreateProcessW(",
    "CreateProcessWithTokenW(",
    "ShellExecuteExW(",
    "ShellExecuteW(",
];

/// True when the line names the elevating verb in any spelling: `"runas"`, `-Verb RunAs`,
/// `-Verb:RunAs`, `-verb runas`, `Verb = 'RunAs'`, `Verb='runas'`, `w!("RunAs")`. Case-insensitive,
/// and `runas` must be a whole word, so the Inno flag `runasoriginaluser` is not a hit.
fn names_elevation(line: &str) -> bool {
    let lower = line.to_lowercase();
    let bytes = lower.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut from = 0;
    while let Some(at) = lower[from..].find("runas") {
        let start = from + at;
        let end = start + "runas".len();
        let before_ok = start == 0 || !word(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !word(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// Rule 2 allow-list, one entry per line: (path suffix, text the line must contain, why). A second
/// elevating line in the same file is a finding. Each entry is the script installer elevating its own
/// PowerShell child once (the machine-wide half); the per-user half then runs as the person, in a
/// separate non-elevated process, and nothing here relaunches the tray.
const ELEVATION_ALLOWED: &[(&str, &str, &str)] = &[
    (
        "installer/setup.ps1",
        "Start-Process -FilePath 'powershell.exe' -ArgumentList $childArgs -Verb RunAs",
        "script install: elevates its machine-wide child (setup-elevated.ps1)",
    ),
    (
        "installer/uninstall.ps1",
        "Start-Process -FilePath 'powershell.exe' -ArgumentList $childArgs -Verb RunAs",
        "script uninstall: elevates its machine-wide child (uninstall-elevated.ps1)",
    ),
];

/// Rule 4 allow-list, one entry per line: (path suffix, text the line must contain, why). A process
/// start in a file that also names the daemon, and is not the daemon. The scan fails on an entry that
/// matches no line, so this list cannot collect dead entries.
const DAEMON_FILE_STARTS_ALLOWED: &[(&str, &str, &str)] = &[(
    "installer/setup.ps1",
    "Start-Process -FilePath 'powershell.exe' -ArgumentList $childArgs -Verb RunAs",
    "elevates the machine-wide child script, not the daemon",
)];

/// Rule 3 lines that look like a launch but are text, not a call: the installer's printed plan
/// (`$L.Add(...)`) describes steps in words.
const TEXT_LINES: &[&str] = &["$L.Add("];

#[derive(Debug, PartialEq, Eq)]
struct Finding {
    file: String,
    line: usize,
    rule: u8,
    text: String,
}

fn is_comment(trimmed: &str, ext: &str) -> bool {
    match ext {
        "rs" => trimmed.starts_with("//"),
        "ps1" => trimmed.starts_with('#'),
        "iss" => trimmed.starts_with("//") || trimmed.starts_with(';') || trimmed.starts_with('{'),
        "cmd" | "bat" => trimmed.to_lowercase().starts_with("rem ") || trimmed.starts_with("::"),
        _ => false,
    }
}

/// The comment part of a line (from its comment marker on), or `None` for pure code.
fn comment_part<'a>(line: &'a str, ext: &str) -> Option<&'a str> {
    let trimmed = line.trim_start();
    if is_comment(trimmed, ext) {
        return Some(trimmed);
    }
    let marker = match ext {
        "rs" => line.find("//"),
        "ps1" => line.find('#'),
        "iss" => [line.find("//"), line.find('{')].into_iter().flatten().min(),
        "cmd" | "bat" => line.to_lowercase().find("& rem ").or_else(|| line.find("&::")),
        _ => None,
    }?;
    Some(&line[marker..])
}

fn says_token(line: &str, ext: &str) -> bool {
    comment_part(line, ext).is_some_and(|c| c.contains("token:"))
}

/// Which allow-list entries suppressed a finding: (list id, index). 1 = elevation, 2 = daemon file.
type Used = Vec<(u8, usize)>;

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


/// Scans one file's text. `rel` is the repo-relative path with `/` separators.
fn scan_text(rel: &str, text: &str) -> Vec<Finding> {
    scan_text_used(rel, text, &mut Vec::new())
}

fn scan_text_used(rel: &str, text: &str, used: &mut Used) -> Vec<Finding> {
    let ext = rel.rsplit('.').next().unwrap_or("");
    let in_service = rel.starts_with("crates/canvas-svc/");
    let lines: Vec<&str> = text.lines().collect();

    // Which lines are code that names the daemon (rule 4's "this file names the daemon"). In the
    // Inno script only [Code] counts: [Files] must list canvas-rgbd.exe to ship it.
    let mut section = String::new();
    let mut sections = Vec::with_capacity(lines.len());
    for raw in &lines {
        let trimmed = raw.trim();
        if ext == "iss" && trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed.to_string();
        }
        sections.push(section.clone());
    }
    let code_line = |i: usize| {
        let t = lines[i].trim();
        !t.is_empty() && !is_comment(t, ext) && !TEXT_LINES.iter().any(|x| lines[i].contains(x))
    };
    let file_names_daemon = (0..lines.len()).any(|i| {
        code_line(i)
            && (ext != "iss" || sections[i] == "[Code]")
            && names_daemon_in(lines[i])
    });

    let mut found = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || is_comment(trimmed, ext) {
            continue;
        }
        let at = |rule: u8| Finding { file: rel.to_string(), line: i + 1, rule, text: trimmed.to_string() };
        let section = sections[i].as_str();
        let names_daemon = names_daemon_in(raw);
        let is_text = TEXT_LINES.iter().any(|t| raw.contains(t));
        let starts_process = PROCESS_STARTS.iter().any(|p| raw.contains(p));

        // Rule 1: only the service starts the daemon.
        let mut rule1 = false;
        if !in_service && names_daemon {
            let launches = LAUNCHES.iter().any(|l| raw.contains(l));
            let run_entry = ext == "iss" && section == "[Run]";
            if launches || run_entry {
                found.push(at(1));
                rule1 = true;
            }
        }

        // Rule 2: no elevating start, unless that exact line is allow-listed.
        if names_elevation(raw) && !is_text && !allowed(ELEVATION_ALLOWED, 1, rel, raw, used) {
            found.push(at(2));
        }

        // Rule 3: a process start names its token, in a comment.
        if starts_process && !is_text {
            let from = i.saturating_sub(8);
            if !lines[from..=i].iter().any(|l| says_token(l, ext)) {
                found.push(at(3));
            }
        }

        // Rule 4: no process start in a file that names the daemon.
        if !in_service
            && starts_process
            && !is_text
            && !rule1
            && file_names_daemon
            && !allowed(DAEMON_FILE_STARTS_ALLOWED, 2, rel, raw, used)
        {
            found.push(at(4));
        }
    }
    found
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn skipped_dir(name: &str) -> bool {
    matches!(
        name,
        "target" | "node_modules" | "examples" | "tests" | ".git" | "out" | ".next"
    )
}

fn skipped_file(name: &str) -> bool {
    name == "tests.rs" || name.ends_with("_tests.rs")
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
        } else if !skipped_file(&name)
            && matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("rs" | "ps1" | "iss" | "cmd" | "bat")
            )
        {
            out.push(path);
        }
    }
}

fn scan_repo() -> (usize, Vec<Finding>, Used) {
    let root = repo_root();
    let mut files = Vec::new();
    for dir in ["crates", "app/src-tauri", "installer", "tools"] {
        collect(&root.join(dir), &mut files);
    }
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
        // Lossy: a non-UTF-8 byte must not hide a file from the scan.
        let text =
            String::from_utf8_lossy(&fs::read(path).expect("read a source file")).into_owned();
        found.extend(scan_text_used(&rel, &text, &mut used));
    }
    (files.len(), found, used)
}

#[test]
fn only_the_service_starts_the_daemon_and_every_launch_names_its_token() {
    let (scanned, found, used) = scan_repo();
    assert!(
        scanned > 50,
        "the scan found only {scanned} files: the roots moved?"
    );
    let report: Vec<String> = found
        .iter()
        .map(|f| {
            let what = match f.rule {
                1 => "starts or registers canvas-rgbd outside canvas-svc (only the service launches the daemon)",
                2 => "starts a process with an elevating verb (start setup unelevated and let it raise itself)",
                4 => "starts a process in a file that names the daemon (only the service launches it; allow-list the line if it is not the daemon)",
                _ => "starts a process without a `token:` comment saying which token it runs with",
            };
            format!("{}:{}: {what}: {}", f.file, f.line, f.text)
        })
        .collect();
    assert!(
        report.is_empty(),
        "process-launch findings:\n{}",
        report.join("\n")
    );
    // An allow-list entry that suppressed nothing is a hole waiting for the next line that matches it.
    let mut stale = Vec::new();
    for (id, list) in [(1u8, ELEVATION_ALLOWED), (2u8, DAEMON_FILE_STARTS_ALLOWED)] {
        for (i, (suffix, text, why)) in list.iter().enumerate() {
            if !used.contains(&(id, i)) {
                stale.push(format!("{suffix}: `{text}` ({why})"));
            }
        }
    }
    assert!(stale.is_empty(), "allow-list entries that matched no line (delete them):\n{}", stale.join("\n"));
}

// ---- the scanner itself: each rule fires on the shape of the bug it exists for ----

#[test]
fn rule_1_catches_the_daemon_run_key_write_of_issue_6() {
    let old = "Set-ItemProperty -LiteralPath $runKey -Name $c.RunKeyDaemon -Value ('\"{0}\"' -f (Join-Path $InstallDir $c.DaemonExe))";
    let found = scan_text("installer/setup-user.ps1", old);
    assert_eq!(found.iter().map(|f| f.rule).collect::<Vec<_>>(), vec![1]);
    // The same line is fine inside the service crate, and deleting the value is not a launch.
    assert!(scan_text(
        "crates/canvas-svc/src/x.rs",
        "Command::new(\"canvas-rgbd\") // token: user"
    )
    .is_empty());
    assert!(scan_text(
        "installer/setup-user.ps1",
        "Remove-ItemProperty -LiteralPath $runKey -Name $c.RunKeyDaemon"
    )
    .is_empty());
}

#[test]
fn rule_1_catches_an_installer_that_execs_the_daemon_or_runs_it_from_the_run_section() {
    let exec = "  Exec(ExpandConstant('{app}\\canvas-rgbd.exe'), '', '', SW_HIDE, ewNoWait, Code);";
    assert_eq!(
        scan_text("installer/inno/canvas-rgb.iss", exec)
            .iter()
            .map(|f| f.rule)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    let run = "[Run]\nFilename: \"{app}\\canvas-rgbd.exe\"; Flags: nowait";
    assert!(scan_text("installer/inno/canvas-rgb.iss", run)
        .iter()
        .any(|f| f.rule == 1));
    let files = "[Files]\nSource: \"x\\canvas-rgbd.exe\"; DestDir: \"{app}\"";
    assert!(scan_text("installer/inno/canvas-rgb.iss", files).is_empty());
}

#[test]
fn rule_2_catches_the_elevated_tray_launch_of_issue_28() {
    let old = "            lpVerb: w!(\"runas\"),";
    assert_eq!(
        scan_text("app/src-tauri/src/update/launch.rs", old)
            .iter()
            .map(|f| f.rule)
            .collect::<Vec<_>>(),
        vec![2]
    );
    let hand = "Start-Process -FilePath .\\canvas-rgb-setup.exe -Verb RunAs";
    assert!(scan_text("docs/x.ps1", hand).iter().any(|f| f.rule == 2));
    assert!(scan_text(
        "installer/setup.ps1",
        "$p = Start-Process -FilePath 'powershell.exe' -ArgumentList $childArgs -Verb RunAs # token: elevated"
    )
    .is_empty());
}

#[test]
fn rule_3_wants_a_token_comment_within_eight_lines() {
    let bare = "fn f() {\n    let c = std::process::Command::new(\"x\");\n}";
    assert_eq!(
        scan_text("crates/a/src/lib.rs", bare)
            .iter()
            .map(|f| f.rule)
            .collect::<Vec<_>>(),
        vec![3]
    );
    let tagged = "fn f() {\n    // token: the caller's, non-elevated.\n    let c = std::process::Command::new(\"x\");\n}";
    assert!(scan_text("crates/a/src/lib.rs", tagged).is_empty());
    let far = format!(
        "// token: x\n{}let c = Command::new(\"x\");",
        "\n".repeat(9)
    );
    assert_eq!(scan_text("crates/a/src/lib.rs", &far).len(), 1);
}

// ---- the scanner against the evasions a reviewer found ----

fn rules(rel: &str, text: &str) -> Vec<u8> {
    scan_text(rel, text).iter().map(|f| f.rule).collect()
}

#[test]
fn rule_2_matches_every_spelling_of_the_elevating_verb() {
    let spellings = [
        "Start-Process x -Verb:RunAs",
        "Start-Process x -verb runas",
        "Start-Process x -VERB 'RUNAS'",
        "$a = @{ Verb = 'RunAs' }",
        "$a = @{ Verb='runas' }",
        "$a = @{ Verb = \"RunAs\" }",
        "lpVerb: w!(\"RunAs\"),",
        "let verb = \"RUNAS\";",
    ];
    for line in spellings {
        let ext = if line.contains("w!") || line.contains("let ") { "crates/a/src/x.rs" } else { "tools/x.ps1" };
        assert!(rules(ext, line).contains(&2), "missed: {line}");
    }
    // Not the elevating verb: an Inno flag that merely contains the letters.
    assert!(rules("installer/inno/canvas-rgb.iss", "Filename: \"{app}\\canvas-rgb.exe\"; Flags: nowait runasoriginaluser").is_empty());
}

#[test]
fn rule_3_needs_token_in_a_comment_not_in_code() {
    let code = "fn f() {\n    let token: u32 = 1;\n    let c = std::process::Command::new(\"x\");\n}";
    assert_eq!(rules("crates/a/src/lib.rs", code), vec![3]);
    let trailing = "fn f() {\n    let c = std::process::Command::new(\"x\"); // token: the user's\n}";
    assert!(rules("crates/a/src/lib.rs", trailing).is_empty());
    let ps = "$token: = 1\nStart-Process x";
    assert_eq!(rules("tools/x.ps1", ps), vec![3]);
    let ps_ok = "# token: the person's\nStart-Process x";
    assert!(rules("tools/x.ps1", ps_ok).is_empty());
    let iss_ok = "{ token: the original user's }\nExec(A, B, C, SW_HIDE, ewWait, Code);";
    assert!(rules("installer/inno/canvas-rgb.iss", iss_ok).is_empty());
    let iss_code = "[Code]\nvar token: Integer;\nExec(A, B, C, SW_HIDE, ewWait, Code);";
    assert_eq!(rules("installer/inno/canvas-rgb.iss", iss_code), vec![3]);
}

#[test]
fn the_elevation_allow_list_is_per_line_not_per_file() {
    let allowed_line = "# token: elevated\n$p = Start-Process -FilePath 'powershell.exe' -ArgumentList $childArgs -Verb RunAs -Wait";
    assert!(rules("installer/setup.ps1", allowed_line).is_empty());
    let second = format!("{allowed_line}\n# token: elevated\nStart-Process -FilePath 'tray.exe' -Verb RunAs");
    assert_eq!(rules("installer/setup.ps1", &second), vec![2]);
}

#[test]
fn rule_4_catches_a_daemon_launch_through_a_variable_path() {
    let sneaky = "struct S { daemon_path: PathBuf }\nfn go(s: &S) {\n    // token: the user's\n    let p = &s.daemon_path;\n    let c = Command::new(p);\n}";
    assert_eq!(rules("crates/canvas-daemon/src/x.rs", sneaky), vec![4]);
    // Naming the daemon on the launching line itself is rule 1.
    assert_eq!(rules("crates/canvas-daemon/src/x.rs", "// token: x\nCommand::new(&s.daemon_path);"), vec![1]);
    // The service is the one place that may.
    assert!(rules("crates/canvas-svc/src/x.rs", sneaky).is_empty());
    // A file that never names the daemon is not flagged by rule 4 (the accepted residual gap above).
    let other = "fn go() {\n    // token: the user's\n    let c = Command::new(\"node\");\n}";
    assert!(rules("tools/y/src/main.rs", other).is_empty());
    // Inno: [Files] listing the exe does not make every Exec a finding; naming it in [Code] does.
    let files = "[Files]\nSource: \"x\\canvas-rgbd.exe\"; DestDir: \"{app}\"\n[Code]\n// token: x\nExec(A, B, C, SW_HIDE, ewWait, Code);";
    assert!(rules("installer/inno/canvas-rgb.iss", files).is_empty());
    let code = "[Code]\nDaemon := '{app}\\canvas-rgbd.exe';\n// token: x\nExec(Daemon, B, C, SW_HIDE, ewWait, Code);";
    assert_eq!(rules("installer/inno/canvas-rgb.iss", code), vec![4]);
}

#[test]
fn rule_4_matches_daemon_names_in_any_case_including_the_constant() {
    let src = "const DAEMON_EXE: &str = \"x\";\nfn go() {\n    // token: x\n    let c = Command::new(p);\n}";
    assert_eq!(rules("app/src-tauri/src/y.rs", src), vec![4]);
    let upper = "// token: x\nlet n = \"CANVAS-RGBD.EXE\";\nlet c = Command::new(n);";
    assert_eq!(rules("crates/canvas-daemon/src/y.rs", upper), vec![4]);
}

#[test]
fn the_other_process_start_spellings_need_a_token_line() {
    for call in ["CreateProcessA(x)", "ShellExecuteA(x)", "ShellExecuteExA(x)", "WinExec(x)", "saps x", "Invoke-Item x"] {
        let file = if call.contains('(') { "crates/a/src/x.rs" } else { "tools/x.ps1" };
        assert_eq!(rules(file, call), vec![3], "{call}");
    }
}

#[test]
fn a_used_allow_list_entry_is_recorded_and_an_unused_one_is_not() {
    let mut used = Used::new();
    let line = "# token: x\nStart-Process -FilePath 'powershell.exe' -ArgumentList $childArgs -Verb RunAs";
    assert!(scan_text_used("installer/setup.ps1", line, &mut used).iter().all(|f| f.rule != 2));
    assert!(used.contains(&(1, 0)));
    assert!(!used.contains(&(1, 1)));
}
