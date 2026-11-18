//! The OpenRGB supervisor.
//!
//! Milestone 1: the service does not exist yet, so the daemon starts OpenRGB itself,
//! non-elevated, and supervises it. When the service is up and reports that it owns
//! OpenRGB, the daemon only connects to it.
//!
//! The supervisor reconnects with backoff (250 ms to 5 s), re-applies the saved zone
//! sizes on every connect (OpenRGB does not persist SDK resizes), re-discovers the
//! devices and re-sets the engine layout.
//!
//! It never launches a real OpenRGB in the tests: the tests use the fake server.

use canvas_proto::api::LinkState;
use canvas_proto::device::DeviceInfo;
use canvas_proto::pipe::{OpenRgbServerState, PipeRequest, PipeResponse};
use canvas_proto::traits::DeviceAdapter;
use std::sync::Arc;
use std::time::Duration;

use crate::pipe_client::PipeClient;
use crate::state::{ZoneHolds, ZoneSizes};

/// The backoff bounds for reconnecting to OpenRGB.
pub const BACKOFF_MIN: Duration = Duration::from_millis(250);
pub const BACKOFF_MAX: Duration = Duration::from_secs(5);

/// How many times OpenRGB may be restarted in the restart window.
pub const RESTART_LIMIT: u32 = 3;
/// The restart window, in seconds.
pub const RESTART_WINDOW_S: u64 = 300;

/// Where the OpenRGB executable is looked for.
///
/// `CANVAS_RGB_OPENRGB` wins; otherwise it sits next to the daemon in `openrgb\`.
pub fn openrgb_exe_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("CANVAS_RGB_OPENRGB") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    exe_dir().join("openrgb").join("OpenRGB.exe")
}

/// The directory the daemon executable lives in.
pub fn exe_dir() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// The arguments the daemon passes to the OpenRGB server it starts.
///
/// The server binds 127.0.0.1 only: nothing in canvas-rgb listens on a non-loopback
/// address.
pub fn openrgb_args(port: u16) -> Vec<String> {
    vec![
        "--server".into(),
        "--server-host".into(),
        canvas_proto::consts::OPENRGB_HOST.into(),
        "--server-port".into(),
        port.to_string(),
        "--noautoconnect".into(),
        "--config".into(),
        crate::paths::openrgb_config_dir().to_string_lossy().into_owned(),
    ]
}

/// A backoff that grows from [`BACKOFF_MIN`] to [`BACKOFF_MAX`].
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    current: Duration,
}

impl Backoff {
    pub fn new() -> Self {
        Self { current: BACKOFF_MIN }
    }

    /// The delay to wait before the next attempt.
    pub fn current(&self) -> Duration {
        self.current
    }

    /// Grows the delay and returns the new one. The backoff itself does not change, so
    /// the caller keeps the value it gets.
    pub fn next(&self) -> Duration {
        (self.current * 2).min(BACKOFF_MAX)
    }

    /// The next delay, and the backoff that follows it.
    pub fn advanced(&self) -> (Duration, Self) {
        let next = self.next();
        (next, Self { current: next })
    }

    /// Resets the delay after a successful connect.
    pub fn reset(&self) -> Self {
        Self { current: BACKOFF_MIN }
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

/// Counts restarts in a sliding window.
#[derive(Debug, Default)]
pub struct RestartBudget {
    times: Vec<std::time::Instant>,
}

impl RestartBudget {
    /// Records a restart. Returns false when the budget is exhausted.
    pub fn record(&mut self) -> bool {
        let now = std::time::Instant::now();
        self.times.retain(|t| now.duration_since(*t).as_secs() < RESTART_WINDOW_S);
        if self.times.len() as u32 >= RESTART_LIMIT {
            return false;
        }
        self.times.push(now);
        true
    }
}

/// The state the supervisor keeps about the OpenRGB process it started.
pub struct ServerProcess {
    child: std::process::Child,
}

impl ServerProcess {
    pub fn new(child: std::process::Child) -> Self {
        Self { child }
    }

    /// Whether the process is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Stops the process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Starts OpenRGB with no window (`CREATE_NO_WINDOW`).
///
/// Returns `None` when the executable is missing; the daemon then keeps running with
/// `openrgb: down`.
#[cfg(windows)]
pub fn start_server(exe: &std::path::Path, args: &[String]) -> Result<ServerProcess, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if !exe.exists() {
        return Err(format!("{} does not exist", exe.display()));
    }
    let config_dir = crate::paths::openrgb_config_dir();
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;

    // token: the daemon's own (the logged-on user, non-elevated).
    let child = std::process::Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    Ok(ServerProcess::new(child))
}

#[cfg(not(windows))]
pub fn start_server(exe: &std::path::Path, args: &[String]) -> Result<ServerProcess, String> {
    if !exe.exists() {
        return Err(format!("{} does not exist", exe.display()));
    }
    // token: the daemon's own (the logged-on user, non-elevated).
    let child = std::process::Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    Ok(ServerProcess::new(child))
}

/// Whether something already answers on the OpenRGB port.
pub async fn port_is_open(host: &str, port: u16) -> bool {
    tokio::net::TcpStream::connect((host, port)).await.is_ok()
}

/// Asks the service whether it owns the OpenRGB server.
///
/// The service does not exist yet in milestone 1, so this normally fails and the daemon
/// starts OpenRGB itself.
pub async fn service_owns_openrgb() -> Option<OpenRgbServerState> {
    let mut client = match PipeClient::connect().await {
        Ok(client) => client,
        Err(_) => return None,
    };
    let hello = PipeRequest::Hello {
        protocol: canvas_proto::pipe::PROTOCOL_VERSION,
        daemon_pid: std::process::id(),
    };
    if client.request(&hello).await.is_err() {
        return None;
    }
    match client.request(&PipeRequest::OpenRgbState).await {
        Ok(PipeResponse::OpenRgb { state, .. }) => Some(state),
        _ => None,
    }
}

/// Starts the OpenRGB server, unless the service owns it or something already answers.
///
/// Returns the process when the daemon started it; the caller kills it on quit.
pub async fn ensure_openrgb_running(port: u16) -> Option<ServerProcess> {
    // The service owns OpenRGB in milestone 3: it is already running.
    if let Some(state) = service_owns_openrgb().await {
        if state == OpenRgbServerState::Running {
            tracing::info!("the service owns the OpenRGB server");
            return None;
        }
    }

    // Something else already answers on the port: connect to it, do not start a second
    // server.
    if port_is_open(canvas_proto::consts::OPENRGB_HOST, port).await {
        tracing::info!("OpenRGB is already running on the port");
        return None;
    }

    // Milestone 1: the daemon starts OpenRGB itself, non-elevated.
    let exe = openrgb_exe_path();
    let args = openrgb_args(port);
    match start_server(&exe, &args) {
        Ok(process) => {
            tracing::info!(exe = %exe.display(), "the daemon started the OpenRGB server");
            Some(process)
        }
        Err(e) => {
            tracing::warn!(error = %e, "the OpenRGB server could not be started");
            None
        }
    }
}

/// Connects to OpenRGB, re-applies the zone sizes and re-discovers the devices.
///
/// Returns the devices as OpenRGB reports them after the zone sizes were applied.
/// What decides "the device list changed": ids, connection, and every zone's LED count.
fn device_key(devices: &[DeviceInfo]) -> Vec<(String, bool, Vec<u32>)> {
    let mut key: Vec<_> = devices
        .iter()
        .map(|d| (d.id.0.clone(), d.connected, d.zones.iter().map(|z| z.led_count).collect()))
        .collect();
    key.sort();
    key
}

/// How long a rescan may hold the adapter for its re-read: about one reply timeout of the OpenRGB
/// adapter (5 s). The handlers that take the adapter lock (a zone resize, the first run, the
/// keyboard's frame writer) wait for it, so a stalled OpenRGB delays them this long and no longer
/// (#233). The OpenRGB frame writers do not wait: they write through the adapter's own link
/// (`App::frame_lane`), so those lights keep moving. The adapter's own reply timeout ends a single
/// stalled request; this ends a slow trickle of them.
///
/// The budget covers the lock wait and the re-read only. The wait for OpenRGB's detection to go
/// quiet (seconds) runs before it, on the frame lane and with no adapter lock, bounded by
/// `RescanTiming::cap` (#290). A detection that times out therefore leaves the whole budget for
/// the re-read, and a rescan that gives up on detection can never cut the re-read short and drop
/// the link. Only a re-read that is itself slower than the budget (OpenRGB stalled) does.
pub const RESCAN_BUDGET: Duration = Duration::from_secs(5);

/// How long `GET /api/v1/devices?rescan=1` waits for the supervisor past the rescan's own budget:
/// the supervisor may be partway through its own refresh when the request arrives. The editor
/// gives the request `DEVICE_RESCAN_TIMEOUT_MS` (30 s, `app/web/src/lib/api.ts`), so the daemon
/// answers first.
pub const RESCAN_QUEUE_WAIT: Duration = Duration::from_secs(5);

/// A rescan the owner asked for, sent to the supervisor (#233). The supervisor is the one task
/// that reads devices and applies them, so a rescan takes the same path as every other list.
#[derive(Debug)]
pub struct Rescan {
    /// How long the supervisor may hold the adapter for it.
    pub budget: Duration,
    /// The devices OpenRGB reported, already applied; or why it could not be done.
    pub reply: tokio::sync::oneshot::Sender<Result<Vec<DeviceInfo>, String>>,
}

/// What a rescan read.
#[derive(Debug)]
pub struct RescanReading {
    /// The devices to answer with.
    pub devices: Vec<DeviceInfo>,
    /// `devices` is the list the supervisor already held: OpenRGB's list is still changing (or is
    /// empty), so nothing was read and there is nothing to apply again. The adapter keeps its own
    /// list the same way, and the settled list reaches the 5 s refresh by itself.
    pub kept_held: bool,
}

/// Whether a rescan asked OpenRGB to re-detect, and what that took.
#[derive(Clone, Copy, Debug)]
pub enum Detected {
    /// OpenRGB was not asked, or could not be: the list is read as it stands.
    No,
    /// OpenRGB re-detected and its list went quiet; `settled_after` is how long, when known.
    /// `by_events`: OpenRGB said the detection was over itself (103, protocol 6 and later), so a list
    /// read now, empty or not, is the real one; without it an empty list cannot be told from a
    /// detection that has found nothing yet.
    Yes { settled_after: Option<Duration>, by_events: bool },
}

/// How the detection part of a rescan went.
#[derive(Debug)]
enum Detection {
    /// There was nothing to ask: no frame lane, or the link is down.
    NotAsked,
    Redetected(canvas_adapters::openrgb::Redetected),
    /// Nothing was re-detected and the list was not touched: reading it is safe.
    NotRedetected,
    /// OpenRGB's list is still changing, or empty, after the wait: detection is still running.
    Unsettled,
    Failed(String),
}

/// How long a list that lacks a device the daemon holds must stay the same, with no detection heard, before the
/// daemon believes the device is gone (#290).
///
/// release_1.1 sends a client that connects nothing of a detection already running (`NetworkServer.cpp:886` is a
/// broadcast; the server starts before detection, `ResourceManager.cpp:1362` then `:1377`), and no protocol query
/// says whether one is. A first read right after the connect, or a read during a detection the client missed,
/// can be empty or a part list, so a list that drops a device is not believed on sight. A 103, or the owner's
/// own rescan, settles it at once. This bound is for what neither says: a hot-plug removal (a 100, no 103). It
/// bounds how long an unplug can stay hidden; it does not guard against a wipe (a wipe is stopped by the rule
/// that nothing missing is believed without a 103 or a rescan, and by an empty list never being believed
/// here). A wrong value only delays showing an unplug: 30 s is longer than the detections seen (the
/// 15 s a rescan waits for one) and short enough that an unplugged stick leaves the list within the minute.
pub const UNPLUG_QUIET: Duration = Duration::from_secs(30);

/// What the frame lane knows of the list it last read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ListView {
    /// A read after a 103 found the list, and no list change was heard since (`OpenRgbAdapter::list_is_final`).
    final_list: bool,
    /// The 101 and 102 heard on this connection (`OpenRgbAdapter::detection_signals`).
    signals: u64,
}

/// The lane's view of its list. With no lane, or no link, nothing says the list is part of a detection, so
/// it is final.
fn list_view(lane: Option<&canvas_adapters::openrgb::OpenRgbAdapter>) -> ListView {
    match lane {
        Some(lane) if lane.is_connected() => ListView { final_list: lane.list_is_final(), signals: lane.detection_signals() },
        _ => ListView { final_list: true, signals: 0 },
    }
}

/// What to do with a list the supervisor has just read.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Apply,
    /// Keep the list held: the read dropped a device it cannot be told was unplugged.
    Keep,
}

/// A list that drops devices, and since when it has stayed the same.
struct Seen {
    key: Vec<(String, bool, Vec<u32>)>,
    since: tokio::time::Instant,
    signals: u64,
}

/// The rule for what the daemon applies from OpenRGB's list, asymmetric because the client cannot know whether
/// a detection is running (see [`UNPLUG_QUIET`]):
///
/// 1. a list that keeps every held device applies at once, final or not (growing is always safe);
/// 2. a list that drops a held device applies only when OpenRGB's list is final (a 103 was heard and no list
///    change since), when it is the answer to the owner's own rescan, or when the same list has been read
///    for [`UNPLUG_QUIET`] with no 101 or 102 heard in that time;
/// 3. an empty list replaces a held one only by the first two of those, never by waiting.
///
/// With nothing held, anything applies.
#[derive(Default)]
struct Shrinks {
    seen: Option<Seen>,
}

impl Shrinks {
    /// A new connection counts its own 101 and 102 from zero: what was seen on the last one means nothing.
    fn reset(&mut self) {
        self.seen = None;
    }

    /// `by_rescan`: `read` is what the owner's rescan returned after OpenRGB said its detection was over.
    fn judge(
        &mut self,
        held: &[DeviceInfo],
        read: &[DeviceInfo],
        list: ListView,
        by_rescan: bool,
        now: tokio::time::Instant,
    ) -> Verdict {
        let drops_a_device = held.iter().any(|kept| !read.iter().any(|device| device.id == kept.id));
        if !drops_a_device || list.final_list || by_rescan {
            self.seen = None;
            return Verdict::Apply;
        }
        if read.is_empty() {
            self.seen = None;
            return Verdict::Keep;
        }
        let key = device_key(read);
        match &self.seen {
            Some(seen) if seen.key == key && seen.signals == list.signals => {
                if now.duration_since(seen.since) >= UNPLUG_QUIET {
                    self.seen = None;
                    return Verdict::Apply;
                }
            }
            _ => self.seen = Some(Seen { key, since: now, signals: list.signals }),
        }
        Verdict::Keep
    }
}

/// Asks OpenRGB to re-detect on the frame lane and waits for its list to go quiet, holding no
/// adapter lock (#290): the wait takes seconds, and the keyboard's frame writer and the API
/// handlers wait on that lock.
async fn detect_on_lane(lane: &canvas_adapters::openrgb::OpenRgbAdapter) -> Detection {
    use canvas_adapters::openrgb::{rescan_miss, RescanMiss};
    match lane.rescan_shared().await {
        Ok(done) => Detection::Redetected(done),
        Err(error) => match rescan_miss(&error) {
            Some(RescanMiss::NotRedetected) => {
                tracing::info!(error = %error, "OpenRGB did not re-detect; reading the list as it stands");
                Detection::NotRedetected
            }
            Some(RescanMiss::Unsettled) => {
                // The API can say only the list it holds: `DevicesResponse` has no field for "detection
                // is still running" (a spine change), so the log says it.
                tracing::warn!(
                    error = %error,
                    "OpenRGB is still detecting devices: answering with the list held; the settled list is applied when OpenRGB's list stops changing"
                );
                Detection::Unsettled
            }
            None => Detection::Failed(format!("could not rescan OpenRGB devices: {error}")),
        },
    }
}

/// A rescan the owner asked for (#233): reads the devices OpenRGB reports, after asking it to look
/// at the hardware again when `detected` does not say it already did (#290).
///
/// `ask` is for an adapter with no frame lane: it is asked through `adapter.rescan()` here, under
/// the lock the caller holds. The daemon's adapter has a lane and asks it before taking the lock
/// (see [`supervise_with_lane`]), so it passes `ask = false` and says what happened in `detected`.
///
/// A list is read only when it can be trusted. An adapter that cannot re-detect, or OpenRGB
/// starting no detection, is logged and the list is read as it stands. A detection that is still
/// running reads nothing and answers with the `held` list: OpenRGB's list is mid-detection and may
/// be empty. A list that comes back empty while `held` has devices is not applied either, unless
/// OpenRGB said itself that the detection is over (`by_events`, protocol 6 and later): without that, an
/// empty list is a detection that has not found its first controller yet and cannot be told from every
/// device gone, and this keeps the lights managed. With it, an empty list is every device gone. Any other failure keeps PR #289's contract: an error and
/// the held list alone. With no connection the adapter connects first, so a rescan is also a retry
/// of the link.
pub async fn rescan_devices(
    adapter: &mut dyn DeviceAdapter,
    zone_sizes: &std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>,
    held: &[DeviceInfo],
    detected: Detected,
    ask: bool,
) -> Result<RescanReading, String> {
    use canvas_adapters::openrgb::{rescan_miss, RescanMiss};
    let keep_held = || RescanReading { devices: held.to_vec(), kept_held: true };
    let mut detected = detected;
    if ask && !adapter.is_connected() {
        // The link is down: connecting reads the list OpenRGB has now, and OpenRGB is not asked to
        // look at the hardware again. Say so, or "rescanned" would claim more than was done.
        tracing::info!("OpenRGB rescan: not connected; connecting and re-reading the list only, OpenRGB was not asked to re-detect");
    }
    if ask && adapter.is_connected() {
        match adapter.rescan().await {
            Ok(()) => detected = Detected::Yes { settled_after: None, by_events: false },
            Err(error) => match rescan_miss(&error) {
                Some(RescanMiss::NotRedetected) => {
                    tracing::info!(error = %error, "OpenRGB did not re-detect; reading the list as it stands");
                }
                Some(RescanMiss::Unsettled) => {
                    tracing::warn!(
                        error = %error,
                        "OpenRGB is still detecting devices: answering with the list held; the settled list is applied when OpenRGB's list stops changing"
                    );
                    return Ok(keep_held());
                }
                None => return Err(format!("could not rescan OpenRGB devices: {error}")),
            },
        }
    }
    let devices = if adapter.is_connected() {
        refresh_devices(adapter, zone_sizes).await?
    } else {
        connect_and_prepare(adapter, zone_sizes).await?
    };
    if let Detected::Yes { settled_after, by_events } = detected {
        if devices.is_empty() && !held.is_empty() && !by_events {
            tracing::warn!(
                held = held.len(),
                "OpenRGB re-detected and reports no devices; keeping the list held (an empty list is not told from a detection that has not found its first controller)"
            );
            return Ok(keep_held());
        }
        match settled_after {
            Some(after) => tracing::info!(
                count = devices.len(),
                settled_ms = after.as_millis() as u64,
                "OpenRGB re-detected: {} devices, list settled after {} ms",
                devices.len(),
                after.as_millis()
            ),
            None => tracing::info!(count = devices.len(), "OpenRGB re-detected: {} devices", devices.len()),
        }
    }
    Ok(RescanReading { devices, kept_held: false })
}

/// The sizes the supervisor applies: the saved sizes with each hold the setups took overlaid on
/// them (#338).
///
/// Chain setup raises a header to its longest length while the owner drags; without this the 5 s
/// refresh reads the raise as an unsaved size and resizes it back mid-drag. A hold is never saved:
/// it lives in `App::zone_holds` and ends when the setup drops it, and then the saved size returns.
pub fn effective_sizes(saved: &ZoneSizes, holds: &ZoneHolds) -> ZoneSizes {
    let mut sizes = saved.clone();
    for ((device, zone), leds) in holds {
        sizes.entry(device.clone()).or_default().insert(*zone, *leds);
    }
    sizes
}

/// The saved sizes with the holds overlaid, read now. Callers hold the adapter lock, so no one
/// who takes these two locks may hold either while waiting for the adapter (#338).
async fn current_sizes(
    zone_sizes: &tokio::sync::RwLock<ZoneSizes>,
    holds: &tokio::sync::RwLock<ZoneHolds>,
) -> ZoneSizes {
    let saved = zone_sizes.read().await.clone();
    let held = holds.read().await.clone();
    effective_sizes(&saved, &held)
}

/// Re-reads the device list on a live connection. A saved zone size that the device doesn't
/// have yet (a device that appeared since connect) is applied, then the list is read again.
async fn refresh_devices(
    adapter: &mut dyn DeviceAdapter,
    zone_sizes: &std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>,
) -> Result<Vec<DeviceInfo>, String> {
    let devices = adapter.discover().await.map_err(|e| format!("discovery failed: {e}"))?;
    let mut resized = false;
    for device in &devices {
        let Some(sizes) = zone_sizes.get(&device.id.0) else {
            continue;
        };
        for (zone, leds) in sizes {
            let current = device.zones.iter().find(|z| z.index == *zone).map(|z| z.led_count);
            if current.is_some_and(|c| c != *leds) {
                match adapter.set_zone_size(&device.id, *zone, *leds).await {
                    Ok(()) => {
                        resized = true;
                        tracing::info!(device = %device.id, zone, leds, "applied the saved zone size");
                    }
                    Err(e) => tracing::warn!(device = %device.id, zone, leds, error = %e, "could not apply the saved zone size"),
                }
            }
        }
    }
    if resized {
        adapter.discover().await.map_err(|e| format!("discovery failed: {e}"))
    } else {
        Ok(devices)
    }
}

pub async fn connect_and_prepare(
    adapter: &mut dyn DeviceAdapter,
    zone_sizes: &std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>,
) -> Result<Vec<DeviceInfo>, String> {
    adapter.connect().await.map_err(|e| format!("could not connect to OpenRGB: {e}"))?;

    let mut devices = adapter.discover().await.map_err(|e| format!("discovery failed: {e}"))?;

    // OpenRGB forgets SDK resizes when it restarts, so they are re-applied on every
    // connect (the done-line for the JARGB headers).
    for device in &mut devices {
        let Some(sizes) = zone_sizes.get(&device.id.0) else {
            continue;
        };
        for (zone, leds) in sizes {
            match adapter.set_zone_size(&device.id, *zone, *leds).await {
                Ok(()) => {
                    tracing::info!(device = %device.id, zone, leds, "re-applied the zone size");
                }
                Err(e) => {
                    tracing::warn!(device = %device.id, zone, leds, error = %e, "could not re-apply the zone size");
                }
            }
        }
    }

    // Discover again so the devices carry the new LED counts.
    if !zone_sizes.is_empty() {
        devices = adapter
            .discover()
            .await
            .map_err(|e| format!("rediscovery failed: {e}"))?;
    }

    Ok(devices)
}

/// The reconnect loop: keeps trying to connect to OpenRGB, applying the zones each time.
///
/// The loop stops when `stop` is signalled.
pub async fn supervise(
    adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>>,
    zone_sizes: Arc<tokio::sync::RwLock<std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>>>,
    mut on_devices: impl FnMut(Vec<DeviceInfo>),
    mut on_link: impl FnMut(LinkState),
    stop: tokio::sync::watch::Receiver<bool>,
) {
    supervise_with_rescans(
        adapter,
        zone_sizes,
        // Nothing holds a zone here: this path sizes to the saved sizes only.
        Arc::new(tokio::sync::RwLock::new(ZoneHolds::new())),
        move |devices| {
            on_devices(devices);
            std::future::ready(())
        },
        move |link| {
            on_link(link);
            std::future::ready(())
        },
        None,
        stop,
    )
    .await;
}

/// Waits for the next rescan; never answers when there is no inbox or it closed.
async fn next_rescan(inbox: &mut Option<tokio::sync::mpsc::Receiver<Rescan>>) -> Rescan {
    if let Some(receiver) = inbox {
        if let Some(rescan) = receiver.recv().await {
            return rescan;
        }
        *inbox = None;
    }
    std::future::pending().await
}

/// Runs `work` for at most `budget` (no limit for `None`). A timeout is an error, and the flag says
/// the work was cut off: its future was dropped mid-way, so it may have left a half-made state.
async fn within<T>(
    budget: Option<Duration>,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> (Result<T, String>, bool) {
    match budget {
        None => (work.await, false),
        Some(budget) => match tokio::time::timeout(budget, work).await {
            Ok(result) => (result, false),
            Err(_) => (Err("OpenRGB did not answer in time".to_string()), true),
        },
    }
}

/// Moves the rescans that already queued (and whose owner still waits) into `waiting`: they share
/// the next attempt and its answer.
fn take_queued(inbox: &mut Option<tokio::sync::mpsc::Receiver<Rescan>>, waiting: &mut Vec<Rescan>) {
    if let Some(receiver) = inbox {
        while let Ok(rescan) = receiver.try_recv() {
            waiting.push(rescan);
        }
    }
    // An owner that gave up needs no answer, and no attempt on its account.
    waiting.retain(|rescan| !rescan.reply.is_closed());
}

/// Answers every waiting rescan with the same result.
fn answer_all(waiting: &mut Vec<Rescan>, result: &Result<Vec<DeviceInfo>, String>) {
    for rescan in waiting.drain(..) {
        let _ = rescan.reply.send(result.clone());
    }
}

/// The delay before the next attempt.
///
/// The backoff paces the automatic attempts. An attempt the owner asked for does not make the
/// next automatic one wait longer: the backoff is not advanced for it, and the delay it waits is
/// the one it already had.
fn next_delay(backoff: &mut Backoff, from_rescan: bool) -> Duration {
    if from_rescan {
        backoff.current()
    } else {
        let (delay, next_backoff) = backoff.advanced();
        *backoff = next_backoff;
        delay
    }
}

/// [`supervise`], with callbacks the supervisor awaits and the rescans the owner asks for (#233).
///
/// `on_devices` is awaited before the loop goes on, so the lists are applied in the order they
/// were read; a rescan is answered after its list was applied. A rescan while the link is down
/// ends the wait for the next attempt and tries to connect at once, within its budget; rescans
/// that arrive meanwhile share that attempt and its answer. A rescan never advances the automatic
/// backoff, and a rescan whose owner gave up starts nothing.
pub async fn supervise_with_rescans<D, L>(
    adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>>,
    zone_sizes: Arc<tokio::sync::RwLock<std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>>>,
    holds: Arc<tokio::sync::RwLock<ZoneHolds>>,
    on_devices: impl FnMut(Vec<DeviceInfo>) -> D,
    on_link: impl FnMut(LinkState) -> L,
    rescans: Option<tokio::sync::mpsc::Receiver<Rescan>>,
    stop: tokio::sync::watch::Receiver<bool>,
) where
    D: std::future::Future<Output = ()>,
    L: std::future::Future<Output = ()>,
{
    supervise_with_lane(adapter, None, zone_sizes, holds, on_devices, on_link, rescans, stop).await;
}

/// [`supervise_with_rescans`], with the frame lane of the OpenRGB link (`App::frame_lane`).
///
/// A rescan asks OpenRGB to re-detect and waits for its list to go quiet on the lane, which shares
/// the link and needs no adapter lock: the wait takes seconds, and the keyboard's frame writer
/// (`crates/canvas-daemon/src/writer.rs`, the non-OpenRGB path) and the API handlers take that
/// lock. The lock is taken afterwards, for the re-read only, within the rescan's budget (#290).
/// With no lane the adapter is asked under the lock, as the trait says.
#[allow(clippy::too_many_arguments)]
pub async fn supervise_with_lane<D, L>(
    adapter: Arc<tokio::sync::Mutex<Box<dyn DeviceAdapter>>>,
    lane: Option<canvas_adapters::openrgb::OpenRgbAdapter>,
    zone_sizes: Arc<tokio::sync::RwLock<std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>>>,
    holds: Arc<tokio::sync::RwLock<ZoneHolds>>,
    mut on_devices: impl FnMut(Vec<DeviceInfo>) -> D,
    mut on_link: impl FnMut(LinkState) -> L,
    mut rescans: Option<tokio::sync::mpsc::Receiver<Rescan>>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) where
    D: std::future::Future<Output = ()>,
    L: std::future::Future<Output = ()>,
{
    // The list the supervisor last applied: what a rescan that cannot be trusted answers with, and what
    // stays applied across a reconnect that lands in the middle of OpenRGB's detection (#290). It outlives
    // the connection: a restart of OpenRGB must not replace it with a part of its detection.
    let mut held: Vec<DeviceInfo> = Vec::new();
    let mut shrinks = Shrinks::default();
    let mut backoff = Backoff::new();
    // The rescans that ended the wait for the next attempt: that attempt answers them.
    let mut waiting: Vec<Rescan> = Vec::new();
    loop {
        if *stop.borrow() {
            return;
        }
        take_queued(&mut rescans, &mut waiting);
        let from_rescan = !waiting.is_empty();
        // Every attempt is "connecting"; `Down` only follows a failure.
        on_link(LinkState::Connecting).await;
        let (result, cut) = within(waiting.iter().map(|r| r.budget).max(), async {
            let mut guard = adapter.lock().await;
            // Read under the adapter lock: a setup inserts its hold before it takes the lock to
            // raise, so sizes read here can never be older than a raise the device already has.
            let sizes = current_sizes(&zone_sizes, &holds).await;
            connect_and_prepare(guard.as_mut(), &sizes).await
        })
        .await;
        // The rescans that queued while the attempt ran share its answer: one attempt answers
        // them all, and none of them waits for another's turn.
        take_queued(&mut rescans, &mut waiting);
        if cut {
            // The attempt was dropped mid-way, possibly between the socket and the handshake: a
            // link that says "connected" with no protocol version would be taken for a good one.
            let _ = adapter.lock().await.disconnect().await;
        }
        match result {
            Ok(devices) => {
                tracing::info!(count = devices.len(), "connected to OpenRGB");
                // A connection hears nothing of a detection already running, so its first read may be empty or
                // a part of the detection's (`Shrinks`): a list that adds devices applies, one that drops a held
                // device waits for a 103, the owner's rescan or the quiet period. With no list held, the read is
                // what there is.
                shrinks.reset();
                let devices = match shrinks.judge(&held, &devices, list_view(lane.as_ref()), false, tokio::time::Instant::now()) {
                    Verdict::Keep => {
                        tracing::info!(
                            read = devices.len(),
                            held = held.len(),
                            "OpenRGB's list lacks devices the daemon holds and nothing says it is final: keeping the list held"
                        );
                        held.clone()
                    }
                    Verdict::Apply => devices,
                };
                if from_rescan {
                    tracing::info!(
                        "OpenRGB rescan: the link was down; connected and read the list, OpenRGB was not asked to re-detect"
                    );
                }
                let mut last = device_key(&devices);
                held = devices.clone();
                on_devices(devices.clone()).await;
                answer_all(&mut waiting, &Ok(devices));
                backoff = backoff.reset();
                // Wait for the connection to drop before trying again. Every 5 s, re-read the
                // device list: OpenRGB may still be detecting when we connect (seen live: 0
                // devices at connect, never refreshed), and devices can come and go.
                //
                // One interval for the whole connection: a stream of rescans must not push the
                // link check and the refresh back by restarting a sleep each time.
                let mut tick = tokio::time::interval(Duration::from_millis(500));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                tick.tick().await;
                let mut ticks = 0u32;
                loop {
                    if *stop.borrow() {
                        return;
                    }
                    tokio::select! {
                        _ = tick.tick() => {}
                        rescan = next_rescan(&mut rescans) => {
                            // The owner gave up on it: do not hold the adapter for nobody.
                            if rescan.reply.is_closed() {
                                continue;
                            }
                            // The rescans that queued meanwhile share this attempt and its
                            // answer: one discovery answers them all, and none of them waits for
                            // another's turn (#233).
                            let mut group = vec![rescan];
                            let budget = group.iter().map(|r| r.budget).max();
                            // OpenRGB is asked first, on the lane and with no adapter lock: the
                            // wait for its list to go quiet takes seconds (#290).
                            let detection = match &lane {
                                Some(lane) if lane.is_connected() => detect_on_lane(lane).await,
                                _ => {
                                    if lane.is_some() {
                                        tracing::info!("OpenRGB rescan: not connected; re-reading the list only, OpenRGB was not asked to re-detect");
                                    }
                                    Detection::NotAsked
                                }
                            };
                            // OpenRGB re-detected for this rescan: the list read now is its own result.
                            let by_rescan = matches!(detection, Detection::Redetected(_));
                            let (result, cut) = match detection {
                                Detection::Failed(error) => (Err(error), false),
                                // Mid-detection: read nothing, answer with the list held.
                                Detection::Unsettled => (Ok(RescanReading { devices: held.clone(), kept_held: true }), false),
                                other => {
                                    let detected = match other {
                                        Detection::Redetected(done) => Detected::Yes {
                                            settled_after: Some(done.settled_after),
                                            by_events: done.by_events,
                                        },
                                        _ => Detected::No,
                                    };
                                    let ask = lane.is_none();
                                    within(budget, async {
                                        let mut guard = adapter.lock().await;
                                        let sizes = current_sizes(&zone_sizes, &holds).await;
                                        rescan_devices(guard.as_mut(), &sizes, &held, detected, ask).await
                                    })
                                    .await
                                }
                            };
                            // The rescans that queued while the attempt ran share its answer.
                            take_queued(&mut rescans, &mut group);
                            if cut {
                                // The attempt was dropped mid-way: a saved zone size may have
                                // been applied and the list not read again. The link is dropped,
                                // as on the connect path, so the next attempt reads everything
                                // again instead of trusting half-applied state.
                                let _ = adapter.lock().await.disconnect().await;
                            }
                            let result = match result {
                                Ok(reading) => {
                                    // A read that was not the result of OpenRGB's own re-detection (it did not
                                    // start one, or none could be asked) is judged like any other.
                                    let reading = if reading.kept_held {
                                        reading
                                    } else {
                                        let now = tokio::time::Instant::now();
                                        match shrinks.judge(&held, &reading.devices, list_view(lane.as_ref()), by_rescan, now) {
                                            Verdict::Apply => reading,
                                            Verdict::Keep => RescanReading { devices: held.clone(), kept_held: true },
                                        }
                                    };
                                    if !reading.kept_held {
                                        last = device_key(&reading.devices);
                                        held = reading.devices.clone();
                                        on_devices(reading.devices.clone()).await;
                                    }
                                    Ok(reading.devices)
                                }
                                Err(error) => Err(error),
                            };
                            answer_all(&mut group, &result);
                            continue;
                        }
                    }
                    let connected = {
                        let guard = adapter.lock().await;
                        guard.is_connected()
                    };
                    if !connected {
                        tracing::info!("OpenRGB went away; reconnecting");
                        on_link(LinkState::Connecting).await;
                        break;
                    }
                    ticks += 1;
                    // A detection that ended while the list held is a part of it: read the final one now,
                    // not at the next 5 s refresh.
                    let end_unread = lane.as_ref().is_some_and(|lane| lane.detection_end_unread());
                    if !ticks.is_multiple_of(10) && !end_unread {
                        continue;
                    }
                    let refreshed = {
                        let mut guard = adapter.lock().await;
                        let sizes = current_sizes(&zone_sizes, &holds).await;
                        refresh_devices(guard.as_mut(), &sizes).await
                    };
                    // The adapter answers with the list it holds while OpenRGB's is still changing, so
                    // a list read mid-detection is never a different list here (#290).
                    match refreshed {
                        Ok(devices) => {
                            let now = tokio::time::Instant::now();
                            match shrinks.judge(&held, &devices, list_view(lane.as_ref()), false, now) {
                                // A list that drops a device nothing says is gone: what OpenRGB lists may be a
                                // part of a detection, and the list held stays (#290).
                                Verdict::Keep => {
                                    tracing::debug!(read = devices.len(), held = held.len(), "OpenRGB's list lacks held devices and is not final; keeping the list held");
                                }
                                Verdict::Apply if device_key(&devices) != last => {
                                    tracing::info!(count = devices.len(), "the OpenRGB device list changed");
                                    last = device_key(&devices);
                                    held = devices.clone();
                                    on_devices(devices).await;
                                }
                                Verdict::Apply => {}
                            }
                        }
                        Err(e) => tracing::debug!(error = %e, "could not re-read the OpenRGB devices"),
                    }
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "OpenRGB is not reachable yet");
                answer_all(&mut waiting, &Err(e));
                on_link(LinkState::Down).await;
                // The backoff paces the automatic attempts: an attempt the owner asked for does
                // not make the next automatic one wait longer.
                let delay = next_delay(&mut backoff, from_rescan);
                let pause = tokio::time::sleep(delay);
                tokio::pin!(pause);
                loop {
                    tokio::select! {
                        () = &mut pause => break,
                        _ = stop.changed() => return,
                        rescan = next_rescan(&mut rescans) => {
                            // An owner that gave up does not cut the wait short.
                            if rescan.reply.is_closed() {
                                continue;
                            }
                            waiting.push(rescan);
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn effective_sizes_overlays_a_hold_on_a_saved_size() {
        let saved = ZoneSizes::from([(
            "openrgb:mb".to_string(),
            BTreeMap::from([(0u32, 120u32), (1u32, 60u32)]),
        )]);
        let holds = ZoneHolds::from([(("openrgb:mb".to_string(), 0u32), 300u32)]);

        let effective = effective_sizes(&saved, &holds);

        let mb = effective.get("openrgb:mb").expect("the device is in the sizes");
        assert_eq!(mb.get(&0), Some(&300), "the hold replaces the saved 120");
        assert_eq!(mb.get(&1), Some(&60), "the other zone keeps its saved size");
    }

    #[test]
    fn effective_sizes_adds_a_hold_the_file_lacks() {
        let saved = ZoneSizes::from([(
            "openrgb:dram".to_string(),
            BTreeMap::from([(0u32, 8u32)]),
        )]);
        let holds = ZoneHolds::from([(("openrgb:mb".to_string(), 0u32), 300u32)]);

        let effective = effective_sizes(&saved, &holds);

        assert_eq!(
            effective.get("openrgb:mb").and_then(|zones| zones.get(&0)),
            Some(&300),
            "the held device is added"
        );
        assert_eq!(
            effective.get("openrgb:dram").and_then(|zones| zones.get(&0)),
            Some(&8),
            "the saved device is kept"
        );
    }

    #[test]
    fn effective_sizes_with_no_holds_is_the_saved_sizes() {
        let saved = ZoneSizes::from([(
            "openrgb:mb".to_string(),
            BTreeMap::from([(0u32, 120u32)]),
        )]);

        assert_eq!(effective_sizes(&saved, &ZoneHolds::new()), saved);
    }

    #[test]
    fn the_backoff_grows_from_250ms_to_5s() {
        let mut backoff = Backoff::new();
        assert_eq!(backoff.current(), Duration::from_millis(250));
        let mut seen = vec![backoff.current()];
        for _ in 0..10 {
            let (delay, next) = backoff.advanced();
            seen.push(delay);
            backoff = next;
        }
        assert_eq!(seen[1], Duration::from_millis(500));
        assert_eq!(*seen.last().unwrap(), BACKOFF_MAX);
        assert_eq!(backoff.reset().current(), Duration::from_millis(250));
    }

    #[test]
    fn a_rescan_does_not_advance_the_automatic_backoff() {
        let mut backoff = Backoff::new();
        // The automatic attempts back off: 500 ms, then 1 s.
        assert_eq!(next_delay(&mut backoff, false), Duration::from_millis(500));
        assert_eq!(next_delay(&mut backoff, false), Duration::from_millis(1000));
        // An attempt the owner asked for waits the delay the backoff already had, and does not
        // make the next automatic attempt wait longer.
        assert_eq!(next_delay(&mut backoff, true), Duration::from_millis(1000));
        assert_eq!(next_delay(&mut backoff, false), Duration::from_millis(2000));
    }

    #[test]
    fn the_restart_budget_stops_after_three_in_five_minutes() {
        let mut budget = RestartBudget::default();
        assert!(budget.record());
        assert!(budget.record());
        assert!(budget.record());
        assert!(!budget.record(), "the fourth restart in the window is refused");
    }

    #[test]
    fn openrgb_binds_loopback_only() {
        let args = openrgb_args(6742);
        assert!(args.windows(2).any(|w| w == ["--server-host", "127.0.0.1"]));
        assert!(args.windows(2).any(|w| w == ["--server-port", "6742"]));
        assert!(args.iter().any(|a| a == "--noautoconnect"));
        assert!(args.iter().any(|a| a == "--server"));
    }

    // ---- #290: what a rescan says and reads

    use canvas_proto::device::{AdapterCapabilities, DeviceKind, ZoneInfo, ZoneKind};
    use canvas_proto::traits::AdapterError;
    use canvas_proto::{DeviceId, Rgb8};

    fn device(serial: &str) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(format!("openrgb:{serial}")),
            name: serial.into(),
            vendor: "fake".into(),
            kind: DeviceKind::Dram,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: serial.into(),
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

    /// An adapter that answers a rescan as told and lists what it was given.
    struct Stub {
        rescan: fn() -> Result<(), AdapterError>,
        list: Vec<DeviceInfo>,
        rescans: u32,
        /// Whether the link is up; `connect` brings it up.
        up: bool,
    }

    #[async_trait::async_trait]
    impl DeviceAdapter for Stub {
        fn name(&self) -> &'static str {
            "stub"
        }
        fn capabilities(&self) -> AdapterCapabilities {
            AdapterCapabilities::default()
        }
        async fn connect(&mut self) -> Result<(), AdapterError> {
            self.up = true;
            Ok(())
        }
        async fn disconnect(&mut self) -> Result<(), AdapterError> {
            self.up = false;
            Ok(())
        }
        fn is_connected(&self) -> bool {
            self.up
        }
        async fn rescan(&mut self) -> Result<(), AdapterError> {
            self.rescans += 1;
            (self.rescan)()
        }
        async fn discover(&mut self) -> Result<Vec<DeviceInfo>, AdapterError> {
            Ok(self.list.clone())
        }
        async fn write_frame(&mut self, _device: &DeviceId, _colors: &[Rgb8]) -> Result<(), AdapterError> {
            Ok(())
        }
        async fn set_zone_size(&mut self, _device: &DeviceId, _zone: u32, _leds: u32) -> Result<(), AdapterError> {
            Ok(())
        }
        async fn identify(&mut self, _device: &DeviceId, _zone: u32, _led: u32) -> Result<(), AdapterError> {
            Ok(())
        }
        async fn read_back(&mut self, _device: &DeviceId) -> Result<Vec<Rgb8>, AdapterError> {
            Ok(Vec::new())
        }
    }

    /// Runs `rescan_devices` on a current-thread runtime under the log capture.
    fn rescan_logged(
        stub: &mut Stub,
        held: &[DeviceInfo],
        detected: Detected,
        ask: bool,
    ) -> (Result<RescanReading, String>, String) {
        captured(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime")
                .block_on(rescan_devices(stub, &BTreeMap::new(), held, detected, ask))
        })
    }

    use crate::test_logs::captured;

    /// The log line after a re-detection says what was observed: how many devices, and how long
    /// the list took to settle. The old line claimed OpenRGB "looked at the hardware again" on any
    /// signal.
    #[test]
    fn a_re_detection_logs_what_was_observed() {
        let mut stub = Stub { rescan: || Ok(()), list: vec![device("a"), device("b")], rescans: 0, up: true };
        let detected = Detected::Yes { settled_after: Some(Duration::from_millis(812)), by_events: false };
        let (result, text) = rescan_logged(&mut stub, &[device("a")], detected, false);
        assert_eq!(result.expect("a reading").devices.len(), 2);
        assert!(text.contains("OpenRGB re-detected: 2 devices, list settled after 812 ms"), "{text}");
        assert!(!text.contains("looked at the hardware again"), "{text}");
    }

    /// A rescan that re-detected nothing never says it did.
    #[test]
    fn a_rescan_that_re_detected_nothing_does_not_say_it_did() {
        let mut stub = Stub {
            rescan: || Err(AdapterError::Protocol(canvas_adapters::openrgb::RESCAN_NOT_REDETECTED_MESSAGE.into())),
            list: vec![device("a")],
            rescans: 0,
            up: true,
        };
        let (result, text) = rescan_logged(&mut stub, &[device("a")], Detected::No, true);
        let reading = result.expect("a reading");
        assert_eq!(reading.devices.len(), 1);
        assert!(!reading.kept_held, "the list was read as it stands");
        assert!(!text.contains("re-detected:"), "{text}");
        assert!(text.contains("did not re-detect"), "{text}");
    }

    /// A detection that never went quiet reads nothing: the answer is the list held, flagged so
    /// the supervisor applies nothing, even though the adapter's list is empty right now.
    #[test]
    fn an_unsettled_rescan_answers_with_the_held_list_and_reads_nothing() {
        let mut stub = Stub {
            rescan: || Err(AdapterError::Protocol(canvas_adapters::openrgb::RESCAN_DETECT_TIMEOUT_MESSAGE.into())),
            list: Vec::new(),
            rescans: 0,
            up: true,
        };
        let held = [device("a"), device("b")];
        let (result, _) = rescan_logged(&mut stub, &held, Detected::No, true);
        let reading = result.expect("a reading");
        assert!(reading.kept_held);
        assert_eq!(reading.devices.len(), 2, "the held list answers");
    }

    /// A list that is empty right after a re-detection, while devices are held, is a slow
    /// detection and not every device gone: it is not applied.
    #[test]
    fn an_empty_list_after_a_re_detection_is_not_applied() {
        let mut stub = Stub { rescan: || Ok(()), list: Vec::new(), rescans: 0, up: true };
        let held = [device("a")];
        let (result, text) = rescan_logged(&mut stub, &held, Detected::No, true);
        let reading = result.expect("a reading");
        assert!(reading.kept_held, "an empty list from a rescan was applied");
        assert_eq!(reading.devices.len(), 1);
        assert!(text.contains("keeping the list held"), "{text}");
    }

    /// OpenRGB said the detection is over (103): an empty list is every device gone, and it is applied.
    #[test]
    fn an_empty_list_after_a_detection_openrgb_said_is_over_is_applied() {
        let mut stub = Stub { rescan: || Ok(()), list: Vec::new(), rescans: 0, up: true };
        let detected = Detected::Yes { settled_after: Some(Duration::from_millis(40)), by_events: true };
        let (result, text) = rescan_logged(&mut stub, &[device("a")], detected, false);
        let reading = result.expect("a reading");
        assert!(!reading.kept_held, "the empty list after 103 was not applied");
        assert!(reading.devices.is_empty());
        assert!(text.contains("OpenRGB re-detected: 0 devices"), "{text}");
    }

    /// A list that shrank but is not empty, from a rescan that settled, is applied: a device that
    /// was unplugged is gone.
    #[test]
    fn a_shorter_list_from_a_settled_rescan_is_applied() {
        let mut stub = Stub { rescan: || Ok(()), list: vec![device("a")], rescans: 0, up: true };
        let (result, _) = rescan_logged(&mut stub, &[device("a"), device("b")], Detected::No, true);
        let reading = result.expect("a reading");
        assert!(!reading.kept_held);
        assert_eq!(reading.devices.len(), 1);
    }

    /// A rescan on a link that is down connects and reads the list, and never asks OpenRGB to
    /// re-detect: the log says so, so "rescanned" stays a claim about what was done.
    #[test]
    fn a_rescan_on_a_down_link_says_it_only_re_read_the_list() {
        let mut stub = Stub { rescan: || Ok(()), list: vec![device("a")], rescans: 0, up: false };
        let (result, text) = rescan_logged(&mut stub, &[], Detected::No, true);
        assert_eq!(result.expect("a reading").devices.len(), 1);
        assert_eq!(stub.rescans, 0, "OpenRGB was asked to re-detect on a link that was down");
        assert!(text.contains("not connected; connecting and re-reading the list only"), "{text}");
        assert!(text.contains("was not asked to re-detect"), "{text}");
        assert!(!text.contains("re-detected:"), "{text}");
    }

    /// A detection that is still running at the cap is unsettled, and the line says that OpenRGB is
    /// still detecting and which list answers (the API has no field for it, so the log says it).
    ///
    /// The verdict is independent of wall time (class `cause:timing-test`, #72). The cap is 30 s, 40 times the
    /// stall below, so no real clock ends the wait. The test waits on what it can observe, the fake's packet-140
    /// count and the adapter's count of its ACKs, then pauses the clock: the runtime is idle from
    /// then on, so the paused clock runs to the cap by itself and ends the wait. The clock is paused
    /// only after the I/O is done: a paused clock from the start would run to the cap while the
    /// runtime waits on the socket. The fake holds the ACK back 700 ms, a stall the old 500 ms cap
    /// read as "OpenRGB did not acknowledge the rescan".
    #[test]
    fn an_unsettled_detection_says_openrgb_is_still_detecting_and_which_list_answers() {
        use canvas_adapters::fake::FakeOpenRgbServer;
        use canvas_adapters::openrgb::{OpenRgbAdapter, RescanTiming};
        let (detection, text) = captured(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime").block_on(async {
                let server = FakeOpenRgbServer::start(vec![device("a")]).await.expect("fake server");
                server.hold_detection(true);
                server.delay_the_rescan_ack(700);
                let timing = RescanTiming {
                    first: Duration::from_millis(300),
                    settle: Duration::from_millis(200),
                    cap: Duration::from_secs(30),
                };
                let mut lane = OpenRgbAdapter::new("127.0.0.1", server.port()).with_rescan_timing(timing);
                lane.connect().await.expect("connect");
                let io_done = async {
                    while server.rescan_count() < 1 || lane.rescan_acks() < 1 {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                    tokio::time::pause();
                };
                let (detection, ()) = tokio::join!(detect_on_lane(&lane), io_done);
                assert_eq!(server.rescan_count(), 1, "one packet 140 was sent");
                assert_eq!(lane.rescan_acks(), 1, "its ACK was heard before the verdict");
                detection
            })
        });
        assert!(matches!(detection, Detection::Unsettled), "{detection:?}");
        assert!(text.contains("OpenRGB is still detecting devices"), "{text}");
        assert!(text.contains("answering with the list held"), "{text}");
    }
}
