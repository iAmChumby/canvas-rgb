//! Effect stacks (D95.6, tests 1–9 and 12–14).
//!
//! Everything here runs on the fakes: the daemon's state is built from `common`, no test binds
//! the API's port, and the compositor is driven by hand with an injected clock and fake effects,
//! so a 250 ms hold is checked without waiting.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use canvas_daemon::api;
use canvas_daemon::stack::{
    self, blend_pixel, html_budget, AreaMap, Clock, LeafBuilder, LiveMap, StackEffect,
};
use canvas_daemon::state::App;
use canvas_proto::api::{routes, StackLayerState};
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::{
    Layout, LedSlice, PlacedZone, RenderScale, SamplingMode, ZoneShape,
};
use canvas_proto::props::{InputNeeds, Props, PropertySchema};
use canvas_proto::stack::{BlendMode, LayerArea, Stack, StackLayer, STACKS_FILE};
use canvas_proto::traits::{Canvas, Effect, Inputs};
use common::{FakeAdapter, FakeEngine};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tower::ServiceExt;

/// The token the tests use.
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A valid HTML effect: it parses, draws and moves. The same one the import tests use.
const VALID_HTML: &str = r##"<head>
  <title>Spark</title>
  <meta description="A test effect." />
  <meta property="speed" label="Speed" type="number" min="0" max="100" default="10" />
</head>
<body><canvas id="stage" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("stage");
  var ctx = c.getContext("2d");
  var x = 0;
  function draw() {
    ctx.fillStyle = "#101020";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = "#ffcc00";
    ctx.fillRect(x, 40, 30, 30);
    x = (x + 3) % 300;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

// ---------------------------------------------------------------- the harness

/// A temporary directory that is removed when it is dropped.
struct TempHome {
    path: PathBuf,
}

impl TempHome {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "canvas-rgb-stacks-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temp directory is created");
        Self { path }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The router, the state and the temporary home the tests share.
struct Harness {
    router: axum::Router,
    app: Arc<App>,
    engine: Arc<FakeEngine>,
    home: TempHome,
    folder: PathBuf,
}

/// Builds the daemon on a temporary home, with the user effects the test gives (or the real
/// ones). `stacks.json` may be written into the home before this, so a load path can be tested
/// exactly as a start sees it.
fn build(tag: &str) -> Harness {
    build_with(tag, None)
}

fn build_with(tag: &str, user: Option<Arc<canvas_daemon::user_effects::UserEffects>>) -> Harness {
    harness_in(TempHome::new(tag), user)
}

/// The daemon on a home a test wrote first (a hand-written or corrupt `stacks.json`).
fn harness_at(home: TempHome) -> Harness {
    harness_in(home, None)
}

fn harness_in(home: TempHome, user: Option<Arc<canvas_daemon::user_effects::UserEffects>>) -> Harness {
    let folder = home.path.join("effects");
    std::fs::create_dir_all(&folder).expect("the effects folder is created");
    let store = canvas_daemon::storage::Store::at(&home.path);
    let engine = Arc::new(FakeEngine::default());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        store,
        engine.clone(),
        Box::new(FakeAdapter::default()),
        TOKEN.to_string(),
        "0.1.0-test".to_string(),
        tx,
    );
    // The tests write effects here, never into the owner's folder.
    app.effects_dir = folder.clone();
    if let Some(user) = user {
        app.user_effects = user;
    }
    app.user_effects.rescan(&folder);
    let app = Arc::new(app);
    Harness { router: api::router(app.clone()), app, engine, home, folder }
}

/// A request with the token.
fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .expect("the request builds")
}

/// A request with a body.
fn request_with_body(method: &str, path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:9430")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("the request builds")
}

/// The body of a response, as JSON.
async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("the body is bounded");
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// The path of one stack, with everything a URI may not carry escaped (a space, a control
/// character). Axum decodes `%XX` back before the handler sees the id.
fn stack_path(id: &str) -> String {
    let mut out = String::from("/api/v1/stacks/");
    for byte in id.bytes() {
        let plain = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~' | b':');
        if plain {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

async fn put_stack(harness: &Harness, stack: &Stack) -> StatusCode {
    let body = serde_json::to_string(stack).expect("a stack writes");
    harness
        .router
        .clone()
        .oneshot(request_with_body("PUT", &stack_path(&stack.id), &body))
        .await
        .expect("the router answers")
        .status()
}

// ---------------------------------------------------------------- stack and layer builders

fn layer(id: &str, effect: &str) -> StackLayer {
    StackLayer {
        id: id.to_string(),
        name: format!("Layer {id}"),
        effect: effect.to_string(),
        props: Props::new(),
        area: LayerArea::Canvas,
        blend: BlendMode::Normal,
        opacity: 100,
        visible: true,
    }
}

fn with_area(mut layer: StackLayer, area: LayerArea) -> StackLayer {
    layer.area = area;
    layer
}

fn stack(id: &str, layers: Vec<StackLayer>) -> Stack {
    Stack { id: id.to_string(), layers }
}

/// A clock the tests move by hand: `Instant` plus a millisecond offset.
#[derive(Clone)]
struct ManualClock {
    at: Arc<Mutex<Instant>>,
}

impl ManualClock {
    fn new() -> Self {
        Self { at: Arc::new(Mutex::new(Instant::now())) }
    }

    fn now(&self) -> Instant {
        *self.at.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn advance_ms(&self, ms: u64) {
        let mut at = self.at.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *at += Duration::from_millis(ms);
    }

    fn clock(&self) -> Clock {
        let clock = self.clone();
        Arc::new(move || clock.now())
    }
}

/// A fake layer effect: it fills its canvas with one colour and counts its updates. `cost` is
/// how far the clock moves during one update, which is what the compositor times.
struct Paint {
    colour: [f32; 3],
    updates: Arc<AtomicU64>,
    cost: Option<(ManualClock, u64)>,
    id: String,
}

impl Effect for Paint {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: self.id.clone(),
            name: self.id.clone(),
            description: String::new(),
            needs: InputNeeds::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        if let Some((clock, ms)) = &self.cost {
            clock.advance_ms(*ms);
        }
        canvas.fill(self.colour);
        self.updates.fetch_add(1, Ordering::SeqCst);
    }
}

/// A leaf builder that hands every layer a [`Paint`] and counts the builds.
fn paint_leaves(
    updates: Arc<AtomicU64>,
    builds: Arc<AtomicUsize>,
    colour: [f32; 3],
    cost: Option<(ManualClock, u64)>,
) -> LeafBuilder {
    Arc::new(move |id: &str| {
        builds.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Paint {
            colour,
            updates: Arc::clone(&updates),
            cost: cost.clone(),
            id: id.to_string(),
        }) as Box<dyn Effect>)
    })
}

/// The live map and area book a directly-built effect reads.
fn live_for(stack: &Stack) -> Arc<std::sync::RwLock<LiveMap>> {
    Arc::new(std::sync::RwLock::new(LiveMap::from_stack(stack, 1)))
}

fn areas_for(layout: &Layout, devices: &[DeviceInfo]) -> Arc<std::sync::RwLock<AreaMap>> {
    Arc::new(std::sync::RwLock::new(AreaMap::build(layout, devices, 1)))
}

// ---------------------------------------------------------------- devices and layouts

fn device(id: &str, name: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.into()),
        name: name.into(),
        vendor: "fake".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones,
        connected: true,
    }
}

fn zone(index: u32, name: &str, kind: ZoneKind, leds: u32) -> ZoneInfo {
    ZoneInfo {
        index,
        name: name.into(),
        kind,
        led_count: leds,
        min_leds: 0,
        max_leds: 240,
        resizable: true,
    }
}

fn placed(device_id: &str, zone_index: u32, x: f32, y: f32, w: f32, h: f32) -> PlacedZone {
    PlacedZone {
        device_id: DeviceId(device_id.into()),
        zone_index,
        x,
        y,
        width: w,
        height: h,
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

fn layout(zones: Vec<PlacedZone>, scale: RenderScale, sampling: SamplingMode) -> Layout {
    Layout {
        id: "desk".into(),
        name: "Desk".into(),
        render_scale: scale,
        sampling,
        zones,
        effect: None,
        case_form: canvas_proto::layout::CaseForm::None,
        landmarks: Vec::new(),
    }
}

// The editor's demo desk, ported from `app/web/src/lib/fixtures.ts` (`LAYOUT_DESK`, from line
// 448): its device ids, its zones and LED counts, and its placements with their slices,
// rotations and shapes. Test 5 asserts against it.

const ROOF_STRIP: &str = "openrgb:strip-1";
const FRONT_HUB: &str = "openrgb:fan-hub-1";
const FLOOR_HUB: &str = "openrgb:fan-hub-2";
const REAR_HUB: &str = "openrgb:fan-hub-3";
const RAM_1: &str = "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x71";
const RAM_2: &str = "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x73";
const RAM_3: &str = "openrgb:I2C: PawnIO SMBus PIIX4 0, address 0x75";
const GPU: &str = "openrgb:gpu-1";
const MOUSE: &str = "openrgb:mouse-1";
const KEYBOARD: &str = "royuan:3151:4015";
const MIC: &str = r"openrgb:HID: \\?\HID#VID_03F0&PID_028C&MI_00#a&eb32b1c&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";

fn desk_devices() -> Vec<DeviceInfo> {
    vec![
        device(ROOF_STRIP, "Arctic Liquid Freezer III", vec![zone(0, "Strip", ZoneKind::Linear, 60)]),
        device(FRONT_HUB, "Lian Li UNI FAN", vec![zone(0, "Front fan", ZoneKind::Linear, 40)]),
        device(FLOOR_HUB, "Lian Li UNI FAN", vec![zone(0, "Floor fan", ZoneKind::Linear, 40)]),
        device(REAR_HUB, "Lian Li UNI FAN", vec![zone(0, "Rear fan", ZoneKind::Linear, 16)]),
        device(RAM_1, "ENE DRAM", vec![zone(0, "DRAM", ZoneKind::Linear, 8)]),
        device(RAM_2, "ENE DRAM", vec![zone(0, "DRAM", ZoneKind::Linear, 8)]),
        device(RAM_3, "ENE DRAM", vec![zone(0, "DRAM", ZoneKind::Linear, 8)]),
        device(GPU, "ASUS TUF Gaming", vec![zone(0, "Graphics card", ZoneKind::Linear, 16)]),
        device(KEYBOARD, "ROYUAN MonsGeek Keyboard", vec![zone(0, "Keyboard", ZoneKind::Single, 1)]),
        device(MOUSE, "Logitech G Pro X Superlight", vec![zone(0, "Mouse", ZoneKind::Single, 1)]),
        device(MIC, "HyperX Quadcast S", vec![zone(0, "Microphone", ZoneKind::Single, 2)]),
    ]
}

fn desk_zones() -> Vec<PlacedZone> {
    let mut zones = Vec::new();
    // The strip along the roof of the case.
    let mut roof = placed(ROOF_STRIP, 0, 106.0, 30.0, 100.0, 5.0);
    roof.shape = ZoneShape::Strip;
    roof.label = Some("Roof strip".into());
    zones.push(roof);
    // The three fans at the front, one header, three runs of twelve LEDs.
    for i in 0..3u32 {
        let mut fan = placed(FRONT_HUB, 0, 103.0, 38.0 + (i as f32) * 32.0, 30.0, 30.0);
        fan.shape = ZoneShape::Ring;
        fan.leds = Some(LedSlice { start: i * 12, count: 12 });
        fan.label = Some(format!("Front fan {}", i + 1));
        zones.push(fan);
    }
    // The three fans on the floor, seen from the side.
    for i in 0..3u32 {
        let mut fan = placed(FLOOR_HUB, 0, 104.0 + (i as f32) * 32.0, 136.0, 30.0, 7.0);
        fan.shape = ZoneShape::Strip;
        fan.leds = Some(LedSlice { start: i * 12, count: 12 });
        fan.label = Some(format!("Floor fan {}", i + 1));
        zones.push(fan);
    }
    // The fan at the back.
    let mut rear = placed(REAR_HUB, 0, 186.0, 38.0, 30.0, 30.0);
    rear.shape = ZoneShape::Ring;
    rear.label = Some("Rear fan".into());
    zones.push(rear);
    // The RAM, the stick that was taken out, and the graphics card.
    for (index, id) in [(0, RAM_1), (1, RAM_2), (2, RAM_3)] {
        let mut ram = placed(id, 0, 140.0 + (index as f32) * 8.0, 42.0, 6.0, 34.0);
        ram.shape = ZoneShape::Strip;
        ram.label = Some(format!("RAM {}", index + 1));
        zones.push(ram);
    }
    let mut gpu = placed(GPU, 0, 140.0, 120.0, 70.0, 14.0);
    gpu.shape = ZoneShape::Strip;
    zones.push(gpu);
    // The keyboard, the mouse and the microphone, on the desk.
    let mut keyboard = placed(KEYBOARD, 0, 16.0, 158.0, 110.0, 30.0);
    keyboard.shape = ZoneShape::Grid { columns: 6 };
    zones.push(keyboard);
    let mut mouse = placed(MOUSE, 0, 136.0, 158.0, 20.0, 30.0);
    mouse.shape = ZoneShape::Point;
    zones.push(mouse);
    let mut mic = placed(MIC, 0, 166.0, 160.0, 16.0, 28.0);
    mic.shape = ZoneShape::Point;
    zones.push(mic);
    zones
}

fn desk_layout(scale: RenderScale, sampling: SamplingMode) -> Layout {
    layout(desk_zones(), scale, sampling)
}

/// The sampled colours of every device, in `devices` order.
fn sampled(layout: &Layout, devices: &[DeviceInfo], canvas: &Canvas) -> Vec<Vec<canvas_proto::Rgb8>> {
    let plan = stack::sample_plan(layout, devices);
    stack::sample_devices(&plan, canvas, layout.sampling)
}

// ================================================================ test 1

#[tokio::test]
async fn stacks_round_trip_and_a_broken_file_never_stops_the_daemon() {
    // A missing file is no stacks.
    let harness = build("t1-missing");
    let body = json(harness.router.clone().oneshot(request("GET", routes::STACKS)).await.unwrap()).await;
    assert_eq!(body["stacks"].as_array().map(Vec::len), Some(0));

    // A saved stack round-trips through the file and through a fresh store.
    let saved = stack("stack:Sunset glow", vec![layer("l1", "gradient")]);
    assert_eq!(put_stack(&harness, &saved).await, StatusCode::OK);
    let on_disk = std::fs::read_to_string(harness.home.path.join(STACKS_FILE)).expect("the file is written");
    assert!(on_disk.contains("Sunset glow"), "{on_disk}");
    let reread = canvas_daemon::storage::Store::load_from(&harness.home.path);
    assert_eq!(reread.home, harness.home.path);
    let fresh = canvas_daemon::stacks::Stacks::new(&harness.home.path);
    fresh.reload(&harness.app.user_effects);
    assert_eq!(fresh.all(), vec![saved.clone()]);

    // A corrupt file is quarantined and the daemon starts with none.
    let corrupt = TempHome::new("t1-corrupt");
    std::fs::write(corrupt.path.join(STACKS_FILE), b"{ not json {{").unwrap();
    let harness = harness_at(corrupt);
    let response = harness.router.clone().oneshot(request("GET", routes::STATUS)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "the daemon still answers");
    assert!(!harness.home.path.join(STACKS_FILE).exists(), "the corrupt file is moved aside");
    let body = json(harness.router.clone().oneshot(request("GET", routes::STACKS)).await.unwrap()).await;
    assert_eq!(body["stacks"].as_array().map(Vec::len), Some(0));

    // A hand-written file with a NaN rectangle, an opacity over 100 and a negative width loads
    // with those layers neutralised, and a frame over it does not panic.
    let hand = TempHome::new("t1-hand");
    std::fs::write(
        hand.path.join(STACKS_FILE),
        r#"{"schema_version":1,"stacks":[
            {"id":"stack:Hand","layers":[
              {"id":"l1","name":"Opaque","effect":"solid","opacity":255,
               "area":{"kind":"rect","x":0.0,"y":0.0,"width":10.0,"height":10.0}},
              {"id":"l2","name":"Infinite rect","effect":"solid",
               "area":{"kind":"rect","x":1e39,"y":0.0,"width":10.0,"height":10.0}},
              {"id":"l3","name":"Negative","effect":"solid",
               "area":{"kind":"rect","x":0.0,"y":0.0,"width":-4.0,"height":10.0}}]}]}"#,
    )
    .unwrap();
    let harness = harness_at(hand);
    let loaded = harness.app.stacks.get("stack:Hand").expect("the hand-written stack loads");
    assert_eq!(loaded.layers.len(), 3, "its layers are kept, not dropped");
    // A frame over it: the NaN rectangle is no area, so the layer paints nothing.
    let updates = Arc::new(AtomicU64::new(0));
    let builds = Arc::new(AtomicUsize::new(0));
    let clock = ManualClock::new();
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    let mut effect = StackEffect::new(
        &loaded,
        paint_leaves(updates.clone(), builds.clone(), [1.0, 0.0, 0.0], None),
        live_for(&loaded),
        areas_for(&lay, &devices),
    )
    .with_clock(clock.clock());
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(1.0);
    effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
    // Layer l1 paints (opacity clamped to 100), l2 has no area, l3 has no area.
    assert_eq!(updates.load(Ordering::SeqCst), 1, "only the layer with an area runs");
    assert_eq!(canvas.get(5, 5), [1.0, 0.0, 0.0]);
}

// ================================================================ test 2

/// A stack body as JSON, so a test can write a value serde would refuse (an opacity of -1).
fn stack_json(id: &str, layers: serde_json::Value) -> String {
    serde_json::json!({ "id": id, "layers": layers }).to_string()
}

fn layer_json(id: &str, effect: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "name": format!("Layer {id}"), "effect": effect })
}

/// A PUT of a raw body, so a test can send what serde_json would not write (a NaN).
async fn put_stack_body(harness: &Harness, id: &str, body: &str) -> (StatusCode, String) {
    let response = harness
        .router
        .clone()
        .oneshot(request_with_body("PUT", &stack_path(id), body))
        .await
        .expect("the router answers");
    let status = response.status();
    let value = json(response).await;
    (status, value["message"].as_str().unwrap_or_default().to_string())
}

#[tokio::test]
async fn names_ids_caps_and_ranges_are_refused() {
    let harness = build("t2");
    // A user effect, so its display name is taken.
    std::fs::write(harness.folder.join("spark.html"), VALID_HTML).unwrap();
    harness.app.user_effects.rescan(&harness.folder);

    assert_eq!(put_stack(&harness, &stack("stack:Sunset glow", vec![layer("l1", "solid")])).await, StatusCode::OK);

    // A duplicate name, ignoring case, is a conflict.
    let duplicate = harness
        .router
        .clone()
        .oneshot(request_with_body(
            "PUT",
            &stack_path("stack:Sunset Glow"),
            &stack_json("stack:Sunset Glow", serde_json::json!([layer_json("l1", "solid")])),
        ))
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    for name in [
        "Fire",  // a built-in's display name
        "Spark", // a user effect's display name
        "Hello", // a hidden onboarding effect's display name
        "Sweep",
        "onboard-hello",
        "onboard-sweep",
        "con", // a name Windows reserves
        "nul.a",
        "a:b",       // a character a name may not hold
        "a\u{1}b",   // a control character
        &"a".repeat(61),
        "trailing ",
        "trailingdot.",
    ] {
        let id = format!("stack:{name}");
        let (status, message) = put_stack_body(
            &harness,
            &id,
            &stack_json(&id, serde_json::json!([layer_json("l1", "solid")])),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{name:?}: {message}");
    }

    // A layer id with a dot is refused (the merged key is `{id}.{key}`).
    let (status, _) = put_stack_body(
        &harness,
        "stack:Dots",
        &stack_json("stack:Dots", serde_json::json!([layer_json("l1.1", "solid")])),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // A ninth layer and a third HTML layer are refused, naming the limit.
    let nine: Vec<StackLayer> = (1..=9).map(|i| layer(&format!("l{i}"), "solid")).collect();
    let (status, message) = put_stack_body(
        &harness,
        "stack:Nine",
        &serde_json::to_string(&stack("stack:Nine", nine)).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(message.contains("at most 8 layers"), "{message}");

    let html: Vec<StackLayer> = (1..=3).map(|i| layer(&format!("l{i}"), "html:ink-drop")).collect();
    let (status, message) = put_stack_body(
        &harness,
        "stack:Html",
        &serde_json::to_string(&stack("stack:Html", html)).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(message.contains("at most 2 HTML layers"), "{message}");

    // A stack layer is refused.
    let (status, message) = put_stack_body(
        &harness,
        "stack:Nested",
        &stack_json("stack:Nested", serde_json::json!([layer_json("l1", "stack:Other")])),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(message.contains("cannot contain a stack"), "{message}");

    // Opacity 101 and -1, a NaN rectangle and a 400-wide one.
    let (status, message) = put_stack_body(
        &harness,
        "stack:Opacity",
        r#"{"id":"stack:Opacity","layers":[{"id":"l1","name":"One","effect":"solid","opacity":101}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(message.contains("0..=100"), "the range is named: {message}");

    let (status, _) = put_stack_body(
        &harness,
        "stack:Negative",
        r#"{"id":"stack:Negative","layers":[{"id":"l1","name":"One","effect":"solid","opacity":-1}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "opacity -1 is refused");

    // JSON has no `NaN` token, so a client sends the overflow: it parses to an `f32` infinity,
    // and the rectangle rule refuses it. A body spelling `NaN` is refused by the parser (422).
    let (status, message) = put_stack_body(
        &harness,
        "stack:Rect",
        r#"{"id":"stack:Rect","layers":[{"id":"l1","name":"One","effect":"solid","area":{"kind":"rect","x":1e39,"y":0.0,"width":10.0,"height":10.0}}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "a non-finite rectangle is refused: {message}");
    assert!(message.contains("finite"), "the rule is named: {message}");
    let (status, _) = put_stack_body(
        &harness,
        "stack:Rect",
        r#"{"id":"stack:Rect","layers":[{"id":"l1","name":"One","effect":"solid","area":{"kind":"rect","x":NaN,"y":0.0,"width":10.0,"height":10.0}}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "a body spelling a NaN is refused");

    let (status, message) = put_stack_body(
        &harness,
        "stack:Rect",
        r#"{"id":"stack:Rect","layers":[{"id":"l1","name":"One","effect":"solid","area":{"kind":"rect","x":0.0,"y":0.0,"width":400.0,"height":10.0}}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "a 400-wide rectangle is refused: {message}");
}

// ================================================================ test 3

/// The blend table of D95.3.3 lives in `stack::blend_pixel` and is tested there, against the
/// table itself (every mode at 0, 50 and 100 %, Add at 0.5/0.8/0.5 = 0.75, and every mode at
/// s = 0). This is the same table, reached through a frame of 1 x 1 pixels: the compositor's own
/// per-pixel call is the function under test, so a mistake in the frame's wiring shows here.
#[test]
fn the_blend_maths_is_the_specs_table() {
    // Every mode at the three opacities, on a mid-grey under an 80 % part.
    for (mode, want_one) in [
        (BlendMode::Normal, [0.8, 0.8, 0.8]),
        (BlendMode::Add, [1.0, 1.0, 1.0]),
        (BlendMode::Multiply, [0.4, 0.4, 0.4]),
        (BlendMode::Screen, [0.9, 0.9, 0.9]),
    ] {
        assert_eq!(blend_pixel(mode, [0.5; 3], [0.8; 3], 0.0), [0.5; 3], "{mode:?} at 0 %");
        assert_eq!(blend_pixel(mode, [0.5; 3], [0.8; 3], 1.0), want_one, "{mode:?} at 100 %");
        let half = blend_pixel(mode, [0.5; 3], [0.8; 3], 0.5);
        for (got, one) in half.iter().zip(want_one) {
            assert!((got - (0.5 + 0.5 * (one - 0.5))).abs() < 1e-6, "{mode:?} at 50 %");
        }
    }
    // Add at d = 0.5, s = 0.8, a = 0.5 is 0.75: clamped before the opacity, not after.
    assert!((blend_pixel(BlendMode::Add, [0.5; 3], [0.8; 3], 0.5)[0] - 0.75).abs() < 1e-6);
    // Every mode with a black part of the layer, at full and at half opacity.
    for (mode, at_one, at_half) in [
        (BlendMode::Normal, 0.0, 0.25),
        (BlendMode::Add, 0.5, 0.5),
        (BlendMode::Multiply, 0.0, 0.25),
        (BlendMode::Screen, 0.5, 0.5),
    ] {
        assert!((blend_pixel(mode, [0.5; 3], [0.0; 3], 1.0)[0] - at_one).abs() < 1e-6, "{mode:?} s=0 a=1");
        assert!((blend_pixel(mode, [0.5; 3], [0.0; 3], 0.5)[0] - at_half).abs() < 1e-6, "{mode:?} s=0 a=0.5");
    }
}

// ================================================================ test 4

#[test]
fn one_layer_at_normal_and_full_opacity_is_that_effect_alone() {
    let props = {
        let effect = canvas_effects::create("gradient").expect("the built-in exists");
        effect.schema().defaults()
    };
    let stack = stack("stack:Alone", vec![layer("l1", "gradient")]);
    let clock = ManualClock::new();
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 8)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 320.0, 200.0)], RenderScale::P100, SamplingMode::Nearest);
    let areas = areas_for(&lay, &devices);
    let live = live_for(&stack);

    // The real effect, run alone.
    let mut alone = canvas_effects::create("gradient").expect("the built-in exists");
    alone.init(&props);
    let mut alone_canvas = Canvas::with_scale(1.0);
    // The stack, with the real effect as its one layer.
    let leaves: LeafBuilder = Arc::new(|id: &str| {
        canvas_effects::create(id).ok_or_else(|| format!("no effect with the id {id}"))
    });
    let mut stacked = StackEffect::new(&stack, leaves, live, areas).with_clock(clock.clock());
    stacked.init(&props);
    let mut stacked_canvas = Canvas::with_scale(1.0);

    let inputs = Inputs::default();
    for _ in 0..10 {
        alone.update(1.0 / 30.0, &mut alone_canvas, &inputs);
        stacked.update(1.0 / 30.0, &mut stacked_canvas, &inputs);
    }
    assert_eq!(alone_canvas.pixels, stacked_canvas.pixels, "a one-layer stack at Normal 100 % is the effect alone");
}

// ================================================================ test 5

/// A stack with one layer painting red over `area`, composited at `scale`/`sampling`.
fn paint_area(area: LayerArea, layout: &Layout, devices: &[DeviceInfo]) -> Canvas {
    let stack = stack("stack:Area", vec![with_area(layer("l1", "paint"), area)]);
    let updates = Arc::new(AtomicU64::new(0));
    let builds = Arc::new(AtomicUsize::new(0));
    let colour = [1.0, 0.0, 0.0];
    let mut effect = StackEffect::new(
        &stack,
        paint_leaves(updates, builds, colour, None),
        live_for(&stack),
        areas_for(layout, devices),
    )
    .with_clock(ManualClock::new().clock());
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(layout.render_scale.factor());
    effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
    canvas
}

#[test]
fn areas_paint_only_their_own_pixels_and_their_own_leds() {
    let devices = desk_devices();
    let desk = desk_layout(RenderScale::P50, SamplingMode::AreaAverage);

    // A 40 x 20 rectangle at the origin changes only those pixels.
    let rect = paint_area(LayerArea::Rect { x: 0.0, y: 0.0, width: 40.0, height: 20.0 }, &desk, &devices);
    assert_eq!((rect.width, rect.height), (160, 100), "the desk renders at 50 %");
    for y in 0..100 {
        for x in 0..160 {
            let want = if x < 20 && y < 10 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 0.0] };
            assert_eq!(rect.get(x, y), want, "pixel {x},{y} of the rectangle");
        }
    }

    // The keyboard's device area: only its own LEDs read the layer, and every other device's
    // LEDs sample exactly what they did before it was there.
    for (scale, sampling) in [
        (RenderScale::P50, SamplingMode::AreaAverage),
        (RenderScale::P100, SamplingMode::Nearest),
    ] {
        let lay = desk_layout(scale, sampling);
        let black = Canvas::with_scale(scale.factor());
        let before = sampled(&lay, &devices, &black);
        let painted = paint_area(LayerArea::Device { device: DeviceId(KEYBOARD.into()) }, &lay, &devices);
        let after = sampled(&lay, &devices, &painted);
        let keyboard = devices.iter().position(|d| d.id.0 == KEYBOARD).expect("the keyboard is on the desk");
        for (index, device) in devices.iter().enumerate() {
            if index == keyboard {
                assert!(
                    after[index].iter().all(|c| *c == canvas_proto::Rgb8::new(255, 0, 0)),
                    "the keyboard's own LED reads the layer at {scale:?}/{sampling:?}"
                );
            } else {
                assert_eq!(
                    after[index], before[index],
                    "{} sampled differently at {scale:?}/{sampling:?}",
                    device.id.0
                );
            }
        }
    }

    // A rotated placement: one front fan turned 30 degrees still reads the layer on every LED.
    let mut zones = desk_zones();
    for zone in &mut zones {
        if zone.device_id.0 == FRONT_HUB && zone.leds == Some(LedSlice { start: 0, count: 12 }) {
            zone.rotation_deg = 30.0;
        }
    }
    let turned = layout(zones, RenderScale::P100, SamplingMode::AreaAverage);
    let painted = paint_area(LayerArea::Device { device: DeviceId(FRONT_HUB.into()) }, &turned, &devices);
    let after = sampled(&turned, &devices, &painted);
    let front = devices.iter().position(|d| d.id.0 == FRONT_HUB).expect("the fan hub is on the desk");
    assert_eq!(after[front].len(), 40);
    // The header has forty LEDs and the three fans claim twelve each: the last four belong to
    // nobody and stay black, so a rotated fan is checked on the LEDs its placements own.
    assert!(
        after[front][..36].iter().all(|c| *c == canvas_proto::Rgb8::new(255, 0, 0)),
        "a rotated fan's own LEDs read the layer: {:?}",
        &after[front][..36]
    );
    assert!(
        after[front][36..].iter().all(|c| *c == canvas_proto::Rgb8::BLACK),
        "the four LEDs no placement owns stay black"
    );
}

// ================================================================ test 6

#[test]
fn a_moved_part_paints_at_its_new_place_without_a_rebuild() {
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let first = layout(vec![placed("d", 0, 0.0, 0.0, 40.0, 20.0)], RenderScale::P100, SamplingMode::AreaAverage);
    let stack = stack(
        "stack:Moves",
        vec![with_area(layer("l1", "paint"), LayerArea::Device { device: DeviceId("d".into()) })],
    );
    let builds = Arc::new(AtomicUsize::new(0));
    let clock = ManualClock::new();
    let areas = Arc::new(std::sync::RwLock::new(AreaMap::build(&first, &devices, 1)));
    let mut effect = StackEffect::new(
        &stack,
        paint_leaves(Arc::new(AtomicU64::new(0)), builds.clone(), [1.0, 0.0, 0.0], None),
        live_for(&stack),
        Arc::clone(&areas),
    )
    .with_clock(clock.clock());
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(1.0);
    effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
    let at_origin = canvas.get(5, 5);
    assert_eq!(at_origin, [1.0, 0.0, 0.0], "the part is at the origin, and the pixel is {at_origin:?}");

    // The part moves; the area book is republished, and the next frame paints there.
    let moved = layout(vec![placed("d", 0, 200.0, 100.0, 40.0, 20.0)], RenderScale::P100, SamplingMode::AreaAverage);
    {
        let mut book = canvas_daemon::stack::write_recovering(&areas);
        *book = AreaMap::build(&moved, &devices, 2);
    }
    effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
    assert_eq!(canvas.get(5, 5), [0.0, 0.0, 0.0], "the old place is black");
    assert_eq!(canvas.get(205, 105), [1.0, 0.0, 0.0], "the new place is painted");
    assert_eq!(builds.load(Ordering::SeqCst), 1, "one layer built, and nothing rebuilt");
}

// ================================================================ test 7

#[tokio::test]
async fn a_missing_effect_a_removed_device_and_an_unplaced_zone_leave_the_rest_running() {
    let harness = build("t7");
    let devices = vec![device("d", "Fan hub", vec![
        zone(0, "Front fan", ZoneKind::Linear, 12),
        zone(1, "Rear fan", ZoneKind::Linear, 12),
    ])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    harness.app.stacks.publish_areas(&lay, &devices);

    let saved = stack(
        "stack:Holes",
        vec![
            layer("l1", "solid"),
            layer("l2", "no-such-effect"),
            with_area(layer("l3", "solid"), LayerArea::Device { device: DeviceId("gone".into()) }),
            with_area(layer("l4", "solid"), LayerArea::Zone { device: DeviceId("d".into()), zone: 1 }),
        ],
    );
    assert_eq!(put_stack(&harness, &saved).await, StatusCode::OK);

    let detail = json(
        harness
            .router
            .clone()
            .oneshot(request("GET", &stack_path("stack:Holes")))
            .await
            .unwrap(),
    )
    .await;
    let statuses = detail["layers"].as_array().expect("the statuses are listed");
    let status_of = |id: &str| {
        statuses
            .iter()
            .find(|status| status["id"] == id)
            .map(|status| status["status"].as_str().unwrap_or_default().to_string())
            .unwrap_or_default()
    };
    assert_eq!(status_of("l1"), "ok");
    assert_eq!(status_of("l2"), "missing_effect");
    assert_eq!(status_of("l3"), "no_area", "a device that is gone");
    assert_eq!(status_of("l4"), "no_area", "a zone that is not placed");

    // The rest of the stack runs: the base layer paints, the three others are skipped.
    let updates = Arc::new(AtomicU64::new(0));
    let builds = Arc::new(AtomicUsize::new(0));
    let mut effect = StackEffect::new(
        &saved,
        paint_leaves(updates.clone(), builds.clone(), [0.0, 0.0, 1.0], None),
        live_for(&saved),
        areas_for(&lay, &devices),
    )
    .with_clock(ManualClock::new().clock());
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(1.0);
    effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
    assert_eq!(canvas.get(160, 100), [0.0, 0.0, 1.0], "the base layer painted");
}

// ================================================================ test 8

#[tokio::test]
async fn a_stack_is_listed_activated_and_tiled_with_the_seeded_builders() {
    let tile_calls = Arc::new(AtomicUsize::new(0));
    let real = canvas_daemon::user_effects::html_builder();
    let counter = Arc::clone(&tile_calls);
    let tile_builder: canvas_daemon::user_effects::Builder = Arc::new(move |id: &str, source: &str| {
        counter.fetch_add(1, Ordering::SeqCst);
        real(id, source)
    });
    let user = Arc::new(canvas_daemon::user_effects::UserEffects::with_stock_and_tiles(
        canvas_daemon::user_effects::html_builder(),
        tile_builder,
        &[],
    ));
    let harness = build_with("t8", Some(user));
    std::fs::write(harness.folder.join("spark.html"), VALID_HTML).unwrap();
    harness.app.user_effects.rescan(&harness.folder);

    let saved = stack("stack:Glow", vec![layer("l1", "html:spark")]);
    assert_eq!(put_stack(&harness, &saved).await, StatusCode::OK);

    // It is in the effect list, with a generated description and its own needs.
    let effects = json(harness.router.clone().oneshot(request("GET", routes::EFFECTS)).await.unwrap()).await;
    let listed = effects["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|effect| effect["id"] == "stack:Glow")
        .expect("the stack is in the library");
    assert_eq!(listed["name"], "Glow");
    assert_eq!(listed["description"], "1 layer: Spark");

    // It activates through the normal route.
    let response = harness
        .router
        .clone()
        .oneshot(request_with_body(
            "POST",
            "/api/v1/effects/stack:Glow/activate",
            &serde_json::json!({ "props": {}, "preset": null }).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(harness.engine.effect_id.lock().unwrap().clone(), Some("stack:Glow".to_string()));

    // Its tile is rendered with the seeded (tile) builders, and twice is the same bytes.
    let first = harness
        .router
        .clone()
        .oneshot(request("GET", "/api/v1/effects/stack:Glow/thumbnail"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first_bytes = axum::body::to_bytes(first.into_body(), 8 * 1024 * 1024).await.unwrap();
    assert_eq!(tile_calls.load(Ordering::SeqCst), 1, "the tile builder built the layer");
    let second = harness
        .router
        .clone()
        .oneshot(request("GET", "/api/v1/effects/stack:Glow/thumbnail"))
        .await
        .unwrap();
    let second_bytes = axum::body::to_bytes(second.into_body(), 8 * 1024 * 1024).await.unwrap();
    assert_eq!(first_bytes, second_bytes, "the same stack renders the same tile");
    assert_eq!(tile_calls.load(Ordering::SeqCst), 1, "the second answer came from the cache");

    // Editing a layer moves the tiles cache key with the stack, so the tile changes.
    let mut edited = saved.clone();
    edited.layers[0].opacity = 40;
    assert_eq!(put_stack(&harness, &edited).await, StatusCode::OK);
    let third = harness
        .router
        .clone()
        .oneshot(request("GET", "/api/v1/effects/stack:Glow/thumbnail"))
        .await
        .unwrap();
    let third_status = third.status();
    let third_bytes = axum::body::to_bytes(third.into_body(), 8 * 1024 * 1024).await.unwrap();
    let calls = tile_calls.load(Ordering::SeqCst);
    let same = first_bytes == third_bytes;
    assert!(
        calls == 2,
        "the edited stack is rendered again: calls {calls}, status {third_status}, stacks generation {}, user generation {}, first {} bytes, third {} bytes, same {same}",
        harness.app.stacks.generation(),
        harness.app.user_effects.generation(),
        first_bytes.len(),
        third_bytes.len()
    );
    assert!(!same, "an edited layer changes the tile");
}

// ================================================================ test 9

#[tokio::test]
async fn the_props_guard_and_the_two_delete_routes() {
    let harness = build("t9");
    let saved = stack("stack:Guarded", vec![layer("l1", "solid")]);
    assert_eq!(put_stack(&harness, &saved).await, StatusCode::OK);
    // A preset and a favourite, both keyed by the stack's id.
    let response = harness
        .router
        .clone()
        .oneshot(request_with_body(
            "POST",
            "/api/v1/effects/stack:Guarded/presets",
            &serde_json::json!({ "name": "Warm", "props": {} }).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    harness
        .router
        .clone()
        .oneshot(request_with_body(
            "PUT",
            "/api/v1/effects/stack:Guarded/favorite",
            &serde_json::json!({ "favorite": true }).to_string(),
        ))
        .await
        .unwrap();

    // Activate it, so the active layout names it.
    harness
        .router
        .clone()
        .oneshot(request_with_body(
            "POST",
            "/api/v1/effects/stack:Guarded/activate",
            &serde_json::json!({ "props": {}, "preset": null }).to_string(),
        ))
        .await
        .unwrap();

    // The props route refuses a stack and names the builder.
    let guarded = harness
        .router
        .clone()
        .oneshot(request_with_body("PATCH", routes::EFFECT_ACTIVE_PROPS, "{}"))
        .await
        .unwrap();
    assert_eq!(guarded.status(), StatusCode::CONFLICT);
    let message = json(guarded).await["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("builder"), "{message}");

    // The effect delete route names the stack route.
    let wrong_route = harness
        .router
        .clone()
        .oneshot(request("DELETE", "/api/v1/effects/stack:Guarded"))
        .await
        .unwrap();
    assert_eq!(wrong_route.status(), StatusCode::CONFLICT);
    let message = json(wrong_route).await["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("/api/v1/stacks/"), "{message}");

    // Deleting the running stack is a conflict unless forced.
    let in_use = harness
        .router
        .clone()
        .oneshot(request("DELETE", &stack_path("stack:Guarded")))
        .await
        .unwrap();
    assert_eq!(in_use.status(), StatusCode::CONFLICT);
    let message = json(in_use).await["message"].as_str().unwrap_or_default().to_string();
    assert!(message.contains("Default"), "the active layout is named: {message}");

    let forced = harness
        .router
        .clone()
        .oneshot(request("DELETE", &format!("{}?force=true", stack_path("stack:Guarded"))))
        .await
        .unwrap();
    assert_eq!(forced.status(), StatusCode::OK);
    let answer = json(forced).await;
    assert_eq!(answer["presets_removed"], 1);
    assert_eq!(answer["favorite_removed"], true);
    assert!(answer["note"].as_str().is_some(), "the lights went black and it says so");
    assert!(harness.app.stacks.get("stack:Guarded").is_none());
    let store = harness.app.store.read().await;
    assert!(!store.effects.presets.contains_key("stack:Guarded"));
    assert!(!store.effects.favorites.iter().any(|f| f == "stack:Guarded"));
    assert_eq!(harness.engine.effect_id.lock().unwrap().clone(), Some("solid".to_string()));
}

// ================================================================ test 12

#[tokio::test]
async fn a_layout_switch_during_a_draft_whose_effect_fails_puts_the_prior_back() {
    let harness = build("t12");
    // The active layout runs Solid; another layout names an effect that does not exist.
    {
        let mut store = harness.app.store.write().await;
        let active = store.layouts.active.clone();
        if let Some(layout) = store.layouts.layouts.iter_mut().find(|l| l.id == active) {
            layout.effect = Some(canvas_proto::layout::ActiveEffect {
                id: "solid".into(),
                props: Props::new(),
            });
        }
        store.layouts.layouts.push(Layout {
            id: "broken".into(),
            name: "Broken".into(),
            render_scale: RenderScale::P50,
            sampling: SamplingMode::AreaAverage,
            zones: Vec::new(),
            effect: Some(canvas_proto::layout::ActiveEffect {
                id: "no-such-effect".into(),
                props: Props::new(),
            }),
            case_form: canvas_proto::layout::CaseForm::None,
            landmarks: Vec::new(),
        });
        store.save_layouts().unwrap();
    }
    // Put Solid on the lights, as a start would.
    harness
        .router
        .clone()
        .oneshot(request_with_body(
            "POST",
            "/api/v1/effects/solid/activate",
            &serde_json::json!({ "props": {}, "preset": null }).to_string(),
        ))
        .await
        .unwrap();

    let log = common::LogBuf::default();
    {
        let _guard = log.install();
        // A draft takes the lights.
        let draft = harness
            .router
            .clone()
            .oneshot(request_with_body(
                "PUT",
                routes::EFFECT_DRAFT,
                &serde_json::json!({ "html": VALID_HTML, "props": {} }).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(draft.status(), StatusCode::OK);
        assert!(harness.app.draft.is_active());

        // The switch: the draft ends, its new effect fails, and the prior effect goes back.
        let switched = harness
            .router
            .clone()
            .oneshot(request_with_body(
                "PUT",
                routes::LAYOUT_ACTIVE,
                &serde_json::json!({ "id": "broken" }).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(switched.status(), StatusCode::OK);
    }
    assert!(!harness.app.draft.is_active(), "no draft is left in the slot");
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("solid".to_string()),
        "the draft's prior effect went back on the lights"
    );
    assert!(
        log.text().contains("prior effect went back"),
        "the log says what happened: {}",
        log.text()
    );
}

// ================================================================ test 13

#[tokio::test]
async fn the_export_carries_the_stacks_and_a_bundle_without_them_imports() {
    let harness = build("t13");
    let saved = stack("stack:Exported", vec![layer("l1", "solid")]);
    assert_eq!(put_stack(&harness, &saved).await, StatusCode::OK);

    let bundle = json(harness.router.clone().oneshot(request("GET", routes::CONFIG_EXPORT)).await.unwrap()).await;
    assert_eq!(bundle["stacks"].as_array().map(Vec::len), Some(1), "the export carries the stacks");

    // A bundle without the field imports, and replaces the saved stacks with none.
    let mut old = bundle.clone();
    old.as_object_mut().unwrap().remove("stacks");
    let imported = harness
        .router
        .clone()
        .oneshot(request_with_body("POST", routes::CONFIG_IMPORT, &old.to_string()))
        .await
        .unwrap();
    assert_eq!(imported.status(), StatusCode::OK, "a bundle without stacks imports");
    assert!(harness.app.stacks.get("stack:Exported").is_none());

    // A bundle whose stack names an HTML layer the machine does not have reports it Missing.
    let mut with_html = old.clone();
    with_html["stacks"] = serde_json::json!([
        { "id": "stack:Gone", "layers": [
            { "id": "l1", "name": "One", "effect": "html:not-here" }
        ] }
    ]);
    let imported = harness
        .router
        .clone()
        .oneshot(request_with_body("POST", routes::CONFIG_IMPORT, &with_html.to_string()))
        .await
        .unwrap();
    assert_eq!(imported.status(), StatusCode::OK);
    let detail = json(
        harness
            .router
            .clone()
            .oneshot(request("GET", &stack_path("stack:Gone")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(detail["layers"][0]["status"], "missing_effect", "{detail}");
    let list = json(harness.router.clone().oneshot(request("GET", routes::STACKS)).await.unwrap()).await;
    assert_eq!(list["stacks"][0]["missing"], serde_json::json!(["l1"]));
    assert_eq!(list["stacks"][0]["html_layers"], 1);
}

// ================================================================ test 14

/// The compositor's frame cost, measured rather than asserted (D95.6 test 14). Run it by hand:
/// `cargo test -p canvas-daemon --test stacks -- --ignored --nocapture`.
#[test]
#[ignore]
fn frame_cost_by_layer_count_and_scale() {
    let devices = desk_devices();
    for layers in [1usize, 3, 8] {
        for scale in [RenderScale::P25, RenderScale::P50, RenderScale::P100] {
            let lay = desk_layout(scale, SamplingMode::AreaAverage);
            let built: Vec<StackLayer> = (1..=layers).map(|i| layer(&format!("l{i}"), "aurora")).collect();
            let stack = stack("stack:Cost", built);
            let leaves: LeafBuilder = Arc::new(|id: &str| {
                canvas_effects::create(id).ok_or_else(|| format!("no effect with the id {id}"))
            });
            let mut effect = StackEffect::new(
                &stack,
                leaves,
                live_for(&stack),
                areas_for(&lay, &devices),
            )
            .with_clock(ManualClock::new().clock());
            effect.init(&Props::new());
            let mut canvas = Canvas::with_scale(scale.factor());
            let inputs = Inputs::default();
            // Thirty frames of warm-up.
            for _ in 0..30 {
                effect.update(1.0 / 30.0, &mut canvas, &inputs);
            }
            let started = Instant::now();
            for _ in 0..60 {
                effect.update(1.0 / 30.0, &mut canvas, &inputs);
            }
            let each = started.elapsed().as_secs_f64() * 1000.0 / 60.0;
            println!(
                "{layers} layers at {}: {each:.3} ms/frame",
                scale.percent()
            );
        }
    }

    // The HTML layers, which are the slow kind: one layer each, at 100 % and at 50 %.
    let user = Arc::new(canvas_daemon::user_effects::UserEffects::with_stock(
        canvas_daemon::user_effects::html_builder(),
        canvas_daemon::user_effects::STOCK_EFFECTS,
    ));
    let leaves: LeafBuilder = {
        let user = Arc::clone(&user);
        Arc::new(move |id: &str| {
            user.create(id)
                .unwrap_or_else(|| canvas_effects::create(id).map(Ok).unwrap_or_else(|| Err(format!("no effect {id}"))))
        })
    };
    for (stem, _) in canvas_daemon::user_effects::STOCK_EFFECTS {
        for scale in [RenderScale::P50, RenderScale::P100] {
            let lay = desk_layout(scale, SamplingMode::AreaAverage);
            let stack = stack("stack:Html", vec![layer("l1", &format!("html:{stem}"))]);
            let mut effect = StackEffect::new(&stack, Arc::clone(&leaves), live_for(&stack), areas_for(&lay, &devices))
                .with_clock(ManualClock::new().clock())
                .with_fps(Arc::new(std::sync::atomic::AtomicU32::new(30)));
            effect.init(&Props::new());
            let mut canvas = Canvas::with_scale(scale.factor());
            let inputs = Inputs::default();
            for _ in 0..20 {
                effect.update(1.0 / 30.0, &mut canvas, &inputs);
            }
            let started = Instant::now();
            for _ in 0..180 {
                effect.update(1.0 / 30.0, &mut canvas, &inputs);
            }
            let each = started.elapsed().as_secs_f64() * 1000.0 / 180.0;
            println!("html {stem} at {}: {each:.3} ms/frame", scale.percent());
        }
    }
}

/// The compositor's own buffers: eight layers over sixty frames, with the layer canvases never
/// reallocated after the first frame.
#[test]
fn the_compositor_does_not_allocate_per_frame() {
    let devices = desk_devices();
    let lay = desk_layout(RenderScale::P50, SamplingMode::AreaAverage);
    let layers: Vec<StackLayer> = (1..=8).map(|i| layer(&format!("l{i}"), "solid")).collect();
    let stack = stack("stack:Budget", layers);
    let leaves: LeafBuilder = Arc::new(|id: &str| {
        canvas_effects::create(id).ok_or_else(|| format!("no effect with the id {id}"))
    });
    let mut effect = StackEffect::new(&stack, leaves, live_for(&stack), areas_for(&lay, &devices))
        .with_clock(ManualClock::new().clock());
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(0.5);
    let inputs = Inputs::default();
    effect.update(1.0 / 30.0, &mut canvas, &inputs);
    let after_first = effect.layer_canvas_capacities();
    assert_eq!(after_first.len(), 8);
    for _ in 0..60 {
        effect.update(1.0 / 30.0, &mut canvas, &inputs);
    }
    assert_eq!(
        effect.layer_canvas_capacities(),
        after_first,
        "a layer's canvas was reallocated after the first frame"
    );
}

/// One frame of a directly-built compositor, at the daemon's default step.
fn step(effect: &mut StackEffect, canvas: &mut Canvas) {
    effect.update(1.0 / 30.0, canvas, &Inputs::default());
}

/// The hold: 250 ms, 500 ms, 1 s, then Faulted, with the last picture composited throughout.
#[test]
fn a_slow_layer_is_held_for_growing_periods_then_faulted() {
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    let stack = stack("stack:Slow", vec![layer("l1", "html:slow")]);
    let updates = Arc::new(AtomicU64::new(0));
    let clock = ManualClock::new();
    let mut effect = StackEffect::new(
        &stack,
        paint_leaves(
            updates.clone(),
            Arc::new(AtomicUsize::new(0)),
            [0.0, 1.0, 0.0],
            Some((clock.clone(), 20)),
        ),
        live_for(&stack),
        areas_for(&lay, &devices),
    )
    .with_clock(clock.clock())
    .with_fps(Arc::new(std::sync::atomic::AtomicU32::new(30)));
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(1.0);

    // The first overrun holds the layer for 250 ms; it keeps its last picture while held.
    step(&mut effect, &mut canvas);
    assert_eq!(updates.load(Ordering::SeqCst), 1);
    assert_eq!(effect.layer_status("l1").map(|(state, _)| state), Some(StackLayerState::Held));
    clock.advance_ms(100);
    step(&mut effect, &mut canvas);
    assert_eq!(updates.load(Ordering::SeqCst), 1, "a held layer is not updated");
    assert_eq!(canvas.get(0, 0), [0.0, 1.0, 0.0], "a held layer keeps its last picture");

    // The trial after the hold overruns: a 500 ms hold from where that trial started.
    clock.advance_ms(151);
    step(&mut effect, &mut canvas);
    assert_eq!(updates.load(Ordering::SeqCst), 2, "the trial frame runs");
    clock.advance_ms(400);
    step(&mut effect, &mut canvas);
    assert_eq!(updates.load(Ordering::SeqCst), 2, "the second hold is 500 ms");
    // and once 500 ms have passed since the trial, it runs again: a 1 s hold.
    clock.advance_ms(100);
    step(&mut effect, &mut canvas);
    assert_eq!(updates.load(Ordering::SeqCst), 3);
    clock.advance_ms(900);
    step(&mut effect, &mut canvas);
    assert_eq!(updates.load(Ordering::SeqCst), 3, "the third hold is 1 s");
    clock.advance_ms(100);
    step(&mut effect, &mut canvas);
    assert_eq!(updates.load(Ordering::SeqCst), 4, "the trial after the third hold runs");
    assert_eq!(
        effect.layer_status("l1").map(|(state, _)| state),
        Some(StackLayerState::Faulted),
        "four overrunning updates in all, then the layer is faulted"
    );

    // Faulted: never updated again, still composited.
    for _ in 0..3 {
        clock.advance_ms(5000);
        step(&mut effect, &mut canvas);
    }
    assert_eq!(updates.load(Ordering::SeqCst), 4, "a faulted layer is never updated again");
    assert_eq!(canvas.get(0, 0), [0.0, 1.0, 0.0], "a faulted layer keeps its last picture");
}

/// A trial frame that fits resets the escalation to 250 ms.
#[test]
fn a_trial_frame_that_fits_resets_the_escalation() {
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    let stack = stack("stack:Resets", vec![layer("l1", "html:slow")]);
    let updates = Arc::new(AtomicU64::new(0));
    let clock = ManualClock::new();
    // A switch the test flips: the fake overruns while it is set.
    let slow = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let slow_in_layer = Arc::clone(&slow);
    let clock_in_layer = clock.clone();
    let updates_in_layer = Arc::clone(&updates);
    let leaves: LeafBuilder = Arc::new(move |id: &str| {
        Ok(Box::new(Switchable {
            colour: [1.0, 1.0, 0.0],
            updates: Arc::clone(&updates_in_layer),
            clock: clock_in_layer.clone(),
            slow: Arc::clone(&slow_in_layer),
            id: id.to_string(),
        }) as Box<dyn Effect>)
    });
    let mut effect = StackEffect::new(&stack, leaves, live_for(&stack), areas_for(&lay, &devices))
        .with_clock(clock.clock())
        .with_fps(Arc::new(std::sync::atomic::AtomicU32::new(30)));
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(1.0);

    // One overrun: held 250 ms.
    step(&mut effect, &mut canvas);
    assert_eq!(effect.layer_status("l1").map(|(state, _)| state), Some(StackLayerState::Held));
    // A fitting trial resets it.
    slow.store(false, Ordering::SeqCst);
    assert_eq!(html_budget(30), Duration::from_millis(8));
    clock.advance_ms(251);
    step(&mut effect, &mut canvas);
    assert_eq!(effect.layer_status("l1").map(|(state, _)| state), Some(StackLayerState::Ok));
    assert_eq!(updates.load(Ordering::SeqCst), 2);
    // An overrun after the reset holds for 250 ms again, not 500: 300 ms later it runs.
    slow.store(true, Ordering::SeqCst);
    step(&mut effect, &mut canvas);
    assert_eq!(effect.layer_status("l1").map(|(state, _)| state), Some(StackLayerState::Held));
    clock.advance_ms(300);
    step(&mut effect, &mut canvas);
    assert_eq!(updates.load(Ordering::SeqCst), 4, "the hold after a reset is 250 ms again");
}

/// A fake effect whose cost the test switches on and off.
struct Switchable {
    colour: [f32; 3],
    updates: Arc<AtomicU64>,
    clock: ManualClock,
    slow: Arc<std::sync::atomic::AtomicBool>,
    id: String,
}

impl Effect for Switchable {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: self.id.clone(),
            name: self.id.clone(),
            description: String::new(),
            needs: InputNeeds::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        if self.slow.load(Ordering::SeqCst) {
            self.clock.advance_ms(20);
        }
        canvas.fill(self.colour);
        self.updates.fetch_add(1, Ordering::SeqCst);
    }
}

/// The properties of a stack's schema are namespaced, and the merged map comes apart again.
#[test]
fn the_schema_namespaces_every_layer_property() {
    let stack = stack(
        "stack:Namespaced",
        vec![
            layer("l1", "gradient"),
            layer("l2", "fire"),
        ],
    );
    let resolve = |id: &str| {
        canvas_effects::create(id).map(|effect| effect.schema())
    };
    let schema = stack::schema_for(&stack, &resolve);
    assert_eq!(schema.id, "stack:Namespaced");
    assert_eq!(schema.name, "Namespaced");
    assert_eq!(schema.description, "2 layers: Gradient, Fire");
    assert!(schema.properties.iter().all(|def| def.key.starts_with("l1.") || def.key.starts_with("l2.")));
    assert!(schema.properties.iter().any(|def| def.label.starts_with("Layer l1 · ")));
    let merged = stack::stack_props(&stack, &resolve);
    assert!(merged.keys().all(|key| key.starts_with("l1.") || key.starts_with("l2.")));
}
