//! In-test fakes of the `canvas-proto` traits.
//!
//! The daemon's unit tests never call the real `canvas-core`, `canvas-effects` or
//! `canvas-adapters` crates: they build the daemon's state from these fakes instead, so
//! the tests do not depend on another area's code being finished.
//!
//! Not every test file uses every fake, so the dead-code warnings are off here.
#![allow(dead_code)]

use canvas_proto::device::{AdapterCapabilities, DeviceId, DeviceInfo, ZoneInfo, ZoneKind};
use canvas_proto::props::{Props, PropertySchema};
use canvas_proto::traits::{
    AdapterError, AudioSpectrum, Canvas, DeviceAdapter, Effect, Engine, EngineSettings, FrameSink,
    InputSource, PreviewSink, RenderStats, ScreenFrame,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A test engine that records what it was told.
pub struct FakeEngine {
    pub layout: Mutex<Option<(canvas_proto::layout::Layout, Vec<DeviceInfo>)>>,
    pub effect_id: Mutex<Option<String>>,
    pub props: Mutex<Props>,
    pub fps: AtomicU32,
    pub power: Mutex<canvas_proto::api::Power>,
    pub brightness: AtomicU32,
    pub preview: Mutex<Option<Arc<dyn PreviewSink>>>,
    pub sinks: Mutex<BTreeMap<DeviceId, Arc<dyn FrameSink>>>,
    pub input_source: Mutex<Option<Arc<dyn InputSource>>>,
    pub solid_writes: Mutex<Vec<canvas_proto::Rgb8>>,
    pub frames: AtomicU64,
    pub shutdown: AtomicU32,
    pub stats: Mutex<RenderStats>,
}

impl Default for FakeEngine {
    fn default() -> Self {
        Self {
            layout: Mutex::new(None),
            effect_id: Mutex::new(None),
            props: Mutex::new(Props::new()),
            fps: AtomicU32::new(30),
            power: Mutex::new(canvas_proto::api::Power::Running),
            brightness: AtomicU32::new(100),
            preview: Mutex::new(None),
            sinks: Mutex::new(BTreeMap::new()),
            input_source: Mutex::new(None),
            solid_writes: Mutex::new(Vec::new()),
            frames: AtomicU64::new(0),
            shutdown: AtomicU32::new(0),
            stats: Mutex::new(RenderStats::default()),
        }
    }
}

impl Engine for FakeEngine {
    fn set_layout(&self, layout: canvas_proto::layout::Layout, devices: Vec<DeviceInfo>) {
        *self.layout.lock().unwrap() = Some((layout, devices));
    }

    fn set_effect(&self, effect: Box<dyn Effect>, props: &Props) {
        *self.effect_id.lock().unwrap() = Some(effect.schema().id);
        *self.props.lock().unwrap() = props.clone();
    }

    fn set_props(&self, props: &Props) {
        *self.props.lock().unwrap() = props.clone();
    }

    fn set_fps(&self, fps: u32) {
        self.fps.store(fps, Ordering::SeqCst);
    }

    fn set_power(&self, power: canvas_proto::api::Power) {
        *self.power.lock().unwrap() = power;
    }

    fn set_brightness(&self, brightness: u8) {
        self.brightness.store(brightness as u32, Ordering::SeqCst);
    }

    fn add_sink(&self, device: DeviceId, sink: Arc<dyn FrameSink>) {
        self.sinks.lock().unwrap().insert(device, sink);
    }

    fn remove_sink(&self, device: &DeviceId) {
        self.sinks.lock().unwrap().remove(device);
    }

    fn set_preview(&self, sink: Option<Arc<dyn PreviewSink>>) {
        *self.preview.lock().unwrap() = sink;
    }

    fn set_input_source(&self, source: Option<Arc<dyn InputSource>>) {
        *self.input_source.lock().unwrap() = source;
    }

    fn write_solid(&self, color: canvas_proto::Rgb8) {
        self.solid_writes.lock().unwrap().push(color);
    }

    fn stats(&self) -> RenderStats {
        self.stats.lock().unwrap().clone()
    }

    fn shutdown(&self) {
        self.shutdown.store(1, Ordering::SeqCst);
    }
}

/// A test adapter that records the frames it was given.
pub struct FakeAdapter {
    pub connected: Mutex<bool>,
    pub devices: Mutex<Vec<DeviceInfo>>,
    pub frames: Mutex<Vec<(DeviceId, Vec<canvas_proto::Rgb8>)>>,
    pub zone_sizes: Mutex<Vec<(DeviceId, u32, u32)>>,
    pub identified: Mutex<Vec<(DeviceId, u32, u32)>>,
    pub read_back: Mutex<Vec<canvas_proto::Rgb8>>,
}

impl Default for FakeAdapter {
    fn default() -> Self {
        Self {
            connected: Mutex::new(false),
            devices: Mutex::new(Vec::new()),
            frames: Mutex::new(Vec::new()),
            zone_sizes: Mutex::new(Vec::new()),
            identified: Mutex::new(Vec::new()),
            read_back: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl DeviceAdapter for FakeAdapter {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            resizable_zones: true,
            identify: true,
            hardware_brightness: false,
            needs_elevation: false,
        }
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        *self.connected.lock().unwrap() = true;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        *self.connected.lock().unwrap() = false;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        *self.connected.lock().unwrap()
    }

    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
        Ok(self.devices.lock().unwrap().clone())
    }

    async fn write_frame(
        &mut self,
        device: &DeviceId,
        colors: &[canvas_proto::Rgb8],
    ) -> Result<(), AdapterError> {
        self.frames.lock().unwrap().push((device.clone(), colors.to_vec()));
        Ok(())
    }

    async fn set_zone_size(
        &mut self,
        device: &DeviceId,
        zone: u32,
        leds: u32,
    ) -> Result<(), AdapterError> {
        self.zone_sizes.lock().unwrap().push((device.clone(), zone, leds));
        Ok(())
    }

    async fn identify(&mut self, device: &DeviceId, zone: u32, led: u32) -> Result<(), AdapterError> {
        self.identified.lock().unwrap().push((device.clone(), zone, led));
        Ok(())
    }

    async fn read_back(&mut self, _device: &DeviceId) -> Result<Vec<canvas_proto::Rgb8>, AdapterError> {
        Ok(self.read_back.lock().unwrap().clone())
    }
}

/// A test effect with a schema of its own.
pub struct FakeEffect {
    pub schema: PropertySchema,
    pub updates: AtomicU64,
}

impl FakeEffect {
    pub fn new(id: &str) -> Self {
        Self {
            schema: PropertySchema {
                id: id.to_string(),
                name: id.to_string(),
                description: String::new(),
                needs: Default::default(),
                properties: Vec::new(),
                presets: Vec::new(),
            },
            updates: AtomicU64::new(0),
        }
    }
}

impl Effect for FakeEffect {
    fn schema(&self) -> PropertySchema {
        self.schema.clone()
    }

    fn init(&mut self, _props: &Props) {}

    fn update(&mut self, _dt: f32, _canvas: &mut Canvas, _inputs: &canvas_proto::traits::Inputs) {
        self.updates.fetch_add(1, Ordering::SeqCst);
    }
}

/// An input source with fixed values.
pub struct FakeInputs {
    pub screen: Option<Arc<ScreenFrame>>,
    pub audio: Option<Arc<AudioSpectrum>>,
}

impl FakeInputs {
    pub fn empty() -> Self {
        Self { screen: None, audio: None }
    }
}

impl InputSource for FakeInputs {
    fn screen(&self) -> Option<Arc<ScreenFrame>> {
        self.screen.clone()
    }

    fn audio(&self) -> Option<Arc<AudioSpectrum>> {
        self.audio.clone()
    }
}

/// A device with one zone.
pub fn device(name: &str, kind: canvas_proto::device::DeviceKind, leds: u32) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(format!("fake:{name}")),
        name: name.to_string(),
        vendor: "fake".to_string(),
        kind,
        adapter: "fake".to_string(),
        adapter_index: 0,
        serial: name.to_string(),
        location: String::new(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "Zone 1".to_string(),
            kind: ZoneKind::Linear,
            led_count: leds,
            min_leds: 1,
            max_leds: 512,
            resizable: true,
        }],
        connected: true,
    }
}

/// The engine settings the daemon starts with.
pub fn engine_settings() -> EngineSettings {
    EngineSettings { fps: 30, power: canvas_proto::api::Power::Running, brightness: 100 }
}

/// What a [`FakeDisplay`] was asked to do, and what it did. Tests set the `found`, `fail_*` and
/// `refuse_at` knobs before the call under test and read the counters after it.
#[derive(Debug, Default)]
pub struct DisplayLog {
    /// The displays the enumeration reports.
    pub found: Vec<canvas_proto::display::DisplayInfo>,
    pub connected: bool,
    pub connects: u32,
    pub disconnects: u32,
    pub discovers: u32,
    /// Accepted writes only, in order.
    pub writes: Vec<(DeviceId, canvas_proto::display::DisplayValue)>,
    /// Makes this many connects fail.
    pub fail_connect: u32,
    /// Makes this many writes fail. A failed write drops the connection, as the CHIONE does.
    pub fail_write: u32,
    /// Refuses a value at or above this. A refusal keeps the connection and writes nothing.
    pub refuse_at: Option<f32>,
}

/// A numeric display that records everything. Never opens a device.
pub struct FakeDisplay(pub std::sync::Arc<Mutex<DisplayLog>>);

/// The CHIONE as the enumeration reports it.
pub fn chione_info() -> canvas_proto::display::DisplayInfo {
    canvas_proto::display::DisplayInfo {
        id: DeviceId("chione:1b80:b53c".into()),
        name: "GAMDIAS CHIONE Digital Display".into(),
        digits: 4,
        max_decimals: 1,
        modes: vec![
            canvas_proto::display::DisplayMode::Temperature,
            canvas_proto::display::DisplayMode::FanRpm,
        ],
        connected: false,
    }
}

#[async_trait::async_trait]
impl canvas_proto::traits::NumericDisplayAdapter for FakeDisplay {
    fn name(&self) -> &'static str {
        "fake-display"
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        let mut log = self.0.lock().unwrap();
        log.connects += 1;
        if log.fail_connect > 0 {
            log.fail_connect -= 1;
            return Err(AdapterError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "no reply")));
        }
        if log.found.is_empty() {
            return Err(AdapterError::NotConnected);
        }
        log.connected = true;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        let mut log = self.0.lock().unwrap();
        log.disconnects += 1;
        log.connected = false;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.0.lock().unwrap().connected
    }

    async fn discover(&mut self) -> Result<Vec<canvas_proto::display::DisplayInfo>, AdapterError> {
        let mut log = self.0.lock().unwrap();
        log.discovers += 1;
        Ok(log.found.clone())
    }

    async fn write_value(
        &mut self,
        device: &DeviceId,
        value: &canvas_proto::display::DisplayValue,
    ) -> Result<(), AdapterError> {
        let mut log = self.0.lock().unwrap();
        if !log.connected {
            return Err(AdapterError::NotConnected);
        }
        if log.fail_write > 0 {
            log.fail_write -= 1;
            log.connected = false;
            return Err(AdapterError::Io(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "write failed")));
        }
        if let (Some(at), Some(v)) = (log.refuse_at, value.value) {
            if v >= at {
                return Err(AdapterError::Protocol(format!("{v} does not fit four digits")));
            }
        }
        log.writes.push((device.clone(), *value));
        Ok(())
    }

    fn write_interval_ms(&self) -> u32 {
        250
    }
}
