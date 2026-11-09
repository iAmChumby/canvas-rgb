//! The White effect, the warmth stage and the white balance reach every device (#50).
//!
//! A real engine with two fake devices: the White effect from the registry must write
//! the white of the chosen temperature to every LED of every sink, the warmth stage must
//! scale every effect's output on every sink, and one device's white balance must scale
//! that device alone. No device, no port.

use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::{Layout, PlacedZone, RenderScale, SamplingMode, ZoneShape};
use canvas_proto::props::{PropValue, Props};
use canvas_proto::traits::{Engine, EngineSettings, FrameSink};
use canvas_proto::Rgb8;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A sink that keeps the last frame it is given.
#[derive(Default)]
struct RecordingSink {
    last: Mutex<Option<Vec<Rgb8>>>,
}

impl RecordingSink {
    fn last(&self) -> Option<Vec<Rgb8>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl FrameSink for RecordingSink {
    fn submit(&self, colors: Vec<Rgb8>) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(colors);
    }
}

fn wait_until(timeout: Duration, check: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    check()
}

fn device(id: &str, leds: u32) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.to_string()),
        name: id.to_string(),
        vendor: "test".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "zone0".into(),
            kind: ZoneKind::Linear,
            led_count: leds,
            min_leds: leds,
            max_leds: leds,
            resizable: false,
        }],
        connected: true,
    }
}

fn placed(id: &str, x: f32) -> PlacedZone {
    PlacedZone {
        device_id: DeviceId(id.to_string()),
        zone_index: 0,
        x,
        y: 0.0,
        width: 160.0,
        height: 200.0,
        rotation_deg: 0.0,
        reversed: false,
        visible: true,
        shape: ZoneShape::Auto,
        label: None,
        leds: None,
        part: None,
        locked: false,
    }
}

fn layout() -> Layout {
    Layout {
        id: "test".into(),
        name: "test".into(),
        render_scale: RenderScale::P100,
        sampling: SamplingMode::AreaAverage,
        zones: vec![placed("fans", 0.0), placed("strip", 160.0)],
        effect: None,
        case_form: canvas_proto::layout::CaseForm::None,
        landmarks: Vec::new(),
    }
}

fn at(kelvin: f64) -> Props {
    let mut props = Props::new();
    props.insert("temperature".into(), PropValue::Number(kelvin));
    props
}

#[test]
fn white_writes_the_chosen_white_to_every_led_of_every_sink() {
    let engine: Arc<dyn Engine> = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    });
    engine.set_layout(layout(), vec![device("fans", 24), device("strip", 60)]);
    let sinks = [("fans", 24usize), ("strip", 60usize)].map(|(id, leds)| {
        let sink = Arc::new(RecordingSink::default());
        engine.add_sink(DeviceId(id.into()), sink.clone());
        (id, leds, sink)
    });

    for kelvin in [2700.0, 6500.0] {
        let want = Rgb8::from_f32(canvas_effects::color::kelvin_to_led(kelvin as f32));
        let effect = canvas_effects::create("white").expect("White is a built-in effect");
        engine.set_effect(effect, &at(kelvin));
        for (id, leds, sink) in &sinks {
            let reached = wait_until(Duration::from_secs(20), || {
                sink.last().is_some_and(|frame| frame.len() == *leds && frame.iter().all(|c| *c == want))
            });
            assert!(reached, "{id} never showed {want:?} for {kelvin} K; last frame starts {:?}", sink.last().map(|f| f[..3.min(f.len())].to_vec()));
        }
    }
    engine.shutdown();
}

fn white_canvas() -> (Box<dyn canvas_proto::Effect>, Props) {
    let mut props = Props::new();
    props.insert("color".into(), PropValue::Color(Rgb8::WHITE));
    (canvas_effects::create("solid").expect("Solid is a built-in effect"), props)
}

fn scaled(gains: [f32; 3]) -> Rgb8 {
    Rgb8::from_f32(gains)
}

#[test]
fn the_warmth_stage_and_white_balance_scale_what_every_sink_gets() {
    let engine: Arc<dyn Engine> = canvas_core::spawn(EngineSettings {
        fps: 30,
        power: canvas_proto::api::Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    });
    engine.set_layout(layout(), vec![device("fans", 24), device("strip", 60)]);
    let sinks = [("fans", 24usize), ("strip", 60usize)].map(|(id, leds)| {
        let sink = Arc::new(RecordingSink::default());
        engine.add_sink(DeviceId(id.into()), sink.clone());
        (id, leds, sink)
    });
    let shows = |want: [Rgb8; 2], what: &str| {
        for ((id, leds, sink), want) in sinks.iter().zip(want) {
            let reached = wait_until(Duration::from_secs(20), || {
                sink.last().is_some_and(|frame| frame.len() == *leds && frame.iter().all(|c| *c == want))
            });
            assert!(reached, "{what}: {id} never showed {want:?}; last frame starts {:?}", sink.last().map(|f| f[..3.min(f.len())].to_vec()));
        }
    };
    let (effect, props) = white_canvas();
    engine.set_effect(effect, &props);
    shows([Rgb8::WHITE; 2], "warmth off");

    // The warmth stage turns a white canvas into the LED white of that temperature, on
    // every device (Solid, not White: the stage applies to every effect).
    engine.set_warmth(1800);
    let warm = scaled(canvas_proto::color::warmth_gains(1800.0));
    assert!(warm.b < 10 && warm.r == 255, "1800 K warmth is not amber: {warm:?}");
    shows([warm; 2], "warmth 1800 K");
    engine.set_warmth(canvas_proto::consts::WARMTH_OFF);
    shows([Rgb8::WHITE; 2], "warmth back off");

    // One device's white balance scales that device and leaves the other alone.
    let balance = canvas_proto::color::WhiteBalance { red: 0.5, green: 1.0, blue: 0.25 };
    engine.set_white_balance([(DeviceId("fans".into()), balance)].into_iter().collect());
    shows([scaled(balance.gains()), Rgb8::WHITE], "fans balanced");

    // Brightness, warmth and balance multiply.
    engine.set_brightness(50);
    engine.set_warmth(4000);
    let w = canvas_proto::color::warmth_gains(4000.0);
    let fans = scaled([0, 1, 2].map(|i| 0.5 * w[i] * balance.gains()[i]));
    let strip = scaled([0, 1, 2].map(|i| 0.5 * w[i]));
    shows([fans, strip], "brightness x warmth x balance");

    // An empty map is neutral again.
    engine.set_white_balance(Default::default());
    shows([strip, strip], "balance cleared");
    engine.shutdown();
}
