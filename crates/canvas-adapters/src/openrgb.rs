//! OpenRGB SDK client implementing `DeviceAdapter`.
//!
//! One TCP connection per adapter. A reader task owns the read half, routes replies to the
//! single in-flight request, and marks the session lost on EOF or error, so `is_connected()`
//! turns false as soon as the socket drops. Clones share the connection, so several device
//! tasks can each hold a clone and write concurrently; writes take a short lock on the socket.

use async_trait::async_trait;
use canvas_proto::{
    AdapterCapabilities, AdapterError, DeviceAdapter, DeviceId, DeviceInfo, DeviceKind, Rgb8,
    ZoneInfo, ZoneKind,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::protocol::*;

/// Most controllers one OpenRGB server may report. The count comes off the wire, so it is
/// capped before it sizes an allocation or a request loop.
const MAX_CONTROLLERS: u32 = 1024;
/// Most LEDs one controller may report. UPDATE_LEDS carries a u16 count, so a larger device
/// could not be written anyway.
const MAX_LEDS_PER_CONTROLLER: u64 = u16::MAX as u64;

/// How long a request waits for its reply before the call fails.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const ZONE_TYPE_SINGLE: i32 = 0;
const ZONE_TYPE_LINEAR: i32 = 1;
const ZONE_TYPE_MATRIX: i32 = 2;

#[derive(Clone)]
pub struct OpenRgbAdapter {
    host: String,
    port: u16,
    link: Arc<Link>,
}

struct Link {
    session: Mutex<Session>,
    writer: tokio::sync::Mutex<Option<OwnedWriteHalf>>,
    /// Serializes request/reply pairs: at most one reply is ever awaited.
    request_gate: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Session {
    /// Bumped on every connect and disconnect, so a stale reader cannot mark a newer session lost.
    epoch: u64,
    connected: bool,
    protocol_version: u32,
    pending: Option<Pending>,
    reader: Option<JoinHandle<()>>,
    controllers: Vec<Controller>,
    /// Controllers already switched to direct/custom mode on this connection.
    custom_mode: HashSet<u32>,
}

struct Pending {
    device: u32,
    packet_id: u32,
    reply: oneshot::Sender<Vec<u8>>,
}

#[derive(Debug, Clone)]
struct Controller {
    index: u32,
    wire: WireController,
}

impl Controller {
    fn led_count(&self) -> u32 {
        self.wire.zones.iter().map(|zone| zone.leds_count).sum()
    }

    fn zone_offset(&self, zone: usize) -> usize {
        self.wire.zones.iter().take(zone).map(|z| z.leds_count as usize).sum()
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Link {
    /// Ends the session `epoch` if it is still the current one; aborts its reader.
    fn lose(&self, epoch: u64) {
        let reader = {
            let mut session = guard(&self.session);
            if session.epoch != epoch {
                return;
            }
            session.connected = false;
            session.pending = None;
            session.reader.take()
        };
        if let Some(reader) = reader {
            reader.abort();
        }
    }

    fn lose_current(&self) {
        let epoch = guard(&self.session).epoch;
        self.lose(epoch);
    }
}

impl OpenRgbAdapter {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
            link: Arc::new(Link {
                session: Mutex::new(Session::default()),
                writer: tokio::sync::Mutex::new(None),
                request_gate: tokio::sync::Mutex::new(()),
            }),
        }
    }

    fn require_connected(&self) -> Result<(), AdapterError> {
        if guard(&self.link.session).connected { Ok(()) } else { Err(AdapterError::NotConnected) }
    }

    /// Looks up a controller by its daemon-facing id. Enumerates once when this connection has
    /// not discovered anything yet, so a caller that skipped `discover()` still works.
    async fn controller(&self, device: &DeviceId) -> Result<Controller, AdapterError> {
        let (known, empty) = {
            let session = guard(&self.link.session);
            if !session.connected {
                return Err(AdapterError::NotConnected);
            }
            let known = session.controllers.iter()
                .find(|controller| &device_info(controller).id == device)
                .cloned();
            (known, session.controllers.is_empty())
        };
        if let Some(controller) = known {
            return Ok(controller);
        }
        if empty {
            self.enumerate().await?;
            return self.controller_cached(device);
        }
        Err(AdapterError::UnknownDevice(device.clone()))
    }

    fn controller_cached(&self, device: &DeviceId) -> Result<Controller, AdapterError> {
        guard(&self.link.session).controllers.iter()
            .find(|controller| &device_info(controller).id == device)
            .cloned()
            .ok_or_else(|| AdapterError::UnknownDevice(device.clone()))
    }

    async fn send(&self, device: u32, packet_id: u32, payload: &[u8]) -> Result<(), AdapterError> {
        let bytes = encode_packet(device, packet_id, payload)?;
        let mut writer = self.link.writer.lock().await;
        let Some(stream) = writer.as_mut() else {
            return Err(AdapterError::NotConnected);
        };
        let written = async {
            stream.write_all(&bytes).await?;
            stream.flush().await
        };
        match written.await {
            Ok(()) => Ok(()),
            Err(error) => {
                *writer = None;
                drop(writer);
                self.link.lose_current();
                Err(AdapterError::Io(error))
            }
        }
    }

    /// Sends a request and waits for the reply with the same device index and packet id.
    async fn request(&self, device: u32, packet_id: u32, payload: &[u8]) -> Result<Vec<u8>, AdapterError> {
        let _gate = self.link.request_gate.lock().await;
        let (reply, received) = oneshot::channel();
        {
            let mut session = guard(&self.link.session);
            session.pending = Some(Pending { device, packet_id, reply });
        }
        // Checked after the reply slot is set: a reader that ended first has already cleared
        // the session, so the request fails here instead of waiting out the timeout.
        if !guard(&self.link.session).connected {
            guard(&self.link.session).pending = None;
            return Err(AdapterError::NotConnected);
        }
        if let Err(error) = self.send(device, packet_id, payload).await {
            guard(&self.link.session).pending = None;
            return Err(error);
        }
        match tokio::time::timeout(REPLY_TIMEOUT, received).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(_)) => Err(AdapterError::NotConnected),
            Err(_) => {
                guard(&self.link.session).pending = None;
                Err(AdapterError::Protocol(format!(
                    "no reply to packet {packet_id} for device {device} within {}s",
                    REPLY_TIMEOUT.as_secs())))
            }
        }
    }

    fn protocol_version(&self) -> u32 {
        guard(&self.link.session).protocol_version
    }

    /// Re-enumerates every controller. OpenRGB reports zone resizes through this same path.
    async fn enumerate(&self) -> Result<Vec<Controller>, AdapterError> {
        self.require_connected()?;
        let version = self.protocol_version();
        let count_reply = self.request(0, REQUEST_CONTROLLER_COUNT, &[]).await?;
        let count = read_u32(&count_reply, "controller count")?;
        if count > MAX_CONTROLLERS {
            return Err(AdapterError::Protocol(format!(
                "OpenRGB reports {count} controllers; the limit is {MAX_CONTROLLERS}")));
        }
        let mut controllers = Vec::with_capacity(count as usize);
        for index in 0..count {
            let payload = self.request(index, REQUEST_CONTROLLER_DATA, &version.to_le_bytes()).await?;
            let wire = WireController::decode(&payload, version)?;
            let leds: u64 = wire.zones.iter().map(|zone| u64::from(zone.leds_count)).sum();
            if leds > MAX_LEDS_PER_CONTROLLER {
                // One absurd controller must not hide the others: skip it.
                tracing::warn!(index, leds, "skipping an OpenRGB controller over the LED limit of {MAX_LEDS_PER_CONTROLLER}");
                continue;
            }
            controllers.push(Controller { index, wire });
        }
        let mut session = guard(&self.link.session);
        if session.connected {
            session.controllers = controllers.clone();
        }
        Ok(controllers)
    }

    /// Switches a controller to direct/custom mode, once per connection, then sends a frame.
    async fn send_leds(&self, index: u32, colors: &[Rgb8]) -> Result<(), AdapterError> {
        let needs_mode = !guard(&self.link.session).custom_mode.contains(&index);
        if needs_mode {
            self.send(index, SET_CUSTOM_MODE, &[]).await?;
            guard(&self.link.session).custom_mode.insert(index);
        }
        self.send(index, UPDATE_LEDS, &update_leds_payload(colors)?).await
    }
}

fn read_u32(bytes: &[u8], what: &str) -> Result<u32, AdapterError> {
    match bytes.get(..4) {
        Some(four) => Ok(u32::from_le_bytes([four[0], four[1], four[2], four[3]])),
        None => Err(AdapterError::Protocol(format!("short {what} reply"))),
    }
}

#[async_trait]
impl DeviceAdapter for OpenRgbAdapter {
    fn name(&self) -> &'static str {
        "openrgb"
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
        if guard(&self.link.session).connected {
            return Ok(());
        }
        self.disconnect().await?;
        let stream = TcpStream::connect((self.host.as_str(), self.port)).await?;
        stream.set_nodelay(true)?;
        let (read, write) = stream.into_split();
        let epoch = {
            let mut session = guard(&self.link.session);
            session.epoch += 1;
            session.epoch
        };
        *self.link.writer.lock().await = Some(write);
        let reader = tokio::spawn(read_loop(read, Arc::clone(&self.link), epoch));
        {
            let mut session = guard(&self.link.session);
            session.reader = Some(reader);
            session.connected = true;
        }

        let handshake = async {
            let reply = self.request(0, REQUEST_PROTOCOL_VERSION, &CLIENT_MAX_PROTOCOL_VERSION.to_le_bytes()).await?;
            let server = read_u32(&reply, "protocol version")?;
            if server < MIN_PROTOCOL_VERSION {
                return Err(AdapterError::Protocol(format!(
                    "OpenRGB speaks protocol {server}; canvas-rgb needs {MIN_PROTOCOL_VERSION} or newer")));
            }
            // The server answers with the highest version both sides know.
            guard(&self.link.session).protocol_version = server.min(CLIENT_MAX_PROTOCOL_VERSION);
            self.send(0, SET_CLIENT_NAME, &client_name_payload(canvas_proto::consts::OPENRGB_CLIENT_NAME)).await
        };
        if let Err(error) = handshake.await {
            self.disconnect().await?;
            return Err(error);
        }
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        let reader = {
            let mut session = guard(&self.link.session);
            session.epoch += 1;
            session.connected = false;
            session.pending = None;
            session.controllers.clear();
            session.custom_mode.clear();
            session.reader.take()
        };
        if let Some(reader) = reader {
            reader.abort();
        }
        *self.link.writer.lock().await = None;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        guard(&self.link.session).connected
    }

    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
        let controllers = self.enumerate().await?;
        Ok(controllers.iter().map(device_info).collect())
    }

    async fn write_frame(&mut self, device: &DeviceId, colors: &[Rgb8]) -> Result<(), AdapterError> {
        let controller = self.controller(device).await?;
        let led_count = controller.led_count() as usize;
        if colors.len() != led_count {
            return Err(AdapterError::Protocol(format!(
                "frame has {} colors but {device} has {led_count} LEDs", colors.len())));
        }
        self.send_leds(controller.index, colors).await
    }

    async fn set_zone_size(&mut self, device: &DeviceId, zone: u32, leds: u32) -> Result<(), AdapterError> {
        let controller = self.controller(device).await?;
        let info = controller.wire.zones.get(zone as usize).ok_or_else(|| AdapterError::Zone {
            device: device.clone(), zone, reason: "unknown zone".into(),
        })?;
        if !zone_resizable(info) {
            return Err(AdapterError::Zone { device: device.clone(), zone, reason: "zone is not resizable".into() });
        }
        let size = leds.max(info.leds_min).min(info.leds_max);
        self.send(controller.index, RESIZE_ZONE, &resize_zone_payload(zone, size)).await?;
        // The next frame to this controller must re-enter custom mode after OpenRGB reconfigures it.
        guard(&self.link.session).custom_mode.remove(&controller.index);
        // Ordered after the resize on the same socket, so the reply reflects the new size.
        self.enumerate().await?;
        Ok(())
    }

    async fn identify(&mut self, device: &DeviceId, zone: u32, led: u32) -> Result<(), AdapterError> {
        let controller = self.controller(device).await?;
        let info = controller.wire.zones.get(zone as usize).ok_or_else(|| AdapterError::Zone {
            device: device.clone(), zone, reason: "unknown zone".into(),
        })?;
        if led >= info.leds_count {
            return Err(AdapterError::Zone {
                device: device.clone(), zone,
                reason: format!("LED {led} is outside the zone's {} LEDs", info.leds_count),
            });
        }
        let total = controller.led_count() as usize;
        let mut colors = vec![Rgb8 { r: 0, g: 0, b: 0 }; total];
        colors[controller.zone_offset(zone as usize) + led as usize] = Rgb8 { r: 255, g: 255, b: 255 };
        self.send_leds(controller.index, &colors).await
    }

    async fn read_back(&mut self, device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
        let controller = self.controller(device).await?;
        let payload = self.request(controller.index, REQUEST_CONTROLLER_DATA, &self.protocol_version().to_le_bytes()).await?;
        Ok(WireController::decode(&payload, self.protocol_version())?.colors)
    }
}

async fn read_loop(mut read: OwnedReadHalf, link: Arc<Link>, epoch: u64) {
    while let Ok((device, packet_id, payload)) = read_packet(&mut read).await {
        // DEVICE_LIST_UPDATED needs no action here: every discover() and set_zone_size()
        // re-enumerates, so the next call already sees the new device list.
        if packet_id == DEVICE_LIST_UPDATED {
            continue;
        }
        let reply = {
            let mut session = guard(&link.session);
            if session.epoch != epoch {
                return;
            }
            session.pending.take_if(|p| p.device == device && p.packet_id == packet_id)
        };
        if let Some(pending) = reply {
            let _ = pending.reply.send(payload);
        }
    }
    link.lose(epoch);
}

async fn read_packet(read: &mut OwnedReadHalf) -> Result<(u32, u32, Vec<u8>), AdapterError> {
    let mut header = [0; HEADER_LEN];
    read.read_exact(&mut header).await?;
    let (device, packet_id, size) = decode_header(&header)?;
    let mut payload = vec![0; size];
    read.read_exact(&mut payload).await?;
    Ok((device, packet_id, payload))
}

/// Linear zones with a size range can be resized over the SDK (MSI JARGB headers, for example).
fn zone_resizable(zone: &WireZone) -> bool {
    zone.zone_type == ZONE_TYPE_LINEAR && zone.leds_min != zone.leds_max
}

fn zone_info(index: u32, zone: &WireZone) -> ZoneInfo {
    let kind = match zone.zone_type {
        ZONE_TYPE_SINGLE => ZoneKind::Single,
        ZONE_TYPE_LINEAR => ZoneKind::Linear,
        ZONE_TYPE_MATRIX => {
            let (height, width, values) = zone.matrix.clone().unwrap_or((0, 0, Vec::new()));
            ZoneKind::Matrix {
                width,
                height,
                map: values.into_iter().map(|v| (v != MATRIX_GAP).then_some(v)).collect(),
            }
        }
        _ => ZoneKind::Linear,
    };
    ZoneInfo {
        index,
        name: zone.name.clone(),
        kind,
        led_count: zone.leds_count,
        min_leds: zone.leds_min,
        max_leds: zone.leds_max,
        resizable: zone_resizable(zone),
    }
}

fn device_info(controller: &Controller) -> DeviceInfo {
    let wire = &controller.wire;
    let fingerprint = if wire.serial.trim().is_empty() { &wire.location } else { &wire.serial };
    DeviceInfo {
        id: DeviceId(format!("openrgb:{fingerprint}")),
        name: wire.name.clone(),
        vendor: wire.vendor.clone(),
        kind: device_kind(wire.device_type),
        adapter: "openrgb".into(),
        adapter_index: controller.index,
        serial: wire.serial.clone(),
        location: wire.location.clone(),
        zones: wire.zones.iter().enumerate().map(|(i, zone)| zone_info(i as u32, zone)).collect(),
        connected: true,
    }
}

/// OpenRGB device type to canvas-rgb's kind. Values follow OpenRGB's `DeviceType` enum.
fn device_kind(value: i32) -> DeviceKind {
    match value {
        0 => DeviceKind::Motherboard,
        1 => DeviceKind::Dram,
        2 => DeviceKind::Gpu,
        4 => DeviceKind::LedStrip,
        5 => DeviceKind::Keyboard,
        6 => DeviceKind::Mouse,
        8 => DeviceKind::Headset,
        16 => DeviceKind::Microphone,
        _ => DeviceKind::Other,
    }
}

// ---- hostile OpenRGB listener: wire sizes must not drive allocation or arithmetic

#[cfg(test)]
mod hostile_tests {
    use crate::openrgb::OpenRgbAdapter;
    use crate::protocol::*;
    use canvas_proto::DeviceAdapter;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Answers the handshake, then `controller_count` and `controller_data` as given.
    async fn serve(count: u32, data: Vec<Vec<u8>>) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            loop {
                let mut header = [0u8; HEADER_LEN];
                if stream.read_exact(&mut header).await.is_err() {
                    return;
                }
                let (device, id, size) = decode_header(&header).unwrap();
                let mut payload = vec![0u8; size];
                if stream.read_exact(&mut payload).await.is_err() {
                    return;
                }
                let reply = match id {
                    REQUEST_PROTOCOL_VERSION => 5u32.to_le_bytes().to_vec(),
                    REQUEST_CONTROLLER_COUNT => count.to_le_bytes().to_vec(),
                    REQUEST_CONTROLLER_DATA => data[(device as usize).min(data.len() - 1)].clone(),
                    _ => continue,
                };
                if stream.write_all(&encode_packet(device, id, &reply).unwrap()).await.is_err() {
                    return;
                }
            }
        });
        port
    }

    fn controller(zone_leds: &[u32]) -> Vec<u8> {
        let wire = WireController {
            name: "Evil".into(),
            serial: "x".into(),
            modes: vec![WireMode { name: "Direct".into(), value: 0, color_mode: 1, ..Default::default() }],
            zones: zone_leds
                .iter()
                .map(|&n| WireZone { name: "Z".into(), zone_type: 1, leds_count: n, ..Default::default() })
                .collect(),
            ..Default::default()
        };
        wire.encode().unwrap()
    }

    #[tokio::test]
    async fn a_huge_controller_count_is_refused_not_allocated() {
        let port = serve(u32::MAX, vec![controller(&[3])]).await;
        let mut adapter = OpenRgbAdapter::new("127.0.0.1", port);
        adapter.connect().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), adapter.discover()).await;
        assert!(matches!(result, Ok(Err(_))), "expected an error, got {result:?}");
    }

    #[tokio::test]
    async fn a_controller_over_the_led_cap_is_skipped_not_fatal() {
        let port = serve(2, vec![controller(&[u32::MAX, u32::MAX]), controller(&[3])]).await;
        let mut adapter = OpenRgbAdapter::new("127.0.0.1", port);
        adapter.connect().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), adapter.discover()).await;
        assert!(matches!(&result, Ok(Ok(devices)) if devices.len() == 1), "expected the one good controller, got {result:?}");
    }
}
