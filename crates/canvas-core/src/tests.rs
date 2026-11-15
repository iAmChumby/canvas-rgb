//! Integration tests for the render loop.
//!
//! These use their own tiny test effects (never the `canvas-effects` crate) and a
//! fake sink, so they never touch hardware.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canvas_proto::api::Power;
use canvas_proto::color::Rgb8;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::LedSlice;
use canvas_proto::props::{InputNeeds, PropKind, PropertyDef, PropertySchema, Props};
use canvas_proto::traits::{Canvas, Effect, EngineSettings, FrameSink};

use crate::spawn;

/// The deadline for a first frame to arrive (#143, class `cause:timing-test`; same shape as
/// #59, which raised the web waits to 10 s). The waits stay event-based: `wait_until`
/// returns on the first frame, so a passing run costs nothing; 20 s is only the ceiling a
/// starved runner gets. No assertion here measures time, so this never widens a measured
/// budget.
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(20);

/// A sink that records every frame it receives.
#[derive(Default)]
struct RecordingSink {
    frames: Mutex<Vec<Vec<Rgb8>>>,
    count: AtomicU32,
}

impl RecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn last(&self) -> Option<Vec<Rgb8>> {
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).last().cloned()
    }

    /// Every frame the sink received, oldest first.
    fn all_frames(&self) -> Vec<Vec<Rgb8>> {
        self.frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn frame_count(&self) -> u32 {
        self.count.load(Ordering::Acquire)
    }
}

impl FrameSink for RecordingSink {
    fn submit(&self, colors: Vec<Rgb8>) {
        self.count.fetch_add(1, Ordering::AcqRel);
        self.frames.lock().unwrap_or_else(|e| e.into_inner()).push(colors);
    }
}

/// An effect that paints the whole canvas one color.
struct SolidEffect {
    color: [f32; 3],
    initialized: Arc<AtomicBool>,
}

impl SolidEffect {
    fn new(color: [f32; 3], initialized: Arc<AtomicBool>) -> Self {
        SolidEffect { color, initialized }
    }
}

impl Effect for SolidEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_solid".into(),
            name: "Test Solid".into(),
            description: "Paints the canvas one color.".into(),
            needs: InputNeeds::default(),
            properties: vec![PropertyDef {
                key: "color".into(),
                label: "Color".into(),
                kind: PropKind::Color { default: Rgb8::WHITE },
            }],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {
        self.initialized.store(true, Ordering::Release);
    }

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        canvas.fill(self.color);
    }
}

/// An effect that counts how many times it was initialised.
struct CountingEffect {
    inits: Arc<AtomicU32>,
}

impl Effect for CountingEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_counting".into(),
            name: "Test Counting".into(),
            description: "Counts init calls.".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {
        self.inits.fetch_add(1, Ordering::AcqRel);
    }

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        canvas.fill([0.0, 0.0, 0.0]);
    }
}

/// An effect that paints a horizontal gradient: red at x=0, blue at the right edge.
struct GradientEffect;

impl Effect for GradientEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_gradient".into(),
            name: "Test Gradient".into(),
            description: "A horizontal red-to-blue gradient.".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        let w = canvas.width;
        for y in 0..canvas.height {
            for x in 0..w {
                let t = if w > 1 { x as f32 / (w - 1) as f32 } else { 0.0 };
                canvas.set(x, y, [1.0 - t, 0.0, t]);
            }
        }
    }
}

fn settings(fps: u32) -> EngineSettings {
    EngineSettings {
        fps,
        power: Power::Running,
        brightness: 100,
        warmth: canvas_proto::consts::WARMTH_OFF,
    }
}

/// Waits until `check` is true, or the timeout expires. Returns whether it became true.
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

/// An effect that panics in `update`. `updates` counts its calls.
struct PanicUpdateEffect {
    updates: Arc<AtomicU32>,
}

impl Effect for PanicUpdateEffect {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "test_panic_update".into(),
            name: "Test Panic Update".into(),
            description: "Panics in update (#99).".into(),
            needs: InputNeeds::default(),
            properties: vec![],
            presets: vec![],
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        self.updates.fetch_add(1, Ordering::AcqRel);
        panic!("update boom");
    }
}

/// A parked engine that a consumer reaches between its decision to park and its wait must not
/// sleep through that change (#250).
///
/// `decided_hook` holds the render thread inside the gap. The test adds a scene and a sink
/// there, and the thread must draw: the change bumps the engine's change counter, and the wait
/// compares against the counter it read with its decision. The wait used to read the counter
/// after the gap, so it counted the change as already seen and slept until the next unrelated
/// change: a `capture_wiring` run on CI waited 10 s for a frame that the thread never drew.
#[test]
fn a_consumer_that_arrives_between_the_park_decision_and_the_wait_is_not_slept_through() {
    use canvas_proto::scene::{zone_light_id, Arrangement, GroupMode, Light};
    use canvas_proto::traits::{Engine, EngineGroup};
    use std::sync::mpsc;

    let (reached_tx, reached_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let reached_tx = Mutex::new(reached_tx);
    let release_rx = Mutex::new(release_rx);
    // Only the first decision is held: that is the one made with no consumer.
    let held = AtomicBool::new(false);
    let hook: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if !held.swap(true, Ordering::AcqRel) {
            let _ = reached_tx.lock().unwrap_or_else(|e| e.into_inner()).send(());
            let _ = release_rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
        }
    });
    let core = crate::engine::new_core_with_decided_hook(&settings(30), hook);
    let engine: Arc<dyn Engine> = core.clone();
    crate::engine::start_core(&core);

    // The thread has decided to park, and has not waited yet.
    reached_rx
        .recv_timeout(FIRST_FRAME_WAIT)
        .expect("the render thread made its first decision");

    // The consumer arrives in the gap: a scene and a sink on its light's device.
    let dev = device("d0", vec![zone(0, ZoneKind::Linear, 4)]);
    let device_id = dev.id.clone();
    let light = Light {
        id: zone_light_id(&device_id, 0),
        device_id: device_id.clone(),
        zone_index: 0,
        leds: None,
        name: None,
        arrangement: Arrangement::Strip,
        reversed: false,
    };
    engine.set_scene(
        vec![light],
        vec![dev],
        vec![EngineGroup {
            id: "all".into(),
            members: vec![zone_light_id(&device_id, 0)],
            mode: GroupMode::FlowAcross,
            brightness: 100,
            effect: Some(Box::new(SolidEffect::new(
                [1.0, 0.0, 0.0],
                Arc::new(AtomicBool::new(false)),
            ))),
            props: Props::new(),
        }],
        false,
    );
    let sink = RecordingSink::new();
    engine.add_sink(device_id, sink.clone());
    release_tx.send(()).expect("the thread is waiting at the hook");

    assert!(
        wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 0),
        "the sink that arrived between the decision and the wait was never given a frame"
    );
    engine.shutdown();
}

fn zone(index: u32, kind: ZoneKind, led_count: u32) -> ZoneInfo {
    ZoneInfo {
        index,
        name: format!("zone{index}"),
        kind,
        led_count,
        min_leds: led_count,
        max_leds: led_count,
        resizable: false,
    }
}

fn device(id: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.to_string()),
        name: id.to_string(),
        vendor: "test".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones,
        connected: true,
    }
}

// ---------------------------------------------------------------------------
// Scenes (0.14.0, #310): one effect per group, merged per device
// ---------------------------------------------------------------------------

mod scene {
    use super::*;
    use canvas_proto::consts::PREVIEW_MAX_FPS;
    use canvas_proto::scene::{zone_light_id, Arrangement, GroupMode, Light, SCENE_FADE_MS};
    use canvas_proto::traits::{Engine, EngineGroup, LightFrame, LightPreviewSink};

    const RED: Rgb8 = Rgb8::new(255, 0, 0);
    const BLUE: Rgb8 = Rgb8::new(0, 0, 255);
    const GREEN: Rgb8 = Rgb8::new(0, 255, 0);

    /// A recording light-preview sink.
    #[derive(Default)]
    struct RecordingLights {
        frames: Mutex<Vec<LightFrame>>,
    }

    impl RecordingLights {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn last(&self) -> Option<LightFrame> {
            self.frames.lock().unwrap_or_else(|e| e.into_inner()).last().cloned()
        }
        fn count(&self) -> usize {
            self.frames.lock().unwrap_or_else(|e| e.into_inner()).len()
        }
    }

    impl LightPreviewSink for RecordingLights {
        fn submit(&self, frame: LightFrame) {
            self.frames.lock().unwrap_or_else(|e| e.into_inner()).push(frame);
        }
    }

    /// A device with one linear zone of `leds` LEDs.
    fn strip_device(id: &str, leds: u32) -> DeviceInfo {
        device(id, vec![zone(0, ZoneKind::Linear, leds)])
    }

    /// The light that is the whole of zone 0 of device `id`.
    fn light_of(id: &str) -> Light {
        let device_id = DeviceId(id.to_string());
        Light {
            id: zone_light_id(&device_id, 0),
            device_id,
            zone_index: 0,
            leds: None,
            name: None,
            arrangement: Arrangement::Strip,
            reversed: false,
        }
    }

    /// A light that is a run of zone `zone` of device `id`.
    fn slice_light(id: &str, zone: u32, start: u32, count: u32) -> Light {
        let device_id = DeviceId(id.to_string());
        Light {
            id: format!("{id}#z{zone}s{start}"),
            device_id,
            zone_index: zone,
            leds: Some(LedSlice { start, count }),
            name: None,
            arrangement: Arrangement::Strip,
            reversed: false,
        }
    }

    fn group(id: &str, members: &[&str], effect: Option<Box<dyn Effect>>) -> EngineGroup {
        EngineGroup {
            id: id.to_string(),
            members: members.iter().map(|m| m.to_string()).collect(),
            mode: GroupMode::FlowAcross,
            brightness: 100,
            effect,
            props: Props::new(),
        }
    }

    fn solid(color: [f32; 3]) -> Option<Box<dyn Effect>> {
        Some(Box::new(SolidEffect::new(color, Arc::new(AtomicBool::new(false)))))
    }

    /// Two devices `d1`, `d2` of 12 LEDs, each with its own sink.
    fn two_devices() -> (Arc<dyn Engine>, Vec<DeviceInfo>, Arc<RecordingSink>, Arc<RecordingSink>) {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12), strip_device("d2", 12)];
        let a = RecordingSink::new();
        let b = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), a.clone());
        engine.add_sink(DeviceId("d2".into()), b.clone());
        (engine, devices, a, b)
    }

    fn lights_d1_d2() -> Vec<Light> {
        vec![light_of("d1"), light_of("d2")]
    }

    fn all(frame: &[Rgb8], color: Rgb8) -> bool {
        frame.iter().all(|c| *c == color)
    }

    /// An effect that panics in `update` and again in `Drop` (#310 review 1).
    ///
    /// The `update` panic marks the group failed, and a failed group keeps its effect in
    /// the scene's state (the render thread does not take it out again). The effect is
    /// therefore in the state when the scene ends, which is what the next `set_scene` drops.
    struct FailingEffectThatPanicsInDrop;

    impl Effect for FailingEffectThatPanicsInDrop {
        fn schema(&self) -> PropertySchema {
            PropertySchema {
                id: "test_fail_and_panic_in_drop".into(),
                name: "Test Fail And Panic In Drop".into(),
                description: "Panics in update and in Drop.".into(),
                needs: InputNeeds::default(),
                properties: vec![],
                presets: vec![],
                thumbnail: None,
            }
        }

        fn init(&mut self, _props: &Props) {}

        fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
            panic!("update boom");
        }
    }

    impl Drop for FailingEffectThatPanicsInDrop {
        fn drop(&mut self) {
            panic!("drop boom");
        }
    }

    /// An effect that fails in `update` and, when it is dropped, calls back into the
    /// engine (#310 review 3).
    ///
    /// The `update` panic marks the group failed, and a failed group keeps its effect in
    /// the scene's state, so the effect is in the state when the scene ends: that is
    /// what a replacing `set_scene` drops. The drop calls back into the engine, which takes the
    /// state lock again: a drop under that lock would wait for the lock it is holding,
    /// and the call would never return.
    struct EffectThatCallsBackInDrop {
        /// The engine, in a slot the effect empties when it is dropped, so the effect
        /// does not keep the engine alive.
        engine: Arc<Mutex<Option<Arc<dyn Engine>>>>,
    }

    impl Effect for EffectThatCallsBackInDrop {
        fn schema(&self) -> PropertySchema {
            PropertySchema {
                id: "test_calls_back_in_drop".into(),
                name: "Test Calls Back In Drop".into(),
                description: "Fails in update and calls back into the engine in Drop.".into(),
                needs: InputNeeds::default(),
                properties: vec![],
                presets: vec![],
                thumbnail: None,
            }
        }

        fn init(&mut self, _props: &Props) {}

        fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
            panic!("update boom");
        }
    }

    impl Drop for EffectThatCallsBackInDrop {
        fn drop(&mut self) {
            let engine = self.engine.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(engine) = engine {
                // The state lock, taken again while this drop runs.
                engine.set_brightness(50);
            }
        }
    }

    #[test]
    fn two_groups_play_two_effects_at_once() {
        let (engine, devices, a, b) = two_devices();
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![
                group("a", &["d1#z0"], solid([1.0, 0.0, 0.0])),
                group("b", &["d2#z0"], solid([0.0, 0.0, 1.0])),
            ],
            false,
        );
        assert!(
            wait_until(FIRST_FRAME_WAIT, || {
                a.last().is_some_and(|f| all(&f, RED)) && b.last().is_some_and(|f| all(&f, BLUE))
            }),
            "a = {:?}, b = {:?}",
            a.last(),
            b.last()
        );
        engine.shutdown();
    }

    #[test]
    fn lights_of_two_groups_on_one_device_merge_into_one_frame() {
        let engine = spawn(settings(60));
        let devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 24)])];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d0".into()), sink.clone());
        engine.set_scene(
            vec![slice_light("d0", 0, 0, 12), slice_light("d0", 0, 12, 12)],
            devices,
            vec![
                group("a", &["d0#z0s0"], solid([1.0, 0.0, 0.0])),
                group("b", &["d0#z0s12"], solid([0.0, 0.0, 1.0])),
            ],
            false,
        );
        let mut want = vec![RED; 12];
        want.extend(vec![BLUE; 12]);
        assert!(
            wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| f == want)),
            "last = {:?}",
            sink.last()
        );
        engine.shutdown();
    }

    #[test]
    fn leds_no_group_owns_are_black() {
        let engine = spawn(settings(60));
        // Zone 0 has 8 LEDs nobody owns; the light is the first 12 of zone 1's 20.
        let devices = vec![device(
            "d0",
            vec![zone(0, ZoneKind::Linear, 8), zone(1, ZoneKind::Linear, 20)],
        )];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d0".into()), sink.clone());
        engine.set_scene(
            vec![slice_light("d0", 1, 0, 12)],
            devices,
            vec![group("a", &["d0#z1s0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || {
            sink.last().is_some_and(|f| f.len() == 28 && f[8] == RED)
        }));
        let frame = sink.last().expect("a frame");
        assert!(all(&frame[..8], Rgb8::BLACK), "zone 0 is nobody's: {frame:?}");
        assert!(all(&frame[8..20], RED), "the light's 12 LEDs: {frame:?}");
        assert!(all(&frame[20..], Rgb8::BLACK), "the rest of zone 1 is nobody's: {frame:?}");
        engine.shutdown();
    }

    #[test]
    fn a_group_set_to_off_is_black() {
        let (engine, devices, a, b) = two_devices();
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![group("a", &["d1#z0"], None), group("b", &["d2#z0"], solid([0.0, 0.0, 1.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || b.last().is_some_and(|f| all(&f, BLUE))));
        let frame = a.last().expect("device a still gets frames");
        assert_eq!(frame.len(), 12);
        assert!(all(&frame, Rgb8::BLACK));
        engine.shutdown();
    }

    #[test]
    fn group_brightness_multiplies_with_global() {
        let engine = spawn(settings(60));
        engine.set_brightness(50);
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        let mut g = group("a", &["d1#z0"], solid([200.0 / 255.0; 3]));
        g.brightness = 50;
        engine.set_scene(vec![light_of("d1")], devices, vec![g], false);
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > 3));
        let frame = sink.last().expect("a frame");
        for c in &frame {
            for channel in [c.r, c.g, c.b] {
                assert!((49..=51).contains(&channel), "200 x 50% x 50% is 50, got {frame:?}");
            }
        }
        engine.shutdown();
    }

    #[test]
    fn a_panicking_group_goes_dark_and_reports_while_the_other_keeps_running() {
        let (engine, devices, a, b) = two_devices();
        let updates = Arc::new(AtomicU32::new(0));
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![
                group("a", &["d1#z0"], Some(Box::new(PanicUpdateEffect { updates: updates.clone() }))),
                group("b", &["d2#z0"], solid([0.0, 0.0, 1.0])),
            ],
            false,
        );
        assert!(
            wait_until(FIRST_FRAME_WAIT, || {
                engine.group_states().iter().any(|s| s.id == "a" && s.failed.is_some())
            }),
            "the panic is reported: {:?}",
            engine.group_states()
        );
        let states = engine.group_states();
        let failed = states.iter().find(|s| s.id == "a").and_then(|s| s.failed.clone());
        assert!(
            failed.is_some_and(|t| t.contains("update boom")),
            "the text of the panic: {states:?}"
        );
        assert!(states.iter().find(|s| s.id == "b").is_some_and(|s| s.failed.is_none()));
        // The other group keeps running; the failed one is dark and never called again.
        let frames_b = b.frame_count();
        assert!(wait_until(FIRST_FRAME_WAIT, || b.frame_count() > frames_b + 5));
        assert!(b.last().is_some_and(|f| all(&f, BLUE)));
        assert!(a.last().is_some_and(|f| all(&f, Rgb8::BLACK)));
        assert_eq!(updates.load(Ordering::Acquire), 1, "a failed group is not called again");
        engine.shutdown();
    }

    #[test]
    fn replacing_a_failed_groups_effect_clears_the_failure() {
        let (engine, devices, a, _b) = two_devices();
        let updates = Arc::new(AtomicU32::new(0));
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![
                group("a", &["d1#z0"], Some(Box::new(PanicUpdateEffect { updates }))),
                group("b", &["d2#z0"], solid([0.0, 0.0, 1.0])),
            ],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || {
            engine.group_states().iter().any(|s| s.id == "a" && s.failed.is_some())
        }));
        engine.set_group_effect("a", solid([1.0, 0.0, 0.0]), &Props::new());
        assert!(
            wait_until(FIRST_FRAME_WAIT, || a.last().is_some_and(|f| all(&f, RED))),
            "the new effect plays: {:?}",
            a.last()
        );
        assert!(
            engine.group_states().iter().all(|s| s.failed.is_none()),
            "{:?}",
            engine.group_states()
        );
        engine.shutdown();
    }

    #[test]
    fn set_group_props_reaches_only_that_group() {
        let (engine, devices, a, _b) = two_devices();
        let inits_a = Arc::new(AtomicU32::new(0));
        let inits_b = Arc::new(AtomicU32::new(0));
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![
                group("a", &["d1#z0"], Some(Box::new(CountingEffect { inits: inits_a.clone() }))),
                group("b", &["d2#z0"], Some(Box::new(CountingEffect { inits: inits_b.clone() }))),
            ],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || a.frame_count() > 2));
        let (before_a, before_b) = (inits_a.load(Ordering::Acquire), inits_b.load(Ordering::Acquire));
        engine.set_group_props("a", &Props::new());
        let frames = a.frame_count();
        assert!(wait_until(FIRST_FRAME_WAIT, || a.frame_count() > frames + 3));
        assert_eq!(inits_a.load(Ordering::Acquire), before_a + 1, "group a re-initialised");
        assert_eq!(inits_b.load(Ordering::Acquire), before_b, "group b did not");
        engine.shutdown();
    }

    #[test]
    fn set_scene_with_fade_blends_then_settles() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, RED))));
        let before = sink.frame_count() as usize;
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("a", &["d1#z0"], solid([0.0, 0.0, 1.0]))],
            true,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, BLUE))));
        let frames = sink.all_frames();
        let mid = frames[before..]
            .iter()
            .any(|f| f[0].r > 0 && f[0].r < 255 && f[0].b > 0 && f[0].b < 255);
        assert!(mid, "a frame between the old red and the new blue: {:?}", &frames[before..]);
        // The fade ends on the new scene and stays there: from the first all-new frame on, every
        // frame is all new (the fade is SCENE_FADE_MS long and the frames below outlast it).
        let settled = frames.len();
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() as usize > settled + 10));
        let frames = sink.all_frames();
        let first_blue = frames[before..].iter().position(|f| all(f, BLUE)).expect("settled") + before;
        assert!(
            frames[first_blue..].iter().all(|f| all(f, BLUE)),
            "after {SCENE_FADE_MS} ms the scene is the new one: {:?}",
            &frames[first_blue..]
        );
        engine.shutdown();
    }

    #[test]
    fn set_scene_without_fade_cuts() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, RED))));
        let before = sink.frame_count() as usize;
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("a", &["d1#z0"], solid([0.0, 0.0, 1.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, BLUE))));
        for frame in &sink.all_frames()[before..] {
            assert!(all(frame, RED) || all(frame, BLUE), "no blended frame on a cut: {frame:?}");
        }
        engine.shutdown();
    }

    #[test]
    fn light_preview_carries_each_lights_colours_before_global_brightness() {
        let engine = spawn(settings(60));
        engine.set_brightness(50);
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        let lights = RecordingLights::new();
        engine.set_light_preview(Some(lights.clone()));
        let g = group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]));
        engine.set_scene(vec![light_of("d1")], devices, vec![g], false);
        assert!(wait_until(FIRST_FRAME_WAIT, || lights.count() > 0 && sink.frame_count() > 0));
        let preview = lights.last().expect("a light frame");
        let (id, colours) = &preview.lights[0];
        assert_eq!(id, "d1#z0");
        assert_eq!(colours.len(), 12);
        assert!(colours.iter().all(|c| *c == RED), "before the global stage: {colours:?}");
        let device_frame = sink.last().expect("a frame");
        assert!(
            device_frame.iter().all(|c| (127..=129).contains(&c.r)),
            "after it: {device_frame:?}"
        );
        engine.shutdown();
    }

    /// #310 review: the preview's cap is a wall-clock limit (15 a second), so the test
    /// measures the previews against the wall time they had. The old bound counted the
    /// frames the machine rendered and compared the previews with a quarter of them: a
    /// machine that renders at 30 fps a second sends half as many frames in the same
    /// second, and a bound that counts the frames then fails correct code on a busy
    /// machine (and the bound before it, `previews * 2 < frames`, let a cap over 15
    /// through).
    #[test]
    fn light_preview_is_rate_limited() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        let lights = RecordingLights::new();
        engine.set_light_preview(Some(lights.clone()));
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || lights.count() > 0));
        // The window: from the first preview to a second's worth of frames.
        let started = Instant::now();
        let l0 = lights.count();
        let s0 = sink.frame_count();
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > s0 + 60));
        let seconds = started.elapsed().as_secs_f64();
        let previews = lights.count() - l0;
        // The cap is 15 a second, so `seconds` of wall time cannot hold more than
        // `seconds * 15` previews, plus the one the window's edges can hold. The bound
        // follows the wall clock, so a slow machine cannot fail it. The other side of that:
        // a looser cap is caught only when the engine renders faster than the looser cap
        // (a 25/s cap is caught at 40 fps and up, not at 30).
        assert!(previews > 0, "a second of frames sent no preview at all");
        let limit = seconds * f64::from(PREVIEW_MAX_FPS) + 2.0;
        assert!(
            previews as f64 <= limit,
            "{previews} previews in {seconds:.3}s: the preview runs at more than {PREVIEW_MAX_FPS} a second"
        );
        engine.shutdown();
    }

    /// Review Focus 2: a zone's LED count changes while its light is in a group.
    #[test]
    fn resize_rebuilds_plan_without_stale_frame() {
        let engine = spawn(settings(60));
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d1")],
            vec![strip_device("d1", 12)],
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| f.len() == 12)));
        engine.set_scene(
            vec![light_of("d1")],
            vec![strip_device("d1", 20)],
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        let seen = sink.frame_count() as usize;
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.frame_count() as usize > seen + 5));
        let frames = sink.all_frames();
        for frame in &frames[seen..] {
            assert_eq!(frame.len(), 20, "a frame after the resize has the new length");
            assert!(all(frame, RED), "and every LED of it plays: {frame:?}");
        }
        engine.shutdown();
    }

    #[test]
    fn flow_across_shows_one_gradient_over_two_lights_and_each_its_own_repeats_it() {
        for (mode, flows) in [(GroupMode::FlowAcross, true), (GroupMode::EachItsOwn, false)] {
            let (engine, devices, a, b) = two_devices();
            let mut g = group("a", &["d1#z0", "d2#z0"], Some(Box::new(GradientEffect)));
            g.mode = mode;
            engine.set_scene(lights_d1_d2(), devices, vec![g], false);
            // The first frames can predate the effect's first draw; wait for the gradient.
            assert!(wait_until(FIRST_FRAME_WAIT, || {
                a.frame_count() > 3
                    && b.frame_count() > 3
                    && a.last().is_some_and(|f| f[0].r > 200)
                    && b.last().is_some_and(|f| f[0].r > 0 || f[11].r > 0 || f[11].b > 0)
            }));
            let (fa, fb) = (a.last().unwrap(), b.last().unwrap());
            if flows {
                // One gradient: red falls along d1 and goes on falling along d2.
                assert!(fa[0].r > 230, "the left edge is red: {fa:?}");
                assert!(fb[11].r < 25, "the right edge is blue: {fb:?}");
                assert!(fa.iter().all(|c| c.r >= 115), "d1 holds the left half: {fa:?}");
                assert!(fb.iter().all(|c| c.r <= 140), "d2 holds the right half: {fb:?}");
                let seam = i32::from(fa[11].r) - i32::from(fb[0].r);
                assert!(seam.abs() < 40, "the seam is smooth: {fa:?} {fb:?}");
            } else {
                // Each its own: both lights show the whole gradient.
                assert!(fa[0].r > 230 && fb[0].r > 230, "{fa:?} {fb:?}");
                assert!(fa[11].r < 25 && fb[11].r < 25, "{fa:?} {fb:?}");
            }
            engine.shutdown();
        }
    }

    /// Task 4 gate: the planner's samples are in the logical 320x200 canvas, the group's canvas
    /// is `SCENE_RENDER_SCALE` of that (80x50). Without the scaling every LED past the first
    /// quarter of the run clamps to the last column and the gradient collapses to one colour.
    #[test]
    fn a_12_and_36_led_flow_across_group_spreads_the_gradient_over_all_48_leds() {
        let engine = spawn(settings(60));
        let devices = vec![device(
            "d0",
            vec![zone(0, ZoneKind::Linear, 12), zone(1, ZoneKind::Linear, 36)],
        )];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d0".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d0"), slice_light("d0", 1, 0, 36)],
            devices,
            vec![group("a", &["d0#z0", "d0#z1s0"], Some(Box::new(GradientEffect)))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || {
            sink.last().is_some_and(|f| f.len() == 48 && f[0].r > 200)
        }));
        let frame = sink.last().expect("a frame");
        // Red falls from the first LED to the last, one step at a time over the whole run.
        assert!(frame[0].r > 230, "the first LED is red: {frame:?}");
        assert!(frame[47].r < 25, "the last LED is blue: {frame:?}");
        assert!((90..=165).contains(&frame[24].r), "the middle LED is half way: {frame:?}");
        assert!(
            frame.windows(2).all(|w| w[1].r <= w[0].r),
            "red never rises along the run: {frame:?}"
        );
        let distinct = frame.iter().map(|c| c.r).collect::<std::collections::BTreeSet<_>>().len();
        assert!(distinct >= 30, "{distinct} distinct values over 48 LEDs: {frame:?}");
        engine.shutdown();
    }

    #[test]
    fn a_member_naming_no_light_or_an_absent_device_is_skipped() {
        let (engine, devices, a, _b) = two_devices();
        engine.set_scene(
            // d2's light is declared, but its device is not in the list.
            lights_d1_d2(),
            devices.into_iter().take(1).collect(),
            vec![group("a", &["nope", "d2#z0", "d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || a.last().is_some_and(|f| all(&f, RED))));
        engine.shutdown();
    }

    #[test]
    fn group_states_lists_every_group_in_scene_order_and_is_empty_outside_scene_mode() {
        let (engine, devices, _a, _b) = two_devices();
        assert!(engine.group_states().is_empty());
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![group("b", &["d2#z0"], solid([0.0, 0.0, 1.0])), group("a", &["d1#z0"], None)],
            false,
        );
        let ids: Vec<String> = engine.group_states().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["b".to_string(), "a".to_string()]);
        engine.shutdown();
    }

    /// #310 review 1: a scene replaced by another drops its effects behind the guard, outside
    /// the state lock, exactly like the scene's own end. An effect whose `Drop` panics must not
    /// take the caller down, and the engine must keep rendering.
    ///
    /// What the drop being *outside* the lock buys is proved by
    /// `a_scene_effect_whose_drop_calls_back_into_the_engine_does_not_deadlock_it`.
    #[test]
    fn replacing_a_scene_drops_its_effect_that_panics_in_drop_and_the_engine_keeps_rendering() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group(
                "a",
                &["d1#z0"],
                Some(Box::new(FailingEffectThatPanicsInDrop)),
            )],
            false,
        );
        // The group failed, so the effect stays in the scene's state: the render thread
        // does not take a failed group's effect out again.
        assert!(
            wait_until(FIRST_FRAME_WAIT, || {
                engine.group_states().iter().any(|s| s.id == "a" && s.failed.is_some())
            }),
            "the group failed: {:?}",
            engine.group_states()
        );
        let before = sink.frame_count();

        // The effect panics when it is dropped by the scene that replaces it. The call must
        // return.
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("b", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );

        // And the engine keeps rendering: the new scene delivers frames.
        assert!(
            wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > before + 5),
            "the engine kept rendering: {} frames",
            sink.frame_count()
        );
        assert!(sink.last().is_some_and(|f| all(&f, RED)), "the new scene renders red");
        engine.shutdown();
    }

    /// #310 review 3: a `Drop` that calls back into the engine must not deadlock it. The
    /// scene's effects drop outside the state lock, so the callback takes the lock again
    /// without waiting for the lock it is holding.
    ///
    /// The call runs on its own thread with a deadline, so a drop that did hold the lock
    /// fails the test instead of hanging the suite.
    #[test]
    fn a_scene_effect_whose_drop_calls_back_into_the_engine_does_not_deadlock_it() {
        let engine = spawn(settings(60));
        let slot: Arc<Mutex<Option<Arc<dyn Engine>>>> = Arc::new(Mutex::new(None));
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(engine.clone());
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group(
                "a",
                &["d1#z0"],
                Some(Box::new(EffectThatCallsBackInDrop { engine: slot.clone() })),
            )],
            false,
        );
        // The group failed, so the effect stays in the scene's state.
        assert!(
            wait_until(FIRST_FRAME_WAIT, || {
                engine.group_states().iter().any(|s| s.id == "a" && s.failed.is_some())
            }),
            "the group failed: {:?}",
            engine.group_states()
        );

        // Replacing the scene drops the effect, and the drop calls back into the engine.
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = {
            let engine = engine.clone();
            std::thread::spawn(move || {
                engine.set_scene(
                    vec![light_of("d1")],
                    devices,
                    vec![group("b", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
                    false,
                );
                let _ = tx.send(());
            })
        };
        assert!(
            rx.recv_timeout(Duration::from_secs(10)).is_ok(),
            "set_scene returned: the effect's drop ran outside the state lock"
        );
        let _ = worker.join();

        // And the engine still renders.
        let before = sink.frame_count();
        assert!(
            wait_until(FIRST_FRAME_WAIT, || sink.frame_count() > before + 5),
            "the engine kept rendering: {} frames",
            sink.frame_count()
        );
        engine.shutdown();
    }

    /// #310 review 2: a sink whose device the scene does not hold gets a black frame of
    /// its own length. It never keeps the colours it was last sent.
    #[test]
    fn a_sink_whose_device_is_not_in_the_scene_gets_a_black_frame() {
        let engine = spawn(settings(60));
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        // The device is known first, so the sink's frames have 12 LEDs.
        let devices = vec![strip_device("d1", 12)];
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, RED))));

        // Now a scene without that device: the sink stays, the device does not.
        engine.set_scene(vec![], vec![strip_device("d2", 12)], vec![], false);
        // Wait for the black frame itself, not for a number of frames: the red frames
        // before it can satisfy a frame count first, and then the last frame is red.
        assert!(
            wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, Rgb8::BLACK))),
            "the sink whose device is gone went dark: {:?}",
            sink.last()
        );
        let frame = sink.last().expect("a frame");
        assert_eq!(frame.len(), 12, "the sink's own length");
        assert!(
            all(&frame, Rgb8::BLACK),
            "the device is gone: it goes dark, never lit: {frame:?}"
        );
        engine.shutdown();
    }

    /// #310 review: a fade that follows a pause starts from what the strip is showing.
    /// The pause wrote a black frame, so the strip is dark and the fade blends from
    /// black: the red the scene had before the pause is not on the strip any more.
    #[test]
    fn a_fade_after_a_pause_starts_from_the_black_the_strip_shows() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());

        // Scene A: red.
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, RED))));

        // The pause writes one black frame: the strip goes dark.
        engine.set_power(Power::Paused);
        assert!(
            wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, Rgb8::BLACK))),
            "the pause wrote a black frame"
        );
        let before = sink.frame_count() as usize;

        // Scene B with a fade, set while the engine is paused.
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("a", &["d1#z0"], solid([0.0, 1.0, 0.0]))],
            true,
        );
        engine.set_power(Power::Running);

        // The fade settles on green.
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, GREEN))));
        let frames = sink.all_frames();
        let after = &frames[before..];
        assert!(
            !after.iter().any(|f| all(f, RED)),
            "the fade never shows the red the strip was not showing: {after:?}"
        );
        assert!(
            after.iter().any(|f| f[0].g > 0),
            "a frame between the black and the green: {after:?}"
        );
        engine.shutdown();
    }

    /// #310 review: a device that left the scene was sent black frames, so a fade that
    /// follows starts from black: the colours it had before it left are not on the strip
    /// any more.
    #[test]
    fn a_fade_after_a_device_left_the_scene_starts_from_the_black_it_shows() {
        let engine = spawn(settings(60));
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());

        // Scene A: red, on d1.
        engine.set_scene(
            vec![light_of("d1")],
            devices.clone(),
            vec![group("a", &["d1#z0"], solid([1.0, 0.0, 0.0]))],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, RED))));

        // Scene B: another device. d1's sink goes dark.
        engine.set_scene(vec![], vec![strip_device("d2", 12)], vec![], false);
        assert!(
            wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, Rgb8::BLACK))),
            "the device that left the scene went dark"
        );
        let before = sink.frame_count() as usize;

        // Scene C with a fade, with d1 back in the scene.
        engine.set_scene(
            vec![light_of("d1")],
            devices,
            vec![group("a", &["d1#z0"], solid([0.0, 1.0, 0.0]))],
            true,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || sink.last().is_some_and(|f| all(&f, GREEN))));
        let frames = sink.all_frames();
        let after = &frames[before..];
        assert!(
            !after.iter().any(|f| all(f, RED)),
            "the fade never shows the colours the device was not showing: {after:?}"
        );
        assert!(
            after.iter().any(|f| f[0].g > 0),
            "a frame between the black and the green: {after:?}"
        );
        engine.shutdown();
    }

    /// #310 review 4: `set_group_brightness` changes one group and leaves the others
    /// alone.
    #[test]
    fn set_group_brightness_changes_one_group_and_leaves_the_other_alone() {
        let (engine, devices, a, b) = two_devices();
        engine.set_scene(
            lights_d1_d2(),
            devices,
            vec![
                group("a", &["d1#z0"], solid([1.0, 0.0, 0.0])),
                group("b", &["d2#z0"], solid([0.0, 0.0, 1.0])),
            ],
            false,
        );
        assert!(wait_until(FIRST_FRAME_WAIT, || {
            a.last().is_some_and(|f| all(&f, RED)) && b.last().is_some_and(|f| all(&f, BLUE))
        }));
        engine.set_group_brightness("a", 50);
        assert!(
            wait_until(FIRST_FRAME_WAIT, || {
                a.last().is_some_and(|f| f.iter().all(|c| (127..=129).contains(&c.r)))
            }),
            "the group's own brightness halves its colours: {:?}",
            a.last()
        );
        assert!(
            b.last().is_some_and(|f| all(&f, BLUE)),
            "the other group is unchanged: {:?}",
            b.last()
        );
        engine.shutdown();
    }

    /// #310 review 4: the light preview carries the group's own brightness, and the
    /// device's frame carries the global one on top of it. A group at 50% and the global
    /// at 50% on (200, 200, 200): the preview shows 100, the device 50.
    #[test]
    fn the_light_preview_carries_the_group_brightness_and_the_device_frame_the_global_one() {
        let engine = spawn(settings(60));
        engine.set_brightness(50);
        let devices = vec![strip_device("d1", 12)];
        let sink = RecordingSink::new();
        engine.add_sink(DeviceId("d1".into()), sink.clone());
        let lights = RecordingLights::new();
        engine.set_light_preview(Some(lights.clone()));
        let mut g = group("a", &["d1#z0"], solid([200.0 / 255.0; 3]));
        g.brightness = 50;
        engine.set_scene(vec![light_of("d1")], devices, vec![g], false);
        assert!(wait_until(FIRST_FRAME_WAIT, || lights.count() > 0 && sink.frame_count() > 0));
        let preview = lights.last().expect("a light frame");
        let (_, colours) = &preview.lights[0];
        assert!(
            colours.iter().all(|c| (99..=101).contains(&c.r)),
            "the preview carries the group's brightness: {colours:?}"
        );
        let frame = sink.last().expect("a frame");
        assert!(
            frame.iter().all(|c| (49..=51).contains(&c.r)),
            "the device's frame carries the global one too: {frame:?}"
        );
        engine.shutdown();
    }
}
