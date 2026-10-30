//! A real TCP server speaking the OpenRGB SDK protocol, backed by in-memory devices.
//! Used by tests in this crate, the daemon and the tools. Never touches hardware.
//! It binds 127.0.0.1 only.

use canvas_proto::{DeviceInfo, DeviceKind, Rgb8, ZoneKind};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{AbortHandle, JoinHandle};

use crate::protocol::*;

pub struct FakeOpenRgbServer {
    port: u16,
    state: Arc<Mutex<FakeState>>,
    connections: Arc<Mutex<Vec<AbortHandle>>>,
    acceptor: JoinHandle<()>,
}

struct FakeState {
    devices: Vec<FakeDevice>,
    /// When set, device `i` is announced only once `first + i * step` has passed since `since`,
    /// as real OpenRGB finds its controllers one by one while it detects.
    detection: Option<Detection>,
}

#[derive(Clone, Copy)]
struct Detection {
    since: std::time::Instant,
    first: std::time::Duration,
    step: std::time::Duration,
}

impl FakeState {
    /// How many controllers the server announces right now.
    fn visible(&self) -> usize {
        match self.detection {
            None => self.devices.len(),
            Some(d) => {
                let elapsed = d.since.elapsed();
                (0..self.devices.len())
                    .filter(|i| elapsed >= d.first + d.step * (*i as u32))
                    .count()
            }
        }
    }
}

struct FakeDevice {
    info: DeviceInfo,
    /// Zone sizes as configured at start; `restart()` returns resizable zones to these.
    initial_zone_leds: Vec<u32>,
    colors: Vec<Rgb8>,
    custom_mode_writes: u32,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeOpenRgbServer {
    /// Binds 127.0.0.1 on an ephemeral port and serves `devices` in order (controller index =
    /// position). Zone LED counts and matrix maps are reported as given; a zone accepts
    /// RESIZEZONE only when its `resizable` flag is set and its size range is non-empty.
    pub async fn start(devices: Vec<DeviceInfo>) -> std::io::Result<Self> {
        Self::start_inner(devices, None).await
    }

    /// Like [`start`](Self::start), but the devices appear over time, as in a real OpenRGB
    /// that is still detecting: the controller count is 0 until `first` has passed, and device
    /// `i` is announced `first + i * step` after the server started.
    pub async fn start_detecting(
        devices: Vec<DeviceInfo>,
        first: std::time::Duration,
        step: std::time::Duration,
    ) -> std::io::Result<Self> {
        let detection = Detection { since: std::time::Instant::now(), first, step };
        Self::start_inner(devices, Some(detection)).await
    }

    async fn start_inner(devices: Vec<DeviceInfo>, detection: Option<Detection>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let devices = devices.into_iter().map(|info| {
            let colors = vec![Rgb8 { r: 0, g: 0, b: 0 }; info.led_count() as usize];
            let initial_zone_leds = info.zones.iter().map(|zone| zone.led_count).collect();
            FakeDevice { info, initial_zone_leds, colors, custom_mode_writes: 0 }
        }).collect();
        let state = Arc::new(Mutex::new(FakeState { devices, detection }));
        let connections: Arc<Mutex<Vec<AbortHandle>>> = Arc::new(Mutex::new(Vec::new()));
        let acceptor = {
            let state = Arc::clone(&state);
            let connections = Arc::clone(&connections);
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let handle = tokio::spawn(serve(stream, Arc::clone(&state)));
                    let mut open = guard(&connections);
                    open.retain(|conn| !conn.is_finished());
                    open.push(handle.abort_handle());
                }
            })
        };
        Ok(Self { port, state, connections, acceptor })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Current LED colors of controller `index`, as last written.
    pub fn colors(&self, index: u32) -> Vec<Rgb8> {
        guard(&self.state).devices.get(index as usize).map(|device| device.colors.clone()).unwrap_or_default()
    }

    /// How many times SETCUSTOMMODE was received for controller `index`.
    pub fn custom_mode_writes(&self, index: u32) -> u32 {
        guard(&self.state).devices.get(index as usize).map_or(0, |device| device.custom_mode_writes)
    }

    /// Current LED count of zone `zone` on controller `index`.
    pub fn zone_leds(&self, index: u32, zone: u32) -> Option<u32> {
        guard(&self.state).devices.get(index as usize)?.info.zones.get(zone as usize).map(|z| z.led_count)
    }

    /// Simulates OpenRGB restarting: every open connection is dropped, and SDK zone resizes
    /// are forgotten (resizable zones return to their starting size, as real OpenRGB does).
    pub async fn restart(&self) {
        {
            let mut state = guard(&self.state);
            for device in &mut state.devices {
                for (zone, initial) in device.info.zones.iter_mut().zip(&device.initial_zone_leds) {
                    zone.led_count = *initial;
                }
                device.colors = vec![Rgb8 { r: 0, g: 0, b: 0 }; device.info.led_count() as usize];
                device.custom_mode_writes = 0;
            }
        }
        for connection in guard(&self.connections).drain(..) {
            connection.abort();
        }
    }
}

impl Drop for FakeOpenRgbServer {
    fn drop(&mut self) {
        self.acceptor.abort();
        for connection in guard(&self.connections).drain(..) {
            connection.abort();
        }
    }
}

async fn serve(mut stream: TcpStream, state: Arc<Mutex<FakeState>>) -> std::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        loop {
            if buffer.len() < HEADER_LEN {
                break;
            }
            let mut header = [0_u8; HEADER_LEN];
            header.copy_from_slice(&buffer[..HEADER_LEN]);
            let (device, packet_id, size) = decode_header(&header).map_err(to_io)?;
            if buffer.len() < HEADER_LEN + size {
                break;
            }
            let payload = buffer[HEADER_LEN..HEADER_LEN + size].to_vec();
            buffer.drain(..HEADER_LEN + size);
            let reply = handle(&state, device, packet_id, &payload);
            if !reply.is_empty() {
                stream.write_all(&reply).await?;
                stream.flush().await?;
            }
        }
    }
}

fn to_io(error: canvas_proto::AdapterError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}

/// Handles one client packet. Returns the bytes to send back (empty when there is no reply).
fn handle(state: &Mutex<FakeState>, device: u32, packet_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut state = guard(state);
    let reply = |device: u32, packet_id: u32, body: &[u8]| encode_packet(device, packet_id, body).unwrap_or_default();
    match packet_id {
        REQUEST_PROTOCOL_VERSION => {
            let client = payload.get(..4).map_or(CLIENT_MAX_PROTOCOL_VERSION, |b| {
                u32::from_le_bytes([b[0], b[1], b[2], b[3]])
            });
            reply(0, packet_id, &client.min(CLIENT_MAX_PROTOCOL_VERSION).to_le_bytes())
        }
        REQUEST_CONTROLLER_COUNT => reply(0, packet_id, &(state.visible() as u32).to_le_bytes()),
        REQUEST_CONTROLLER_DATA => match state.devices.get(device as usize).filter(|_| (device as usize) < state.visible()) {
            Some(target) => match wire_controller(target).encode() {
                Ok(body) => reply(device, packet_id, &body),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        },
        SET_CUSTOM_MODE => {
            if let Some(target) = state.devices.get_mut(device as usize) {
                target.custom_mode_writes += 1;
            }
            Vec::new()
        }
        UPDATE_LEDS => {
            // size (u32, includes itself), count (u16), then 4 bytes per LED.
            let count = payload.get(4..6).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as usize);
            let body = payload.get(6..).unwrap_or_default();
            let (quads, _) = body.as_chunks::<4>();
            let colors: Vec<Rgb8> = quads.iter().take(count).map(|q| Rgb8 { r: q[0], g: q[1], b: q[2] }).collect();
            if let Some(target) = state.devices.get_mut(device as usize) {
                for (slot, color) in target.colors.iter_mut().zip(colors) {
                    *slot = color;
                }
            }
            Vec::new()
        }
        RESIZE_ZONE => {
            if let (Some(zone_bytes), Some(size_bytes)) = (payload.get(..4), payload.get(4..8)) {
                let zone = u32::from_le_bytes([zone_bytes[0], zone_bytes[1], zone_bytes[2], zone_bytes[3]]) as usize;
                let leds = u32::from_le_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]);
                if let Some(target) = state.devices.get_mut(device as usize) {
                    if let Some(zone) = target.info.zones.get_mut(zone) {
                        if zone.resizable && zone.min_leds <= zone.max_leds {
                            zone.led_count = leds.max(zone.min_leds).min(zone.max_leds);
                        }
                    }
                    target.colors = vec![Rgb8 { r: 0, g: 0, b: 0 }; target.info.led_count() as usize];
                    // OpenRGB sends no reply to RESIZEZONE; it announces the new device list.
                    return reply(0, DEVICE_LIST_UPDATED, &[]);
                }
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// The controller-data view of a fake device, in the v5 layout OpenRGB sends.
fn wire_controller(device: &FakeDevice) -> WireController {
    let info = &device.info;
    let zones = info.zones.iter().map(|zone| {
        let (zone_type, matrix) = match &zone.kind {
            ZoneKind::Single => (0, None),
            ZoneKind::Linear => (1, None),
            ZoneKind::Matrix { width, height, map } => {
                let values = map.iter().map(|cell| cell.unwrap_or(MATRIX_GAP)).collect();
                (2, Some((*height, *width, values)))
            }
        };
        WireZone {
            name: zone.name.clone(),
            zone_type,
            leds_min: zone.min_leds,
            leds_max: zone.max_leds,
            leds_count: zone.led_count,
            matrix,
            segments: Vec::new(),
            flags: 0,
        }
    }).collect();
    WireController {
        device_type: kind_raw(info.kind),
        name: info.name.clone(),
        vendor: info.vendor.clone(),
        description: "canvas-rgb fake".into(),
        version: "1".into(),
        serial: info.serial.clone(),
        location: info.location.clone(),
        active_mode: 0,
        modes: vec![WireMode { name: "Direct".into(), value: 0, color_mode: 1, ..Default::default() }],
        zones,
        leds: (0..device.colors.len()).map(|i| (format!("LED {i}"), i as u32)).collect(),
        colors: device.colors.clone(),
        led_alt_names: Vec::new(),
        flags: 0,
    }
}

fn kind_raw(kind: DeviceKind) -> i32 {
    match kind {
        DeviceKind::Motherboard => 0,
        DeviceKind::Dram => 1,
        DeviceKind::Gpu => 2,
        DeviceKind::LedStrip => 4,
        DeviceKind::Keyboard => 5,
        DeviceKind::Mouse => 6,
        DeviceKind::Headset => 8,
        DeviceKind::Microphone => 16,
        _ => 21,
    }
}
