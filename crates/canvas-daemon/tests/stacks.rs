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
    self, blend_pixel, html_budget, AreaMap, Clock, LeafBuilder, LiveMap, Mask, StackEffect,
};
use canvas_daemon::state::App;
use canvas_proto::api::{routes, StackLayerState};
use canvas_proto::config::EFFECTS_FILE;
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
    /// The second engine a stack preview runs on, installed as a fake.
    preview_engine: Arc<FakeEngine>,
    home: TempHome,
    folder: PathBuf,
    /// The draft lease's clock, which a test moves by hand.
    draft_clock: LeaseClock,
}

/// Builds the daemon on a temporary home, with the user effects the test gives (or the real
/// ones). `stacks.json` may be written into the home before this, so a load path can be tested
/// exactly as a start sees it.
fn build(tag: &str) -> Harness {
    build_with(tag, None)
}

fn build_with(tag: &str, user: Option<Arc<canvas_daemon::user_effects::UserEffects>>) -> Harness {
    harness_in(TempHome::new(tag), user, None)
}

/// The daemon whose draft route builds with `builder` instead of the real HTML builder: a test
/// that counts a draft's builds, or makes one panic.
fn build_with_draft(tag: &str, builder: canvas_daemon::user_effects::Builder) -> Harness {
    harness_in(TempHome::new(tag), None, Some(builder))
}

/// The daemon on a home a test wrote first (a hand-written or corrupt `stacks.json`).
fn harness_at(home: TempHome) -> Harness {
    harness_in(home, None, None)
}

fn harness_in(
    home: TempHome,
    user: Option<Arc<canvas_daemon::user_effects::UserEffects>>,
    draft_builder: Option<canvas_daemon::user_effects::Builder>,
) -> Harness {
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
    // The preview engine is a fake, so a test reads its sinks, rate and layout directly.
    let preview_engine = Arc::new(FakeEngine::default());
    app.stack_preview.install_engine(preview_engine.clone());
    // A stack draft's lease is measured on a clock the test moves, never the wall clock.
    let draft_clock = LeaseClock::new();
    app.draft = match draft_builder {
        Some(builder) => canvas_daemon::draft::Draft::with_builder(draft_clock.clock(), builder),
        None => canvas_daemon::draft::Draft::with_clock(draft_clock.clock()),
    };
    let app = Arc::new(app);
    Harness {
        router: api::router(app.clone()),
        app,
        engine,
        preview_engine,
        home,
        folder,
        draft_clock,
    }
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

/// A PUT to the stack preview route.
async fn put_preview(
    harness: &Harness,
    stack: &Stack,
    epoch: &str,
    seq: u64,
) -> axum::response::Response {
    let body = serde_json::json!({ "stack": stack, "epoch": epoch, "seq": seq }).to_string();
    harness
        .router
        .clone()
        .oneshot(request_with_body("PUT", routes::STACK_PREVIEW, &body))
        .await
        .expect("the router answers")
}

/// The body of a stack draft PUT.
fn stack_draft_body(stack: &Stack, epoch: &str, seq: u64) -> String {
    serde_json::json!({
        "stack": stack,
        "epoch": epoch,
        "seq": seq,
        "props": {},
    })
    .to_string()
}

/// A PUT to the draft route with a stack body.
async fn put_stack_draft(
    harness: &Harness,
    stack: &Stack,
    epoch: &str,
    seq: u64,
) -> axum::response::Response {
    put_stack_draft_router(harness.router.clone(), stack, epoch, seq).await
}

/// [`put_stack_draft`] on a router the caller owns, so a test can spawn it.
async fn put_stack_draft_router(
    router: axum::Router,
    stack: &Stack,
    epoch: &str,
    seq: u64,
) -> axum::response::Response {
    let body = stack_draft_body(stack, epoch, seq);
    router
        .oneshot(request_with_body("PUT", routes::EFFECT_DRAFT, &body))
        .await
        .expect("the router answers")
}

/// A temporary server for the preview WebSockets. It binds port 0: no test touches the
/// daemon's own 9430.
async fn serve(router: axum::Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the port is free");
    let addr = listener.local_addr().expect("the address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    addr
}

/// Opens a preview WebSocket and returns it once the 101 is read. The token rides in the
/// query, as a browser's must.
async fn open_preview_socket(
    addr: std::net::SocketAddr,
    extra_query: &str,
) -> tokio::net::TcpStream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("the socket opens");
    let request = format!(
        "GET {}?token={TOKEN}{extra_query} HTTP/1.1\r\nHost: 127.0.0.1:9430\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
        routes::PREVIEW
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("the handshake is written");
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.expect("the answer is read");
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    assert!(head.contains("101"), "the socket opened: {head}");
    stream
}

/// Reads one server binary message (the server's frames are unmasked).
async fn read_binary_message(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut header = [0u8; 2];
    stream
        .read_exact(&mut header)
        .await
        .expect("a frame header is read");
    assert_eq!(header[0] & 0x0f, 0x2, "a binary frame");
    assert_eq!(header[1] & 0x80, 0, "a server frame is unmasked");
    let mut len = u64::from(header[1] & 0x7f);
    if len == 126 {
        let mut extended = [0u8; 2];
        stream.read_exact(&mut extended).await.expect("the length");
        len = u64::from(u16::from_be_bytes(extended));
    } else if len == 127 {
        let mut extended = [0u8; 8];
        stream.read_exact(&mut extended).await.expect("the length");
        len = u64::from_be_bytes(extended);
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .expect("the payload is read");
    payload
}

/// Waits (without a sleep) until the engine has a preview sink, then takes it.
async fn preview_sink(engine: &FakeEngine) -> Arc<dyn canvas_proto::traits::PreviewSink> {
    for _ in 0..2_000 {
        if let Some(sink) = engine.preview.lock().unwrap().clone() {
            return sink;
        }
        tokio::task::yield_now().await;
    }
    panic!("the preview sink was never registered");
}

/// A one-frame submit, so the socket has something to forward.
fn submit_frame(sink: &Arc<dyn canvas_proto::traits::PreviewSink>, rgb: &[u8]) {
    sink.submit(2, 1, rgb.to_vec());
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

/// The draft lease's clock, moved by hand: a lease is tested without waiting.
#[derive(Clone)]
struct LeaseClock {
    now: Arc<Mutex<Duration>>,
}

impl LeaseClock {
    fn new() -> Self {
        Self { now: Arc::new(Mutex::new(Duration::ZERO)) }
    }

    fn clock(&self) -> canvas_daemon::draft::Clock {
        let now = Arc::clone(&self.now);
        Arc::new(move || *now.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }

    fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *now += by;
    }
}

/// A hold on the next builder call: arm it, be told the build started, release it.
type BuildHold = (
    Arc<std::sync::atomic::AtomicBool>,
    std::sync::mpsc::Sender<()>,
    Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
);

/// A user-effects library whose HTML builder counts every build, so a test can prove a PUT
/// rebuilt nothing. Loading a file into the library is itself a build, so a hold is armed by
/// the test after its setup, not counted.
fn counting_user(
    block: Option<BuildHold>,
) -> (Arc<canvas_daemon::user_effects::UserEffects>, Arc<AtomicUsize>) {
    let builds = Arc::new(AtomicUsize::new(0));
    let base = canvas_daemon::user_effects::html_builder();
    let builder: canvas_daemon::user_effects::Builder = {
        let builds = Arc::clone(&builds);
        Arc::new(move |id: &str, source: &str| {
            builds.fetch_add(1, Ordering::SeqCst);
            if let Some((armed, started, go)) = &block {
                // One build is held open; later builds (the newer PUT) run normally.
                if armed.swap(false, Ordering::SeqCst) {
                    let _ = started.send(());
                    let _ = go
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .recv();
                }
            }
            base(id, source)
        })
    };
    (
        Arc::new(canvas_daemon::user_effects::UserEffects::new(builder)),
        builds,
    )
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

/// A fake layer whose `init` panics: the compositor must fault it, not unwind into the engine.
struct PanickyInit;

impl Effect for PanickyInit {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "panicky".into(),
            name: "Panicky".into(),
            description: String::new(),
            needs: InputNeeds::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {
        panic!("a layer's init panicked");
    }

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        canvas.fill([1.0, 0.0, 0.0]);
    }
}

/// A fake tile layer whose start unwinds with the tile render's own failure payload: a stack
/// tile must still refuse the tile rather than draw a quietly different picture.
struct TileFailingInit;

impl Effect for TileFailingInit {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "tile".into(),
            name: "Tile".into(),
            description: String::new(),
            needs: InputNeeds::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {
        std::panic::resume_unwind(Box::new(canvas_daemon::thumbnail::TileFailure {
            reason: "the tile's script failed".into(),
            load_dependent: false,
        }));
    }

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {}
}

/// A fake tile layer whose `update` unwinds with the tile render's own failure payload: a stack
/// tile must refuse the tile rather than cache a picture the failing layer never drew.
struct TileFailingUpdate;

impl Effect for TileFailingUpdate {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "tile-update".into(),
            name: "Tile update".into(),
            description: String::new(),
            needs: InputNeeds::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &Inputs) {
        std::panic::resume_unwind(Box::new(canvas_daemon::thumbnail::TileFailure {
            reason: "the layer's script failed while the tile was drawn".into(),
            load_dependent: false,
        }));
    }
}

/// A fake layer that paints its top half red and panics on its second frame, so its canvas is
/// half-written when it faults.
struct HalfThenPanic {
    frames: Arc<AtomicU64>,
}

impl Effect for HalfThenPanic {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "half".into(),
            name: "Half".into(),
            description: String::new(),
            needs: InputNeeds::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        let frame = self.frames.fetch_add(1, Ordering::SeqCst) + 1;
        for y in 0..canvas.height / 2 {
            for x in 0..canvas.width {
                canvas.set(x, y, [1.0, 0.0, 0.0]);
            }
        }
        if frame >= 2 {
            panic!("a layer's update panicked");
        }
    }
}

/// A fake layer whose output breaks the canvas invariant: it empties its own picture, which the
/// composite path reads. There the panic comes from `composite_layer`, not from the layer.
struct BrokeItsCanvas;

impl Effect for BrokeItsCanvas {
    fn schema(&self) -> PropertySchema {
        PropertySchema {
            id: "broken".into(),
            name: "Broken".into(),
            description: String::new(),
            needs: InputNeeds::default(),
            properties: Vec::new(),
            presets: Vec::new(),
            thumbnail: None,
        }
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, canvas: &mut Canvas, _inputs: &Inputs) {
        canvas.pixels.clear();
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

// ================================================================ a load check

/// A stack read from the file is validated like a save: one that breaks a rule (here an id
/// without the `stack:` prefix) is dropped with its reason logged, and the rest load.
#[tokio::test]
async fn a_stack_that_fails_its_rules_is_dropped_as_it_loads() {
    let home = TempHome::new("t-load-check");
    std::fs::write(
        home.path.join(STACKS_FILE),
        r#"{"schema_version":1,"stacks":[
            {"id":"stack:Good","layers":[{"id":"l1","name":"Base","effect":"solid"}]},
            {"id":"Glow","layers":[{"id":"l1","name":"Base","effect":"solid"}]}]}"#,
    )
    .unwrap();
    let harness = harness_at(home);

    let effects = json(harness.router.clone().oneshot(request("GET", routes::EFFECTS)).await.unwrap()).await;
    let ids: Vec<&str> = effects["effects"]
        .as_array()
        .expect("the effect list is an array")
        .iter()
        .filter_map(|effect| effect["id"].as_str())
        .collect();
    assert!(ids.contains(&"stack:Good"), "the valid stack loads: {ids:?}");
    assert!(!ids.contains(&"stack:Glow"), "the stack with no prefix is dropped: {ids:?}");

    let stored = json(harness.router.clone().oneshot(request("GET", routes::STACKS)).await.unwrap()).await;
    let names: Vec<&str> = stored["stacks"]
        .as_array()
        .expect("the stack list is an array")
        .iter()
        .filter_map(|stack| stack["id"].as_str())
        .collect();
    assert_eq!(names, vec!["stack:Good"]);
}

// ================================================================ a failed delete

/// A directory where the file should be, so `write_json` cannot rename over it.
fn block(path: &std::path::Path) {
    if path.exists() {
        std::fs::remove_file(path).expect("the file is removed");
    }
    std::fs::create_dir(path).expect("a directory stands where the file was");
}

/// A delete that cannot write the stack file is an error, and the stack, its presets and its
/// favourite are all still there.
#[tokio::test]
async fn a_delete_that_cannot_write_keeps_the_stack_and_its_presets() {
    let harness = build("t-delete-fail");
    let stack = stack("stack:Doomed", vec![layer("l1", "solid")]);
    assert_eq!(put_stack(&harness, &stack).await, StatusCode::OK);
    {
        let mut store = harness.app.store.write().await;
        store.effects.presets.insert("stack:Doomed".into(), Vec::new());
        store.effects.favorites.push("stack:Doomed".into());
    }
    block(&harness.home.path.join(STACKS_FILE));

    let response = harness
        .router
        .clone()
        .oneshot(request("DELETE", &stack_path("stack:Doomed")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let stored = json(harness.router.clone().oneshot(request("GET", routes::STACKS)).await.unwrap()).await;
    assert_eq!(stored["stacks"].as_array().map(Vec::len), Some(1), "the stack is still there: {stored}");
    let store = harness.app.store.read().await;
    assert!(store.effects.presets.contains_key("stack:Doomed"), "its presets are back");
    assert!(
        store.effects.favorites.iter().any(|favorite| favorite == "stack:Doomed"),
        "its favourite is back"
    );
}

/// A delete whose preset save fails removes nothing: the stack is still on the engine's book.
#[tokio::test]
async fn a_delete_whose_presets_cannot_be_saved_removes_nothing() {
    let harness = build("t-delete-presets");
    let stack = stack("stack:Doomed", vec![layer("l1", "solid")]);
    assert_eq!(put_stack(&harness, &stack).await, StatusCode::OK);
    {
        let mut store = harness.app.store.write().await;
        store.effects.presets.insert("stack:Doomed".into(), Vec::new());
        store.effects.favorites.push("stack:Doomed".into());
    }
    block(&harness.home.path.join(EFFECTS_FILE));

    let response = harness
        .router
        .clone()
        .oneshot(request("DELETE", &stack_path("stack:Doomed")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let stored = json(harness.router.clone().oneshot(request("GET", routes::STACKS)).await.unwrap()).await;
    assert_eq!(stored["stacks"].as_array().map(Vec::len), Some(1), "the stack is still there: {stored}");
    let store = harness.app.store.read().await;
    assert!(store.effects.presets.contains_key("stack:Doomed"), "its presets are back");
    assert!(
        store.effects.favorites.iter().any(|favorite| favorite == "stack:Doomed"),
        "its favourite is back"
    );
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

// ================================================================ a display name

/// A layer saved with an effect's display name is stored with the id the builder takes, so it
/// paints; a layer whose effect does not build reports `missing`, never an ok that draws nothing.
#[tokio::test]
async fn a_layer_saved_with_a_display_name_runs_that_effect() {
    let harness = build("t-display-name");
    assert_eq!(
        put_stack(&harness, &stack("stack:Firey", vec![layer("l1", "Fire")])).await,
        StatusCode::OK
    );
    let saved = harness.app.stacks.get("stack:Firey").expect("the stack was saved");
    assert_eq!(saved.layers[0].effect, "fire", "the display name is stored as the id");

    // Its detail says ok, and a frame over it paints.
    let detail = json(
        harness
            .router
            .clone()
            .oneshot(request("GET", &stack_path("stack:Firey")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(detail["layers"][0]["status"], "ok", "{detail}");
    let mut effect = canvas_daemon::create_effect(
        &harness.app.user_effects,
        &harness.app.stacks,
        "stack:Firey",
    )
    .expect("the stack builds");
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(1.0);
    effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
    assert!(
        canvas.pixels.iter().any(|pixel| pixel.iter().any(|c| *c > 0.0)),
        "the layer paints"
    );

    // A layer that names a stack is missing: `find_schema` resolves a stack, and the layer
    // builder does not, so an ok here would be a layer that draws nothing.
    assert_eq!(
        put_stack(&harness, &stack("stack:Glow", vec![layer("l1", "solid")])).await,
        StatusCode::OK
    );
    assert_eq!(
        put_stack(&harness, &stack("stack:Home", vec![layer("l1", "Glow")])).await,
        StatusCode::OK
    );
    let detail = json(
        harness
            .router
            .clone()
            .oneshot(request("GET", &stack_path("stack:Home")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(detail["layers"][0]["status"], "missing_effect", "{detail}");
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

/// True when the two masks cover at least one shared pixel.
fn masks_share(a: &Mask, b: &Mask) -> bool {
    let (Some(ear), Some(ebr)) = (a.extent(), b.extent()) else {
        return false;
    };
    let x0 = ear[0].max(ebr[0]).floor().max(0.0) as u32;
    let y0 = ear[1].max(ebr[1]).floor().max(0.0) as u32;
    let x1 = ear[2].min(ebr[2]).ceil().max(0.0) as u32;
    let y1 = ear[3].min(ebr[3]).ceil().max(0.0) as u32;
    (y0..y1).any(|y| (x0..x1).any(|x| a.contains(x, y) && b.contains(x, y)))
}

/// For each device, whether each of its LEDs has an owner.
type OwnedLeds = Vec<Vec<bool>>;
/// For each device, each zone's `(index, start, count)` in that device's colour list.
type ZoneRanges = Vec<Vec<(u32, usize, usize)>>;

/// The desk's LEDs from the sample plan: for each device, whether each LED has an owner, and
/// each zone's `(index, start, count)` range in that device's colour list.
fn desk_leds(layout: &Layout, devices: &[DeviceInfo]) -> (OwnedLeds, ZoneRanges) {
    let plan = stack::sample_plan(layout, devices);
    let mut owned = Vec::new();
    let mut ranges = Vec::new();
    for device in &plan.devices {
        let mut flags = Vec::new();
        let mut zones = Vec::new();
        for zone in &device.zones {
            let start = flags.len();
            flags.extend(zone.leds.iter().map(|led| led.sampled));
            zones.push((zone.zone_index, start, zone.leds.len()));
        }
        owned.push(flags);
        ranges.push(zones);
    }
    (owned, ranges)
}

/// The other desk devices whose sample footprint shares a pixel with `device`'s at `scale`:
/// the canvas has no z-order, so two parts drawn over each other share those pixels (D95.2.3).
/// The roof strip and the top front fan meet at the fan's top edge at 50 % area average, and
/// at 100 % nearest the two are four pixels apart.
fn shares_with(device: &str, scale: RenderScale) -> Vec<&'static str> {
    if !matches!(scale, RenderScale::P50) {
        return Vec::new();
    }
    match device {
        ROOF_STRIP => vec![FRONT_HUB],
        FRONT_HUB => vec![ROOF_STRIP],
        _ => Vec::new(),
    }
}

#[test]
fn areas_paint_their_own_leds_and_share_the_pixels_their_footprints_share() {
    let devices = desk_devices();
    let desk = desk_layout(RenderScale::P50, SamplingMode::AreaAverage);
    let red = canvas_proto::Rgb8::new(255, 0, 0);

    // A 40 x 20 rectangle at the origin changes only those pixels.
    let rect = paint_area(LayerArea::Rect { x: 0.0, y: 0.0, width: 40.0, height: 20.0 }, &desk, &devices);
    assert_eq!((rect.width, rect.height), (160, 100), "the desk renders at 50 %");
    for y in 0..100 {
        for x in 0..160 {
            let want = if x < 20 && y < 10 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 0.0] };
            assert_eq!(rect.get(x, y), want, "pixel {x},{y} of the rectangle");
        }
    }

    // Every device, painted as a Device area and then as a Zone area, at both sampling
    // settings. The area's own LEDs read the layer; a device whose sample footprint shares
    // no pixel with the painted one is byte-identical; the devices that do share are exactly
    // the known pair named in `shares_with`, and they change.
    for (scale, sampling) in [
        (RenderScale::P50, SamplingMode::AreaAverage),
        (RenderScale::P100, SamplingMode::Nearest),
    ] {
        let lay = desk_layout(scale, sampling);
        let (owned, ranges) = desk_leds(&lay, &devices);
        let map = AreaMap::build(&lay, &devices, 1);
        let before = sampled(&lay, &devices, &Canvas::with_scale(scale.factor()));

        for (index, device) in devices.iter().enumerate() {
            let painted = paint_area(LayerArea::Device { device: device.id.clone() }, &lay, &devices);
            let after = sampled(&lay, &devices, &painted);
            for (led, has_owner) in owned[index].iter().enumerate() {
                let want = if *has_owner { red } else { canvas_proto::Rgb8::BLACK };
                assert_eq!(
                    after[index][led], want,
                    "{} LED {led} as a Device area at {scale:?}/{sampling:?}",
                    device.id.0
                );
            }
            let mut shared: Vec<&str> = Vec::new();
            for (other_index, other) in devices.iter().enumerate() {
                if other_index == index {
                    continue;
                }
                let shares = masks_share(
                    map.devices.get(&device.id).expect("the device has a mask"),
                    map.devices.get(&other.id).expect("the other device has a mask"),
                );
                if shares {
                    shared.push(other.id.0.as_str());
                    assert_ne!(
                        after[other_index], before[other_index],
                        "{} shares pixels with {} at {scale:?}/{sampling:?}, so its LEDs must read the layer",
                        other.id.0, device.id.0
                    );
                } else {
                    assert_eq!(
                        after[other_index], before[other_index],
                        "{} shares no pixel with {} at {scale:?}/{sampling:?}, so it must not change",
                        other.id.0, device.id.0
                    );
                }
            }
            shared.sort();
            let mut expected = shares_with(&device.id.0, scale);
            expected.sort();
            assert_eq!(
                shared, expected,
                "the devices sharing pixels with {} at {scale:?}/{sampling:?}",
                device.id.0
            );

            for (zone_index, start, count) in &ranges[index] {
                let painted = paint_area(
                    LayerArea::Zone { device: device.id.clone(), zone: *zone_index },
                    &lay,
                    &devices,
                );
                let after = sampled(&lay, &devices, &painted);
                for (led, has_owner) in owned[index].iter().enumerate() {
                    let want = if !*has_owner {
                        canvas_proto::Rgb8::BLACK
                    } else if (*start..*start + *count).contains(&led) {
                        red
                    } else {
                        before[index][led]
                    };
                    assert_eq!(
                        after[index][led], want,
                        "{} LED {led} as zone {zone_index} at {scale:?}/{sampling:?}",
                        device.id.0
                    );
                }
                let mut shared: Vec<&str> = Vec::new();
                for (other_index, other) in devices.iter().enumerate() {
                    if other_index == index {
                        continue;
                    }
                    let shares = masks_share(
                        map.zones
                            .get(&(device.id.clone(), *zone_index))
                            .expect("the zone has a mask"),
                        map.devices.get(&other.id).expect("the other device has a mask"),
                    );
                    if shares {
                        shared.push(other.id.0.as_str());
                        assert_ne!(
                            after[other_index], before[other_index],
                            "{} shares pixels with {}'s zone {zone_index} at {scale:?}/{sampling:?}",
                            other.id.0, device.id.0
                        );
                    } else {
                        assert_eq!(
                            after[other_index], before[other_index],
                            "{} shares no pixel with {}'s zone {zone_index} at {scale:?}/{sampling:?}",
                            other.id.0, device.id.0
                        );
                    }
                }
                shared.sort();
                let mut expected = shares_with(&device.id.0, scale);
                expected.sort();
                assert_eq!(
                    shared, expected,
                    "the devices sharing pixels with {}'s zone {zone_index} at {scale:?}/{sampling:?}",
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

/// A layer whose `init` panics is faulted, and the rest of the stack keeps running: a panic in
/// one layer's start never unwinds into the engine (D95.3.6).
#[test]
fn a_layer_whose_init_panics_is_faulted_and_the_rest_run() {
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    let stack = stack("stack:Init", vec![layer("l1", "panicky"), layer("l2", "paint")]);
    let leaves: LeafBuilder = Arc::new(|id: &str| {
        if id == "panicky" {
            Ok(Box::new(PanickyInit) as Box<dyn Effect>)
        } else {
            Ok(Box::new(Paint {
                colour: [0.0, 1.0, 0.0],
                updates: Arc::new(AtomicU64::new(0)),
                cost: None,
                id: id.to_string(),
            }) as Box<dyn Effect>)
        }
    });
    let mut effect = StackEffect::new(&stack, leaves, live_for(&stack), areas_for(&lay, &devices))
        .with_clock(ManualClock::new().clock());
    effect.init(&Props::new());
    assert_eq!(
        effect.layer_status("l1"),
        Some((StackLayerState::Faulted, None)),
        "the panicking layer is faulted"
    );

    let mut canvas = Canvas::with_scale(1.0);
    step(&mut effect, &mut canvas);
    assert_eq!(canvas.get(5, 5), [0.0, 1.0, 0.0], "the other layer still paints");
    assert_eq!(
        effect.layer_status("l2").map(|(state, _)| state),
        Some(StackLayerState::Ok),
        "the other layer is fine"
    );
}

/// A panic in the compositor's own frame work is a black frame for that frame, not an unwind
/// into the engine: the clock here panics where a compositor bug would.
#[test]
fn a_panic_in_the_compositor_clock_is_a_black_frame() {
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    let stack = stack("stack:Broken", vec![layer("l1", "paint")]);
    let clock: Clock = Arc::new(|| panic!("the compositor's own work panicked"));
    let mut effect = StackEffect::new(
        &stack,
        paint_leaves(Arc::new(AtomicU64::new(0)), Arc::new(AtomicUsize::new(0)), [1.0, 0.0, 0.0], None),
        live_for(&stack),
        areas_for(&lay, &devices),
    )
    .with_clock(clock);
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(1.0);
    canvas.fill([1.0, 1.0, 1.0]);
    effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
    assert_eq!(canvas.get(5, 5), [0.0, 0.0, 0.0], "the frame is black");
    // The stack keeps running: a second frame is black too, and nothing unwinds.
    effect.update(1.0 / 30.0, &mut canvas, &Inputs::default());
    assert_eq!(canvas.get(5, 5), [0.0, 0.0, 0.0]);
}

/// A tile's own failure unwinds through the layer's start: `StackEffect::init` hands the payload
/// on as it is, rather than faulting the layer and drawing a different picture.
#[test]
fn a_tile_failure_from_a_layers_init_is_handed_on_by_the_stacks_init() {
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    let stack = stack("stack:Tile", vec![layer("l1", "tile")]);
    let leaves: LeafBuilder = Arc::new(|_id: &str| Ok(Box::new(TileFailingInit) as Box<dyn Effect>));
    let mut effect = StackEffect::new(&stack, leaves, live_for(&stack), areas_for(&lay, &devices))
        .with_clock(ManualClock::new().clock());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        effect.init(&Props::new());
    }));
    let payload = outcome.expect_err("the tile failure is handed on, not swallowed");
    assert!(
        payload.downcast_ref::<canvas_daemon::thumbnail::TileFailure>().is_some(),
        "the payload is still the tile's own"
    );
}

/// A tile's own failure raised by a layer's `update` is handed on to the tile render too: the
/// stack tile is refused (`RenderError::Failed`) instead of being cached with that layer missing.
#[test]
fn a_tile_failure_from_a_layers_update_refuses_the_stack_tile() {
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    let stack = stack("stack:TileUpdate", vec![layer("l1", "tile-update")]);
    let leaves: LeafBuilder = Arc::new(|_id: &str| Ok(Box::new(TileFailingUpdate) as Box<dyn Effect>));
    let mut effect = StackEffect::new(&stack, leaves, live_for(&stack), areas_for(&lay, &devices));
    let outcome = canvas_daemon::thumbnail::render_within(
        &mut effect,
        &Props::new(),
        Duration::from_secs(5),
    );
    match outcome {
        Err(canvas_daemon::thumbnail::RenderError::Failed(reason)) => {
            assert!(reason.contains("the layer's script failed"), "{reason}");
        }
        other => panic!("the stack tile was not refused: {other:?}"),
    }
}

/// A panic in the composite path is a black frame for that frame, not an unwind into the engine.
/// The panic here comes from a layer's output (it empties its own canvas), so `composite_layer`
/// reads past the picture.
#[test]
fn a_panic_in_the_composite_path_is_a_black_frame() {
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    let stack = stack("stack:BrokenLayer", vec![layer("l1", "broken")]);
    let leaves: LeafBuilder = Arc::new(|_id: &str| Ok(Box::new(BrokeItsCanvas) as Box<dyn Effect>));
    let mut effect = StackEffect::new(&stack, leaves, live_for(&stack), areas_for(&lay, &devices))
        .with_clock(ManualClock::new().clock());
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(1.0);
    canvas.fill([1.0, 1.0, 1.0]);
    step(&mut effect, &mut canvas);
    assert_eq!(canvas.get(5, 5), [0.0, 0.0, 0.0], "the frame is black");
    // The stack keeps running: the next frame is black too, and nothing unwinds.
    canvas.fill([1.0, 1.0, 1.0]);
    step(&mut effect, &mut canvas);
    assert_eq!(canvas.get(5, 5), [0.0, 0.0, 0.0]);
}

/// A layer that paints half its picture and then panics leaves no half-written canvas behind:
/// the faulted layer's picture is black, so its red does not reach the frame.
#[test]
fn a_layer_whose_update_panics_leaves_no_half_written_picture() {
    let devices = vec![device("d", "d", vec![zone(0, "z", ZoneKind::Linear, 4)])];
    let lay = layout(vec![placed("d", 0, 0.0, 0.0, 100.0, 20.0)], RenderScale::P100, SamplingMode::Nearest);
    let stack = stack("stack:Half", vec![layer("l1", "half")]);
    let frames = Arc::new(AtomicU64::new(0));
    let leaves: LeafBuilder = Arc::new({
        let frames = Arc::clone(&frames);
        move |_id: &str| {
            Ok(Box::new(HalfThenPanic { frames: Arc::clone(&frames) }) as Box<dyn Effect>)
        }
    });
    let mut effect = StackEffect::new(&stack, leaves, live_for(&stack), areas_for(&lay, &devices))
        .with_clock(ManualClock::new().clock());
    effect.init(&Props::new());
    let mut canvas = Canvas::with_scale(1.0);
    step(&mut effect, &mut canvas);
    assert_eq!(canvas.get(5, 5), [1.0, 0.0, 0.0], "the first frame paints the top half");
    step(&mut effect, &mut canvas);
    assert_eq!(
        canvas.get(5, 5),
        [0.0, 0.0, 0.0],
        "the panicked layer's half-written canvas never reaches the frame"
    );
    assert_eq!(
        effect.layer_status("l1").map(|(state, _)| state),
        Some(StackLayerState::Faulted)
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

// ================================================================ test 10

/// An HTML effect with a prop the test changes without changing the structure.
const TINTED_HTML: &str = r##"<head>
  <title>Tinted</title>
  <meta property="tint" label="Tint" type="color" default="#ffcc00" />
</head>
<body><canvas id="c" width="320" height="200"></canvas></body>
<script>
  var c = document.getElementById("c");
  var ctx = c.getContext("2d");
  var x = 0;
  function draw() {
    ctx.fillStyle = "#101020";
    ctx.fillRect(0, 0, 320, 200);
    ctx.fillStyle = tint;
    ctx.fillRect(x, 40, 40, 40);
    x = (x + 4) % 280;
    window.requestAnimationFrame(draw);
  }
  window.requestAnimationFrame(draw);
</script>"##;

/// Writes an HTML effect into the harness's folder and loads it.
fn add_html(harness: &Harness, file: &str, html: &str) {
    std::fs::write(harness.folder.join(file), html).expect("the effect file is written");
    harness.app.user_effects.rescan(&harness.folder);
}

/// Puts Solid on the lights and names it on the active layout: what a draft takes over from.
async fn prior_is_solid(harness: &Harness) {
    {
        let mut store = harness.app.store.write().await;
        store.layouts.layouts[0].effect = Some(canvas_proto::layout::ActiveEffect {
            id: "solid".into(),
            props: Props::new(),
        });
    }
    canvas_daemon::apply_effect(&harness.app, "solid", &Props::new())
        .await
        .expect("solid runs");
}

fn tint(hex: &str) -> canvas_proto::PropValue {
    canvas_proto::PropValue::Color(hex.parse().expect("a #rrggbb colour"))
}

#[tokio::test]
async fn the_stack_preview_runs_on_its_own_double_sinkless_engine() {
    let (user, _builds) = counting_user(None);
    let harness = build_with("t10-engine", Some(user));
    add_html(&harness, "Spark.html", VALID_HTML);
    let preview = stack("stack:Screen", vec![layer("l1", "html:Spark")]);

    let response = put_preview(&harness, &preview, "0123456789abcdef", 1).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    assert_eq!(body["applied"], true, "{body}");
    assert_eq!(body["running"], true, "{body}");
    assert_eq!(body["seq"], 1);
    assert_eq!(body["layers"][0]["id"], "l1");
    assert_eq!(body["layers"][0]["status"], "ok");

    // The preview engine has no device sink and was never given one.
    assert!(
        harness.preview_engine.sinks.lock().unwrap().is_empty(),
        "a preview must never have a device sink"
    );
    // 15 fps is its render rate and the sink's cap.
    assert_eq!(
        harness.preview_engine.fps.load(Ordering::SeqCst),
        canvas_proto::consts::PREVIEW_MAX_FPS
    );
    assert_eq!(harness.preview_engine.fps.load(Ordering::SeqCst), 15);
    // The layout reached it as it reached the main engine.
    assert!(harness.preview_engine.layout.lock().unwrap().is_some());
    // Neither engine has an input source on this commit; the preview has the main engine's.
    assert!(harness.preview_engine.input_source.lock().unwrap().is_none());
    assert!(harness.engine.input_source.lock().unwrap().is_none());
    // The main engine was not touched by a preview.
    assert_eq!(*harness.engine.effect_id.lock().unwrap(), None);
    assert_eq!(
        harness.preview_engine.effect_id.lock().unwrap().clone(),
        Some("stack:Screen".to_string())
    );

    // `?stack=1` streams the preview engine; the plain socket still streams the running effect.
    let addr = serve(harness.router.clone()).await;
    let mut stack_socket = open_preview_socket(addr, "&stack=1").await;
    let stack_sink = preview_sink(&harness.preview_engine).await;
    submit_frame(&stack_sink, &[1, 2, 3, 4, 5, 6]);
    let frame = tokio::time::timeout(Duration::from_secs(5), read_binary_message(&mut stack_socket))
        .await
        .expect("a stack preview frame arrives");
    assert_eq!(frame, vec![2, 0, 1, 0, 1, 2, 3, 4, 5, 6], "width, height, then RGB");
    assert!(
        harness.engine.preview.lock().unwrap().is_none(),
        "the stack socket did not touch the main engine"
    );

    let mut main_socket = open_preview_socket(addr, "").await;
    let main_sink = preview_sink(&harness.engine).await;
    submit_frame(&main_sink, &[9, 9, 9, 8, 8, 8]);
    let frame = tokio::time::timeout(Duration::from_secs(5), read_binary_message(&mut main_socket))
        .await
        .expect("a running-effect frame arrives");
    assert_eq!(frame, vec![2, 0, 1, 0, 9, 9, 9, 8, 8, 8]);
    assert!(
        harness.preview_engine.preview.lock().unwrap().is_some(),
        "the plain socket did not take over the preview engine"
    );
}

#[tokio::test]
async fn a_preview_put_is_superseded_by_a_newer_epoch_or_sequence_and_delete_clears_it() {
    let harness = build("t10-sequence");
    let preview = stack("stack:Sup", vec![layer("l1", "solid")]);
    let page_a = "aaaaaaaaaaaaaaaa";
    let page_b = "bbbbbbbbbbbbbbbb";

    // An older page opens: its epoch owns the preview.
    let accepted = put_preview(&harness, &preview, page_a, 1).await;
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(json(accepted).await["applied"], true);

    // A newer page (a reload or a second tab) wins and replaces the watermark.
    let accepted = put_preview(&harness, &preview, page_b, 1).await;
    assert_eq!(accepted.status(), StatusCode::OK);
    assert_eq!(json(accepted).await["applied"], true);

    // A lower sequence in the accepted epoch is superseded and says who owns the preview.
    let stale = put_preview(&harness, &preview, page_b, 0).await;
    assert_eq!(stale.status(), StatusCode::OK, "superseded is still a 200");
    let body = json(stale).await;
    assert_eq!(body["applied"], false);
    assert_eq!(body["epoch"], page_b);
    assert_eq!(body["seq"], 1);

    // The older tab, whose epoch was replaced, learns it lost even with a higher sequence.
    let older = put_preview(&harness, &preview, page_a, 99).await;
    assert_eq!(older.status(), StatusCode::OK);
    let body = json(older).await;
    assert_eq!(body["applied"], false);
    assert_eq!(body["epoch"], page_b);

    // A preview shares the two render slots with `POST /effects/preview`.
    let first = harness.app.preview_slots.clone().try_acquire_owned().unwrap();
    let second = harness.app.preview_slots.clone().try_acquire_owned().unwrap();
    let busy = put_preview(&harness, &preview, page_b, 2).await;
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        busy.headers().get("retry-after").and_then(|v| v.to_str().ok()),
        Some("1")
    );
    drop(first);
    drop(second);

    // DELETE drops the effect, paints black and clears the live sequence.
    let dropped = harness
        .router
        .clone()
        .oneshot(request("DELETE", routes::STACK_PREVIEW))
        .await
        .unwrap();
    assert_eq!(dropped.status(), StatusCode::OK);
    assert_eq!(json(dropped).await["running"], true);
    assert!(harness.app.stack_preview.snapshot().is_none());
    assert!(harness.app.stack_preview.watermark().is_none());
    assert_eq!(
        harness.preview_engine.effect_id.lock().unwrap().clone(),
        Some("solid".to_string()),
        "the preview engine no longer shows the stack"
    );
    assert_eq!(*harness.preview_engine.props.lock().unwrap(), canvas_daemon::draft::black());

    // The older page does not take the preview back after the DELETE (item 4).
    let after = put_preview(&harness, &preview, page_a, 2).await;
    assert_eq!(after.status(), StatusCode::OK);
    let body = json(after).await;
    assert_eq!(body["applied"], false);
    assert_eq!(body["epoch"], page_b, "the newest page still owns the preview");

    // The newest page can take it again with its next PUT.
    let resumed = put_preview(&harness, &preview, page_b, 2).await;
    assert_eq!(resumed.status(), StatusCode::OK);
    assert_eq!(json(resumed).await["applied"], true);
}

#[tokio::test]
async fn a_preview_epoch_still_loses_after_forty_page_loads() {
    let harness = build("t10-forty");
    let preview = stack("stack:Forty", vec![layer("l1", "solid")]);
    let oldest = "0000000000000000";
    assert_eq!(
        json(put_preview(&harness, &preview, oldest, 1).await).await["applied"],
        true
    );
    for page in 1..=40u64 {
        let epoch = format!("{page:016x}");
        let answer = put_preview(&harness, &preview, &epoch, 1).await;
        assert_eq!(answer.status(), StatusCode::OK);
        assert_eq!(json(answer).await["applied"], true, "a new page always wins");
    }
    let stale = put_preview(&harness, &preview, oldest, 2).await;
    assert_eq!(stale.status(), StatusCode::OK);
    assert_eq!(
        json(stale).await["applied"],
        false,
        "the tab that owned the preview 40 loads ago is still older"
    );
}

#[tokio::test]
async fn preview_value_changes_never_rebuild_an_html_layer() {
    let (user, builds) = counting_user(None);
    let harness = build_with("t10-values", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    builds.store(0, Ordering::SeqCst);

    let mut preview = stack("stack:Tuned", vec![layer("l1", "html:Tinted")]);
    let first = put_preview(&harness, &preview, "eeeeeeeeeeeeeeee", 1).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(builds.load(Ordering::SeqCst), 1, "the first preview built the layer");

    // A props change, a blend change, an opacity drag, a visibility toggle and an area change
    // are all values: the structure (layer ids, effects, HTML source) did not move.
    preview.layers[0].props.insert("tint".into(), tint("#2200ff"));
    assert_eq!(put_preview(&harness, &preview, "eeeeeeeeeeeeeeee", 2).await.status(), StatusCode::OK);

    preview.layers[0].blend = BlendMode::Add;
    assert_eq!(put_preview(&harness, &preview, "eeeeeeeeeeeeeeee", 3).await.status(), StatusCode::OK);

    for (seq, opacity) in [(4, 90), (5, 70), (6, 50), (7, 30)] {
        preview.layers[0].opacity = opacity;
        assert_eq!(put_preview(&harness, &preview, "eeeeeeeeeeeeeeee", seq).await.status(), StatusCode::OK);
    }

    preview.layers[0].visible = false;
    assert_eq!(put_preview(&harness, &preview, "eeeeeeeeeeeeeeee", 8).await.status(), StatusCode::OK);

    preview.layers[0].area = LayerArea::Rect { x: 10.0, y: 20.0, width: 100.0, height: 50.0 };
    assert_eq!(put_preview(&harness, &preview, "eeeeeeeeeeeeeeee", 9).await.status(), StatusCode::OK);

    assert_eq!(
        builds.load(Ordering::SeqCst),
        1,
        "no value ever rebuilt the HTML layer"
    );
    // The values still reached the effect through the running instance.
    let props = harness.preview_engine.props.lock().unwrap().clone();
    assert_eq!(props.get("l1.tint"), Some(&tint("#2200ff")));
}

#[tokio::test]
async fn two_preview_puts_out_of_order_leave_the_newer_one_on_screen() {
    let held = HeldBuild::new();
    let (user, _builds) = counting_user(Some(held.hold));
    let harness = build_with("t10-order", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    add_html(&harness, "Other.html", &TINTED_HTML.replace("Tinted", "Other"));

    // Two structures: a different HTML file is a different build, not a value.
    let mut red = stack("stack:Order", vec![layer("l1", "html:Tinted")]);
    red.layers[0].props.insert("tint".into(), tint("#ff0000"));
    let mut green = stack("stack:Order", vec![layer("l1", "html:Other")]);
    green.layers[0].props.insert("tint".into(), tint("#00ff00"));

    // The first PUT's build is held open.
    held.armed.store(true, Ordering::SeqCst);
    let first_router = harness.router.clone();
    let first_body =
        serde_json::json!({ "stack": red, "epoch": "eeeeeeeeeeeeeeee", "seq": 1 }).to_string();
    let first = tokio::spawn(async move {
        first_router
            .oneshot(request_with_body("PUT", routes::STACK_PREVIEW, &first_body))
            .await
            .expect("the router answers")
    });
    wait_started(held.started).await;

    // The newer PUT takes the route over and waits for the build in flight to let go: one build
    // runs at a time per route.
    let newer_router = harness.router.clone();
    let newer_body =
        serde_json::json!({ "stack": green, "epoch": "eeeeeeeeeeeeeeee", "seq": 2 }).to_string();
    let newer = tokio::spawn(async move {
        newer_router
            .oneshot(request_with_body("PUT", routes::STACK_PREVIEW, &newer_body))
            .await
            .expect("the router answers")
    });
    // The newer PUT has taken the route over once its sequence owns the watermark.
    for _ in 0..2_000 {
        if harness.app.stack_preview.watermark() == Some(("eeeeeeeeeeeeeeee".to_string(), 2)) {
            break;
        }
        tokio::task::yield_now().await;
    }

    held.go.send(()).expect("the first build is released");
    let older = first.await.expect("the task ends");
    assert_eq!(older.status(), StatusCode::OK);
    assert_eq!(json(older).await["applied"], false, "the superseded build landed nowhere");

    let newer = newer.await.expect("the task ends");
    assert_eq!(newer.status(), StatusCode::OK);
    assert_eq!(json(newer).await["applied"], true);

    let props = harness.preview_engine.props.lock().unwrap().clone();
    assert_eq!(
        props.get("l1.tint"),
        Some(&tint("#00ff00")),
        "the late older build did not land over the newer one"
    );
    assert_eq!(
        harness.preview_engine.effect_id.lock().unwrap().clone(),
        Some("stack:Order".to_string())
    );
}

// ================================================================ test 11

#[tokio::test]
async fn a_stack_draft_builds_once_across_its_values_and_a_save() {
    let (user, builds) = counting_user(None);
    let harness = build_with("t11-save", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    builds.store(0, Ordering::SeqCst);
    prior_is_solid(&harness).await;

    let mut draft_stack = stack("stack:Session", vec![layer("l1", "html:Tinted")]);
    let first = put_stack_draft(&harness, &draft_stack, "eeeeeeeeeeeeeeee", 1).await;
    assert_eq!(first.status(), StatusCode::OK);
    let status: canvas_proto::api::DraftStatus =
        serde_json::from_value(json(first).await).expect("a DraftStatus");
    assert_eq!(status.applied, Some(true));
    assert!(status.active);
    assert_eq!(status.id.as_deref(), Some("stack:Session"));
    assert_eq!(status.epoch.as_deref(), Some("eeeeeeeeeeeeeeee"));
    assert_eq!(status.seq, Some(1));
    assert_eq!(builds.load(Ordering::SeqCst), 1, "the draft built the layer once");
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("stack:Session".to_string()),
        "the stack draft reached the main engine"
    );

    // An opacity drag, a blend change, a visibility toggle and an area change are values.
    for (seq, opacity) in [(2, 90), (3, 70), (4, 50), (5, 30)] {
        draft_stack.layers[0].opacity = opacity;
        let response = put_stack_draft(&harness, &draft_stack, "eeeeeeeeeeeeeeee", seq).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json(response).await["applied"], true);
    }
    draft_stack.layers[0].blend = BlendMode::Add;
    assert_eq!(put_stack_draft(&harness, &draft_stack, "eeeeeeeeeeeeeeee", 6).await.status(), StatusCode::OK);
    draft_stack.layers[0].visible = false;
    assert_eq!(put_stack_draft(&harness, &draft_stack, "eeeeeeeeeeeeeeee", 7).await.status(), StatusCode::OK);
    draft_stack.layers[0].area = LayerArea::Rect { x: 4.0, y: 8.0, width: 80.0, height: 40.0 };
    assert_eq!(put_stack_draft(&harness, &draft_stack, "eeeeeeeeeeeeeeee", 8).await.status(), StatusCode::OK);

    assert_eq!(
        builds.load(Ordering::SeqCst),
        1,
        "opacity, blend, visibility and area never rebuilt the layer"
    );

    // Save (write the file, then activate) reuses the running effect: one build for the session.
    assert_eq!(put_stack(&harness, &draft_stack).await, StatusCode::OK);
    let activated = harness
        .router
        .clone()
        .oneshot(request_with_body(
            "POST",
            &routes::EFFECT_ACTIVATE.replace("{id}", "stack:Session"),
            &serde_json::json!({ "props": {}, "preset": null }).to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(activated.status(), StatusCode::OK);
    assert_eq!(
        builds.load(Ordering::SeqCst),
        1,
        "Save with the same structure reused the running effect"
    );
    assert!(!harness.app.draft.is_active(), "an activate drops the draft");
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("stack:Session".to_string())
    );
}

#[tokio::test]
async fn a_stack_draft_restores_the_prior_on_stop_and_on_expiry() {
    let (user, _builds) = counting_user(None);
    let harness = build_with("t11-restore", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    prior_is_solid(&harness).await;
    let draft_stack = stack("stack:Restore", vec![layer("l1", "html:Tinted")]);

    assert_eq!(put_stack_draft(&harness, &draft_stack, "eeeeeeeeeeeeeeee", 1).await.status(), StatusCode::OK);
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("stack:Restore".to_string())
    );
    let stopped = harness
        .router
        .clone()
        .oneshot(request("DELETE", routes::EFFECT_DRAFT))
        .await
        .unwrap();
    assert_eq!(stopped.status(), StatusCode::OK);
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("solid".to_string()),
        "Stop put the prior effect back"
    );
    assert!(!harness.app.draft.is_active());

    // The lease runs out on the injected clock: the prior comes back without a Stop.
    assert_eq!(put_stack_draft(&harness, &draft_stack, "eeeeeeeeeeeeeeee", 2).await.status(), StatusCode::OK);
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("stack:Restore".to_string())
    );
    harness
        .draft_clock
        .advance(canvas_proto::consts::DRAFT_LEASE + Duration::from_secs(1));
    let state = harness
        .router
        .clone()
        .oneshot(request("GET", routes::EFFECT_DRAFT))
        .await
        .unwrap();
    let state: canvas_proto::api::DraftStatus =
        serde_json::from_value(json(state).await).expect("a DraftStatus");
    assert!(!state.active, "the lease ended");
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("solid".to_string()),
        "expiry put the prior effect back"
    );
}

#[tokio::test]
async fn a_stop_during_a_slow_stack_build_answers_without_waiting() {
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (started_tx, started) = std::sync::mpsc::channel::<()>();
    let (go, go_rx) = std::sync::mpsc::channel::<()>();
    let (user, _builds) =
        counting_user(Some((Arc::clone(&armed), started_tx, Arc::new(Mutex::new(go_rx)))));
    let harness = build_with("t11-slow", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    add_html(&harness, "Other.html", &TINTED_HTML.replace("Tinted", "Other"));
    prior_is_solid(&harness).await;

    let first = stack("stack:Slow", vec![layer("l1", "html:Tinted")]);
    assert_eq!(put_stack_draft(&harness, &first, "eeeeeeeeeeeeeeee", 1).await.status(), StatusCode::OK);

    // A structure change: another HTML file, whose build is held open.
    armed.store(true, Ordering::SeqCst);
    let changed = stack("stack:Slow", vec![layer("l1", "html:Other")]);
    let slow_router = harness.router.clone();
    let slow_body = stack_draft_body(&changed, "eeeeeeeeeeeeeeee", 2);
    let slow = tokio::spawn(async move {
        slow_router
            .oneshot(request_with_body("PUT", routes::EFFECT_DRAFT, &slow_body))
            .await
            .expect("the router answers")
    });
    tokio::task::spawn_blocking(move || {
        started
            .recv_timeout(Duration::from_secs(10))
            .expect("the slow build started")
    })
    .await
    .expect("the wait ends");

    // Stop answers while the build is still running, and puts the prior effect back.
    let stop = harness
        .router
        .clone()
        .oneshot(request("DELETE", routes::EFFECT_DRAFT))
        .await
        .unwrap();
    assert_eq!(stop.status(), StatusCode::OK);
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("solid".to_string()),
        "Stop did not wait for the build"
    );
    assert!(!harness.app.draft.is_active());

    go.send(()).expect("the build is released");
    let late = slow.await.expect("the task ends");
    assert_eq!(late.status(), StatusCode::OK);
    assert_eq!(
        json(late).await["applied"],
        false,
        "the build that finished after Stop did not land"
    );
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("solid".to_string()),
        "the stopped draft stayed off the lights"
    );
}

// ================================================================ round 2 (job 72)

/// A page epoch: exactly the 16 hexadecimal characters the builder makes.
fn page(n: u64) -> String {
    format!("{n:016x}")
}

/// A build a test holds open.
struct HeldBuild {
    /// Arms the next build and tells the test when it started.
    hold: BuildHold,
    /// The flag that arms the next build.
    armed: Arc<std::sync::atomic::AtomicBool>,
    /// Says the held build started.
    started: std::sync::mpsc::Receiver<()>,
    /// Releases the held build.
    go: std::sync::mpsc::Sender<()>,
}

impl HeldBuild {
    fn new() -> Self {
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (started_tx, started) = std::sync::mpsc::channel::<()>();
        let (go, go_rx) = std::sync::mpsc::channel::<()>();
        Self {
            hold: (Arc::clone(&armed), started_tx, Arc::new(Mutex::new(go_rx))),
            armed,
            started,
            go,
        }
    }
}

/// Waits for a held build to start, without a sleep over 100 ms.
async fn wait_started(started: std::sync::mpsc::Receiver<()>) {
    tokio::task::spawn_blocking(move || {
        started
            .recv_timeout(Duration::from_secs(10))
            .expect("the held build started")
    })
    .await
    .expect("the wait ends");
}

#[tokio::test]
async fn a_stack_draft_build_slower_than_the_beat_still_lands_once() {
    let held = HeldBuild::new();
    let (user, builds) = counting_user(Some(held.hold));
    let harness = build_with("r2-draft-beats", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    builds.store(0, Ordering::SeqCst);
    prior_is_solid(&harness).await;
    let epoch = page(0xd1);
    let draft_stack = stack("stack:Slowbeat", vec![layer("l1", "html:Tinted")]);

    // The first PUT's build is held open, as a slow HTML layer would be.
    held.armed.store(true, Ordering::SeqCst);
    let first_router = harness.router.clone();
    let first_body = stack_draft_body(&draft_stack, &epoch, 1);
    let first = tokio::spawn(async move {
        first_router
            .oneshot(request_with_body("PUT", routes::EFFECT_DRAFT, &first_body))
            .await
            .expect("the router answers")
    });
    wait_started(held.started).await;

    // Beats arrive faster than the build: each one joins it instead of starting another.
    for seq in 2..=6 {
        let beat = put_stack_draft(&harness, &draft_stack, &epoch, seq).await;
        assert_eq!(beat.status(), StatusCode::OK);
        assert_eq!(
            json(beat).await["applied"],
            true,
            "a beat of the same structure is not superseded"
        );
    }
    assert_eq!(builds.load(Ordering::SeqCst), 1, "one build for six beats");

    held.go.send(()).expect("the build is released");
    let landed = first.await.expect("the task ends");
    assert_eq!(landed.status(), StatusCode::OK);
    assert_eq!(json(landed).await["applied"], true);
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("stack:Slowbeat".to_string()),
        "the build that outlived the beats landed"
    );
    assert!(harness.app.draft.is_active());
    assert_eq!(builds.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_preview_build_slower_than_the_beat_still_lands_once() {
    let held = HeldBuild::new();
    let (user, builds) = counting_user(Some(held.hold));
    let harness = build_with("r2-preview-beats", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    builds.store(0, Ordering::SeqCst);
    let epoch = page(0xd2);
    let preview = stack("stack:Quickview", vec![layer("l1", "html:Tinted")]);

    // The preview's first open builds; its build is held open.
    held.armed.store(true, Ordering::SeqCst);
    let first_router = harness.router.clone();
    let first_body =
        serde_json::json!({ "stack": preview, "epoch": epoch, "seq": 1 }).to_string();
    let first = tokio::spawn(async move {
        first_router
            .oneshot(request_with_body("PUT", routes::STACK_PREVIEW, &first_body))
            .await
            .expect("the router answers")
    });
    wait_started(held.started).await;

    for seq in 2..=6 {
        let beat = put_preview(&harness, &preview, &epoch, seq).await;
        assert_eq!(beat.status(), StatusCode::OK);
        assert_eq!(
            json(beat).await["applied"],
            true,
            "a beat of the structure already building joins it"
        );
    }
    assert_eq!(builds.load(Ordering::SeqCst), 1, "one build for six PUTs");

    held.go.send(()).expect("the build is released");
    let landed = first.await.expect("the task ends");
    assert_eq!(landed.status(), StatusCode::OK);
    let body = json(landed).await;
    assert_eq!(body["applied"], true);
    assert_eq!(body["running"], true);
    assert_eq!(
        harness.preview_engine.effect_id.lock().unwrap().clone(),
        Some("stack:Quickview".to_string()),
        "the preview that outlived the beats landed"
    );
    assert_eq!(builds.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_preview_and_a_draft_refuse_a_stack_a_save_would_refuse() {
    let harness = build("r2-caps");
    let three_html = stack(
        "stack:ThreeHtml",
        vec![layer("l1", "html:A"), layer("l2", "html:B"), layer("l3", "html:C")],
    );
    let nine = stack(
        "stack:NineLayers",
        (1..=9).map(|n| layer(&format!("l{n}"), "solid")).collect(),
    );
    let epoch = page(2);

    // Three HTML layers is over the cap of two, on both routes.
    let preview = put_preview(&harness, &three_html, &epoch, 1).await;
    assert_eq!(preview.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let draft = put_stack_draft(&harness, &three_html, &epoch, 1).await;
    assert_eq!(draft.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Nine layers is over the cap of eight, on both routes.
    let preview = put_preview(&harness, &nine, &epoch, 2).await;
    assert_eq!(preview.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let draft = put_stack_draft(&harness, &nine, &epoch, 2).await;
    assert_eq!(draft.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Neither route built anything or lit a light.
    assert_eq!(*harness.engine.effect_id.lock().unwrap(), None);
    assert_eq!(*harness.preview_engine.effect_id.lock().unwrap(), None);
    assert!(!harness.app.draft.is_active());
}

#[tokio::test]
async fn a_stack_draft_epoch_still_loses_after_forty_page_loads() {
    let (user, _builds) = counting_user(None);
    let harness = build_with("r2-draft-forty", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    prior_is_solid(&harness).await;
    let draft_stack = stack("stack:Fortyloads", vec![layer("l1", "html:Tinted")]);
    let oldest = "0000000000000000";

    assert_eq!(
        json(put_stack_draft(&harness, &draft_stack, oldest, 1).await).await["applied"],
        true
    );
    for load in 1..=40u64 {
        let answer = put_stack_draft(&harness, &draft_stack, &page(load), 1).await;
        assert_eq!(answer.status(), StatusCode::OK);
        assert_eq!(json(answer).await["applied"], true, "a new page always wins");
    }
    let stale = put_stack_draft(&harness, &draft_stack, oldest, 2).await;
    assert_eq!(stale.status(), StatusCode::OK);
    assert_eq!(
        json(stale).await["applied"],
        false,
        "the tab that owned the draft 40 loads ago is still older"
    );
}

#[tokio::test]
async fn a_tab_that_lost_the_draft_cannot_take_it_back_after_a_stop_or_an_expiry() {
    let (user, _builds) = counting_user(None);
    let harness = build_with("r2-draft-tabs", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    prior_is_solid(&harness).await;
    let draft_stack = stack("stack:Twotabs", vec![layer("l1", "html:Tinted")]);
    let page_a = page(0xa);
    let page_b = page(0xb);

    // A owns the draft; B (a newer page) takes it.
    assert_eq!(
        json(put_stack_draft(&harness, &draft_stack, &page_a, 1).await).await["applied"],
        true
    );
    assert_eq!(
        json(put_stack_draft(&harness, &draft_stack, &page_b, 1).await).await["applied"],
        true
    );

    // B Stops.
    let stopped = harness
        .router
        .clone()
        .oneshot(request("DELETE", routes::EFFECT_DRAFT))
        .await
        .unwrap();
    assert_eq!(stopped.status(), StatusCode::OK);
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("solid".to_string())
    );

    // A may not re-take the lights with a newer sequence of its older epoch.
    let stale = put_stack_draft(&harness, &draft_stack, &page_a, 2).await;
    assert_eq!(stale.status(), StatusCode::OK);
    assert_eq!(json(stale).await["applied"], false, "the older tab lost the draft");
    assert!(!harness.app.draft.is_active());
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("solid".to_string()),
        "the stale tab did not put its draft back on the lights"
    );

    // The newest page can start again with its next PUT.
    assert_eq!(
        json(put_stack_draft(&harness, &draft_stack, &page_b, 2).await).await["applied"],
        true
    );

    // The lease runs out on the injected clock: A still cannot take the slot back.
    harness
        .draft_clock
        .advance(canvas_proto::consts::DRAFT_LEASE + Duration::from_secs(1));
    let state = harness
        .router
        .clone()
        .oneshot(request("GET", routes::EFFECT_DRAFT))
        .await
        .unwrap();
    let state: canvas_proto::api::DraftStatus =
        serde_json::from_value(json(state).await).expect("a DraftStatus");
    assert!(!state.active, "the lease ended");
    let stale = put_stack_draft(&harness, &draft_stack, &page_a, 3).await;
    assert_eq!(stale.status(), StatusCode::OK);
    assert_eq!(
        json(stale).await["applied"],
        false,
        "the older tab lost the draft to expiry too"
    );
}

#[tokio::test]
async fn the_draft_lease_starts_when_its_build_lands() {
    let held = HeldBuild::new();
    let (user, _builds) = counting_user(Some(held.hold));
    let harness = build_with("r2-draft-lease", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    prior_is_solid(&harness).await;
    let draft_stack = stack("stack:Slowlease", vec![layer("l1", "html:Tinted")]);

    // The build is held open, and the clock moves past the whole lease while it runs.
    held.armed.store(true, Ordering::SeqCst);
    let first_router = harness.router.clone();
    let first_body = stack_draft_body(&draft_stack, &page(0x1e), 1);
    let first = tokio::spawn(async move {
        first_router
            .oneshot(request_with_body("PUT", routes::EFFECT_DRAFT, &first_body))
            .await
            .expect("the router answers")
    });
    wait_started(held.started).await;
    harness
        .draft_clock
        .advance(canvas_proto::consts::DRAFT_LEASE + Duration::from_secs(5));
    held.go.send(()).expect("the build is released");
    let landed = first.await.expect("the task ends");
    assert_eq!(json(landed).await["applied"], true);

    let state = harness
        .router
        .clone()
        .oneshot(request("GET", routes::EFFECT_DRAFT))
        .await
        .unwrap();
    let state: canvas_proto::api::DraftStatus =
        serde_json::from_value(json(state).await).expect("a DraftStatus");
    assert!(state.active, "the lease starts when the draft lands, not when the build did");
    assert_eq!(
        state.lease_remaining_ms,
        canvas_proto::consts::DRAFT_LEASE.as_millis() as u64,
        "the whole lease is left"
    );
}

#[tokio::test]
async fn the_preview_engine_reads_the_apps_input_source_when_it_is_made() {
    let harness = build("r2-input-source");
    let preview = stack("stack:Withinput", vec![layer("l1", "solid")]);
    let source: Arc<dyn canvas_proto::traits::InputSource> =
        Arc::new(common::FakeInputs::empty());
    harness.app.set_input_source(Some(Arc::clone(&source)));

    // The engine is configured the first time a preview asks for it.
    let answer = put_preview(&harness, &preview, &page(6), 1).await;
    assert_eq!(answer.status(), StatusCode::OK);

    let reached = harness
        .preview_engine
        .input_source
        .lock()
        .unwrap()
        .clone()
        .expect("the preview engine was given the app's input source");
    assert!(
        Arc::ptr_eq(&reached, &source),
        "the preview engine reads the source the app has when the engine is made"
    );
    assert!(harness.engine.input_source.lock().unwrap().is_some());
}

#[tokio::test]
async fn a_preview_and_a_draft_require_a_sixteen_hex_epoch() {
    let harness = build("r2-epoch-shape");
    let preview = stack("stack:Epochshape", vec![layer("l1", "solid")]);

    for bad in ["", "e", "old", "0123456789abcde", "0123456789abcdef0", "0123456789abcdeg"] {
        assert_eq!(
            put_preview(&harness, &preview, bad, 1).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "the preview refuses the epoch {bad:?}"
        );
        assert_eq!(
            put_stack_draft(&harness, &preview, bad, 1).await.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "the draft refuses the epoch {bad:?}"
        );
    }

    // The right shape is accepted on both routes.
    assert_eq!(
        put_preview(&harness, &preview, &page(7), 1).await.status(),
        StatusCode::OK
    );
}

// ================================================================ round 3 (job 75)

/// HTML whose script does not parse: the builder refuses it, so a PUT of it answers 422.
const BROKEN_HTML: &str = "<head><title>Broken</title></head><body><canvas id=\"c\"></canvas></body><script>\nvar = ;\n</script>";

/// A PUT to the draft route with one HTML file, as #2's editor sends it.
async fn put_html_draft(harness: &Harness, html: &str) -> axum::response::Response {
    let body = serde_json::json!({ "html": html, "props": {} }).to_string();
    harness
        .router
        .clone()
        .oneshot(request_with_body("PUT", routes::EFFECT_DRAFT, &body))
        .await
        .expect("the router answers")
}

/// A builder that panics once, when it is armed, and then builds as the real one does.
fn panicking_builder() -> (canvas_daemon::user_effects::Builder, Arc<std::sync::atomic::AtomicBool>) {
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&armed);
    let base = canvas_daemon::user_effects::html_builder();
    let builder: canvas_daemon::user_effects::Builder = Arc::new(move |id: &str, source: &str| {
        if flag.swap(false, Ordering::SeqCst) {
            panic!("the builder panicked");
        }
        base(id, source)
    });
    (builder, armed)
}

/// A user-effects library whose builder panics once, when it is armed, and then behaves.
/// Loading a file into the library is itself a build, so the test arms it after its setup.
fn panicking_user() -> (Arc<canvas_daemon::user_effects::UserEffects>, Arc<std::sync::atomic::AtomicBool>) {
    let (builder, armed) = panicking_builder();
    (Arc::new(canvas_daemon::user_effects::UserEffects::new(builder)), armed)
}

#[tokio::test]
async fn a_failed_draft_build_answers_its_error_on_every_beat() {
    let builds = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&builds);
    let base = canvas_daemon::user_effects::html_builder();
    let builder: canvas_daemon::user_effects::Builder = Arc::new(move |id: &str, source: &str| {
        counted.fetch_add(1, Ordering::SeqCst);
        base(id, source)
    });
    let harness = build_with_draft("r3-draft-failure", builder);
    prior_is_solid(&harness).await;

    // Three beats of a file that cannot load: each one is refused, and the route stays free
    // (the second beat answers the stored error rather than joining the dead build).
    for beat in 1..=3 {
        let response = put_html_draft(&harness, BROKEN_HTML).await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "beat {beat} of a broken draft is refused"
        );
    }
    assert_eq!(
        builds.load(Ordering::SeqCst),
        1,
        "the source that cannot load is not loaded again on every beat"
    );
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("solid".to_string()),
        "no failed build reached the lights"
    );
}

#[tokio::test]
async fn a_builder_that_panics_leaves_the_draft_route_free() {
    let (builder, armed) = panicking_builder();
    let harness = build_with_draft("r3-draft-panic", builder);
    prior_is_solid(&harness).await;

    // The first build panics: the daemon answers 500, as a daemon problem, not the file's.
    armed.store(true, Ordering::SeqCst);
    let panicked = put_html_draft(&harness, VALID_HTML).await;
    assert_eq!(panicked.status(), StatusCode::INTERNAL_SERVER_ERROR);

    // The route is free: the next beat builds and lands instead of joining the dead build.
    let landed = put_html_draft(&harness, VALID_HTML).await;
    assert_eq!(landed.status(), StatusCode::OK);
    assert_eq!(json(landed).await["applied"], true);
    let hash = canvas_daemon::user_effects::source_hash(VALID_HTML);
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some(format!("{}{hash}", canvas_proto::consts::DRAFT_ID_PREFIX)),
        "the beat after the panic reached the lights"
    );
}

#[tokio::test]
async fn a_builder_that_panics_leaves_the_preview_route_free() {
    let (user, armed) = panicking_user();
    let harness = build_with("r3-preview-panic", Some(user));
    add_html(&harness, "Spark.html", VALID_HTML);
    let preview = stack("stack:Panes", vec![layer("l1", "html:Spark")]);
    let epoch = page(0xe1);

    armed.store(true, Ordering::SeqCst);
    let panicked = put_preview(&harness, &preview, &epoch, 1).await;
    assert_eq!(panicked.status(), StatusCode::INTERNAL_SERVER_ERROR);

    // The next beat builds the same structure again and it lands on the preview engine.
    let landed = put_preview(&harness, &preview, &epoch, 2).await;
    assert_eq!(landed.status(), StatusCode::OK);
    assert_eq!(json(landed).await["applied"], true);
    assert_eq!(
        harness.preview_engine.effect_id.lock().unwrap().clone(),
        Some("stack:Panes".to_string()),
        "the beat after the panic reached the preview engine"
    );
}

#[tokio::test]
async fn a_layer_written_by_display_name_counts_as_its_html_layer_on_every_route() {
    let harness = build("r3-display-names");
    add_html(&harness, "Spark.html", VALID_HTML);
    let epoch = page(0xf1);

    // Two layers named "Spark" are the two HTML layers they become: allowed everywhere.
    let two = stack(
        "stack:TwoByName",
        vec![layer("l1", "Spark"), layer("l2", "Spark")],
    );
    assert_eq!(put_stack(&harness, &two).await, StatusCode::OK);
    assert_eq!(json(put_preview(&harness, &two, &epoch, 1).await).await["applied"], true);
    assert_eq!(json(put_stack_draft(&harness, &two, &epoch, 1).await).await["applied"], true);

    // Three are over the cap of two, on all three routes, before anything is built.
    let three = stack(
        "stack:ThreeByName",
        vec![layer("l1", "Spark"), layer("l2", "Spark"), layer("l3", "Spark")],
    );
    assert_eq!(
        put_stack(&harness, &three).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a Save counts the layers a display name becomes"
    );
    assert_eq!(
        put_preview(&harness, &three, &epoch, 2).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a preview counts them too"
    );
    assert_eq!(
        put_stack_draft(&harness, &three, &epoch, 2).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "and so does a stack draft"
    );
}

#[tokio::test]
async fn a_stack_and_its_own_name_in_another_case_are_the_same_conflict_everywhere() {
    let harness = build("r3-name-case");
    let epoch = page(0xf2);
    let saved = stack("stack:Foo", vec![layer("l1", "solid")]);
    let other_case = stack("stack:foo", vec![layer("l1", "solid")]);

    assert_eq!(put_stack(&harness, &saved).await, StatusCode::OK);

    // Save's answer for another case of the same name is a 409: the preview and the draft say
    // the same, on the same rule (the stack being edited is left out by its exact id).
    let save = put_stack(&harness, &other_case).await;
    assert_eq!(save, StatusCode::CONFLICT);
    let preview = put_preview(&harness, &other_case, &epoch, 1).await;
    assert_eq!(preview.status(), StatusCode::CONFLICT);
    let body = json(preview).await;
    assert_eq!(body["error"], "conflict");
    assert!(
        body["message"].as_str().unwrap_or_default().contains("another stack's name"),
        "the reason names the rule: {body}"
    );
    let draft = put_stack_draft(&harness, &other_case, &epoch, 1).await;
    assert_eq!(draft.status(), StatusCode::CONFLICT);

    // The saved stack itself is not another stack: both routes still take it.
    assert_eq!(
        json(put_preview(&harness, &saved, &epoch, 2).await).await["applied"],
        true
    );
    assert_eq!(
        json(put_stack_draft(&harness, &saved, &epoch, 2).await).await["applied"],
        true
    );
}

#[tokio::test]
async fn a_beat_that_joins_a_slow_build_keeps_the_draft_alive_until_it_lands() {
    let held = HeldBuild::new();
    let (user, _builds) = counting_user(Some(held.hold));
    let harness = build_with("r3-join-lease", Some(user));
    add_html(&harness, "Tinted.html", TINTED_HTML);
    add_html(&harness, "Other.html", &TINTED_HTML.replace("Tinted", "Other"));
    prior_is_solid(&harness).await;
    let epoch = page(0x31);

    // t=0: a good draft lands, with the whole lease left.
    let landed = stack("stack:Joinlease", vec![layer("l1", "html:Tinted")]);
    assert_eq!(put_stack_draft(&harness, &landed, &epoch, 1).await.status(), StatusCode::OK);
    harness.draft_clock.advance(Duration::from_secs(5));

    // t=5 s: a structure change starts a slow build, held open.
    held.armed.store(true, Ordering::SeqCst);
    let changed = stack("stack:Joinlease", vec![layer("l1", "html:Other")]);
    let slow_router = harness.router.clone();
    let slow_body = stack_draft_body(&changed, &epoch, 2);
    let slow = tokio::spawn(async move {
        slow_router
            .oneshot(request_with_body("PUT", routes::EFFECT_DRAFT, &slow_body))
            .await
            .expect("the router answers")
    });
    wait_started(held.started).await;

    // A beat of the same structure joins the build and renews the lease of the slot on the
    // lights: ten seconds were left, and the whole lease is left now.
    assert_eq!(
        json(put_stack_draft(&harness, &changed, &epoch, 3).await).await["applied"],
        true
    );
    let status: canvas_proto::api::DraftStatus = serde_json::from_value(
        json(harness.router.clone().oneshot(request("GET", routes::EFFECT_DRAFT)).await.unwrap()).await,
    )
    .expect("a DraftStatus");
    assert_eq!(
        status.lease_remaining_ms,
        canvas_proto::consts::DRAFT_LEASE.as_millis() as u64,
        "the joining beat renewed the lease"
    );

    // The clock runs past the lease while the build is still loading: the draft stays active,
    // because the page that owns it has a build in flight for its epoch.
    harness
        .draft_clock
        .advance(canvas_proto::consts::DRAFT_LEASE + Duration::from_secs(5));
    let status: canvas_proto::api::DraftStatus = serde_json::from_value(
        json(harness.router.clone().oneshot(request("GET", routes::EFFECT_DRAFT)).await.unwrap()).await,
    )
    .expect("a DraftStatus");
    assert!(status.active, "a slot does not expire under its own page's build");
    assert_eq!(
        harness.engine.effect_id.lock().unwrap().clone(),
        Some("stack:Joinlease".to_string()),
        "the prior effect was not put back"
    );

    // The build lands, and the lease starts again from the landing.
    held.go.send(()).expect("the build is released");
    let late = slow.await.expect("the task ends");
    assert_eq!(json(late).await["applied"], true);
    let status: canvas_proto::api::DraftStatus = serde_json::from_value(
        json(harness.router.clone().oneshot(request("GET", routes::EFFECT_DRAFT)).await.unwrap()).await,
    )
    .expect("a DraftStatus");
    assert!(status.active, "the build that outlived the lease landed");
    assert_eq!(
        status.lease_remaining_ms,
        canvas_proto::consts::DRAFT_LEASE.as_millis() as u64,
        "the landing set a fresh lease"
    );
}
