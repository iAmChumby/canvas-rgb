//! The diagnostics zip: one code path for the daemon route and the tray command (#575, R1/R3/R5).
//!
//! `GET` lists what the zip would hold, before the person saves; `POST` writes it and answers
//! with the path, the file list and how many names were replaced. Every text file is scrubbed
//! first, then every device id and OpenRGB location is replaced with a per-bundle alias that is
//! the same in every file and log. The token file is never in the list. Machine logs are read
//! only after the fail-closed check in [`crate::service_logs`].

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use canvas_proto::api::{DiagBundle, DiagFile};

use crate::scrub::{self, Scrubber};
use crate::service_logs::{self, LogFs, RealLogFs};

/// The files the bundle contains, relative to the user directory. The token is not among them.
pub const CONFIG_FILES: &[&str] = &[
    "config.toml",
    "layouts.json",
    "zones.json",
    "effects.json",
    "stacks.json",
    "patterns.json",
    "displays.json",
    "fans.last.toml",
];

/// The next free bundle path in the one fixed folder. A clash adds `-2`, `-3`, ...; an existing
/// file is never opened for writing (P2).
pub fn output_path(dir: &Path) -> Result<PathBuf, BundleError> {
    let stamp = crate::paths::timestamp();
    let first = dir.join(format!("canvas-rgb-diag-{stamp}.zip"));
    if !first.exists() {
        return Ok(first);
    }
    for n in 2..=99 {
        let candidate = dir.join(format!("canvas-rgb-diag-{stamp}-{n}.zip"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(BundleError::Io(std::io::Error::other(
        "no free diagnostics zip name in the folder",
    )))
}

/// One piece of a bundle while it is being made. `bytes` are the original on-disk bytes; the
/// size in the preview comes from them, before scrubbing.
struct Source {
    name: String,
    bytes: Vec<u8>,
}

/// Everything the bundle will contain, plus the one service-log decision. A file that is not
/// text is left out and named in the file list with the reason (P5).
struct Sources {
    files: Vec<Source>,
    notices: Vec<DiagFile>,
    service_logs: Option<String>,
}

/// The bundle maker. `new` uses the live paths; tests supply their own directories and a fake
/// machine filesystem so no test reads a real `%ProgramData%`.
pub struct Bundle {
    home: PathBuf,
    logs: PathBuf,
    machine_logs: PathBuf,
    fs: Box<dyn LogFs>,
}

impl Default for Bundle {
    fn default() -> Self {
        Self::new()
    }
}

impl Bundle {
    pub fn new() -> Self {
        Self {
            home: crate::paths::home(),
            logs: crate::paths::log_dir(),
            machine_logs: crate::paths::machine_log_dir(),
            fs: Box::new(RealLogFs::default()),
        }
    }

    /// Test seam: user files on disk, machine logs behind `fs`.
    pub fn with_dirs(home: PathBuf, logs: PathBuf, machine_logs: PathBuf, fs: Box<dyn LogFs>) -> Self {
        Self { home, logs, machine_logs, fs }
    }

    /// Reads every source once. The service logs are opened and checked exactly once; the reason
    /// for a refusal is kept beside the sources so `GET` and `POST` always agree (P3).
    fn sources(&self) -> Sources {
        let mut files = Vec::new();
        let mut notices = Vec::new();
        for name in CONFIG_FILES {
            let path = self.home.join(name);
            if let Ok(bytes) = std::fs::read(&path) {
                if std::str::from_utf8(&bytes).is_ok() {
                    files.push(Source { name: format!("config/{name}"), bytes });
                } else {
                    notices.push(DiagFile {
                        name: format!("config/{name} (not included: not a text file)"),
                        bytes: 0,
                    });
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir(&self.logs) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let (Some(name), Ok(bytes)) =
                        (path.file_name().and_then(|n| n.to_str()), std::fs::read(&path))
                    {
                        if std::str::from_utf8(&bytes).is_ok() {
                            files.push(Source { name: format!("logs/{name}"), bytes });
                        } else {
                            notices.push(DiagFile {
                                name: format!("logs/{name} (not included: not a text file)"),
                                bytes: 0,
                            });
                        }
                    }
                }
            }
        }
        let service_logs = match service_logs::collect(self.fs.as_ref(), &self.machine_logs) {
            Ok(collected) => {
                for (name, bytes) in collected.logs {
                    files.push(Source { name, bytes });
                }
                // Each bad service log names itself in the file list; the other logs stay in.
                for reason in collected.notices {
                    notices.push(DiagFile { name: format!("service-logs/{reason}"), bytes: 0 });
                }
                None
            }
            Err(reason) => Some(reason),
        };
        Sources { files, notices, service_logs }
    }

    /// `GET`: the list of files with their on-disk sizes, before anything is written.
    pub fn list(&self) -> DiagBundle {
        let sources = self.sources();
        let files = sources
            .notices
            .into_iter()
            .chain(sources.files.into_iter().map(|s| DiagFile {
                name: s.name,
                bytes: s.bytes.len() as u64,
            }))
            .collect();
        DiagBundle { path: None, files, replaced: 0, service_logs: sources.service_logs }
    }

    /// `POST`: writes the zip into the one fixed folder, never over an existing file (P2). Every
    /// text file is scrubbed and every device id gets one alias per bundle.
    pub fn write(&self, scrubber: &Scrubber) -> Result<DiagBundle, BundleError> {
        let dir = crate::paths::diagnostics_dir();
        std::fs::create_dir_all(&dir)?;
        // Two writers in the same second can pick the same name; create-new is the arbiter and
        // the loser picks the next free suffix (F5).
        let (out, file) = loop {
            let candidate = output_path(&dir)?;
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
                Ok(file) => break (candidate, file),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        };
        let sources = self.sources();
        let texts: Vec<String> = sources.files.iter().map(|s| String::from_utf8_lossy(&s.bytes).into_owned()).collect();
        let aliases = aliases_for(&texts);
        let options: zip::write::FileOptions<'_, '_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
        let mut files = sources.notices;
        let mut replaced = 0u32;
        for (source, text) in sources.files.iter().zip(texts.iter()) {
            let (scrubbed, n) = scrubber.scrub_count(text);
            replaced = replaced.saturating_add(n);
            let (scrubbed, n) = apply_aliases(&scrubbed, &aliases);
            replaced = replaced.saturating_add(n);
            zip.start_file(source.name.clone(), options)?;
            zip.write_all(scrubbed.as_bytes())?;
            files.push(DiagFile { name: source.name.clone(), bytes: source.bytes.len() as u64 });
        }
        zip.finish()?;
        Ok(DiagBundle {
            path: Some(out.to_string_lossy().into_owned()),
            files,
            replaced,
            service_logs: sources.service_logs,
        })
    }
}

/// Why a bundle could not be written.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
}

/// Every distinct device id and OpenRGB location in the bundle, sorted so one bundle always
/// gives the same device the same alias, regardless of directory order.
fn aliases_for(texts: &[String]) -> Vec<(String, String)> {
    let mut found = BTreeSet::new();
    for text in texts {
        // The config files are JSON: their keys and values are the authoritative device ids.
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            visit_json(&value, &mut found);
        }
        // Logs carry ids in free text: `device=openrgb:ABC`, `[openrgb:ABC]`, `id="hid:…"`.
        // Match the prefix after any non-id character and take the whole id (F3).
        let mut at = 0;
        while at < text.len() {
            if let Some(end) = scrub::device_id_end(text, at) {
                found.insert(text[at..end].to_string());
                at = end;
            } else {
                at += text[at..].chars().next().unwrap().len_utf8();
            }
        }
        // OpenRGB logs the controller location separately from its device id.
        for token in text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']')) {
            let token = token.trim_matches(|c: char| matches!(c, ':' | ',' | '.' | ')' | '(' | '"' | '\''));
            if looks_like_location(token) {
                found.insert(token.to_string());
            }
        }
    }
    let mut sorted: Vec<String> = found.into_iter().collect();
    // Longest first: `openrgb:AB` must not eat the start of `openrgb:ABC` (P4). Ties are
    // lexicographic so one bundle always assigns the same aliases.
    sorted.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    sorted
        .into_iter()
        .enumerate()
        .map(|(i, id)| (id, format!("device-{}", i + 1)))
        .collect()
}

/// Recursively collects every string in a JSON value that names a device.
fn visit_json(value: &serde_json::Value, found: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if looks_like_device_id(key) {
                    found.insert(key.clone());
                }
                visit_json(child, found);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                visit_json(item, found);
            }
        }
        serde_json::Value::String(text)
            if looks_like_device_id(text) || looks_like_location(text) =>
        {
            found.insert(text.clone());
        }
        serde_json::Value::String(_) => {}
        _ => {}
    }
}

/// `<adapter>:<serial-or-location>`. The adapter prefix keeps timestamps and URLs out of it.
fn looks_like_device_id(text: &str) -> bool {
    let Some((prefix, rest)) = text.split_once(':') else {
        return false;
    };
    scrub::ADAPTER_PREFIXES.contains(&prefix) && rest.len() >= 2 && !rest.contains(' ') && !rest.contains('/')
}

/// The location strings Windows and OpenRGB print for a USB device.
fn looks_like_location(text: &str) -> bool {
    let upper = text.to_ascii_uppercase();
    (upper.contains("VID_") || upper.contains("PID_") || upper.contains("HID#") || upper.contains("USB\\"))
        && text.len() >= 8
        && !text.contains(' ')
}

/// Replaces the longest aliases first, so a short id cannot eat the start of a longer one.
/// Replacements are case sensitive: a device id is an exact fingerprint, not a name.
fn apply_aliases(text: &str, aliases: &[(String, String)]) -> (String, u32) {
    let mut out = text.to_string();
    let mut count = 0u32;
    for (id, alias) in aliases {
        if out.contains(id) {
            count += out.matches(id).count() as u32;
            out = out.replace(id, alias);
        }
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_logs::LogFs;
    use crate::scrub::Scrubber;
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::path::PathBuf;

    struct FakeMachine {
        owner: String,
        reparse: HashSet<PathBuf>,
        reads: Cell<usize>,
    }

    impl LogFs for FakeMachine {
        fn exists(&self, _p: &Path) -> Result<bool, String> {
            Ok(true)
        }
        fn is_reparse(&self, p: &Path) -> Result<bool, String> {
            Ok(self.reparse.contains(p))
        }
        fn owner(&self, _p: &Path) -> Result<String, String> {
            Ok(self.owner.clone())
        }
        fn files(&self, _p: &Path) -> Result<Vec<PathBuf>, String> {
            Ok(vec![PathBuf::from("svc.log")])
        }
        fn open_checked(&self, _p: &Path) -> Result<Vec<u8>, String> {
            self.reads.set(self.reads.get() + 1);
            // A fake serial and a fake USB location in the service log.
            Ok(b"controller openrgb:SERIAL-42 at HID#VID_1234 location".to_vec())
        }
    }

    /// One fake home for this test binary; setting the override once keeps parallel tests from
    /// racing on the environment and keeps every write off the real user folder.
    fn home() -> &'static PathBuf {
        static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        HOME.get_or_init(|| {
            let dir = std::env::temp_dir().join(format!(
                "canvas-diag-bundle-{}-{}",
                std::process::id(),
                crate::paths::timestamp()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var(crate::paths::HOME_ENV, &dir);
            dir
        })
    }

    fn tree() -> (PathBuf, PathBuf, FakeMachine) {
        let root = home().clone();
        let home = root.join("home");
        let logs = root.join("logs");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&logs).unwrap();
        // A fake profile path in a config file: it must not survive into the zip.
        let profile = concat!("C:\\", "Use", "rs\\Alice");
        std::fs::write(
            home.join("zones.json"),
            format!(r#"{{"openrgb:SERIAL-42":{{"path":"{profile}\\z.json","leds":4}},"hid:VID_1234":1}}"#),
        )
        .unwrap();
        std::fs::write(home.join("config.toml"), "brightness = 80\n").unwrap();
        // The real token file sits beside the config. It is never one of `CONFIG_FILES`.
        std::fs::write(home.join("api-token"), "SECRET").unwrap();
        (home, logs, FakeMachine { owner: "S-1-5-18".into(), reparse: HashSet::new(), reads: Cell::new(0) })
    }

    fn scrubber() -> Scrubber {
        Scrubber::new(
            Some(concat!("C:\\", "Use", "rs\\Alice").into()),
            Some("Alice".into()),
            Some("ALICE-PC".into()),
        )
    }

    #[test]
    fn a_fake_serial_and_location_are_aliased_the_same_way_everywhere() {
        let (home, logs, machine) = tree();
        let machine_logs = PathBuf::from("machine/logs");
        let bundle = Bundle::with_dirs(home.clone(), logs, machine_logs, Box::new(machine));
        let result = bundle.write(&scrubber()).unwrap();
        let zip = std::fs::read(result.path.clone().unwrap()).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        let mut texts = Vec::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            let mut text = String::new();
            std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
            texts.push(text);
        }
        let joined = texts.join("\n");
        assert!(!joined.contains("SERIAL-42"), "{joined}");
        assert!(!joined.contains("VID_1234"), "{joined}");
        assert!(!joined.to_ascii_lowercase().contains("alice"), "{joined}");
        assert!(!joined.contains("SECRET"), "{joined}");
        let aliases: Vec<_> = joined.matches("device-").collect();
        assert!(aliases.len() >= 2, "one alias per device: {joined}");
        assert!(joined.contains("device-1"), "{joined}");
    }

    #[test]
    fn the_zip_goes_to_the_fixed_folder_and_never_overwrites() {
        let (home, logs, machine) = tree();
        let bundle = Bundle::with_dirs(home.clone(), logs, PathBuf::from("machine/logs"), Box::new(machine));
        let first = bundle.write(&scrubber()).unwrap();
        let first_path = PathBuf::from(first.path.unwrap());
        assert_eq!(first_path.parent().unwrap(), crate::paths::diagnostics_dir());
        // An existing file stays byte-for-byte; the next save gets a new name.
        std::fs::write(&first_path, b"keep me").unwrap();
        let second = bundle.write(&scrubber()).unwrap();
        let second_path = PathBuf::from(second.path.unwrap());
        assert_ne!(first_path, second_path);
        assert_eq!(std::fs::read(&first_path).unwrap(), b"keep me");
        assert!(second_path.exists());
    }

    #[test]
    fn a_config_file_is_never_a_zip_destination() {
        // A caller cannot pass a path any more: the writer has no path argument and always uses
        // the fixed folder. The config file beside it stays byte-for-byte.
        let (home, logs, machine) = tree();
        let config = home.join("config.toml");
        let before = std::fs::read(&config).unwrap();
        let bundle = Bundle::with_dirs(home.clone(), logs, PathBuf::from("machine/logs"), Box::new(machine));
        let answer = bundle.write(&scrubber()).unwrap();
        let path = PathBuf::from(answer.path.unwrap());
        assert_ne!(path, config);
        assert_eq!(std::fs::read(&config).unwrap(), before);
    }

    #[test]
    fn ids_inside_wire_shapes_get_one_alias_each() {
        let text = r#"{"device=openrgb:ABC":1,"note":"[openrgb:ABC] and id=\"hid:VID_1\" and royuan:3151:4015"}"#.to_string();
        let aliases = aliases_for(std::slice::from_ref(&text));
        assert!(aliases.iter().any(|(id, _)| id == "openrgb:ABC"), "{aliases:?}");
        assert!(aliases.iter().any(|(id, _)| id == "hid:VID_1"), "{aliases:?}");
        assert!(aliases.iter().any(|(id, _)| id == "royuan:3151:4015"), "{aliases:?}");
        let (out, _) = apply_aliases(&text, &aliases);
        assert!(!out.contains("ABC") && !out.contains("3151") && !out.contains("VID_1"), "{out}");
    }

    #[test]
    fn a_zip_name_never_clashes() {
        let dir = home().join("clash");
        std::fs::create_dir_all(&dir).unwrap();
        let first = output_path(&dir).unwrap();
        std::fs::write(&first, b"keep me").unwrap();
        let second = output_path(&dir).unwrap();
        assert_ne!(first, second);
        assert!(second.file_name().and_then(|n| n.to_str()).unwrap().ends_with("-2.zip"), "{:?}", second.file_name());
        assert_eq!(std::fs::read(&first).unwrap(), b"keep me");
        assert!(!second.exists());
    }

    #[test]
    fn two_writers_on_one_stamp_get_distinct_files() {
        let dir = home().join("race");
        std::fs::create_dir_all(&dir).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let (tx, rx) = std::sync::mpsc::channel();
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let dir = dir.clone();
                let barrier = barrier.clone();
                let tx = tx.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    loop {
                        let candidate = output_path(&dir).unwrap();
                        match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
                            Ok(_) => {
                                tx.send(candidate).unwrap();
                                break;
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                            Err(e) => panic!("could not reserve a zip: {e}"),
                        }
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let paths: Vec<_> = rx.iter().take(2).collect();
        assert_eq!(paths.len(), 2);
        assert_ne!(paths[0], paths[1], "two racing writers must not share a path");
    }

    #[test]
    fn aliases_replace_the_longest_device_id_first() {
        let text = "openrgb:AB and openrgb:ABC and openrgb:ABCD".to_string();
        let aliases = aliases_for(std::slice::from_ref(&text));
        let (out, _) = apply_aliases(&text, &aliases);
        assert!(!out.contains("AB") && !out.contains("ABC") && !out.contains("ABCD"), "{out}");
        // A short id eating the start of a longer one leaves `device-1C`: no alias may have a
        // serial-like tail.
        let bytes = out.as_bytes();
        for i in 0..out.len().saturating_sub(8) {
            if out[i..].starts_with("device-")
                && bytes.get(i + 7).is_some_and(u8::is_ascii_digit)
                && bytes.get(i + 8).is_some_and(u8::is_ascii_alphanumeric)
            {
                panic!("a partial device alias survived: {out}");
            }
        }
        let names: Vec<_> = aliases.iter().map(|(_, a)| a.as_str()).collect();
        assert_eq!(names.len(), 3, "{aliases:?}");
    }

    #[test]
    fn a_binary_file_is_left_out_and_said_to_be() {
        let (home, logs, machine) = tree();
        std::fs::write(home.join("effects.json"), vec![0xff, 0xfe, 0x00, 0x01]).unwrap();
        let bundle = Bundle::with_dirs(home.clone(), logs, PathBuf::from("machine/logs"), Box::new(machine));
        let answer = bundle.list();
        assert!(answer.files.iter().any(|f| f.name.contains("not included: not a text file")), "{answer:?}");
        let result = bundle.write(&scrubber()).unwrap();
        let zip = std::fs::read(result.path.unwrap()).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        let names: Vec<_> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        assert!(!names.iter().any(|n| n == "config/effects.json"), "{names:?}");
    }

    #[test]
    fn a_refused_machine_tree_names_the_reason_and_is_never_read() {
        let (home, logs, mut machine) = tree();
        machine.reparse.insert(PathBuf::from("machine/logs"));
        let reason = crate::service_logs::refusal(&machine, Path::new("machine/logs")).unwrap();
        assert!(reason.contains("junction or symlink"), "{reason}");
        assert_eq!(machine.reads.get(), 0);
        let bundle = Bundle::with_dirs(home, logs, PathBuf::from("machine/logs"), Box::new(machine));
        assert!(bundle.list().service_logs.unwrap().contains("junction"));
    }

    #[test]
    fn the_token_is_never_in_the_file_list() {
        assert!(!CONFIG_FILES.contains(&"api-token"));
    }
}
