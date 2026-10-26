//! OpenRGB SDK client implementing `DeviceAdapter`.

use async_trait::async_trait;
use canvas_proto::{AdapterCapabilities, AdapterError, DeviceAdapter, DeviceId, DeviceInfo, Rgb8};

pub struct OpenRgbAdapter {
    host: String,
    port: u16,
}

impl OpenRgbAdapter {
    pub fn new(host: &str, port: u16) -> Self {
        Self { host: host.to_string(), port }
    }
}

#[async_trait]
impl DeviceAdapter for OpenRgbAdapter {
    fn name(&self) -> &'static str {
        "openrgb"
    }
    fn capabilities(&self) -> AdapterCapabilities {
        let _ = (&self.host, self.port);
        todo!("canvas-adapters: openrgb")
    }
    async fn connect(&mut self) -> Result<(), AdapterError> {
        todo!()
    }
    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        todo!()
    }
    fn is_connected(&self) -> bool {
        todo!()
    }
    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
        todo!()
    }
    async fn write_frame(&mut self, _device: &DeviceId, _colors: &[Rgb8]) -> Result<(), AdapterError> {
        todo!()
    }
    async fn set_zone_size(&mut self, _device: &DeviceId, _zone: u32, _leds: u32) -> Result<(), AdapterError> {
        todo!()
    }
    async fn identify(&mut self, _device: &DeviceId, _zone: u32, _led: u32) -> Result<(), AdapterError> {
        todo!()
    }
    async fn read_back(&mut self, _device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
        todo!()
    }
}
