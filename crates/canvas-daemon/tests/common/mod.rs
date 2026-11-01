//! In-test fakes of the `canvas-proto` traits.
//!
//! The daemon's unit tests never call the real `canvas-core`, `canvas-effects` or
//! `canvas-adapters` crates: they build the daemon's state from these fakes instead, so
//! the tests do not depend on another area's code being finished.
//!
//! Not every test file uses every fake, so the dead-code warnings are off here.
#![allow(dead_code)]

use canvas_proto::device::{AdapterCapabilities, DeviceId, DeviceInfo, ZoneInfo, ZoneKind};
use canvas_proto::peripheral::{
    PeripheralInfo, PeripheralKind, PeripheralSchema, PeripheralSection, ReadProgress,
};
use canvas_proto::props::{PropKind, PropValue, Props, PropertyDef, PropertySchema};
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
        blocked_by: None,
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

/// What a [`FakePeripheral`] was asked to do, and what it did. The tests read the counters to
/// check that the daemon opened nothing it was not told to.
#[derive(Debug, Clone)]
pub struct PeripheralLog {
    /// The peripherals the enumeration reports. Empty means the device is not plugged in.
    pub found: Vec<PeripheralInfo>,
    pub connected: bool,
    pub connects: u32,
    pub disconnects: u32,
    pub discovers: u32,
    pub describes: u32,
    /// The reads that were answered.
    pub reads: u32,
    /// The writes that were accepted, in order.
    pub writes: Vec<Props>,
    /// Whether the adapter may write. The daemon turns it with the saved access.
    pub writes_enabled: bool,
    /// The values a read returns.
    pub values: Props,
    /// The schema the fake describes.
    pub schema: PeripheralSchema,
    /// The stages a read reports through the sink, in order. Empty: the fake reports none.
    pub stages: Vec<ReadProgress>,
    /// Where the daemon asked the read to report (`set_read_sink`).
    pub sink: SinkSlot,
    /// When set, a read stops after each reported stage until the test lets it go on.
    pub gate: Option<StageGate>,
    /// The gate also holds `connect`, so the "opening" stage can be looked at.
    pub gate_connect: bool,
    /// Makes this many reads fail (after they reported their stages), as a pulled cable does.
    pub fail_read: u32,
}

/// The sink the daemon gave the fake. `ReadSink` has no `Debug`, so the log holds it in here.
#[derive(Clone, Default)]
pub struct SinkSlot(pub Option<canvas_adapters::ReadSink>);

impl std::fmt::Debug for SinkSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SinkSlot({})", if self.0.is_some() { "set" } else { "none" })
    }
}

/// Holds a read between its stages so a test can look at the stage while the read is open: the
/// read sends on `reached` after each stage and waits for a permit on `go`.
#[derive(Clone, Debug)]
pub struct StageGate {
    pub reached: tokio::sync::mpsc::UnboundedSender<()>,
    pub go: Arc<tokio::sync::Semaphore>,
}

/// The id of the fake peripheral.
pub fn fake_peripheral_id() -> DeviceId {
    DeviceId("fake-peripheral:0001:0001".into())
}

/// The schema of the fake peripheral: two DPI stages, a colour and a read-only profile.
pub fn fake_peripheral_schema(device: &DeviceId) -> PeripheralSchema {
    PeripheralSchema {
        device: device.clone(),
        kind: PeripheralKind::Mouse,
        sections: vec![PeripheralSection {
            id: "dpi".into(),
            title: "DPI".into(),
            properties: vec![
                PropertyDef {
                    key: "dpi-stage-1".into(),
                    label: "DPI stage 1".into(),
                    kind: PropKind::Number { min: 100.0, max: 20000.0, step: 50.0, default: 800.0 },
                },
                PropertyDef {
                    key: "dpi-stage-2".into(),
                    label: "DPI stage 2".into(),
                    kind: PropKind::Number { min: 100.0, max: 20000.0, step: 50.0, default: 1600.0 },
                },
                PropertyDef {
                    key: "led-colour".into(),
                    label: "LED colour".into(),
                    kind: PropKind::Color { default: canvas_proto::Rgb8::new(0xEF, 0x3F, 0xFF) },
                },
                PropertyDef {
                    key: "profile".into(),
                    label: "Profile".into(),
                    kind: PropKind::Choice { options: vec!["1".into(), "2".into()], default: "1".into() },
                },
            ],
        }],
        read_only: vec!["profile".into()],
        unreported: vec!["lift-off".into()],
    }
}

/// A peripheral that records everything. It never opens a device.
#[derive(Clone)]
pub struct FakePeripheral(pub Arc<Mutex<PeripheralLog>>);

impl Default for FakePeripheral {
    fn default() -> Self {
        let device = fake_peripheral_id();
        let schema = fake_peripheral_schema(&device);
        let found = vec![PeripheralInfo {
            id: device.clone(),
            name: "Fake Mouse".into(),
            vendor: "Fake".into(),
            kind: PeripheralKind::Mouse,
            adapter: "fake-peripheral".into(),
            serial: String::new(),
            location: "hid:0001:0001:usage0b".into(),
            connected: false,
        }];
        let mut values = Props::new();
        values.insert("dpi-stage-1".into(), PropValue::Number(800.0));
        values.insert("dpi-stage-2".into(), PropValue::Number(1600.0));
        values.insert("led-colour".into(), PropValue::Color(canvas_proto::Rgb8::new(0xEF, 0x3F, 0xFF)));
        Self(Arc::new(Mutex::new(PeripheralLog {
            found,
            connected: false,
            connects: 0,
            disconnects: 0,
            discovers: 0,
            describes: 0,
            reads: 0,
            writes: Vec::new(),
            writes_enabled: false,
            values,
            schema,
            stages: Vec::new(),
            sink: SinkSlot(None),
            gate: None,
            gate_connect: false,
            fail_read: 0,
        })))
    }
}

impl FakePeripheral {
    /// The id of the fake device.
    pub fn id(&self) -> DeviceId {
        self.0.lock().unwrap().schema.device.clone()
    }

    /// What the fake was asked to do.
    pub fn log(&self) -> PeripheralLog {
        self.0.lock().unwrap().clone()
    }
}

/// Checks a value against its kind, the way the real adapters do.
fn check_prop_value(def: &PropertyDef, value: &PropValue) -> Result<(), AdapterError> {
    let wrong = |value: &PropValue| {
        AdapterError::Protocol(format!(
            "the setting {} does not take that kind of value: {value:?}",
            def.key
        ))
    };
    match (&def.kind, value) {
        (PropKind::Number { min, max, .. }, PropValue::Number(number)) => {
            if number < min || number > max {
                return Err(AdapterError::Protocol(format!(
                    "{number} is outside {min}..={max}"
                )));
            }
            Ok(())
        }
        (PropKind::Number { .. }, other) => Err(wrong(other)),
        (PropKind::Bool { .. }, PropValue::Bool(_)) => Ok(()),
        (PropKind::Bool { .. }, other) => Err(wrong(other)),
        (PropKind::Choice { options, .. }, PropValue::Choice(choice)) => {
            if !options.contains(choice) {
                return Err(AdapterError::Protocol(format!(
                    "{choice} is not one of {options:?}"
                )));
            }
            Ok(())
        }
        (PropKind::Choice { .. }, other) => Err(wrong(other)),
        (PropKind::Color { .. }, PropValue::Color(_)) => Ok(()),
        (PropKind::Color { .. }, other) => Err(wrong(other)),
        (PropKind::Hue { .. }, PropValue::Hue(hue)) => {
            if !(0.0..=360.0).contains(hue) {
                return Err(AdapterError::Protocol(format!("a hue must be 0..=360, got {hue}")));
            }
            Ok(())
        }
        (PropKind::Hue { .. }, other) => Err(wrong(other)),
    }
}

#[async_trait::async_trait]
impl canvas_proto::traits::PeripheralAdapter for FakePeripheral {
    fn name(&self) -> &'static str {
        "fake-peripheral"
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        // A gated connect stops like a slow open does, so a test can look at the "opening" stage.
        let gate = {
            let log = self.0.lock().unwrap();
            if log.gate_connect { log.gate.clone() } else { None }
        };
        if let Some(gate) = gate {
            let _ = gate.reached.send(());
            gate.go.acquire().await.expect("the gate stays open").forget();
        }
        let mut log = self.0.lock().unwrap();
        log.connects += 1;
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

    async fn discover(&mut self) -> Result<Vec<PeripheralInfo>, AdapterError> {
        let mut log = self.0.lock().unwrap();
        log.discovers += 1;
        Ok(log.found.clone())
    }

    async fn describe(&mut self, device: &DeviceId) -> Result<PeripheralSchema, AdapterError> {
        let mut log = self.0.lock().unwrap();
        log.describes += 1;
        if &log.schema.device != device {
            return Err(AdapterError::UnknownDevice(device.clone()));
        }
        Ok(log.schema.clone())
    }

    async fn read_settings(&mut self, device: &DeviceId) -> Result<Props, AdapterError> {
        let (stages, sink, gate) = {
            let mut log = self.0.lock().unwrap();
            if &log.schema.device != device {
                return Err(AdapterError::UnknownDevice(device.clone()));
            }
            if !log.connected {
                return Err(AdapterError::NotConnected);
            }
            log.reads += 1;
            (log.stages.clone(), log.sink.0.clone(), log.gate.clone())
        };
        // The way the real adapter does it: report each stage, and carry on.
        for stage in stages {
            if let Some(sink) = &sink {
                sink(stage);
            }
            if let Some(gate) = &gate {
                let _ = gate.reached.send(());
                gate.go.acquire().await.expect("the gate stays open").forget();
            }
        }
        let mut log = self.0.lock().unwrap();
        if log.fail_read > 0 {
            log.fail_read -= 1;
            return Err(AdapterError::Io(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "read failed")));
        }
        Ok(log.values.clone())
    }

    async fn write_settings(&mut self, device: &DeviceId, props: &Props) -> Result<(), AdapterError> {
        let mut log = self.0.lock().unwrap();
        if &log.schema.device != device {
            return Err(AdapterError::UnknownDevice(device.clone()));
        }
        if !log.writes_enabled {
            return Err(AdapterError::Unsupported(
                "config writes are off: turn them on with set_writes_enabled(true) first",
            ));
        }
        if props.is_empty() {
            return Ok(());
        }
        // The adapters' own validation: an unknown key, a read-only key or a value out of
        // range is refused, and nothing is sent.
        for (key, value) in props {
            let Some(def) = log.schema.property(key) else {
                return Err(AdapterError::Protocol(format!(
                    "the device has no setting called {key}"
                )));
            };
            if !log.schema.is_writable(key) {
                return Err(AdapterError::Protocol(format!("{key} is read-only")));
            }
            check_prop_value(def, value)?;
        }
        log.writes.push(props.clone());
        Ok(())
    }

    fn writes_enabled(&self) -> bool {
        self.0.lock().unwrap().writes_enabled
    }
}

impl canvas_daemon::peripherals::Peripheral for FakePeripheral {
    fn set_config_writes(&mut self, on: bool) {
        self.0.lock().unwrap().writes_enabled = on;
    }

    fn set_read_sink(&mut self, sink: Option<canvas_adapters::ReadSink>) {
        self.0.lock().unwrap().sink = SinkSlot(sink);
    }
}
