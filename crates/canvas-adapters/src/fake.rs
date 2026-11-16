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
    /// Bumped by `release_replies`; each connection writes its held replies when it changes.
    release: tokio::sync::watch::Sender<u64>,
    /// Bumped whenever a controller's colours change, so a test can wait for a write to land
    /// instead of polling for it.
    writes: tokio::sync::watch::Sender<u64>,
}

/// How long [`FakeOpenRgbServer::wait_for_colors`] waits before it gives up. It is a hang guard,
/// never a verdict: the wait ends on the write that lands the colours, and the colours it
/// returns are the server's real ones, so a write that never lands fails the caller's assert.
const COLOR_WAIT_CAP: std::time::Duration = std::time::Duration::from_secs(10);

struct FakeState {
    devices: Vec<FakeDevice>,
    /// When set, device `i` is announced only once `first + i * step` has passed since `since`,
    /// as real OpenRGB finds its controllers one by one while it detects.
    detection: Option<Detection>,
    /// While set, the server takes every packet and answers none: a real OpenRGB that stalls.
    stalled: bool,
    /// How many requests the server swallowed while `stalled`.
    swallowed: u32,
    /// While set, the server answers as usual but keeps each reply back until
    /// [`FakeOpenRgbServer::release_replies`]: a real OpenRGB that is slow, not dead.
    holding: bool,
    /// The replies kept back, with the connection each belongs to, in the order they were made.
    held: Vec<(u64, Vec<u8>)>,
    /// While set, the server stops reading the socket: the client's writes fill it and then
    /// block, as they do against a busy OpenRGB that never reads.
    reading_paused: bool,
    /// How many requests the client is still waiting a reply for.
    outstanding: u32,
    /// How many frames arrived while a request was still waiting for its reply.
    frames_while_waiting: u32,
    /// How many `UPDATE_LEDS` packets each controller received.
    led_writes: Vec<u32>,
    /// How many `RESIZE_ZONE` packets each controller received.
    resize_writes: Vec<u32>,
    /// While set, a resize is counted but not applied and no device list is announced, and
    /// no request is answered either: a real OpenRGB that refuses a resize and goes quiet.
    failing_resizes: bool,
    /// How long a resize takes to reach the device; 0 lands it while the request is handled.
    resize_delay: std::time::Duration,
    /// Resizes that are waiting out `resize_delay`, with the connection to announce the new
    /// device list to.
    pending_resizes: Vec<PendingResize>,
    /// How many `REQUEST_RESCAN_DEVICES` (140) packets the client sent.
    rescan_count: u32,
    /// The protocol version this server negotiates in the handshake.
    server_protocol_version: u32,
    /// While set, a rescan clears the list and then keeps announcing list changes forever and
    /// never registers a controller again: a detection that never settles (#290).
    detection_hold: bool,
    /// While set, packet 140 is counted and nothing else happens: OpenRGB with detection
    /// disabled (it forwards the rescan elsewhere and sends no list change) (#290).
    detection_disabled: bool,
    /// Devices OpenRGB has not found yet. A rescan's detection registers them with the others;
    /// until then they are absent from the controller count and data (#290).
    hidden: Vec<FakeDevice>,
    /// How long a rescan leaves the list empty before the first controller registers again.
    rescan_clear: std::time::Duration,
    /// How long between one controller registering again and the next.
    rescan_gap: std::time::Duration,
    /// While a rescan runs: how many controllers have registered again. The count and the data
    /// show only these. `None` when no rescan runs.
    rescan_listed: Option<usize>,
    /// The rescans that are still re-registering controllers, one per connection that asked.
    rescan_runs: Vec<RescanRun>,
    /// How many times the client asked for the controller count while a rescan was still
    /// re-registering controllers: reads of a list that is mid-detection (#290).
    reads_during_detection: u32,
    /// How many frame packets (`UPDATE_LEDS`, `SET_CUSTOM_MODE`) named a controller that was not
    /// registered at that moment of a rescan. release_1.1 finds no controller thread for such an id
    /// and drops the packet without a reply or an error to a protocol 5 client, and the link stays
    /// up (`NetworkServer.cpp:1495-1531`, `:1583-1587`, `:3562-3568`) (#290).
    frames_to_unregistered: u32,
    /// The protocol version each connection negotiated: the lower of the client's and the
    /// server's, as OpenRGB's `NetworkServer` keeps in `client_protocol_version`.
    negotiated: std::collections::HashMap<u64, u32>,
}

/// A rescan that is re-registering controllers (OpenRGB's `DetectionManager` finding them one by
/// one). Each step is announced to the connection that asked.
#[derive(Clone, Copy)]
struct RescanRun {
    connection: u64,
    due: tokio::time::Instant,
    /// The next controller to register again.
    next: usize,
}

#[derive(Clone, Copy)]
struct Detection {
    since: std::time::Instant,
    first: std::time::Duration,
    step: std::time::Duration,
}

#[derive(Clone, Copy)]
struct PendingResize {
    connection: u64,
    device: u32,
    zone: usize,
    leds: u32,
    due: tokio::time::Instant,
}

impl FakeState {
    /// Whether controller `index` is not registered right now, in the middle of a rescan.
    fn unregistered(&self, index: u32) -> bool {
        self.rescan_listed.is_some_and(|listed| index as usize >= listed)
    }

    /// How many controllers the server announces right now.
    fn visible(&self) -> usize {
        if let Some(listed) = self.rescan_listed {
            return listed.min(self.devices.len());
        }
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

impl FakeDevice {
    fn new(info: DeviceInfo) -> Self {
        let colors = vec![Rgb8 { r: 0, g: 0, b: 0 }; info.led_count() as usize];
        let initial_zone_leds = info.zones.iter().map(|zone| zone.led_count).collect();
        Self { info, initial_zone_leds, colors, custom_mode_writes: 0 }
    }
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

    /// Like [`start`](Self::start), but binds `port` on 127.0.0.1 instead of an
    /// ephemeral port. For the standalone fake server (`fake_openrgb` example)
    /// that a fakes-mode daemon connects to on a known, non-real port.
    pub async fn start_on_port(devices: Vec<DeviceInfo>, port: u16) -> std::io::Result<Self> {
        Self::start_inner_on(devices, None, Some(port)).await
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
        Self::start_inner_on(devices, detection, None).await
    }

    async fn start_inner_on(
        devices: Vec<DeviceInfo>,
        detection: Option<Detection>,
        port: Option<u16>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port.unwrap_or(0))).await?;
        let port = listener.local_addr()?.port();
        let devices: Vec<FakeDevice> = devices.into_iter().map(FakeDevice::new).collect();
        let led_writes = vec![0; devices.len()];
        let resize_writes = vec![0; devices.len()];
        let state = Arc::new(Mutex::new(FakeState {
            devices,
            detection,
            stalled: false,
            swallowed: 0,
            holding: false,
            held: Vec::new(),
            reading_paused: false,
            outstanding: 0,
            frames_while_waiting: 0,
            led_writes,
            resize_writes,
            failing_resizes: false,
            resize_delay: std::time::Duration::ZERO,
            pending_resizes: Vec::new(),
            rescan_count: 0,
            server_protocol_version: CLIENT_MAX_PROTOCOL_VERSION,
            detection_hold: false,
            detection_disabled: false,
            hidden: Vec::new(),
            rescan_clear: std::time::Duration::from_millis(20),
            rescan_gap: std::time::Duration::from_millis(10),
            rescan_listed: None,
            rescan_runs: Vec::new(),
            reads_during_detection: 0,
            frames_to_unregistered: 0,
            negotiated: std::collections::HashMap::new(),
        }));
        let connections: Arc<Mutex<Vec<AbortHandle>>> = Arc::new(Mutex::new(Vec::new()));
        let (release, _) = tokio::sync::watch::channel(0_u64);
        let (writes, _) = tokio::sync::watch::channel(0_u64);
        let acceptor = {
            let state = Arc::clone(&state);
            let connections = Arc::clone(&connections);
            let release = release.clone();
            let writes = writes.clone();
            tokio::spawn(async move {
                let mut next_connection = 0_u64;
                while let Ok((stream, _)) = listener.accept().await {
                    next_connection += 1;
                    let handle =
                        tokio::spawn(serve(stream, Arc::clone(&state), next_connection, release.subscribe(), writes.clone()));
                    let mut open = guard(&connections);
                    open.retain(|conn| !conn.is_finished());
                    open.push(handle.abort_handle());
                }
            })
        };
        Ok(Self { port, state, connections, acceptor, release, writes })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// OpenRGB announces one more controller now, after the others: the test decides when, so
    /// no clock is involved. It gets the next index.
    pub fn announce(&self, info: DeviceInfo) {
        let mut state = guard(&self.state);
        state.devices.push(FakeDevice::new(info));
        state.led_writes.push(0);
        state.resize_writes.push(0);
    }

    /// How many rescan packets (140) the client has sent.
    pub fn rescan_count(&self) -> u32 {
        guard(&self.state).rescan_count
    }

    /// Adds a device OpenRGB has not found yet. It is absent from the controller list until a
    /// rescan's detection finishes, as a controller plugged in after OpenRGB started is (#290).
    pub fn add_hidden_device(&self, info: DeviceInfo) {
        guard(&self.state).hidden.push(FakeDevice::new(info));
    }

    /// Removes controller `index` from the server, as a device unplugged since OpenRGB last
    /// detected: the controllers after it move up one index, and the next detection finds one
    /// fewer (#290).
    pub fn unplug_device(&self, index: u32) {
        let mut state = guard(&self.state);
        if (index as usize) < state.devices.len() {
            state.devices.remove(index as usize);
            state.led_writes.remove(index as usize);
            state.resize_writes.remove(index as usize);
        }
    }

    /// While held, a rescan clears the list and then announces list changes (100) forever
    /// without registering a controller again: a detection that never settles, with the list
    /// empty the whole time (#290).
    pub fn hold_detection(&self, held: bool) {
        guard(&self.state).detection_hold = held;
    }

    /// While disabled, packet 140 is counted and nothing follows: OpenRGB with detection turned
    /// off sends no list change and leaves the list alone (#290).
    pub fn disable_detection(&self, disabled: bool) {
        guard(&self.state).detection_disabled = disabled;
    }

    /// How a rescan unfolds on release_1.1: the list is empty for `clear` after packet 140, then
    /// each controller registers again `gap` after the one before, announcing a list change (100)
    /// each time (#290).
    pub fn rescan_timing(&self, clear: std::time::Duration, gap: std::time::Duration) {
        let mut state = guard(&self.state);
        state.rescan_clear = clear;
        state.rescan_gap = gap;
    }

    /// How many times the controller count was read while a rescan was still re-registering
    /// controllers (the list was mid-detection).
    pub fn reads_during_detection(&self) -> u32 {
        guard(&self.state).reads_during_detection
    }

    /// How many frame packets named a controller that was not registered at that moment of a
    /// rescan (dropped, as release_1.1 drops them): proof a frame was still being sent to the index
    /// the client held while OpenRGB re-registered its controllers.
    pub fn frames_to_unregistered(&self) -> u32 {
        guard(&self.state).frames_to_unregistered
    }

    /// The protocol version a connection negotiated, in connection order from 1.
    pub fn negotiated_version(&self, connection: u64) -> Option<u32> {
        guard(&self.state).negotiated.get(&connection).copied()
    }

    /// The highest protocol version this server speaks: it answers the handshake with the lower of
    /// this and the client's. 4 models an OpenRGB with no rescan packet (#290).
    pub fn server_protocol_version(&self, version: u32) {
        guard(&self.state).server_protocol_version = version;
    }

    /// While `stalled`, the server reads every packet and answers none, as a real OpenRGB does
    /// while it is busy. Frames (which have no reply) still land. Starting it again with `false`
    /// makes the server answer again.
    pub fn stall_replies(&self, stalled: bool) {
        guard(&self.state).stalled = stalled;
    }

    /// How many requests the server has swallowed while stalled (see
    /// [`stall_replies`](Self::stall_replies)): a test waits on this to know that a request is now
    /// waiting on a reply that never comes.
    pub fn swallowed_requests(&self) -> u32 {
        guard(&self.state).swallowed
    }

    /// While `holding`, the server answers as usual but keeps each reply back (a slow OpenRGB, not
    /// a dead one) until [`release_replies`](Self::release_replies).
    pub fn hold_replies(&self, holding: bool) {
        guard(&self.state).holding = holding;
    }

    /// How many replies are being kept back.
    pub fn held_replies(&self) -> usize {
        guard(&self.state).held.len()
    }

    /// While set, the server stops reading the socket: the client's writes fill it and then
    /// block, as they do against a busy OpenRGB that never reads.
    pub fn stop_reading(&self, stopped: bool) {
        guard(&self.state).reading_paused = stopped;
    }

    /// How many frames (colour writes) arrived while a request was still waiting for its reply.
    ///
    /// A frame that goes out while the server owes a reply proves the frame did not wait for
    /// the request that is holding the adapter.
    pub fn frames_while_a_request_waited(&self) -> u32 {
        guard(&self.state).frames_while_waiting
    }

    /// How many `UPDATE_LEDS` packets the controller received.
    pub fn update_leds_writes(&self, index: u32) -> u32 {
        guard(&self.state).led_writes.get(index as usize).copied().unwrap_or(0)
    }

    /// Sends every reply that was kept back, in the order they were made, then returns. The
    /// server keeps holding new ones if `hold_replies(true)` is still set.
    pub fn release_replies(&self) {
        self.release.send_modify(|n| *n += 1);
    }

    /// Current LED colors of controller `index`, as last written.
    pub fn colors(&self, index: u32) -> Vec<Rgb8> {
        guard(&self.state).devices.get(index as usize).map(|device| device.colors.clone()).unwrap_or_default()
    }

    /// A receiver that changes whenever this server applies a write to a controller's colours,
    /// so a test can wait on `changed()` instead of polling `colors()`.
    pub fn write_ticks(&self) -> tokio::sync::watch::Receiver<u64> {
        self.writes.subscribe()
    }

    /// Waits until controller `index` holds `expected`, then returns what it holds.
    /// The wait is woken by the write that changes a controller's colours, so it costs nothing
    /// when they have already landed and the test never reads its verdict off a clock. The cap
    /// ([`COLOR_WAIT_CAP`]) only stops a write that never lands from hanging the test; the
    /// colours it returns then are the server's real ones, so the caller's assert says why.
    pub async fn wait_for_colors(&self, index: u32, expected: &[Rgb8]) -> Vec<Rgb8> {
        let mut changed = self.writes.subscribe();
        tokio::time::timeout(COLOR_WAIT_CAP, async {
            loop {
                let colors = self.colors(index);
                if colors == expected {
                    return colors;
                }
                if changed.changed().await.is_err() {
                    return self.colors(index);
                }
            }
        })
        .await
        .unwrap_or_else(|_| self.colors(index))
    }

    /// How many times SETCUSTOMMODE was received for controller `index`.
    pub fn custom_mode_writes(&self, index: u32) -> u32 {
        guard(&self.state).devices.get(index as usize).map_or(0, |device| device.custom_mode_writes)
    }

    /// How many `RESIZE_ZONE` packets the controller received. Every request counts, including
    /// one the server refuses (see [`fail_resizes`](Self::fail_resizes)) and one still waiting
    /// out its delay (see [`delay_resizes`](Self::delay_resizes)).
    pub fn resize_count(&self, index: u32) -> u32 {
        guard(&self.state).resize_writes.get(index as usize).copied().unwrap_or(0)
    }

    /// While `failing`, a `RESIZE_ZONE` leaves the zone's size alone and announces no device
    /// list, and the server answers no request: real OpenRGB gives the client silence when it
    /// refuses a resize, so the adapter's re-read of the list times out and the resize fails.
    pub fn fail_resizes(&self, failing: bool) {
        guard(&self.state).failing_resizes = failing;
    }

    /// How long a resize takes to reach the device, in milliseconds: the zone's new size and
    /// the device list that announces it land `ms` after the request instead of while it is
    /// handled. `0` (the default) lands it at once.
    pub fn delay_resizes(&self, ms: u64) {
        guard(&self.state).resize_delay = std::time::Duration::from_millis(ms);
    }

    /// Current LED count of zone `zone` on controller `index`.
    pub fn zone_leds(&self, index: u32, zone: u32) -> Option<u32> {
        guard(&self.state).devices.get(index as usize)?.info.zones.get(zone as usize).map(|z| z.led_count)
    }

    /// Simulates OpenRGB restarting: every open connection is dropped, and SDK zone resizes
    /// are forgotten (resizable zones return to their starting size, as real OpenRGB does).
    /// A resize that had not landed yet never does, and the arrival counts are kept: they are
    /// the history of this server, not device state.
    pub async fn restart(&self) {
        {
            let mut state = guard(&self.state);
            state.pending_resizes.clear();
            state.rescan_runs.clear();
            state.rescan_listed = None;
            for device in &mut state.devices {
                for (zone, initial) in device.info.zones.iter_mut().zip(&device.initial_zone_leds) {
                    zone.led_count = *initial;
                }
                device.colors = vec![Rgb8 { r: 0, g: 0, b: 0 }; device.info.led_count() as usize];
                device.custom_mode_writes = 0;
            }
        }
        // The colours were reset: a test waiting on them sees the new (black) values.
        self.writes.send_modify(|n| *n += 1);
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

async fn serve(
    mut stream: TcpStream,
    state: Arc<Mutex<FakeState>>,
    connection: u64,
    mut release: tokio::sync::watch::Receiver<u64>,
    writes: tokio::sync::watch::Sender<u64>,
) -> std::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if guard(&state).reading_paused {
            // The client fills the socket: the server stops reading, as a busy OpenRGB does.
            // The connection stays open, so the client's writes block instead of failing.
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            continue;
        }
        // The earliest resize of this connection that is still waiting out its delay.
        let due = {
            let state = guard(&state);
            state.pending_resizes.iter().filter(|resize| resize.connection == connection).map(|resize| resize.due).min()
        };
        // The next step of this connection's rescan, if one is re-registering controllers.
        let rescan_due = guard(&state).rescan_runs.iter().find(|run| run.connection == connection).map(|run| run.due);
        let read = tokio::select! {
            read = stream.read(&mut chunk) => read?,
            _ = tokio::time::sleep_until(rescan_due.unwrap_or_else(tokio::time::Instant::now)), if rescan_due.is_some() => {
                let bytes = step_rescan(&mut guard(&state), connection);
                if !bytes.is_empty() {
                    stream.write_all(&bytes).await?;
                    stream.flush().await?;
                }
                continue;
            }
            changed = release.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
                // Every held reply of this connection goes out now, in the order it was made.
                let mine: Vec<Vec<u8>> = {
                    let mut state = guard(&state);
                    let (mine, rest) = std::mem::take(&mut state.held).into_iter().partition(|(id, _)| *id == connection);
                    state.held = rest;
                    mine.into_iter().map(|(_, bytes)| bytes).collect()
                };
                for bytes in mine {
                    stream.write_all(&bytes).await?;
                }
                stream.flush().await?;
                continue;
            }
            _ = tokio::time::sleep_until(due.unwrap_or_else(tokio::time::Instant::now)), if due.is_some() => {
                // The delay is over: the size lands now, and the device list announces it.
                // Reading and the other branches carry on while a resize waits this out.
                let bytes = {
                    let mut state = guard(&state);
                    let now = tokio::time::Instant::now();
                    let mut landed = Vec::new();
                    let mut keeping = Vec::new();
                    for resize in std::mem::take(&mut state.pending_resizes) {
                        if resize.connection == connection && resize.due <= now {
                            landed.push(resize);
                        } else {
                            keeping.push(resize);
                        }
                    }
                    state.pending_resizes = keeping;
                    let mut bytes = Vec::new();
                    for resize in landed {
                        if apply_resize(&mut state, resize.device, resize.zone, resize.leds) {
                            bytes.extend_from_slice(&device_list_updated());
                        }
                    }
                    bytes
                };
                if !bytes.is_empty() {
                    stream.write_all(&bytes).await?;
                    stream.flush().await?;
                }
                continue;
            }
        };
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
            // Packet 140 gets no direct reply (OpenRGB release_1.1, `NetworkServer.cpp:1355`). Its
            // detection announces itself as list changes (100, ungated, `:4017`) and, to a client
            // that negotiated protocol 6 or later only, as 101 and 103 (`:4003`, `:3949`). A
            // protocol 5 client sees the list cleared and then re-registered, and no 101 or 103.
            // These are writes of their own, not the reply slot of a request, so a stalled or
            // held reply does not hold them back (#290).
            if packet_id == REQUEST_RESCAN_DEVICES {
                let bytes = start_rescan(&mut guard(&state), connection);
                if !bytes.is_empty() {
                    stream.write_all(&bytes).await?;
                    stream.flush().await?;
                }
                continue;
            }
            // A request leaves the client waiting for a reply; a frame does not. The count says
            // whether a frame arrived while the client was still waiting for one.
            {
                let mut state = guard(&state);
                if is_request(packet_id) {
                    state.outstanding += 1;
                } else if packet_id == UPDATE_LEDS && !state.unregistered(device) {
                    if state.outstanding > 0 {
                        state.frames_while_waiting += 1;
                    }
                    if let Some(count) = state.led_writes.get_mut(device as usize) {
                        *count += 1;
                    }
                }
            }
            let reply = handle(&state, connection, device, packet_id, &payload);
            if packet_id == UPDATE_LEDS {
                // The colours are applied: wake a test that waits for them instead of polling.
                writes.send_modify(|n| *n += 1);
            }
            if reply.is_empty() {
                continue;
            }
            {
                let mut state = guard(&state);
                if state.holding {
                    state.held.push((connection, reply));
                    continue;
                }
                state.outstanding = state.outstanding.saturating_sub(1);
            }
            stream.write_all(&reply).await?;
            stream.flush().await?;
        }
    }
}

/// Starts a rescan for `connection`: counts packet 140 and, with detection on, clears the list
/// and announces it. Returns the bytes to send now.
fn start_rescan(state: &mut FakeState, connection: u64) -> Vec<u8> {
    state.rescan_count += 1;
    if state.detection_disabled {
        return Vec::new();
    }
    // The controllers OpenRGB had not found are found by this detection, in order after the rest.
    let hidden = std::mem::take(&mut state.hidden);
    if !hidden.is_empty() {
        state.devices.extend(hidden);
        let count = state.devices.len();
        state.led_writes.resize(count, 0);
        state.resize_writes.resize(count, 0);
    }
    state.rescan_listed = Some(0);
    state.rescan_runs.retain(|run| run.connection != connection);
    let due = tokio::time::Instant::now() + state.rescan_clear;
    state.rescan_runs.push(RescanRun { connection, due, next: 0 });
    let mut bytes = Vec::new();
    if state.negotiated.get(&connection).copied().unwrap_or(0) >= 6 {
        bytes.extend_from_slice(&encode_packet(0, DETECTION_STARTED, &[]).unwrap_or_default());
    }
    // The list was cleared.
    bytes.extend_from_slice(&device_list_updated());
    bytes
}

/// One step of a rescan: the next controller registers again (a list change), or, once all have,
/// detection is over. Returns the bytes to send now.
fn step_rescan(state: &mut FakeState, connection: u64) -> Vec<u8> {
    let Some(at) = state.rescan_runs.iter().position(|run| run.connection == connection) else {
        return Vec::new();
    };
    let gap = state.rescan_gap;
    if state.detection_hold {
        // Never settles: a list change at every gap and no controller back in the list.
        state.rescan_runs[at].due = tokio::time::Instant::now() + gap;
        return device_list_updated();
    }
    let total = state.devices.len();
    let next = state.rescan_runs[at].next;
    if next >= total {
        state.rescan_runs.remove(at);
        state.rescan_listed = None;
        if state.negotiated.get(&connection).copied().unwrap_or(0) >= 6 {
            return encode_packet(0, DETECTION_COMPLETE, &[]).unwrap_or_default();
        }
        return Vec::new();
    }
    state.rescan_listed = Some(next + 1);
    state.rescan_runs[at].next = next + 1;
    state.rescan_runs[at].due = tokio::time::Instant::now() + gap;
    device_list_updated()
}

/// Whether a packet is a request the client waits a reply for.
fn is_request(packet_id: u32) -> bool {
    matches!(packet_id, REQUEST_PROTOCOL_VERSION | REQUEST_CONTROLLER_COUNT | REQUEST_CONTROLLER_DATA)
}

fn to_io(error: canvas_proto::AdapterError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}

/// Applies a resize to the device, as OpenRGB does: a resizable zone takes the size within its
/// range (a zone that cannot resize keeps its own), and the controller's colours are blanked to
/// the new length. Returns whether the controller exists: OpenRGB announces a new device list
/// for any resize of a known controller, as this fake did before the delay hook.
fn apply_resize(state: &mut FakeState, device: u32, zone: usize, leds: u32) -> bool {
    let Some(target) = state.devices.get_mut(device as usize) else {
        return false;
    };
    if let Some(zone) = target.info.zones.get_mut(zone) {
        if zone.resizable && zone.min_leds <= zone.max_leds {
            zone.led_count = leds.max(zone.min_leds).min(zone.max_leds);
        }
    }
    target.colors = vec![Rgb8 { r: 0, g: 0, b: 0 }; target.info.led_count() as usize];
    true
}

/// The packet OpenRGB sends with no reply owed, to say the device list changed.
fn device_list_updated() -> Vec<u8> {
    encode_packet(0, DEVICE_LIST_UPDATED, &[]).unwrap_or_default()
}

/// Handles one client packet. Returns the bytes to send back (empty when there is no reply).
fn handle(state: &Mutex<FakeState>, connection: u64, device: u32, packet_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut state = guard(state);
    let reply = |device: u32, packet_id: u32, body: &[u8]| encode_packet(device, packet_id, body).unwrap_or_default();
    // A resize that fails silences the whole server, the resize packet included: real OpenRGB
    // gives the client silence rather than a refusal (see `fail_resizes`).
    if (state.stalled || state.failing_resizes) && is_request(packet_id) {
        state.swallowed += 1;
        return Vec::new();
    }
    match packet_id {
        REQUEST_PROTOCOL_VERSION => {
            // The server answers with the lower of its own highest version and the client's, and
            // keeps it for the connection: OpenRGB gates some broadcasts on it. A 4 models an old
            // OpenRGB that cannot re-detect (#290).
            let client = payload.get(..4).map_or(CLIENT_MAX_PROTOCOL_VERSION, |b| {
                u32::from_le_bytes([b[0], b[1], b[2], b[3]])
            });
            let version = client.min(state.server_protocol_version);
            state.negotiated.insert(connection, version);
            reply(0, packet_id, &version.to_le_bytes())
        }
        REQUEST_CONTROLLER_COUNT => {
            if state.rescan_listed.is_some() {
                state.reads_during_detection += 1;
            }
            reply(0, packet_id, &(state.visible() as u32).to_le_bytes())
        }
        REQUEST_CONTROLLER_DATA => match state.devices.get(device as usize).filter(|_| (device as usize) < state.visible()) {
            Some(target) => match wire_controller(target).encode() {
                Ok(body) => reply(device, packet_id, &body),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        },
        SET_CUSTOM_MODE | UPDATE_LEDS if state.unregistered(device) => {
            state.frames_to_unregistered += 1;
            Vec::new()
        }
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
            if let Some(count) = state.resize_writes.get_mut(device as usize) {
                *count += 1;
            }
            if state.failing_resizes {
                // Counted, and that is all: the size stays and no device list follows.
                return Vec::new();
            }
            if let (Some(zone_bytes), Some(size_bytes)) = (payload.get(..4), payload.get(4..8)) {
                let zone = u32::from_le_bytes([zone_bytes[0], zone_bytes[1], zone_bytes[2], zone_bytes[3]]) as usize;
                let leds = u32::from_le_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]);
                let delay = state.resize_delay;
                if !delay.is_zero() {
                    // The size lands after the delay, announced then; the request itself is
                    // over, so the other requests keep flowing in the meantime.
                    state.pending_resizes.push(PendingResize {
                        connection,
                        device,
                        zone,
                        leds,
                        due: tokio::time::Instant::now() + delay,
                    });
                    return Vec::new();
                }
                if apply_resize(&mut state, device, zone, leds) {
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
