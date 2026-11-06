//! A standalone fake OpenRGB server for a fakes-mode daemon (#199).
//!
//! Run it as:
//! `cargo run -p canvas-adapters --example fake_server`
//!
//! It speaks the OpenRGB SDK protocol on 127.0.0.1 on the fake port
//! (`CANVAS_RGB_FAKE_OPENRGB_PORT`, default 16742 — never the real 6742) with
//! five fake devices, and touches no hardware. It binds 127.0.0.1 only.
//! Stop it with Ctrl+C.

use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_proto::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};

fn device(name: &str, serial: &str, kind: DeviceKind, zones: Vec<ZoneInfo>) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(format!("openrgb:{serial}")),
        name: name.into(),
        vendor: "Fake".into(),
        kind,
        adapter: "openrgb".into(),
        adapter_index: 0,
        serial: serial.into(),
        location: "usb".into(),
        zones,
        connected: true,
    }
}

fn fixed(name: &str, count: u32, kind: ZoneKind) -> ZoneInfo {
    ZoneInfo { index: 0, name: name.into(), kind, led_count: count, min_leds: count, max_leds: count, resizable: false }
}

fn linear(name: &str, count: u32) -> ZoneInfo {
    fixed(name, count, ZoneKind::Linear)
}

/// A JARGB header: starts at 0 LEDs and accepts any size up to 300.
fn jargb(name: &str) -> ZoneInfo {
    ZoneInfo { index: 0, name: name.into(), kind: ZoneKind::Linear, led_count: 0, min_leds: 0, max_leds: 300, resizable: true }
}

/// The five fake devices a typical test PC reports through OpenRGB.
fn fake_devices() -> Vec<DeviceInfo> {
    vec![
        device("MSI MPG B850 Mystic Light (fake)", "mb", DeviceKind::Motherboard, vec![
            jargb("JARGB1"), jargb("JARGB2"), jargb("JARGB3"), linear("JRGB1", 3)]),
        device("Fake Keyboard", "kb", DeviceKind::Keyboard, vec![ZoneInfo {
            index: 0, name: "Keys".into(),
            kind: ZoneKind::Matrix { width: 2, height: 2, map: vec![Some(0), Some(1), None, Some(2)] },
            led_count: 3, min_leds: 3, max_leds: 3, resizable: false }]),
        device("Fake Microphone", "mic", DeviceKind::Microphone, vec![fixed("Logo", 1, ZoneKind::Single)]),
        device("Fake DDR5 A", "dram-a", DeviceKind::Dram, vec![linear("RGB", 8)]),
        device("Fake DDR5 B", "dram-b", DeviceKind::Dram, vec![linear("RGB", 8)]),
    ]
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> std::process::ExitCode {
    let port: u16 = std::env::var("CANVAS_RGB_FAKE_OPENRGB_PORT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|p| *p != 6742 && *p != 9430 && *p != 19430 && *p != 0)
        .unwrap_or(16742);
    let _server = match FakeOpenRgbServer::start_on_port(fake_devices(), port).await {
        Ok(server) => server,
        Err(e) => {
            eprintln!("error: could not start the fake OpenRGB server on 127.0.0.1:{port}: {e}");
            return std::process::ExitCode::from(1);
        }
    };
    println!("fake-openrgb listening on 127.0.0.1:{port}");
    let _ = tokio::signal::ctrl_c().await;
    println!("fake-openrgb stopped");
    std::process::ExitCode::from(0)
}
