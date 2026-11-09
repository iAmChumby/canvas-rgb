//! The White effect reaches every device (#50).
//!
//! A real engine with two fake devices: the White effect from the registry must write
//! the white of the chosen temperature to every LED of every sink. No device, no port.

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
    });
    engine.set_layout(layout(), vec![device("fans", 24), device("strip", 60)]);
    let sinks = [("fans", 24usize), ("strip", 60usize)].map(|(id, leds)| {
        let sink = Arc::new(RecordingSink::default());
        engine.add_sink(DeviceId(id.into()), sink.clone());
        (id, leds, sink)
    });

    for kelvin in [2700.0, 6500.0] {
        let want = Rgb8::from_f32(canvas_effects::color::kelvin_to_rgb(kelvin as f32));
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
