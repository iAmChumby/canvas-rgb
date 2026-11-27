//! `GET /api/v1/about` and `GET /api/v1/about/report` (#574).
//!
//! The identity is static while the daemon runs. The report is an allow-list: model-level facts
//! about the machine, never a serial, a device id, a raw location or a private path. The one
//! shared scrubber runs last on every string (see [`crate::scrub`]).

use axum::extract::State;
use axum::Json;
use canvas_proto::api::{About, AboutReport, Conflict, LastExit};
use canvas_proto::device::{DeviceInfo, DeviceKind};
use canvas_proto::peripheral::{PeripheralInfo, PeripheralKind};
use std::sync::{Arc, OnceLock};

use crate::state::App;

/// The report's length cap; a longer one ends with "(truncated)" (#575's prefill uses this too).
const MAX_REPORT_CHARS: usize = 6000;
const TRUNCATED: &str = "(truncated)";

/// `GET /api/v1/about`.
pub async fn about(State(app): State<Arc<App>>) -> Json<About> {
    Json(build_about(&app))
}

/// `GET /api/v1/about/report`: the finished markdown and the bug template's fields.
pub async fn report(State(app): State<Arc<App>>) -> Json<AboutReport> {
    Json(build_report(&app).await)
}

/// The facts Help & About shows at once. `commit` and `build_date` come from `build.rs`; neither
/// is a reason to fail a build when git is not there (#574).
pub fn build_about(app: &App) -> About {
    About {
        version: app.version.clone(),
        commit: build_commit(),
        build_date: build_date(),
        os: windows_version(),
        service_version: app.service_version(),
        last_exit: app.last_exit(), // #577: the service's verdict from the first Hello.
    }
}

fn build_commit() -> Option<String> {
    option_env!("CANVAS_BUILD_COMMIT").filter(|s| !s.is_empty()).map(str::to_string)
}

fn build_date() -> Option<String> {
    option_env!("CANVAS_BUILD_DATE").filter(|s| !s.is_empty()).map(str::to_string)
}

/// Windows edition and build, e.g. `Windows 11 Home 10.0.26300`. The registry read happens
/// once, on the first request, and is cached: an async handler must not block on the registry
/// every time someone opens Help (#574 R7).
fn windows_version() -> Option<String> {
    static CACHED: OnceLock<Option<String>> = OnceLock::new();
    CACHED.get_or_init(read_windows_version).clone()
}

/// The label people know, from the build number when ProductName is absent or stale. Build
/// 22000 is the first Windows 11 build; earlier builds are Windows 10.
fn os_label(product: Option<String>, build_number: u32) -> String {
    let mut label = match product {
        Some(name) if !name.trim().is_empty() => name,
        _ => if build_number >= 22000 { "Windows 11".to_string() } else { "Windows 10".to_string() },
    };
    if build_number >= 22000 && label.contains("Windows 10") {
        label = label.replacen("Windows 10", "Windows 11", 1);
    }
    label
}

/// Plain words for `last_exit`: the enum name is not a sentence (#574).
fn last_exit_words(exit: LastExit) -> &'static str {
    match exit {
        LastExit::Clean => "ended normally",
        LastExit::Planned => "stopped by Windows or an update",
        LastExit::Unclean => "crashed",
        LastExit::StartupFailure => "failed to start",
        LastExit::Unknown => "unknown",
    }
}

#[cfg(windows)]
fn read_windows_version() -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe fn read(key: HKEY, name: &str) -> Option<String> {
        let value = wide(name);
        let mut size: u32 = 0;
        if RegQueryValueExW(key, PCWSTR(value.as_ptr()), None, None, None, Some(&mut size)).0 != 0
            || size == 0
        {
            return None;
        }
        let mut buffer = vec![0u16; (size as usize / 2) + 1];
        let mut written = size;
        if RegQueryValueExW(
            key,
            PCWSTR(value.as_ptr()),
            None,
            None,
            Some(buffer.as_mut_ptr().cast::<u8>()),
            Some(&mut written),
        )
        .0
        != 0
        {
            return None;
        }
        let text = String::from_utf16_lossy(&buffer).trim_matches('\0').trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    unsafe {
        let mut key: HKEY = HKEY::default();
        let path = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(path.as_ptr()), None, KEY_READ, &mut key).0 != 0 {
            return None;
        }
        let product = read(key, "ProductName");
        let version = read(key, "CurrentVersion");
        let build = read(key, "CurrentBuild");
        let _ = RegCloseKey(key);

        let build = build?;
        let build_number = build.parse::<u32>().unwrap_or(0);
        let label = os_label(product, build_number);
        // CurrentVersion is 6.3 on some Windows 10/11 installs, but the version people know and
        // the one the consensus prints is 10.0: `Windows 11 Home 10.0.26300` (#574 R7).
        let version = version.filter(|v| v.starts_with("10.")).unwrap_or_else(|| "10.0".to_string());
        Some(format!("{label} {version}.{build}"))
    }
}

#[cfg(not(windows))]
fn read_windows_version() -> Option<String> {
    None
}

async fn build_report(app: &App) -> AboutReport {
    // Clone out of the lock: the scrubber's patterns are small, and a std mutex must not be held
    // across the awaits that follow.
    let scrubber = app.scrubber.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let about = build_about(app);

    let devices = app.devices.read().await.clone();
    let peripherals = app.peripherals.discover().await;
    let hardware = hardware_lines(&devices, &peripherals);

    let conflicts = app.conflicts.read().await.clone();
    let signalrgb = conflicts.iter().any(|c| matches!(c, Conflict::SignalRgb { .. }));
    let other = if signalrgb {
        "SignalRGB is running; other RGB apps: not checked"
    } else {
        "SignalRGB not seen; other RGB apps: not checked"
    };

    let version = scrubber.scrub(&about.version);
    let windows = scrubber.scrub(about.os.as_deref().unwrap_or("unknown"));
    let hardware = scrubber.scrub(&hardware);
    let other = scrubber.scrub(other);
    let commit = scrubber.scrub(about.commit.as_deref().unwrap_or("unknown"));
    let date = scrubber.scrub(about.build_date.as_deref().unwrap_or("unknown"));
    let service = scrubber.scrub(about.service_version.as_deref().unwrap_or("unknown"));
    let last = scrubber.scrub(last_exit_words(about.last_exit));

    let facts = [
        ("canvas-rgb", version.as_str()),
        ("Commit", commit.as_str()),
        ("Built", date.as_str()),
        ("Windows", windows.as_str()),
        ("Service", service.as_str()),
        ("Last run", last.as_str()),
    ];
    let markdown = cap(scrubber.scrub(&assemble(&facts, &hardware, &other)));

    AboutReport { markdown, version, windows, hardware, other_rgb_apps: other }
}

/// The whole report, in plain GitHub Markdown. `status` is left for the person to fill from
/// their own `canvas-rgb status`: the daemon never copies a command's output into the report.
fn assemble(facts: &[(&str, &str)], hardware: &str, other: &str) -> String {
    let mut out = String::from("# canvas-rgb support report\n\n## Version\n");
    for (label, value) in facts {
        out.push_str(&format!("- {label}: {value}\n"));
    }
    out.push_str(&format!(
        "\n## Hardware\n{hardware}\n\n## Other RGB apps\n{other}\n\n## Status\n\
         Paste the output of your own `canvas-rgb status` here; it is not collected automatically.\n"
    ));
    out
}

fn hardware_lines(devices: &[DeviceInfo], peripherals: &[PeripheralInfo]) -> String {
    let mut lines: Vec<String> = devices
        .iter()
        .map(|d| line_for(&d.name, &d.vendor, device_kind(d.kind), &d.location))
        .collect();
    lines.extend(
        peripherals
            .iter()
            .map(|p| line_for(&p.name, &p.vendor, peripheral_kind(p.kind), &p.location)),
    );
    if lines.is_empty() {
        "- No devices were found.".to_string()
    } else {
        lines.join("\n")
    }
}

/// Device name and vendor are free text from OpenRGB. A newline would add a row to the report's
/// table and a pipe would add a cell, so control characters become spaces and pipes are escaped
/// before the scrub runs. Consecutive spaces collapse so the line stays compact.
fn clean_free_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for ch in text.chars() {
        if ch.is_control() {
            space = true;
        } else if ch == '|' {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push_str("\\|");
        } else if ch.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(ch);
        }
    }
    out
}

/// One model-level line. No serial, no `DeviceId`, no raw location: only the two parsed pieces.
/// A location that is not `hid:` or `ble:` gets no connection or VID:PID cells at all.
fn line_for(name: &str, vendor: &str, kind: &str, location: &str) -> String {
    let clean_name = clean_free_text(name);
    let clean_vendor = clean_free_text(vendor);
    match parse_location(location) {
        Some((connection, ids)) => {
            format!("- {clean_name} | {clean_vendor} | {kind} | {connection} | {ids}")
        }
        None => format!("- {clean_name} | {clean_vendor} | {kind}"),
    }
}

/// The strict shape `^(hid|ble):([0-9a-f]{4}):([0-9a-f]{4})`, as a prefix (a location may carry
/// more fields after the ids). Everything else is "not parsed", never a guess (#574).
fn parse_location(location: &str) -> Option<(&'static str, String)> {
    let (connection, rest) = if let Some(rest) = location.strip_prefix("hid:") {
        ("hid", rest)
    } else {
        ("ble", location.strip_prefix("ble:")?)
    };
    let mut parts = rest.split(':');
    let vid = parts.next()?;
    let pid = parts.next()?;
    let four_hex = |s: &str| {
        s.len() == 4
            && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    if four_hex(vid) && four_hex(pid) {
        Some((connection, format!("{vid}:{pid}")))
    } else {
        None
    }
}

fn device_kind(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Motherboard => "motherboard",
        DeviceKind::Dram => "DRAM",
        DeviceKind::Keyboard => "keyboard",
        DeviceKind::Mouse => "mouse",
        DeviceKind::Microphone => "microphone",
        DeviceKind::Headset => "headset",
        DeviceKind::LedStrip => "LED strip",
        DeviceKind::Gpu => "GPU",
        DeviceKind::Other => "other",
    }
}

fn peripheral_kind(kind: PeripheralKind) -> &'static str {
    match kind {
        PeripheralKind::Mouse => "mouse",
        PeripheralKind::GameController => "game controller",
        PeripheralKind::Keyboard => "keyboard",
    }
}

fn cap(text: String) -> String {
    if text.chars().count() <= MAX_REPORT_CHARS {
        return text;
    }
    let mut out: String = text.chars().take(MAX_REPORT_CHARS - TRUNCATED.len()).collect();
    out.push_str(TRUNCATED);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrub::Scrubber;

    #[test]
    fn only_the_strict_location_shape_gives_connection_and_ids() {
        assert_eq!(
            parse_location("hid:1d57:fa60:usage0b"),
            Some(("hid", "1d57:fa60".to_string()))
        );
        assert_eq!(
            parse_location("ble:2a3b:4c5d"),
            Some(("ble", "2a3b:4c5d".to_string()))
        );
        for other in [
            "HID:1d57:fa60",
            "smbus:1d57:fa60",
            "2.4G receiver",
            "hid:1d57:FA60",
            "hid:1d5:fa60",
            "hid:zzzz:fa60",
            "hid:1d57",
        ] {
            assert_eq!(parse_location(other), None, "{other} was parsed");
        }
    }

    #[test]
    fn a_device_line_never_carries_a_serial_id_or_raw_location() {
        let line = line_for("Desk pad", "Maker", "LED strip", "hid:1d57:fa60:unit-12345");
        assert_eq!(line, "- Desk pad | Maker | LED strip | hid | 1d57:fa60");
        assert!(!line.contains("unit-12345"));
        let odd = line_for("Desk pad", "Maker", "LED strip", "smbus:unit-12345");
        assert_eq!(odd, "- Desk pad | Maker | LED strip");
        assert!(!odd.contains("unit-12345"));
    }

    #[test]
    fn the_report_scrubs_free_text_and_the_assembled_markdown() {
        let scrubber = Scrubber::new(
            Some(concat!("C:\\", "Use", "rs\\Alice").into()),
            Some("Alice".into()),
            Some("ALICE-PC".into()),
        );
        let hardware = scrubber.scrub(&line_for(
            concat!("ALICE-PC pad (C:/", "Use", "rs/alice/device)"),
            "Maker",
            "mouse",
            "ble:2a3b:4c5d",
        ));
        let facts = [
            ("canvas-rgb", "0.20.0"),
            ("Commit", "abc1234"),
            ("Built", "2026-11-26"),
            ("Windows", "Windows 11 Home 10.0.26300"),
            ("Service", "0.9.1"),
            ("Last run", "unknown"),
        ];
        let markdown = scrubber.scrub(&assemble(&facts, &hardware, "not checked"));
        assert!(!markdown.to_ascii_lowercase().contains("alice"), "{markdown}");
        assert!(markdown.contains("<profile>"), "{markdown}");
        assert!(markdown.contains("2a3b:4c5d"), "{markdown}");
    }

    #[test]
    fn free_text_cannot_break_the_report_table() {
        // A device's name and vendor are free text: a newline would add a row to the table and a
        // pipe would add a cell, so both are neutralised before the line is assembled.
        let line = line_for("Bad\nName", "Acme|Corp", "mouse", "hid:1d57:fa60");
        assert_eq!(line, "- Bad Name | Acme\\|Corp | mouse | hid | 1d57:fa60");
        assert!(!line.contains('\n'));
        assert!(!line.contains('\r'));
    }

    #[test]
    fn last_exit_is_plain_words() {
        assert_eq!(last_exit_words(LastExit::Clean), "ended normally");
        assert_eq!(last_exit_words(LastExit::Unclean), "crashed");
        assert_eq!(last_exit_words(LastExit::Unknown), "unknown");
    }

    #[test]
    fn the_windows_label_comes_from_the_build_when_the_name_is_missing() {
        assert_eq!(os_label(None, 26100), "Windows 11");
        assert_eq!(os_label(None, 19045), "Windows 10");
        assert_eq!(os_label(Some("Windows 10 Home".into()), 26100), "Windows 11 Home");
        assert_eq!(os_label(Some("Windows 10 Pro".into()), 19045), "Windows 10 Pro");
    }

    #[test]
    fn overlong_reports_say_they_were_truncated() {
        let long = "x".repeat(MAX_REPORT_CHARS + 10);
        let out = cap(long);
        assert!(out.ends_with(TRUNCATED));
        assert_eq!(out.chars().count(), MAX_REPORT_CHARS);
        assert_eq!(cap("short".to_string()), "short");
    }
}
