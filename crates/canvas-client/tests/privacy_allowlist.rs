//! Drift guard for docs/privacy.md.
//!
//! CI already runs this crate's tests (the `rust` job), so a new outbound
//! endpoint or HTTP client in shipped source fails the build until the privacy
//! table and docs/privacy-allowlist.txt are updated together.
//!
//! To update the list: change docs/privacy.md first, add the host, or the
//! constructor together with its repo-relative file, to
//! docs/privacy-allowlist.txt, then run
//! `cargo test -p canvas-client --test privacy_allowlist`.
//!
//! Only shipped source is scanned: crate `src/` folders, the Tauri shell, the
//! editor (minus generated types and tests), the installer scripts and the two
//! config files that name endpoints. Test-only files, the attributed `#[cfg(test)]`
//! item (the item only, never the rest of its file), comments, format-string
//! templates and non-routable fixture addresses cannot make a connection in the
//! released app. Loopback, `.localhost` and `.invalid` addresses are local and
//! are reported in the local table instead.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const LIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/privacy-allowlist.txt");

/// Connections a shipped source file can open. A token is only an error when
/// it appears in source and is missing from the allow-list.
const CONNECTION_TOKENS: &[&str] = &[
    "ureq::Agent::config_builder",
    "reqwest::Client::builder",
    "reqwest::Client::new",
    "reqwest::get(",
    "reqwest::blocking::Client",
    "hyper::Client::",
    "ureq::get(",
    "ureq::agent(",
    "TcpStream::connect",
    "to_socket_addrs",
    "UdpSocket::bind",
    "lookup_host",
    "connect_async",
    "new WebSocket",
    "ws://",
    "wss://",
    "fetch(",
    "XMLHttpRequest",
    "sendBeacon",
    "EventSource",
    "Invoke-WebRequest",
    "Invoke-RestMethod",
    "System.Net.WebClient",
    "tauri_plugin_updater",
    "plugin:opener|open_url",
];

#[derive(Default)]
struct Allow {
    hosts: BTreeSet<String>,
    /// A constructor is allowed in a named file, not everywhere in the tree.
    tokens: BTreeSet<(String, String)>,
}

fn read_allow_list() -> Allow {
    let text = fs::read_to_string(LIST).expect("read docs/privacy-allowlist.txt");
    parse_allow_list(&text)
}

fn parse_allow_list(text: &str) -> Allow {
    let mut allow = Allow::default();
    let mut section = "";
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line {
            "[hosts]" => section = "hosts",
            "[constructors]" => section = "constructors",
            _ => match section {
                "hosts" => {
                    allow.hosts.insert(line.to_ascii_lowercase());
                }
                "constructors" => {
                    let (token, path) = line.split_once('=').unwrap_or_else(|| {
                        panic!("allow-list entry {line:?} must be '<token> = <repo-relative path>'")
                    });
                    let token = token.trim();
                    let path = path.trim().replace('\\', "/");
                    assert!(
                        !token.is_empty() && !path.is_empty(),
                        "allow-list entry {line:?} needs a token and a path"
                    );
                    allow.tokens.insert((token.to_string(), path));
                }
                other => panic!("unknown section {other:?} in docs/privacy-allowlist.txt"),
            },
        }
    }
    allow
}

fn is_comment(line: &str, ext: &str) -> bool {
    let trimmed = line.trim_start();
    match ext {
        "ps1" => trimmed.starts_with('#'),
        "iss" => trimmed.starts_with(';') || trimmed.starts_with("//"),
        "json" => false,
        _ => trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*'),
    }
}

#[derive(Default)]
struct CodeState {
    block_comment: bool,
    string: bool,
    raw: Option<usize>,
}

/// Blank out comments and string literals so their braces cannot end a test item early.
fn code_only(line: &str, state: &mut CodeState) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(hashes) = state.raw {
            if ch == '"' {
                let mut seen = 0;
                while seen < hashes && chars.peek() == Some(&'#') {
                    chars.next();
                    seen += 1;
                }
                if seen == hashes {
                    state.raw = None;
                }
            }
            out.push(' ');
            continue;
        }
        if state.block_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                state.block_comment = false;
            }
            out.push(' ');
            continue;
        }
        if state.string {
            if ch == '\\' {
                chars.next();
                out.push(' ');
            }
            if ch == '"' {
                state.string = false;
            }
            out.push(' ');
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            out.extend(std::iter::repeat_n(' ', line.len().saturating_sub(out.len())));
            break;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            state.block_comment = true;
            out.push_str("  ");
            continue;
        }
        if ch == '"' {
            state.string = true;
            out.push(' ');
            continue;
        }
        // Raw strings: r"..." and r#"..."# (and more hashes).
        if ch == 'r' {
            let mut hashes = 0;
            let mut look = chars.clone();
            while look.peek() == Some(&'#') {
                look.next();
                hashes += 1;
            }
            if look.peek() == Some(&'"') {
                for _ in 0..hashes {
                    chars.next();
                }
                chars.next();
                state.raw = Some(hashes);
                out.extend(std::iter::repeat_n(' ', hashes + 2));
                continue;
            }
        }
        out.push(ch);
    }
    out
}

fn brace_balance(code: &str) -> i32 {
    code.chars()
        .fold(0, |n, ch| match ch {
            '{' => n + 1,
            '}' => n - 1,
            _ => n,
        })
}

/// Lines that can run in the shipped product. The `#[cfg(test)]` attribute is
/// followed by exactly one item: a braced block is skipped by matching braces,
/// a single-line item (`mod tests;`, `use ...;`) by its semicolon. Everything
/// after that item is scanned as usual.
fn shipped_lines<'a>(text: &'a str, ext: &str) -> Vec<(usize, &'a str)> {
    let lines: Vec<&'a str> = text.lines().collect();
    let mut skip = vec![false; lines.len()];
    let mut state = CodeState::default();
    let mut i = 0;
    while i < lines.len() {
        let code = code_only(lines[i], &mut state);
        let Some(pos) = lines[i].find("#[cfg(test)]") else {
            i += 1;
            continue;
        };
        if !lines[i][..pos].trim().is_empty() {
            i += 1;
            continue;
        }
        skip[i] = true;
        let mut depth = 0i32;
        let mut started = false;
        let tail = &code[pos..];
        if let Some(open) = tail.find('{') {
            started = true;
            depth = brace_balance(&tail[open..]);
        } else if tail.contains(';') {
            i += 1;
            continue;
        }
        if started && depth <= 0 {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < lines.len() {
            let item = code_only(lines[j], &mut state);
            skip[j] = true;
            if !started {
                if let Some(open) = item.find('{') {
                    started = true;
                    depth = brace_balance(&item[open..]);
                } else if item.contains(';') {
                    break;
                }
            } else {
                depth += brace_balance(&item);
            }
            if started && depth <= 0 {
                break;
            }
            j += 1;
        }
        i = j + 1;
    }
    lines
        .iter()
        .enumerate()
        .filter(|(n, line)| !skip[*n] && !is_comment(line, ext))
        .map(|(n, line)| (n, *line))
        .collect()
}

/// The host part of a URL, after its `http://` or `https://` prefix.
fn url_host(rest: &str) -> Option<String> {
    let end = rest
        .char_indices()
        .find(|(_, ch)| {
            matches!(ch, '/' | '?' | '#' | ' ' | '\t' | '"' | '\'' | '`' | ')' | '}' | ';' | ',' | '\\')
        })
        .map_or(rest.len(), |(i, _)| i);
    let authority = &rest[..end];
    let hostport = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(inside) = hostport.strip_prefix('[') {
        inside.split(']').next().unwrap_or(inside)
    } else {
        hostport.split(':').next().unwrap_or(hostport)
    };
    let host = host.trim().to_ascii_lowercase();
    // A format-string placeholder is not an address. Nor is a wildcard bind.
    if host.is_empty() || host == "::" || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b':') {
        return None;
    }
    Some(host)
}

fn is_local(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host.starts_with("127.")
        || host == "::1"
        || host == "0.0.0.0"
        || host.ends_with(".invalid")
}

fn collect(dir: &Path, exts: &[&str], skip: &[&str], out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !skip.contains(&name) {
                collect(&path, exts, skip, out);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| exts.contains(&ext))
        {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if stem != "tests" && !stem.ends_with("_tests") {
                out.push(path);
            }
        }
    }
}

fn shipped_sources(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root.join("crates")).expect("the crates folder") {
        let src = entry.expect("crate entry").path().join("src");
        if src.is_dir() {
            collect(&src, &["rs"], &["tests"], &mut files);
        }
    }
    collect(&root.join("app/src-tauri/src"), &["rs"], &[], &mut files);
    collect(&root.join("app/web/src"), &["ts", "tsx"], &["__tests__", "proto"], &mut files);
    collect(&root.join("installer"), &["ps1", "iss"], &["tests"], &mut files);
    files.push(root.join("app/src-tauri/tauri.conf.json"));
    files.push(root.join("installer/openrgb.lock.json"));
    files.sort();
    files
}

#[test]
fn every_outbound_host_and_client_is_in_the_privacy_table() {
    let allow = read_allow_list();
    let root = Path::new(ROOT);
    let files = shipped_sources(root);
    let mut unexpected = Vec::new();
    let mut found_hosts = BTreeSet::new();
    let mut found_tokens = BTreeSet::new();

    for path in &files {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {relative}: {e}"));
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        for (number, line) in shipped_lines(&text, ext) {
            // URLs: report every non-local host this source can name.
            let mut at = 0;
            loop {
                let schemes = [
                    ("http://", 7usize),
                    ("https://", 8usize),
                    ("ws://", 5usize),
                    ("wss://", 6usize),
                ];
                let next = schemes
                    .iter()
                    .filter_map(|(prefix, len)| line[at..].find(prefix).map(|i| (at + i, *len)))
                    .min_by_key(|(offset, _)| *offset);
                let Some((offset, len)) = next else { break };
                let start = offset + len;
                if let Some(host) = url_host(&line[start..]) {
                    if !is_local(&host) {
                        found_hosts.insert(host.clone());
                        if !allow.hosts.contains(&host) {
                            unexpected.push(format!(
                                "{relative}:{}: outbound host {host:?} is not in docs/privacy-allowlist.txt",
                                number + 1
                            ));
                        }
                    }
                }
                at = start;
            }

            // Client constructors: the same guard for connections with no URL literal.
            for token in CONNECTION_TOKENS {
                if line.contains(token) {
                    found_tokens.insert((*token).to_string());
                    if !allow.tokens.contains(&((*token).to_string(), relative.clone())) {
                        unexpected.push(format!(
                            "{relative}:{}: connection constructor {token:?} is not allowed in this file by docs/privacy-allowlist.txt",
                            number + 1
                        ));
                    }
                }
            }
        }
    }

    println!(
        "privacy scan: {} source files, {} outbound hosts, {} connection constructors",
        files.len(),
        found_hosts.len(),
        found_tokens.len()
    );
    assert_eq!(files.len(), 475, "the set of scanned source files moved");
    assert_eq!(found_hosts.len(), 3, "the set of outbound hosts moved");
    assert_eq!(found_tokens.len(), 8, "the set of connection constructors moved");
    assert!(
        unexpected.is_empty(),
        "new network access needs a row in docs/privacy.md and an allow-list entry:\n{}",
        unexpected.join("\n")
    );
}

#[cfg(test)]
mod tests {
    use super::{parse_allow_list, shipped_lines};

    const FIXTURE: &str = r#"use std::net::TcpStream;

#[cfg(test)]
mod t {
    #[test]
    fn test_only() {
        let _ = TcpStream::connect(("127.0.0.1", 1));
    }
}

pub fn shipped() {
    let _ = TcpStream::connect(("nonlocal.example", 1));
}
"#;

    #[test]
    fn cfg_test_item_is_skipped_but_code_after_it_is_scanned() {
        let lines = shipped_lines(FIXTURE, "rs");
        let connects: Vec<(usize, &str)> = lines
            .iter()
            .copied()
            .filter(|(_, line)| line.contains("TcpStream::connect"))
            .collect();
        assert_eq!(
            connects.len(),
            1,
            "the shipped connect after the cfg(test) item must be seen; got {lines:?}"
        );
        assert!(
            connects[0].1.contains("nonlocal.example"),
            "the shipped connect, not the test one, must be seen: {:?}",
            connects[0]
        );
        assert!(
            !lines.iter().any(|(_, line)| line.contains("test_only")),
            "the connect inside the cfg(test) item must be skipped: {lines:?}"
        );
    }

    #[test]
    fn allow_list_is_keyed_by_token_and_file() {
        let allow = parse_allow_list(
            "[constructors]\nfetch( = app/web/src/lib/api.ts\nnew WebSocket = app/web/src/lib/preview.ts\n",
        );
        assert!(allow
            .tokens
            .contains(&("fetch(".to_string(), "app/web/src/lib/api.ts".to_string())));
        assert!(!allow
            .tokens
            .contains(&("fetch(".to_string(), "app/web/src/lib/other.ts".to_string())));
    }
}
