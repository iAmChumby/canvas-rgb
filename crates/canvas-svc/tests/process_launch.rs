//! The `cause:process-launch` class check (docs/bug-classes.md; #6 and #28).
//!
//! Three rules, run over the repo's own source on every `cargo test --workspace` (so in CI):
//!
//! 1. Nothing outside `crates/canvas-svc` starts `canvas-rgbd.exe`. The service launches the daemon
//!    in the user's session with no window; any other path skips those rules (#6). A line that names
//!    the daemon (`canvas-rgbd`, `DaemonExe`, `RunKeyDaemon`) beside a process-start or Run-key
//!    write is a finding, and so is any `[Run]` entry in the Inno script that names it.
//! 2. No code asks Windows to start a process elevated with an elevating `ShellExecute` verb or
//!    `Start-Process -Verb RunAs`, except the commented allow-list below (#28: setup started
//!    elevated relaunched the tray elevated).
//! 3. Every place that starts a process says which token it runs with: a comment containing
//!    `token:` on the same line or in the 8 lines above it.
//!
//! The scan reads `*.rs`, `*.ps1`, `*.iss`, `*.cmd` and `*.bat` under `crates/`, `app/src-tauri/`,
//! `installer/` and `tools/`. Comment lines never count as a launch. Test files are skipped (a
//! `tests/` folder, `tests.rs`, `*_tests.rs`), because the scan lives in one and tests may start
//! anything; so are `target/`, `node_modules/` and `examples/` (hand-run probes, never shipped).

use std::fs;
use std::path::{Path, PathBuf};

/// Names that mean "the daemon".
const DAEMON_NAMES: &[&str] = &["canvas-rgbd", "canvas_rgbd", "DaemonExe", "RunKeyDaemon"];

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

/// Spellings of an elevating start.
const ELEVATING: &[&str] = &[
    "\"runas\"",
    "'runas'",
    "-Verb RunAs",
    "-Verb 'RunAs'",
    "-Verb \"RunAs\"",
];

/// Rule 2 allow-list: (path suffix, why). Each entry is the script installer elevating its own
/// PowerShell child once (the machine-wide half); the per-user half then runs as the person, in a
/// separate non-elevated process, and nothing here relaunches the tray.
const ELEVATION_ALLOWED: &[(&str, &str)] = &[
    (
        "installer/setup.ps1",
        "script install: elevates its machine-wide child (setup-elevated.ps1)",
    ),
    (
        "installer/uninstall.ps1",
        "script uninstall: elevates its machine-wide child (uninstall-elevated.ps1)",
    ),
];

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

/// Scans one file's text. `rel` is the repo-relative path with `/` separators.
fn scan_text(rel: &str, text: &str) -> Vec<Finding> {
    let ext = rel.rsplit('.').next().unwrap_or("");
    let in_service = rel.starts_with("crates/canvas-svc/");
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut section = String::new();
    for (i, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        if ext == "iss" && trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed.to_string();
        }
        if trimmed.is_empty() || is_comment(trimmed, ext) {
            continue;
        }
        let at = |rule: u8| Finding {
            file: rel.to_string(),
            line: i + 1,
            rule,
            text: trimmed.to_string(),
        };
        let names_daemon = DAEMON_NAMES.iter().any(|n| raw.contains(n));

        // Rule 1: only the service starts the daemon.
        if !in_service && names_daemon {
            let launches = LAUNCHES.iter().any(|l| raw.contains(l));
            let run_entry = ext == "iss" && section == "[Run]";
            if launches || run_entry {
                found.push(at(1));
            }
        }

        // Rule 2: no elevating start, unless allow-listed.
        if ELEVATING.iter().any(|e| raw.contains(e))
            && !TEXT_LINES.iter().any(|t| raw.contains(t))
            && !ELEVATION_ALLOWED
                .iter()
                .any(|(suffix, _)| rel.ends_with(suffix))
        {
            found.push(at(2));
        }

        // Rule 3: a process start names its token.
        if PROCESS_STARTS.iter().any(|p| raw.contains(p))
            && !TEXT_LINES.iter().any(|t| raw.contains(t))
        {
            let from = i.saturating_sub(8);
            if !lines[from..=i].iter().any(|l| l.contains("token:")) {
                found.push(at(3));
            }
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

fn scan_repo() -> (usize, Vec<Finding>) {
    let root = repo_root();
    let mut files = Vec::new();
    for dir in ["crates", "app/src-tauri", "installer", "tools"] {
        collect(&root.join(dir), &mut files);
    }
    let mut found = Vec::new();
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
        found.extend(scan_text(&rel, &text));
    }
    (files.len(), found)
}

#[test]
fn only_the_service_starts_the_daemon_and_every_launch_names_its_token() {
    let (scanned, found) = scan_repo();
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
        "$p = Start-Process -FilePath 'powershell.exe' -Verb RunAs # token: elevated"
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
