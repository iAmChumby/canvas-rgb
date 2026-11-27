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

use crate::scrub::Scrubber;
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

/// Adapter prefixes a `DeviceId` may carry. A string with one of these before the colon is a
/// device id; its mapping is fixed for the whole bundle so a serial never appears anywhere.
const ADAPTER_PREFIXES: &[&str] = &[
    "openrgb", "ring", "chione", "royuan", "attack-shark-x6", "8bitdo-ultimate-2", "8bitdo", "hid", "usb",
];

/// The output path of a bundle written to a directory, stamped with the UTC time.
pub fn output_path(dir: &Path) -> PathBuf {
    if dir.is_dir() {
        dir.join(format!("canvas-rgb-diag-{}.zip", crate::paths::timestamp()))
    } else {
        dir.to_path_buf()
    }
}

/// One piece of a bundle while it is being made. `bytes` are the original on-disk bytes; the
/// size in the preview comes from them, before scrubbing.
struct Source {
    name: String,
    bytes: Vec<u8>,
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
            fs: Box::new(RealLogFs),
        }
    }

    /// Test seam: user files on disk, machine logs behind `fs`.
    pub fn with_dirs(home: PathBuf, logs: PathBuf, machine_logs: PathBuf, fs: Box<dyn LogFs>) -> Self {
        Self { home, logs, machine_logs, fs }
    }

    /// Reads every source once. The service logs come back as an explicit reason when refused;
    /// nothing under the machine tree is ever written.
    fn sources(&self) -> Vec<Source> {
        let mut out = Vec::new();
        for name in CONFIG_FILES {
            let path = self.home.join(name);
            if let Ok(bytes) = std::fs::read(&path) {
                out.push(Source { name: format!("config/{name}"), bytes });
            }
        }
        if let Ok(entries) = std::fs::read_dir(&self.logs) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let (Some(name), Ok(bytes)) =
                        (path.file_name().and_then(|n| n.to_str()), std::fs::read(&path))
                    {
                        out.push(Source { name: format!("logs/{name}"), bytes });
                    }
                }
            }
        }
        if let Ok(logs) = service_logs::collect(self.fs.as_ref(), &self.machine_logs) {
            for (name, bytes) in logs {
                out.push(Source { name, bytes });
            }
        }
        out
    }

    /// Why the service logs are not in this bundle, or `None` when they are.
    fn service_logs_reason(&self) -> Option<String> {
        service_logs::refusal(self.fs.as_ref(), &self.machine_logs)
    }

    /// `GET`: the list of files with their on-disk sizes, before anything is written.
    pub fn list(&self) -> DiagBundle {
        let files = self
            .sources()
            .into_iter()
            .map(|s| DiagFile { name: s.name, bytes: s.bytes.len() as u64 })
            .collect();
        DiagBundle { path: None, files, replaced: 0, service_logs: self.service_logs_reason() }
    }

    /// `POST`: scrubs every text file, aliases every device id consistently, writes the zip and
    /// answers with the result. The alias mapping is built from all files at once, so the same
    /// serial in a config file and a log line gets the same alias.
    pub fn write(&self, out: &Path, scrubber: &Scrubber) -> Result<DiagBundle, BundleError> {
        let out = output_path(out);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let sources = self.sources();
        let texts: Vec<String> = sources
            .iter()
            .map(|s| String::from_utf8_lossy(&s.bytes).into_owned())
            .collect();
        let aliases = aliases_for(&texts);
        let options: zip::write::FileOptions<'_, '_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let file = std::fs::File::create(&out)?;
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
        let mut files = Vec::new();
        let mut replaced = 0u32;
        for (source, text) in sources.iter().zip(texts.iter()) {
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
            service_logs: self.service_logs_reason(),
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
        // Logs carry the same ids and the controller location in free text. Scan for the shape
        // of a device id, and for the location shapes OpenRGB and the HID adapters print.
        for token in text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '(' | ')')) {
            let token = token.trim_matches(|c: char| matches!(c, ':' | ',' | '.' | ')' | '(' | '"' | '\''));
            if looks_like_device_id(token) || looks_like_location(token) {
                found.insert(token.to_string());
            }
        }
    }
    found
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
    ADAPTER_PREFIXES.contains(&prefix) && rest.len() >= 2 && !rest.contains(' ') && !rest.contains('/')
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
        fn read(&self, _p: &Path) -> Result<Vec<u8>, String> {
            self.reads.set(self.reads.get() + 1);
            // A fake serial and a fake USB location in the service log.
            Ok(b"controller openrgb:SERIAL-42 at HID#VID_1234 location".to_vec())
        }
    }

    fn tree() -> (PathBuf, PathBuf, FakeMachine) {
        let root = std::env::temp_dir().join(format!(
            "canvas-diag-bundle-{}-{}",
            std::process::id(),
            crate::paths::timestamp()
        ));
        let home = root.join("home");
        let logs = root.join("logs");
        let _ = std::fs::remove_dir_all(&root);
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
        let out = home.join("out");
        let result = bundle.write(&out, &scrubber()).unwrap();
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
        let _ = std::fs::remove_dir_all(home.parent().unwrap());
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
