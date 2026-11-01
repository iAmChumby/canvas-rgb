//! The `cause:sleep-state` class guard (#5, #29).
//!
//! Every adapter the daemon can build says what it does when the machine resumes from sleep.
//! Three things hold that together:
//!
//! - The compiler. The daemon takes its adapters through bounds that include `ResumeBehavior`
//!   (`App::new`, `App::with_displays`, `DisplayHub::new` and the `Peripheral` trait), so an
//!   adapter without it cannot be handed to the daemon.
//! - This test, which drives a simulated resume: an adapter that holds a mode sets it again
//!   (the OpenRGB controllers and the ROYUAN keyboard), and the others declare
//!   `NothingToRestore`. What the `NothingToRestore` adapters send after a resume is checked on
//!   their fake transports, in the adapters' own tests (`a_resume_makes_the_..._send_nothing_extra`).
//! - A scan over the repo's production source, which fails with `file:line` when an adapter
//!   exists that this test does not drive. It reads real `#[cfg(test)]` items and `tests/`
//!   directories as test code and everything else as production code.

use std::collections::BTreeSet;
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
/// The scan below says that every adapter in the production source is on this list, so an
/// adapter that is added has to come here and be driven by this test.
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
    // its line. (An adapter without `ResumeBehavior` never gets this far: it does not compile.)
    let findings = scan_adapters();
    assert!(
        findings.is_empty(),
        "the resume guard found adapters it does not drive:\n{}",
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

/// A directory the scan does not read: build output, and the `tests/` directories (test code).
fn skipped_dir(name: &str) -> bool {
    matches!(name, "target" | "node_modules" | "tests" | ".git" | "out" | ".next")
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
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
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

/// Whether a line is an attribute that makes the next item test-only: `#[cfg(test)]` or
/// `#[cfg(all(test, ...))]`. `#[cfg(not(test))]` and `#[cfg(any(test, ...))]` keep the item in a
/// release build, so they are production code.
fn is_test_cfg(line: &str) -> bool {
    let line = line.trim();
    line == "#[cfg(test)]" || (line.starts_with("#[cfg(all(test") && line.ends_with(")]"))
}

/// The leading whitespace of a line.
fn indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// The first line of the item an attribute at line `at` applies to (after further attributes).
fn item_after(lines: &[&str], at: usize) -> usize {
    let mut item = at + 1;
    while item < lines.len() && lines[item].trim().starts_with("#[") {
        item += 1;
    }
    item
}

/// Which lines of a file are test code: the items under a test-only `#[cfg]`.
///
/// The fakes in them are not shipped, so they are not under the guard.
fn test_lines(lines: &[&str]) -> Vec<bool> {
    let mut skip = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if !is_test_cfg(lines[i]) {
            i += 1;
            continue;
        }
        let item = item_after(lines, i);
        let Some(first) = lines.get(item) else {
            skip[i] = true;
            break;
        };
        let end = if first.trim_end().ends_with(';') || first.trim_end().ends_with('}') {
            // A declaration, or an item that closes on its own line.
            item
        } else {
            // An item with a body: it ends at the line that closes it at the item's indent.
            let margin = indent(first);
            (item + 1..lines.len())
                .find(|&k| lines[k].trim() == "}" && indent(lines[k]) == margin)
                .unwrap_or(lines.len() - 1)
        };
        for line in skip.iter_mut().take(end + 1).skip(i) {
            *line = true;
        }
        i = end + 1;
    }
    skip
}

/// The files that a test-only `mod name;` of this file pulls in: `name.rs` and the `name/`
/// directory, next to a `lib.rs`, `main.rs` or `mod.rs`, or under the file's own name otherwise.
fn test_module_paths(file: &Path, lines: &[&str]) -> Vec<PathBuf> {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = file.parent().unwrap_or(Path::new(""));
    let dir = if matches!(stem, "lib" | "main" | "mod") { parent.to_path_buf() } else { parent.join(stem) };
    let mut paths = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !is_test_cfg(line) {
            continue;
        }
        let Some(decl) = lines.get(item_after(lines, i)).map(|l| l.trim()) else { continue };
        let decl = decl.strip_prefix("pub(crate) ").or_else(|| decl.strip_prefix("pub ")).unwrap_or(decl);
        if let Some(name) = decl.strip_prefix("mod ").and_then(|rest| rest.strip_suffix(';')) {
            paths.push(dir.join(format!("{}.rs", name.trim())));
            paths.push(dir.join(name.trim()));
        }
    }
    paths
}

/// The type of an `impl ... for ... {` line, when it implements an adapter trait.
fn adapter_impl(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("impl ")?;
    let (trait_path, type_path) = rest.split_once(" for ")?;
    let trait_name = trait_path.trim().rsplit("::").next()?;
    if !ADAPTER_TRAITS.contains(&trait_name) {
        return None;
    }
    type_name(type_path)
}

/// The type of an `impl ... for ... {` line.
fn type_name(type_path: &str) -> Option<String> {
    let name = type_path.trim().trim_end_matches('{').trim();
    let name = name.split('<').next()?.trim();
    Some(name.rsplit("::").next()?.to_string())
}

/// Everything the scan found, as lines a human can act on: the adapters in production code
/// that this test does not drive.
fn scan_adapters() -> Vec<String> {
    let root = repo_root();
    let files = source_files(&root);
    let texts: Vec<String> =
        files.iter().map(|(_, path)| fs::read_to_string(path).expect("a source file reads")).collect();

    // The files that are test code as a whole: the targets of a test-only `mod name;`.
    let mut test_modules: BTreeSet<PathBuf> = BTreeSet::new();
    for ((_, path), text) in files.iter().zip(&texts) {
        let lines: Vec<&str> = text.lines().collect();
        test_modules.extend(test_module_paths(path, &lines));
    }

    let mut found: Vec<(String, usize, String)> = Vec::new();
    for ((rel, path), text) in files.iter().zip(&texts) {
        if test_modules.iter().any(|module| path.starts_with(module)) {
            continue;
        }
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

    found
        .iter()
        .filter(|(_, _, type_name)| !GUARDED.contains(&type_name.as_str()))
        .map(|(rel, line, type_name)| {
            format!(
                "{rel}:{line}: {type_name} is not under the resume guard: add it to GUARDED and \
                 drive it in this test"
            )
        })
        .collect()
}

#[test]
fn the_scan_reads_production_code_wherever_it_sits() {
    // Real test items are skipped, in every spelling that is test-only.
    for attr in ["#[cfg(test)]", "#[cfg(all(test, windows))]"] {
        let text = format!(
            "{attr}\nmod fakes {{\n    impl DeviceAdapter for Fake {{\n    }}\n}}\nimpl DeviceAdapter for Real {{"
        );
        let lines: Vec<&str> = text.lines().collect();
        let skip = test_lines(&lines);
        assert!(skip[2], "{attr}: the fake inside a test module is test code");
        assert!(!skip[lines.len() - 1], "{attr}: the impl after the module is production code");
    }
    // `cfg(not(test))` and `cfg(any(test, ..))` code ships, so it is read.
    for attr in ["#[cfg(not(test))]", "#[cfg(any(test, windows))]"] {
        let text = format!("{attr}\nimpl DeviceAdapter for Late {{\n}}");
        let lines: Vec<&str> = text.lines().collect();
        assert!(test_lines(&lines).iter().all(|skipped| !skipped), "{attr}: this is production code");
    }
    // A test-only `mod x;` makes `x.rs` test code. A file that is merely named `*_tests.rs` is
    // not test code unless a test-only `mod` pulls it in.
    let lib = Path::new("crates/c/src/lib.rs");
    let text = "#[cfg(test)]\nmod app_tests;\nmod shipped_tests;";
    let lines: Vec<&str> = text.lines().collect();
    let modules = test_module_paths(lib, &lines);
    assert!(modules.contains(&Path::new("crates/c/src").join("app_tests.rs")));
    assert!(!modules.contains(&Path::new("crates/c/src").join("shipped_tests.rs")));
    // An adapter impl is found by its trait's last path segment.
    assert_eq!(adapter_impl("impl canvas_proto::traits::DeviceAdapter for Late {").as_deref(), Some("Late"));
    assert_eq!(adapter_impl("impl Peripheral for Late {"), None);
}
