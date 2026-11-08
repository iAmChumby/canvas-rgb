//! "Status computed, never drawn" (`cause:status-undrawn`, #246).
//!
//! A view can compute a status message into a field and never read it, so the
//! message never reaches the screen while React draws it. The first case was
//! the Layout preview line, then the Layout stale-layouts notice. This scan is
//! the class's guard: it fails with `file:line field` when a status field in a
//! view has no read in its own module.
//!
//! A status field is one whose name ends in `_error`, `_err`, `_notice`,
//! `_warning`, `_problem` or `_message`, or is exactly `notice`, `note` or
//! `toast`. Its module is the file that declares it plus, for
//! `views/<area>.rs`, its `views/<area>/` submodule directory (where one page's
//! render helpers live). A name declared by more than one struct in a module
//! is checked in its own file instead, so two `save_error`s cannot pass on each
//! other's reads.
//!
//! A field that is deliberately not drawn needs an entry in `ALLOWED` with the
//! reason; an entry that matches nothing fails, so fixing one forces its
//! removal. What the scan cannot catch: a field that is read for a condition
//! (a colour, a branch) but whose words are never drawn, and a message built
//! inline with no field at all.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const SUFFIXES: &[&str] = &["_error", "_err", "_notice", "_warning", "_problem", "_message"];
const EXACT: &[&str] = &["notice", "note", "toast"];

/// (declaring file relative to the crate, field, why it is not drawn)
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "src/views/effects/stack_builder.rs",
        "save_error",
        "#246 sibling in views/effects (another builder's area): React draws it as an ErrorPanel \
         (StackBuilder.tsx:1501-1508) and GPUI only computes it",
    ),
    (
        "src/views/effects/stack_builder.rs",
        "naming_error",
        "#246 sibling in views/effects (another builder's area): React draws it as an error-line \
         (StackBuilder.tsx:2120-2124) and GPUI only computes it",
    ),
];

fn is_status(name: &str) -> bool {
    EXACT.contains(&name) || SUFFIXES.iter().any(|s| name.ends_with(s))
}

/// Every `pub name: ...` line in a file: the line number and the name.
fn declarations(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let t = line.trim_start();
            let rest = t.strip_prefix("pub ")?;
            if rest.starts_with('(') {
                return None;
            }
            let (name, _) = rest.split_once(':')?;
            let name = name.trim();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return None;
            }
            Some((i + 1, name.to_string()))
        })
        .collect()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The files one declaration's module spans: its own file and the page's
/// submodule directory.
fn module_files(file: &Path, views: &Path) -> Vec<PathBuf> {
    let mut out = vec![file.to_path_buf()];
    let Ok(rel) = file.strip_prefix(views) else {
        return out;
    };
    let mut comps = rel.components();
    let Some(first) = comps.next() else {
        return out;
    };
    let area = first.as_os_str().to_string_lossy().to_string();
    let dir = if area.ends_with(".rs") || comps.next().is_some() {
        views.join(area.trim_end_matches(".rs"))
    } else {
        views.join(&area)
    };
    if dir.is_dir() {
        rust_files(&dir, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// The file without its `#[cfg(test)]` items: a test that names a field is
/// not a place the app draws it.
fn without_tests(text: &str) -> String {
    let mut out = text.to_string();
    while let Some(pos) = out.find("#[cfg(test)]") {
        let Some(open_rel) = out[pos..].find('{') else {
            break;
        };
        let open = pos + open_rel;
        let bytes = out.as_bytes();
        let mut depth = 0i32;
        let mut close = None;
        for (i, b) in bytes.iter().enumerate().skip(open) {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = close.map_or(bytes.len(), |c| c + 1);
        let blanked: String = out[pos..end]
            .chars()
            .map(|ch| if ch == '\n' { '\n' } else { ' ' })
            .collect();
        out.replace_range(pos..end, &blanked);
    }
    out
}

/// (reads, writes) of `.name` in `text`. `self.name = ...` is a write;
/// `self.name.is_some()`, `name.clone()` and `name ==` are reads.
fn mentions(text: &str, name: &str) -> (usize, usize) {
    let needle = format!(".{name}");
    let (mut reads, mut writes) = (0usize, 0usize);
    for line in text.lines() {
        let mut from = 0;
        while let Some(pos) = line[from..].find(&needle) {
            let at = from + pos;
            let after = &line[at + needle.len()..];
            let boundary = after
                .chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
            if boundary {
                let rest = after.trim_start();
                if rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>") {
                    writes += 1;
                } else {
                    reads += 1;
                }
            }
            from = at + needle.len();
        }
    }
    (reads, writes)
}

#[test]
fn every_status_field_a_view_computes_is_read_where_it_renders() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let views = manifest.join("src/views");
    let mut files = Vec::new();
    rust_files(&views, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no view files found under {views:?}");

    let mut texts: HashMap<PathBuf, String> = HashMap::new();
    for file in &files {
        let text = fs::read_to_string(file).unwrap_or_else(|e| panic!("read {file:?}: {e}"));
        texts.insert(file.clone(), without_tests(&text));
    }

    // name -> the declaring files in the module, to spot a colliding name.
    let mut hits: Vec<(String, String)> = Vec::new();
    let mut allowed_seen: Vec<(String, String)> = Vec::new();
    for file in &files {
        let Some(text) = texts.get(file) else {
            continue;
        };
        let module = module_files(file, &views);
        for (_, name) in declarations(text) {
            if !is_status(&name) {
                continue;
            }
            let owners: usize = module
                .iter()
                .filter_map(|f| texts.get(f.as_path()))
                .map(|t| {
                    declarations(t)
                        .iter()
                        .filter(|(_, n)| n == &name)
                        .count()
                })
                .sum();
            // Two structs with the same field name in one module: check the
            // declaring file only, or one would pass on the other's reads.
            let scope: Vec<&PathBuf> = if owners > 1 {
                vec![file]
            } else {
                module.iter().collect()
            };
            let reads: usize = scope
                .iter()
                .filter_map(|f| texts.get(*f))
                .map(|t| mentions(t, &name).0)
                .sum();
            if reads == 0 {
                let rel = file
                    .strip_prefix(manifest)
                    .unwrap_or(file)
                    .to_string_lossy()
                    .replace('\\', "/");
                hits.push((rel, name.clone()));
            }
        }
    }
    hits.sort();
    hits.dedup();

    let mut unexplained = Vec::new();
    for (file, name) in &hits {
        let allowed = ALLOWED
            .iter()
            .find(|(f, n, _)| f == file && n == name)
            .is_some();
        if allowed {
            allowed_seen.push((file.clone(), name.clone()));
        } else {
            unexplained.push(format!("{file} {name}"));
        }
    }
    assert!(
        unexplained.is_empty(),
        "status fields computed and never read (React draws each one; draw it, or name it in \
         ALLOWED with the reason): {}",
        unexplained.join(", ")
    );
    for (file, name, _) in ALLOWED {
        assert!(
            allowed_seen.iter().any(|(f, n)| f == file && n == name),
            "ALLOWED names {file} {name}, which is drawn now (or gone): delete the entry"
        );
    }
}

#[test]
fn the_scan_tells_a_write_from_a_read_and_ignores_other_names() {
    assert_eq!(mentions("self.hot_error = None;", "hot_error"), (0, 1));
    assert_eq!(mentions("if let Some(e) = &self.hot_error {", "hot_error"), (1, 0));
    assert_eq!(mentions("self.hot_error.is_some() && x.hot_error == y", "hot_error"), (2, 0));
    assert_eq!(mentions("self.hot_error_clone = 1;", "hot_error"), (0, 0));
    assert!(is_status("layouts_error"));
    assert!(is_status("notice"));
    assert!(!is_status("progress_id"));
    assert!(!is_status("save_error_id"));
}
