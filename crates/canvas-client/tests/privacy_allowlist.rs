//! Drift guard for docs/privacy.md.
//!
//! CI already runs this crate's tests (the `rust` job), so a new outbound
//! endpoint or HTTP client in shipped source fails the build until the privacy
//! table and docs/privacy-allowlist.txt are updated together.
//!
//! To update the list: change docs/privacy.md first, add the host or constructor
//! to docs/privacy-allowlist.txt, then run
//! `cargo test -p canvas-client --test privacy_allowlist`.
//!
//! Only shipped source is scanned: crate `src/` folders, the Tauri shell, the
//! editor (minus generated types and tests), the installer scripts and the two
//! config files that name endpoints. Test-only files, `#[cfg(test)]` modules,
//! comments, format-string templates and non-routable fixture addresses cannot
//! make a connection in the released app. Loopback, `.localhost` and `.invalid`
//! addresses are local and are reported in the local table instead.

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
    "hyper::Client::",
    "TcpStream::connect",
    "UdpSocket::bind",
    "lookup_host",
    "new WebSocket",
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
    tokens: BTreeSet<String>,
}

fn read_allow_list() -> Allow {
    let text = fs::read_to_string(LIST).expect("read docs/privacy-allowlist.txt");
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
                    allow.tokens.insert(line.to_string());
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
        for (number, line) in text.lines().enumerate() {
            // In this repo `#[cfg(test)]` modules sit at the end of a file.
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            if is_comment(line, ext) {
                continue;
            }

            // URLs: report every non-local host this source can name.
            let mut at = 0;
            loop {
                let http = line[at..].find("http://");
                let https = line[at..].find("https://");
                let (offset, prefix) = match (http, https) {
                    (Some(a), Some(b)) if a <= b => (a, 7),
                    (Some(_), Some(b)) => (b, 8),
                    (Some(a), None) => (a, 7),
                    (None, Some(b)) => (b, 8),
                    (None, None) => break,
                };
                let start = at + offset + prefix;
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
                    if !allow.tokens.contains(*token) {
                        unexpected.push(format!(
                            "{relative}:{}: connection constructor {token:?} is not in docs/privacy-allowlist.txt",
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
    assert!(
        unexpected.is_empty(),
        "new network access needs a row in docs/privacy.md and an allow-list entry:\n{}",
        unexpected.join("\n")
    );
}
