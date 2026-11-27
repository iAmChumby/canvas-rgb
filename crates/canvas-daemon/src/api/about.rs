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
use std::sync::Arc;

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
        last_exit: LastExit::Unknown, // #577 writes the marker; until then it is honestly unknown.
    }
}

fn build_commit() -> Option<String> {
    option_env!("CANVAS_BUILD_COMMIT").filter(|s| !s.is_empty()).map(str::to_string)
}

fn build_date() -> Option<String> {
    option_env!("CANVAS_BUILD_DATE").filter(|s| !s.is_empty()).map(str::to_string)
}

/// Windows edition and build, e.g. `Windows 11 Home 10.0.26300`. The registry's ProductName
/// still says "Windows 10" on Windows 11, so the build number decides the label (#574 R7).
#[cfg(windows)]
fn windows_version() -> Option<String> {
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
        let mut label = product.unwrap_or_else(|| "Windows".to_string());
        if build.parse::<u32>().unwrap_or(0) >= 22000 {
            label = label.replacen("Windows 10", "Windows 11", 1);
        }
        let version = version.unwrap_or_else(|| "10.0".to_string());
        Some(format!("{label} {version}.{build}"))
    }
}

#[cfg(not(windows))]
fn windows_version() -> Option<String> {
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
    let last = scrubber.scrub("unknown");

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

/// One model-level line. No serial, no `DeviceId`, no raw location: only the two parsed pieces.
fn line_for(name: &str, vendor: &str, kind: &str, location: &str) -> String {
    let (connection, ids) =
        parse_location(location).unwrap_or(("not parsed", "not parsed".to_string()));
    format!("- {name} | {vendor} | {kind} | {connection} | {ids}")
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
        assert!(odd.contains("not parsed"));
        assert!(!odd.contains("unit-12345"));
    }

    #[test]
    fn the_report_scrubs_free_text_and_the_assembled_markdown() {
        let scrubber = Scrubber::new(
            Some(r"C:\Users\Alice".into()),
            Some("Alice".into()),
            Some("ALICE-PC".into()),
        );
        let hardware = scrubber.scrub(&line_for(
            "ALICE-PC pad (C:/Users/alice/device)",
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
    fn overlong_reports_say_they_were_truncated() {
        let long = "x".repeat(MAX_REPORT_CHARS + 10);
        let out = cap(long);
        assert!(out.ends_with(TRUNCATED));
        assert_eq!(out.chars().count(), MAX_REPORT_CHARS);
        assert_eq!(cap("short".to_string()), "short");
    }
}
