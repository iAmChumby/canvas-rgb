//! A black on pause, quit, lock or suspend paints every device and leaves the controller rings
//! alone (#376). The rings keep their colour in the pad's flash: a paint would write the pad.
//!
//! A real engine with two sinks, one on the ring device and one on a strip, behind the daemon's
//! own `DeviceSink` and identify gate. No device, no port.

use std::sync::Arc;

use canvas_daemon::identify::IdentifyGate;
use canvas_daemon::state::DeviceSink;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::traits::{Engine, EngineSettings};
use canvas_proto::Rgb8;
use tokio::sync::watch;

fn device(id: &str, leds: u32) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.into()),
        name: id.into(),
        vendor: "test".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "z0".into(),
            kind: ZoneKind::Linear,
            led_count: leds,
            min_leds: leds,
            max_leds: leds,
            resizable: false,
        }],
        connected: true,
    }
}

struct Rig {
    engine: Arc<dyn Engine>,
    gate: Arc<IdentifyGate>,
    ring: watch::Receiver<Vec<Rgb8>>,
    strip: watch::Receiver<Vec<Rgb8>>,
}

/// A paused engine (so no scene frame competes) that knows both devices' LED counts.
fn rig() -> Rig {
    let engine: Arc<dyn Engine> = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Paused,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    });
    let gate = Arc::new(IdentifyGate::default());
    let mut receivers = Vec::new();
    for id in ["ring:8bitdo-ultimate-2", "openrgb:strip"] {
        let (tx, rx) = watch::channel::<Vec<Rgb8>>(Vec::new());
        engine.add_sink(DeviceId(id.into()), Arc::new(DeviceSink::with_gate(DeviceId(id.into()), tx, gate.clone())));
        receivers.push(rx);
    }
    engine.set_scene(Vec::new(), vec![device("ring:8bitdo-ultimate-2", 24), device("openrgb:strip", 30)], Vec::new(), false);
    let strip = receivers.pop().unwrap();
    let ring = receivers.pop().unwrap();
    Rig { engine, gate, ring, strip }
}

#[test]
fn a_black_paint_reaches_the_strip_and_not_the_rings() {
    let mut r = rig();
    r.gate.paint_solid(r.engine.as_ref(), Rgb8::BLACK);
    assert_eq!(r.strip.borrow_and_update().len(), 30, "the strip took its black frame");
    assert!(!r.ring.has_changed().unwrap(), "the rings were sent nothing");
    assert!(!r.gate.is_painting_solid(), "the paint is over");
}

/// The control: the engine alone does hand a solid frame to the ring's sink, so the exclusion
/// above is the gate's doing and not a sink the engine never had.
#[test]
fn without_the_gate_the_engine_would_paint_the_rings() {
    let mut r = rig();
    r.engine.write_solid(Rgb8::BLACK);
    assert_eq!(r.ring.borrow_and_update().len(), 24, "the engine paints every sink it has");
}

#[test]
fn the_rings_take_frames_again_once_the_paint_is_over() {
    let mut r = rig();
    r.gate.paint_solid(r.engine.as_ref(), Rgb8::BLACK);
    r.engine.write_solid(Rgb8::new(1, 2, 3));
    assert_eq!(r.ring.borrow_and_update().clone(), vec![Rgb8::new(1, 2, 3); 24]);
}
