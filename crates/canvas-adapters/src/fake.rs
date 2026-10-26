//! A real TCP server speaking the OpenRGB SDK protocol, backed by in-memory devices.
//! Used by tests in this crate, the daemon and the tools. Never touches hardware.

use canvas_proto::{DeviceInfo, Rgb8};

pub struct FakeOpenRgbServer {
    port: u16,
}

impl FakeOpenRgbServer {
    /// Binds 127.0.0.1 on an ephemeral port and serves `devices` (zones and LED counts
    /// as given; matrix maps honored; zone resize allowed when `resizable`).
    pub async fn start(devices: Vec<DeviceInfo>) -> std::io::Result<Self> {
        let _ = devices;
        todo!("canvas-adapters: fake OpenRGB server")
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Current LED colors of controller `index`, as last written.
    pub fn colors(&self, index: u32) -> Vec<Rgb8> {
        let _ = index;
        todo!()
    }

    /// Simulates OpenRGB restarting: drops connections and forgets SDK zone resizes.
    pub async fn restart(&self) {
        todo!()
    }
}
