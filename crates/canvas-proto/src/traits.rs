//! The seams between the engine, the effects, the adapters and the daemon.
//!
//! Exported entry points each area crate provides (signatures are fixed by the spine):
//! - `canvas_core::spawn(settings: EngineSettings) -> std::sync::Arc<dyn Engine>`
//! - `canvas_effects::registry() -> Vec<EffectFactory>` (exactly the 12 in `EFFECT_IDS`)
//! - `canvas_effects::create(id: &str) -> Option<Box<dyn Effect>>`
//! - `canvas_html::HtmlEffect::from_source(id, html, default_overrides) -> Result<HtmlEffect, HtmlError>`
//!   (implements [`Effect`]; runs SignalRGB-format `effect.html` with an embedded JS engine and
//!   a Rust Canvas 2D. No browser engine.) Poison ships as SignalRGB's original file through it.
//!   User HTML effects load from `%APPDATA%\canvas-rgb\effects\*.html` (id = file stem).
//! - `canvas_adapters::openrgb::OpenRgbAdapter::new(host: &str, port: u16) -> OpenRgbAdapter`
//!   (implements [`DeviceAdapter`])
//! - `canvas_adapters::fake::FakeOpenRgbServer::start(devices) -> FakeOpenRgbServer`
//!   (a real TCP server speaking the OpenRGB SDK protocol, for tests)
//! - `canvas_adapters::zones::ZoneStore` (zones.json load/save/apply)

use crate::api::Power;
use crate::color::Rgb8;
use crate::device::{AdapterCapabilities, DeviceId, DeviceInfo};
use crate::layout::{Layout, CANVAS_HEIGHT, CANVAS_WIDTH};
use crate::props::{Props, PropertySchema};
use async_trait::async_trait;
use std::sync::Arc;

// ---------------------------------------------------------------- canvas

/// Linear RGB float buffer, 0..1 per channel, row-major. `width`/`height` are the render
/// size (logical 320x200 times the render scale). Effects draw in logical coordinates
/// and multiply by [`Canvas::scale`].
#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 3]>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![[0.0; 3]; (width * height) as usize] }
    }

    /// Logical canvas at a render-scale factor (1.0, 0.5, 0.25).
    pub fn with_scale(factor: f32) -> Self {
        let w = ((CANVAS_WIDTH as f32 * factor).round() as u32).max(1);
        let h = ((CANVAS_HEIGHT as f32 * factor).round() as u32).max(1);
        Self::new(w, h)
    }

    /// Render pixels per logical unit.
    pub fn scale(&self) -> f32 {
        self.width as f32 / CANVAS_WIDTH as f32
    }

    pub fn get(&self, x: u32, y: u32) -> [f32; 3] {
        self.pixels[(y * self.width + x) as usize]
    }

    pub fn set(&mut self, x: u32, y: u32, c: [f32; 3]) {
        let i = (y * self.width + x) as usize;
        self.pixels[i] = c;
    }

    pub fn fill(&mut self, c: [f32; 3]) {
        self.pixels.fill(c);
    }
}

// ---------------------------------------------------------------- inputs

#[derive(Debug, Clone)]
pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<Rgb8>,
}

/// Log-spaced bands from WASAPI loopback FFT. `magnitudes[i]` is 0..1 for the band
/// centered on `centers_hz[i]`.
#[derive(Debug, Clone)]
pub struct AudioSpectrum {
    pub centers_hz: Vec<f32>,
    pub magnitudes: Vec<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct Inputs {
    /// Seconds since the effect was activated.
    pub time: f64,
    pub screen: Option<Arc<ScreenFrame>>,
    pub audio: Option<Arc<AudioSpectrum>>,
}

/// Supplies capture data to the engine. The daemon implements this.
pub trait InputSource: Send + Sync {
    fn screen(&self) -> Option<Arc<ScreenFrame>>;
    fn audio(&self) -> Option<Arc<AudioSpectrum>>;
}

// ---------------------------------------------------------------- effects

pub trait Effect: Send {
    /// Declared once; the editor renders controls from it.
    fn schema(&self) -> PropertySchema;
    /// Called on activation and on every props change. Missing keys take schema defaults.
    fn init(&mut self, props: &Props);
    /// Draws one frame. `dt` is seconds since the previous frame.
    fn update(&mut self, dt: f32, canvas: &mut Canvas, inputs: &Inputs);
}

pub type EffectFactory = fn() -> Box<dyn Effect>;

// ---------------------------------------------------------------- engine

/// Receives one device's frame, in the device's zone order. Must not block: a slow
/// device drops frames, it never stalls the loop.
pub trait FrameSink: Send + Sync {
    fn submit(&self, colors: Vec<Rgb8>);
}

/// Receives the downscaled canvas for the editor preview (≤ 15 fps).
pub trait PreviewSink: Send + Sync {
    fn submit(&self, width: u32, height: u32, rgb: Vec<u8>);
}

#[derive(Debug, Clone)]
pub struct EngineSettings {
    pub fps: u32,
    pub power: Power,
    /// 0..100, applied after sampling.
    pub brightness: u8,
}

#[derive(Debug, Clone, Default)]
pub struct RenderStats {
    pub fps_measured: f32,
    pub frame_time_ms: f32,
    pub frames_rendered: u64,
    pub parked: bool,
    pub sinks: u32,
    pub preview: bool,
}

/// The render loop handle. One thread, fixed-step at `fps`. Parks with no sinks and no
/// preview. `Paused` writes black to every sink once, then stops rendering.
pub trait Engine: Send + Sync {
    /// Recomputes per-LED sample regions. `devices` supplies zone topology and LED counts.
    fn set_layout(&self, layout: Layout, devices: Vec<DeviceInfo>);
    fn set_effect(&self, effect: Box<dyn Effect>, props: &Props);
    fn set_props(&self, props: &Props);
    fn set_fps(&self, fps: u32);
    fn set_power(&self, power: Power);
    fn set_brightness(&self, brightness: u8);
    fn add_sink(&self, device: DeviceId, sink: Arc<dyn FrameSink>);
    fn remove_sink(&self, device: &DeviceId);
    fn set_preview(&self, sink: Option<Arc<dyn PreviewSink>>);
    fn set_input_source(&self, source: Option<Arc<dyn InputSource>>);
    /// Writes one solid color to every sink now (quit, sleep, identify background).
    fn write_solid(&self, color: Rgb8);
    fn stats(&self) -> RenderStats;
    /// Stops the thread. Does not write anything; callers write black first if wanted.
    fn shutdown(&self);
}

// ---------------------------------------------------------------- adapters

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("not connected")]
    NotConnected,
    #[error("unknown device {0}")]
    UnknownDevice(DeviceId),
    #[error("zone {zone} of {device}: {reason}")]
    Zone { device: DeviceId, zone: u32, reason: String },
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[async_trait]
pub trait DeviceAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> AdapterCapabilities;
    async fn connect(&mut self) -> Result<(), AdapterError>;
    async fn disconnect(&mut self) -> Result<(), AdapterError>;
    fn is_connected(&self) -> bool;
    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError>;
    /// Puts the device in direct/custom mode if needed and writes all LEDs.
    async fn write_frame(&mut self, device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError>;
    async fn set_zone_size(&mut self, device: &DeviceId, zone: u32, leds: u32) -> Result<(), AdapterError>;
    /// One white LED at `led` in `zone`, everything else on the device black.
    async fn identify(&mut self, device: &DeviceId, zone: u32, led: u32) -> Result<(), AdapterError>;
    /// Colors as the hardware side reports them (OpenRGB REQUEST_CONTROLLER_DATA).
    async fn read_back(&mut self, device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError>;
}
