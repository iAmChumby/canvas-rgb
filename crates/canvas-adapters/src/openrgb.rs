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
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime};
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

/// How often `SET_CUSTOM_MODE` is sent again to a controller that is being streamed frames.
///
/// A starting value, to be measured (#5): the ENE DRAM sticks drop back to their built-in
/// rainbow when a sleep cuts their power, and the resume event may never arrive. The cost of
/// each re-send is one 16-byte packet per controller (a header with no payload), which OpenRGB
/// turns into one SMBus mode write. Check the SMBus load on real hardware before lowering it.
pub const CUSTOM_MODE_KEEPALIVE: Duration = Duration::from_secs(10);

const ZONE_TYPE_SINGLE: i32 = 0;
const ZONE_TYPE_LINEAR: i32 = 1;
const ZONE_TYPE_MATRIX: i32 = 2;

/// The clocks the adapter reads.
///
/// `Instant` is a monotonic clock, and the standard library says that it is "not specified
/// whether system suspends count as elapsed time or not" (on Windows it reads
/// `QueryPerformanceCounter`). The keepalive therefore watches the wall clock as well: the
/// wall clock follows the RTC, so a sleep shows up there whether or not the monotonic clock
/// counted it (#5).
#[derive(Clone)]
struct Clocks {
    monotonic: Arc<dyn Fn() -> Instant + Send + Sync>,
    wall: Arc<dyn Fn() -> SystemTime + Send + Sync>,
}

impl Clocks {
    /// The clocks of the machine.
    fn system() -> Self {
        Self {
            monotonic: Arc::new(Instant::now),
            wall: Arc::new(SystemTime::now),
        }
    }

    /// The clocks of the caller: the tests move them.
    #[cfg(test)]
    fn moved(
        monotonic: Arc<dyn Fn() -> Instant + Send + Sync>,
        wall: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    ) -> Self {
        Self { monotonic, wall }
    }

    /// The monotonic clock.
    fn monotonic(&self) -> Instant {
        (self.monotonic)()
    }

    /// The wall clock.
    fn wall(&self) -> SystemTime {
        (self.wall)()
    }
}

impl Default for Clocks {
    fn default() -> Self {
        Self::system()
    }
}

/// When a controller was last switched to direct/custom mode.
#[derive(Clone, Copy)]
struct ModeStamp {
    /// The monotonic clock.
    at: Instant,
    /// The wall clock.
    wall: SystemTime,
}

#[derive(Clone)]
pub struct OpenRgbAdapter {
    host: String,
    port: u16,
    link: Arc<Link>,
    /// The clocks the keepalive reads.
    clocks: Clocks,
}

struct Link {
    session: Mutex<Session>,
    writer: tokio::sync::Mutex<Option<OwnedWriteHalf>>,
    /// Serializes request/reply pairs: at most one reply is ever awaited.
    request_gate: tokio::sync::Mutex<()>,
}

struct Session {
    /// Bumped on every connect and disconnect, so a stale reader cannot mark a newer session lost.
    epoch: u64,
    connected: bool,
    protocol_version: u32,
    pending: Option<Pending>,
    reader: Option<JoinHandle<()>>,
    controllers: Vec<Controller>,
    /// When each controller was last switched to direct/custom mode on this connection.
    /// A missing entry means the next frame sends `SET_CUSTOM_MODE` first.
    custom_mode: HashMap<u32, ModeStamp>,
    /// The controllers `SET_CUSTOM_MODE` was ever sent to on this connection. Only the log reads
    /// it (the first send per connection is INFO, a re-send after a reset says so); it changes
    /// nothing about when the mode is sent.
    mode_sent: HashSet<u32>,
    /// How long a controller stays trusted to be in custom mode before the next frame
    /// sends `SET_CUSTOM_MODE` again.
    keepalive: Duration,
    /// Bumped by every `reset_custom_mode`, so a send in flight can tell it was overtaken.
    resets: u64,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            epoch: 0,
            connected: false,
            protocol_version: 0,
            pending: None,
            reader: None,
            controllers: Vec::new(),
            custom_mode: HashMap::new(),
            mode_sent: HashSet::new(),
            keepalive: CUSTOM_MODE_KEEPALIVE,
            resets: 0,
        }
    }
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

    fn reset_custom_mode(&self) {
        let mut session = guard(&self.session);
        session.custom_mode.clear();
        session.resets += 1;
    }

    fn lose_current(&self) {
        let epoch = guard(&self.session).epoch;
        self.lose(epoch);
    }
}

/// Every live adapter's link, so a resume event (which the daemon hears far from the adapter,
/// behind `Box<dyn DeviceAdapter>`) can reach all of them without a spine change.
static LINKS: Mutex<Vec<Weak<Link>>> = Mutex::new(Vec::new());

/// The system resumed from sleep: every controller may have lost power and fallen back to its
/// built-in mode, so the next frame to each one sends `SET_CUSTOM_MODE` again. Returns how many
/// adapters were told.
pub fn reset_custom_mode_all() -> usize {
    let mut links = guard(&LINKS);
    links.retain(|link| link.strong_count() > 0);
    links.iter().filter_map(Weak::upgrade).map(|link| link.reset_custom_mode()).count()
}

impl OpenRgbAdapter {
    pub fn new(host: &str, port: u16) -> Self {
        let link = Arc::new(Link {
            session: Mutex::new(Session::default()),
            writer: tokio::sync::Mutex::new(None),
            request_gate: tokio::sync::Mutex::new(()),
        });
        {
            let mut links = guard(&LINKS);
            links.retain(|link| link.strong_count() > 0);
            links.push(Arc::downgrade(&link));
        }
        Self { host: host.to_string(), port, link, clocks: Clocks::system() }
    }

    /// Like [`new`](Self::new) with another keepalive interval (tests use a short one).
    pub fn with_keepalive(host: &str, port: u16, keepalive: Duration) -> Self {
        let adapter = Self::new(host, port);
        guard(&adapter.link.session).keepalive = keepalive;
        adapter
    }

    /// Like [`new`](Self::new) with another keepalive interval and clocks of the caller's.
    ///
    /// The tests move the clocks: a sleep that the monotonic clock did not count is a jump
    /// of the wall clock alone (#5).
    #[cfg(test)]
    fn with_clocks(host: &str, port: u16, keepalive: Duration, clocks: Clocks) -> Self {
        let mut adapter = Self::new(host, port);
        guard(&adapter.link.session).keepalive = keepalive;
        adapter.clocks = clocks;
        adapter
    }

    /// Forgets which controllers are in custom mode: the next frame to each sends
    /// `SET_CUSTOM_MODE` first. Called after a resume.
    pub fn reset_custom_mode(&self) {
        self.link.reset_custom_mode();
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

    /// Switches a controller to direct/custom mode, then sends a frame. The mode is sent again
    /// after a reset (resume, resize) and whenever the keepalive interval has passed, so a
    /// controller that fell back to its built-in mode heals on its own.
    async fn send_leds(&self, index: u32, colors: &[Rgb8]) -> Result<(), AdapterError> {
        let reason = {
            let session = guard(&self.link.session);
            match session.custom_mode.get(&index) {
                // Sent before on this connection, and forgotten since: a resume or a zone resize.
                None if session.mode_sent.contains(&index) => Some(ModeReason::AfterReset),
                None => Some(ModeReason::First),
                Some(stamp) => self.stale_reason(stamp, session.keepalive),
            }
        };
        if let Some(reason) = reason {
            // Stamped before the send. A resume that lands during the await bumps `resets`; the
            // controller is then not recorded, so the next frame sends the mode again.
            let sent_at = self.clocks.monotonic();
            let sent_wall = self.clocks.wall();
            let resets = guard(&self.link.session).resets;
            self.send(index, SET_CUSTOM_MODE, &[]).await?;
            let first_on_connection = {
                let mut session = guard(&self.link.session);
                if session.resets == resets {
                    session.custom_mode.insert(index, ModeStamp { at: sent_at, wall: sent_wall });
                }
                session.mode_sent.insert(index)
            };
            self.log_mode_sent(index, reason, first_on_connection);
        }
        self.send(index, UPDATE_LEDS, &update_leds_payload(colors)?).await
    }

    /// Says that `SET_CUSTOM_MODE` went to controller `index` (#81). OpenRGB turns each one into an
    /// SMBus mode write on the stick and reports nothing back over the SDK, so this line (what was
    /// sent, to which controller, and why) is the only record. The first one per connection is
    /// INFO; the keepalive and the re-sends after a reset are DEBUG, so a stream of frames adds
    /// no INFO line.
    fn log_mode_sent(&self, index: u32, reason: ModeReason, first_on_connection: bool) {
        let (name, location) = guard(&self.link.session)
            .controllers
            .iter()
            .find(|controller| controller.index == index)
            .map(|controller| (controller.wire.name.clone(), controller.wire.location.clone()))
            .unwrap_or_default();
        let why = reason.text();
        if first_on_connection {
            tracing::info!(
                controller = index, name, location, reason = why,
                "SET_CUSTOM_MODE sent to OpenRGB controller {index} ({name}, {location}): {why}"
            );
        } else {
            tracing::debug!(
                controller = index, name, location, reason = why,
                "SET_CUSTOM_MODE sent to OpenRGB controller {index} ({name}, {location}): {why}"
            );
        }
    }

    /// Why a controller's mode has to be sent again, or `None` when it does not.
    ///
    /// The keepalive covers a controller that lost power over a sleep: the ENE DRAM sticks fall
    /// back to their built-in rainbow. The monotonic clock may not have counted the sleep (the
    /// standard library does not say), so the wall clock is watched too: it follows the RTC, so
    /// it counts a sleep whatever the monotonic clock did (#5).
    fn stale_reason(&self, stamp: &ModeStamp, keepalive: Duration) -> Option<ModeReason> {
        if self.clocks.monotonic().saturating_duration_since(stamp.at) >= keepalive {
            return Some(ModeReason::Keepalive);
        }
        // A wall clock that went backwards (the clock was set) is not a sleep.
        self.clocks
            .wall()
            .duration_since(stamp.wall)
            .is_ok_and(|gap| gap > keepalive)
            .then_some(ModeReason::WallClockJump)
    }
}

/// Why `SET_CUSTOM_MODE` is sent to a controller (the log line says which).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModeReason {
    /// The first frame to this controller on this connection.
    First,
    /// The mode map was cleared (a resume or a zone resize) after it was sent before.
    AfterReset,
    /// The keepalive interval passed on the monotonic clock.
    Keepalive,
    /// The wall clock moved on by more than the keepalive while the monotonic clock did not: a sleep.
    WallClockJump,
}

impl ModeReason {
    fn text(self) -> &'static str {
        match self {
            Self::First => "the first frame to this controller on this connection",
            Self::AfterReset => "again after a reset (a resume or a zone resize)",
            Self::Keepalive => "again, the keepalive interval passed",
            Self::WallClockJump => "again, the wall clock jumped past the keepalive (a sleep)",
        }
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
            session.mode_sent.clear();
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
        let lit = controller.zone_offset(zone as usize) + led as usize;
        colors[lit] = Rgb8 { r: 255, g: 255, b: 255 };
        self.send_leds(controller.index, &colors).await?;
        // OpenRGB answers nothing to a frame, so what was sent is the whole record (#81).
        tracing::info!(
            controller = controller.index, device = %device, zone, led, lit, leds = total,
            "identify sent to OpenRGB controller {} ({device}): LED {lit} of {total} white, the other {} black",
            controller.index, total - 1
        );
        Ok(())
    }

    async fn read_back(&mut self, device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
        let controller = self.controller(device).await?;
        let payload = self.request(controller.index, REQUEST_CONTROLLER_DATA, &self.protocol_version().to_le_bytes()).await?;
        Ok(WireController::decode(&payload, self.protocol_version())?.colors)
    }
}

impl crate::resume::ResumeBehavior for OpenRgbAdapter {
    fn on_resume(&self) -> crate::resume::OnResume {
        // A controller that lost power over a sleep falls back to its built-in mode, so the
        // next frame to it sends SET_CUSTOM_MODE again. The resume clears the map of the
        // controllers in custom mode (`reset_custom_mode`), and the keepalive covers the case
        // where the resume never reaches the daemon (#5).
        crate::resume::OnResume::SetsModeAgain
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

// ---- the keepalive over a sleep: the wall clock counts it, the monotonic clock may not

#[cfg(test)]
mod keepalive_tests {
    use super::*;
    use crate::fake::FakeOpenRgbServer;
    use std::sync::Mutex;

    const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };

    /// A DRAM stick, as the fake server reports it.
    fn stick() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId("openrgb:dram".into()),
            name: "DDR5".into(),
            vendor: "fake".into(),
            kind: DeviceKind::Dram,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: "dram".into(),
            location: "smbus".into(),
            zones: vec![ZoneInfo {
                index: 0,
                name: "RGB".into(),
                kind: ZoneKind::Linear,
                led_count: 8,
                min_leds: 8,
                max_leds: 8,
                resizable: false,
            }],
            connected: true,
        }
    }

    /// The clocks of an awake machine, which the test moves.
    struct Moved {
        monotonic: Mutex<Instant>,
        wall: Mutex<SystemTime>,
    }

    impl Moved {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                monotonic: Mutex::new(Instant::now()),
                wall: Mutex::new(SystemTime::now()),
            })
        }

        /// The clocks that read this state.
        fn clocks(state: &Arc<Self>) -> Clocks {
            let monotonic = Arc::clone(state);
            let wall = Arc::clone(state);
            Clocks::moved(
                Arc::new(move || *monotonic.monotonic.lock().expect("the clock's lock")),
                Arc::new(move || *wall.wall.lock().expect("the clock's lock")),
            )
        }
    }

    /// The adapter over the fake server, with the clocks the test moves.
    async fn adapter(state: &Arc<Moved>) -> (FakeOpenRgbServer, OpenRgbAdapter) {
        let server = FakeOpenRgbServer::start(vec![stick()]).await.expect("the fake server");
        let mut adapter =
            OpenRgbAdapter::with_clocks("127.0.0.1", server.port(), CUSTOM_MODE_KEEPALIVE, Moved::clocks(state));
        adapter.connect().await.expect("connect");
        (server, adapter)
    }

    /// Writes a frame and waits for the server to have handled it.
    async fn frame(adapter: &mut OpenRgbAdapter) {
        let id = DeviceId("openrgb:dram".into());
        adapter.write_frame(&id, &[RED; 8]).await.expect("frame");
        // The read-back is a request and a reply, so the frame before it has been handled.
        adapter.read_back(&id).await.expect("readback orders the frame");
    }

    /// A second stick, as the fake server reports it (controller 1).
    fn second_stick() -> DeviceInfo {
        let mut info = stick();
        info.id = DeviceId("openrgb:dram2".into());
        info.serial = "dram2".into();
        info.location = "smbus2".into();
        info.adapter_index = 1;
        info
    }

    /// The adapter over a fake server with two sticks, and the log it writes.
    async fn two_sticks(state: &Arc<Moved>) -> (FakeOpenRgbServer, OpenRgbAdapter) {
        let server = FakeOpenRgbServer::start(vec![stick(), second_stick()]).await.expect("the fake server");
        let mut adapter =
            OpenRgbAdapter::with_clocks("127.0.0.1", server.port(), CUSTOM_MODE_KEEPALIVE, Moved::clocks(state));
        adapter.connect().await.expect("connect");
        (server, adapter)
    }

    async fn frame_to(adapter: &mut OpenRgbAdapter, id: &str) {
        let id = DeviceId(id.into());
        adapter.write_frame(&id, &[RED; 8]).await.expect("frame");
        adapter.read_back(&id).await.expect("readback orders the frame");
    }

    /// #81: the first SET_CUSTOM_MODE to each controller on a connection is one INFO line naming the
    /// controller; later frames add none, and the keepalive and a reset re-send at DEBUG.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn the_first_set_custom_mode_per_controller_is_one_info_line_and_the_rest_are_debug() {
        let _serial = crate::resume_serial();
        let logs = crate::test_logs::LogBuf::default();
        let _default = logs.install();
        let state = Moved::new();
        let (server, mut adapter) = two_sticks(&state).await;

        for _ in 0..5 {
            frame_to(&mut adapter, "openrgb:dram").await;
            frame_to(&mut adapter, "openrgb:dram2").await;
        }
        assert_eq!(server.custom_mode_writes(0), 1);
        assert_eq!(server.custom_mode_writes(1), 1);
        let info = logs.lines_with(&["INFO", "SET_CUSTOM_MODE sent to OpenRGB controller"]);
        assert_eq!(info.len(), 2, "one INFO line per controller, none per frame: {info:?}");
        assert!(info[0].contains("controller 0 (DDR5, smbus)") && info[0].contains("first frame"), "{}", info[0]);
        assert!(info[1].contains("controller 1 (DDR5, smbus2)") && info[1].contains("first frame"), "{}", info[1]);

        // The keepalive: DEBUG, not INFO.
        *state.monotonic.lock().expect("the clock's lock") += CUSTOM_MODE_KEEPALIVE;
        frame_to(&mut adapter, "openrgb:dram").await;
        let keepalive = logs.lines_with(&["DEBUG", "controller 0", "keepalive interval passed"]);
        assert_eq!(keepalive.len(), 1, "{}", logs.text());

        // After a reset: DEBUG, and it says so.
        adapter.reset_custom_mode();
        frame_to(&mut adapter, "openrgb:dram2").await;
        let reset = logs.lines_with(&["DEBUG", "controller 1", "after a reset"]);
        assert_eq!(reset.len(), 1, "{}", logs.text());

        let info = logs.lines_with(&["INFO", "SET_CUSTOM_MODE sent to OpenRGB controller"]);
        assert_eq!(info.len(), 2, "the re-sends added no INFO line: {info:?}");

        // A new connection is a new "first".
        adapter.disconnect().await.expect("disconnect");
        adapter.connect().await.expect("connect again");
        frame_to(&mut adapter, "openrgb:dram").await;
        let info = logs.lines_with(&["INFO", "SET_CUSTOM_MODE sent to OpenRGB controller 0"]);
        assert_eq!(info.len(), 2, "the first frame on the new connection is INFO again: {info:?}");
    }

    /// #81: an identify says which controller it went to and what was sent.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn an_identify_is_logged_with_its_controller_and_what_was_sent() {
        let _serial = crate::resume_serial();
        let logs = crate::test_logs::LogBuf::default();
        let _default = logs.install();
        let state = Moved::new();
        let (_server, mut adapter) = two_sticks(&state).await;

        adapter.identify(&DeviceId("openrgb:dram2".into()), 0, 3).await.expect("identify");

        let lines = logs.lines_with(&["INFO", "identify sent to OpenRGB controller 1"]);
        assert_eq!(lines.len(), 1, "{}", logs.text());
        assert!(lines[0].contains("openrgb:dram2"), "{}", lines[0]);
        assert!(lines[0].contains("LED 3 of 8 white, the other 7 black"), "{}", lines[0]);
        // The mode write that went with it is on record too.
        assert_eq!(logs.lines_with(&["SET_CUSTOM_MODE sent to OpenRGB controller 1"]).len(), 1);
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn a_sleep_the_monotonic_clock_did_not_count_still_makes_the_mode_be_set_again() {
        let _serial = crate::resume_serial();
        let state = Moved::new();
        let (server, mut adapter) = adapter(&state).await;
        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 1, "the first frame sets the mode");

        // The machine sleeps for an hour. The monotonic clock stands still (the standard
        // library does not say that a suspend counts), the wall clock follows the RTC.
        *state.wall.lock().expect("the clock's lock") += Duration::from_secs(3600);

        frame(&mut adapter).await;
        assert_eq!(
            server.custom_mode_writes(0),
            2,
            "the frame after the sleep did not set the mode again"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn the_keepalive_fires_when_the_monotonic_clock_passed_it() {
        let _serial = crate::resume_serial();
        let state = Moved::new();
        let (server, mut adapter) = adapter(&state).await;
        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 1);

        *state.monotonic.lock().expect("the clock's lock") += CUSTOM_MODE_KEEPALIVE;

        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 2, "the keepalive set the mode again");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // the lock keeps `note_resume` tests one at a time
    async fn a_wall_clock_set_backwards_is_not_a_sleep() {
        let _serial = crate::resume_serial();
        let state = Moved::new();
        let (server, mut adapter) = adapter(&state).await;
        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 1);

        // The clock was set an hour back: the wall clock moved away from the stamp, so it is
        // not a sleep and the mode stays where it is.
        *state.wall.lock().expect("the clock's lock") -= Duration::from_secs(3600);

        frame(&mut adapter).await;
        assert_eq!(server.custom_mode_writes(0), 1, "the mode was set again for no reason");
    }
}
