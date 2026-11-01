//! The `cause:sleep-state` class guard (#5, #29).
//!
//! Every adapter the daemon can build says what it does when the machine resumes from sleep,
//! and this test checks that it does it:
//!
//! - an adapter that holds a mode sets it again after a simulated resume (the OpenRGB
//!   controllers and the ROYUAN keyboard);
//! - an adapter that holds no mode says so, and does not take part in the resume;
//! - and the scan over the repo's own source says that every adapter there has made that
//!   choice and is on the list below, so an adapter that is added later cannot forget it.
//!
//! The scan is the half that catches the next adapter: it reads the source of every crate and
//! fails with `file:line` when an adapter does not say what it does on a resume, or when it is
//! not on the list this test drives.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use canvas_adapters::attack_shark_x6::AttackSharkX6;
use canvas_adapters::chione::ChioneDisplay;
use canvas_adapters::eightbitdo_ultimate2::EightBitDoUltimate2;
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_adapters::resume::{OnResume, ResumeBehavior};
use canvas_adapters::royuan::{direct_mode_report, Opener, RoyuanAdapter, Transport, PRODUCT_ID, VENDOR_ID};
use canvas_daemon::composite::CompositeAdapter;
use canvas_daemon::displays::NoDisplays;
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::{DeviceAdapter, DeviceId, Rgb8};

const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };

/// The adapters the daemon can build, by the name their `impl` block carries.
///
/// The scan below says that every adapter in the repo is on this list, so an adapter that is
/// added has to come here and be driven by this test.
const GUARDED: &[&str] = &[
    "OpenRgbAdapter",
    "RoyuanAdapter",
    "ChioneDisplay",
    "AttackSharkX6",
    "EightBitDoUltimate2",
    "CompositeAdapter",
    "NoDisplays",
];

/// The traits an adapter of the daemon can implement.
const ADAPTER_TRAITS: &[&str] = &["DeviceAdapter", "NumericDisplayAdapter", "PeripheralAdapter"];

/// A fake keyboard: it records the feature reports the adapter sends.
struct FakeKeyboard {
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Transport for FakeKeyboard {
    fn send_feature(&mut self, data: &[u8]) -> Result<(), String> {
        self.sent.lock().expect("the fake keyboard's lock").push(data.to_vec());
        Ok(())
    }
}

/// A fake opener: it hands out the fake keyboard.
struct FakeOpener {
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Opener for FakeOpener {
    fn present(&self) -> bool {
        true
    }

    fn open(&self) -> Result<Option<Box<dyn Transport>>, String> {
        Ok(Some(Box::new(FakeKeyboard { sent: self.sent.clone() })))
    }
}

/// The keyboard over the fake, and the reports it sent.
fn fake_keyboard() -> (RoyuanAdapter, Arc<Mutex<Vec<Vec<u8>>>>) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let adapter = RoyuanAdapter::with_opener(Box::new(FakeOpener { sent: sent.clone() }));
    (adapter, sent)
}

/// How many direct-mode reports the keyboard sent.
fn direct_reports(sent: &Arc<Mutex<Vec<Vec<u8>>>>) -> usize {
    let sent = sent.lock().expect("the fake keyboard's lock");
    sent.iter().filter(|report| report.as_slice() == direct_mode_report()).count()
}

/// A DRAM stick, as the fake OpenRGB server reports it.
fn stick() -> DeviceInfo {
    DeviceInfo {
        id: DeviceId("openrgb:dram".into()),
        name: "DDR5".into(),
        vendor: "fake".into(),
        kind: DeviceKind::Dram,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: "dram".into(),
        location: "smbus".into(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "RGB".into(),
            kind: ZoneKind::Linear,
            led_count: 8,
            min_leds: 8,
            max_leds: 8,
            resizable: false,
        }],
        connected: true,
    }
}

/// The keyboard's id, as the composite routes it.
fn keyboard_id() -> DeviceId {
    DeviceId(format!("royuan:{VENDOR_ID:04x}:{PRODUCT_ID:04x}"))
}

#[tokio::test]
async fn every_adapter_the_daemon_can_build_says_what_it_does_on_a_resume() {
    // The scan first: an adapter that is not under this guard fails here, with its file and
    // its line.
    let findings = scan_adapters();
    assert!(
        findings.is_empty(),
        "the resume guard found adapters that do not say what they do on a resume:\n{}",
        findings.join("\n")
    );

    // The device adapter the daemon drives: the OpenRGB controllers and the keyboard.
    let server = FakeOpenRgbServer::start(vec![stick()]).await.expect("the fake OpenRGB server");
    let (keyboard, sent) = fake_keyboard();
    let mut adapter = CompositeAdapter::with_keyboard(
        OpenRgbAdapter::new("127.0.0.1", server.port()),
        keyboard,
    );
    assert_eq!(adapter.on_resume(), OnResume::SetsModeAgain, "the composite holds two modes");

    let stick_id = DeviceId("openrgb:dram".into());
    let key_id = keyboard_id();
    adapter.connect().await.expect("connect");
    adapter.write_frame(&stick_id, &[RED; 8]).await.expect("frame");
    // The read-back is a request and a reply, so the frame before it has been handled.
    adapter.read_back(&stick_id).await.expect("readback orders the frame");
    adapter.write_frame(&key_id, &[RED]).await.expect("frame");
    assert_eq!(server.custom_mode_writes(0), 1, "the first frame sets the mode");
    assert_eq!(direct_reports(&sent), 1, "the connection sends the direct-mode report");

    // The resume, on the path the daemon uses when Windows says the machine woke.
    canvas_adapters::note_resume();

    adapter.write_frame(&stick_id, &[RED; 8]).await.expect("frame after the resume");
    adapter.read_back(&stick_id).await.expect("readback orders the frame");
    adapter.write_frame(&key_id, &[RED]).await.expect("frame after the resume");
    assert_eq!(
        server.custom_mode_writes(0),
        2,
        "the resume did not make the next frame set the mode again"
    );
    assert_eq!(
        direct_reports(&sent),
        2,
        "the resume did not make the next write send the direct-mode report again"
    );

    // The adapters with no mode state: they say so, and they do not grow a mode to restore.
    assert_eq!(ChioneDisplay::new().on_resume(), OnResume::NothingToRestore, "the CHIONE");
    assert_eq!(AttackSharkX6::new().on_resume(), OnResume::NothingToRestore, "the X6 mouse");
    assert_eq!(EightBitDoUltimate2::new().on_resume(), OnResume::NothingToRestore, "the 8BitDo pad");
    assert_eq!(NoDisplays.on_resume(), OnResume::NothingToRestore, "no display at all");
}

// ---- the scan over the repo's source

/// The root of the repository.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// A directory the scan does not read.
fn skipped_dir(name: &str) -> bool {
    matches!(name, "target" | "node_modules" | "examples" | "tests" | ".git" | "out" | ".next")
}

/// A file the scan does not read.
fn skipped_file(name: &str) -> bool {
    name == "tests.rs" || name.ends_with("_tests.rs")
}

/// Every Rust file of every crate, with the path it has in the repository.
fn source_files(root: &Path) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    let mut dirs = vec![root.join("crates")];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !skipped_dir(&name) {
                    dirs.push(path);
                }
            } else if !skipped_file(&name) && path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                files.push((rel, path));
            }
        }
    }
    files.sort();
    files
}

/// Which lines of a file are test code: the `#[cfg(test)]` modules.
///
/// The fakes in them are not shipped, so they are not under the guard.
fn test_lines(lines: &[&str]) -> Vec<bool> {
    let mut skip = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with("#[cfg(") && trimmed.contains("test") {
            skip[i] = true;
            let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or("");
            if next.starts_with("mod ") {
                if next.ends_with(';') {
                    // A module of its own: only the declaration is test code.
                    skip[i + 1] = true;
                    i += 2;
                    continue;
                }
                // A module with a body: skip to the line that closes it at the left margin.
                let mut j = i + 1;
                while j < lines.len() && lines[j] != "}" {
                    j += 1;
                }
                for k in i..=j.min(lines.len() - 1) {
                    skip[k] = true;
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    skip
}

/// The type of an `impl ... for ... {` line, when it implements an adapter trait.
fn adapter_impl(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("impl ")?;
    let (trait_path, type_path) = rest.split_once(" for ")?;
    let trait_name = trait_path.trim().rsplit("::").next()?;
    if !ADAPTER_TRAITS.contains(&trait_name) {
        return None;
    }
    Some(type_name(type_path)?)
}

/// The type of an `impl ... for ... {` line.
fn type_name(type_path: &str) -> Option<String> {
    let name = type_path.trim().trim_end_matches('{').trim();
    let name = name.split('<').next()?.trim();
    Some(name.rsplit("::").next()?.to_string())
}

/// Whether a line is the `impl ResumeBehavior for <type>` of the given type.
fn resume_impl(line: &str, wanted: &str) -> bool {
    let Some(rest) = line.trim().strip_prefix("impl ") else { return false };
    let Some((trait_path, type_path)) = rest.split_once(" for ") else { return false };
    if trait_path.trim().rsplit("::").next() != Some("ResumeBehavior") {
        return false;
    }
    type_name(type_path).is_some_and(|name| name == wanted)
}

/// What an adapter says it does on a resume, read from the source.
fn declaration(files: &[(String, PathBuf)], wanted: &str) -> Option<OnResume> {
    for (_, path) in files {
        let text = fs::read_to_string(path).expect("a source file reads");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !resume_impl(line, wanted) {
                continue;
            }
            // The body of the impl, up to the line that closes the method.
            let body: String = lines[i + 1..]
                .iter()
                .take_while(|line| line.trim() != "}")
                .copied()
                .collect::<Vec<&str>>()
                .join("\n");
            if body.contains("NothingToRestore") {
                return Some(OnResume::NothingToRestore);
            }
            if body.contains("SetsModeAgain") {
                return Some(OnResume::SetsModeAgain);
            }
            return None;
        }
    }
    None
}

/// Everything the scan found, as lines a human can act on.
fn scan_adapters() -> Vec<String> {
    let root = repo_root();
    let files = source_files(&root);
    let mut findings = Vec::new();

    // Every adapter in the source, with the file and the line it was found on.
    let mut found: Vec<(String, usize, String)> = Vec::new();
    for (rel, path) in &files {
        let text = fs::read_to_string(path).expect("a source file reads");
        let lines: Vec<&str> = text.lines().collect();
        let skip = test_lines(&lines);
        for (i, line) in lines.iter().enumerate() {
            if skip[i] {
                continue;
            }
            if let Some(type_name) = adapter_impl(line) {
                found.push((rel.clone(), i + 1, type_name));
            }
        }
    }
    assert!(!found.is_empty(), "the scan found no adapter at all: the roots moved?");

    for (rel, line, type_name) in &found {
        // 1. Every adapter says what it does on a resume.
        let Some(declared) = declaration(&files, type_name) else {
            findings.push(format!(
                "{rel}:{line}: {type_name} does not say what it does on a resume \
                 (implement `ResumeBehavior` for it)"
            ));
            continue;
        };
        // 2. Every adapter is on the list this test drives.
        if !GUARDED.contains(&type_name.as_str()) {
            findings.push(format!(
                "{rel}:{line}: {type_name} is not under the resume guard: add it to GUARDED \
                 and drive it in this test"
            ));
        }
        // 3. An adapter with nothing to restore does not take part in the resume.
        if declared == OnResume::NothingToRestore {
            let path = files.iter().find(|(r, _)| r == rel).map(|(_, p)| p);
            if let Some(path) = path {
                let text = fs::read_to_string(path).expect("a source file reads");
                if text.contains("note_resume") || text.contains("resume_epoch") {
                    findings.push(format!(
                        "{rel}:{line}: {type_name} says it has nothing to restore, but it takes \
                         part in the resume"
                    ));
                }
            }
        }
    }
    findings
}
