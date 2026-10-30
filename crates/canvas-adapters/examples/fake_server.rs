//! Serves a device list (the JSON of `canvas-rgb-cli devices list --json`) from the fake OpenRGB
//! server, on the port given as the second argument. Never touches hardware.
//! Usage: fake_server <devices.json> <port>
use canvas_adapters::fake::FakeOpenRgbServer;
use canvas_proto::DeviceInfo;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("devices.json");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let all: Vec<DeviceInfo> = serde_json::from_value(value["devices"].clone()).unwrap();
    let devices: Vec<DeviceInfo> = all.into_iter().filter(|d| d.adapter == "openrgb").collect();
    println!("serving {} devices", devices.len());
    let server = FakeOpenRgbServer::start(devices).await.unwrap();
    println!("fake OpenRGB on 127.0.0.1:{}", server.port());
    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
}
