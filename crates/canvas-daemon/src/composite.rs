//! The daemon's one adapter: OpenRGB for most devices, plus native adapters for the
//! devices OpenRGB drives badly.
//!
//! Every caller (the supervisor, the frame sinks, readback, resize) goes through
//! `App::adapter`, so the native devices join the device list and get frames without any
//! caller knowing which adapter serves them. A device is routed by its id prefix.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_adapters::royuan::RoyuanAdapter;
use canvas_proto::device::AdapterCapabilities;
use canvas_proto::traits::{AdapterError, DeviceAdapter};
use canvas_proto::{DeviceId, DeviceInfo, Rgb8};

/// The id prefix of the ROYUAN MonsGeek keyboard's native adapter.
const ROYUAN_PREFIX: &str = "royuan:";

/// How long a missing or failed keyboard waits before the next open attempt, so an absent
/// device is not re-enumerated on every frame.
const RETRY: Duration = Duration::from_secs(5);

pub struct CompositeAdapter {
    openrgb: OpenRgbAdapter,
    royuan: RoyuanAdapter,
    /// When the keyboard was last opened (or tried).
    royuan_attempt: Option<Instant>,
}

impl CompositeAdapter {
    pub fn new(openrgb: OpenRgbAdapter) -> Self {
        Self {
            openrgb,
            royuan: RoyuanAdapter::new(),
            royuan_attempt: None,
        }
    }

    fn is_royuan(id: &DeviceId) -> bool {
        id.0.starts_with(ROYUAN_PREFIX)
    }

    /// Opens the keyboard if it is closed and the retry window has passed.
    async fn ensure_royuan(&mut self) -> Result<(), AdapterError> {
        if self.royuan.is_connected() {
            return Ok(());
        }
        let now = Instant::now();
        if self
            .royuan_attempt
            .is_some_and(|at| now.saturating_duration_since(at) < RETRY)
        {
            return Err(AdapterError::NotConnected);
        }
        self.royuan_attempt = Some(now);
        self.royuan.connect().await
    }
}

#[async_trait]
impl DeviceAdapter for CompositeAdapter {
    fn name(&self) -> &'static str {
        "composite"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        self.openrgb.capabilities()
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        // The keyboard is optional: its absence never fails the OpenRGB connection.
        let _ = self.ensure_royuan().await;
        self.openrgb.connect().await
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        let _ = self.royuan.disconnect().await;
        self.openrgb.disconnect().await
    }

    /// The link state is OpenRGB's: the supervisor reconnects OpenRGB, the keyboard
    /// reopens on its own.
    fn is_connected(&self) -> bool {
        self.openrgb.is_connected()
    }

    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
        // OpenRGB's copy of a natively driven device is dropped, so it is listed once.
        let mut devices: Vec<DeviceInfo> = self
            .openrgb
            .discover()
            .await?
            .into_iter()
            .filter(|d| crate::is_driven(&d.id))
            .collect();
        match self.royuan.discover().await {
            Ok(keyboard) if !keyboard.is_empty() => {
                if let Err(e) = self.ensure_royuan().await {
                    tracing::debug!(error = %e, "the ROYUAN keyboard could not be opened yet");
                }
                devices.extend(keyboard);
            }
            Ok(_) => {}
            Err(e) => tracing::debug!(error = %e, "the ROYUAN keyboard could not be listed"),
        }
        Ok(devices)
    }

    async fn write_frame(&mut self, device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError> {
        if Self::is_royuan(device) {
            self.ensure_royuan().await?;
            return self.royuan.write_frame(device, colors).await;
        }
        self.openrgb.write_frame(device, colors).await
    }

    async fn set_zone_size(&mut self, device: &DeviceId, zone: u32, leds: u32) -> Result<(), AdapterError> {
        if Self::is_royuan(device) {
            return self.royuan.set_zone_size(device, zone, leds).await;
        }
        self.openrgb.set_zone_size(device, zone, leds).await
    }

    async fn identify(&mut self, device: &DeviceId, zone: u32, led: u32) -> Result<(), AdapterError> {
        if Self::is_royuan(device) {
            self.ensure_royuan().await?;
            return self.royuan.identify(device, zone, led).await;
        }
        self.openrgb.identify(device, zone, led).await
    }

    async fn read_back(&mut self, device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
        if Self::is_royuan(device) {
            return self.royuan.read_back(device).await;
        }
        self.openrgb.read_back(device).await
    }
}
