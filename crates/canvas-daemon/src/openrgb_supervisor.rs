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

/// How long a rescan may hold the adapter: about one reply timeout of the OpenRGB adapter (5 s).
/// The handlers that take the adapter lock (a zone resize, the first run) wait for it, so a stalled
/// OpenRGB delays them this long and no longer (#233). The frame writers do not wait: they write
/// through the adapter's own link (`App::frame_lane`), so the lights keep moving. The adapter's own
/// reply timeout ends a single stalled request; this ends a slow trickle of them.
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

/// A rescan the owner asked for (#233): asks OpenRGB for its devices again and returns them.
///
/// On a live connection the list is read again (and a saved zone size a new device lacks is
/// applied, as the 5 s refresh does). With no connection the adapter connects first, so a rescan
/// is also a retry of the link. This reads what OpenRGB reports now; making OpenRGB look at the
/// hardware again needs a `rescan` on the adapter trait, which lives in the spine.
pub async fn rescan_devices(
    adapter: &mut dyn DeviceAdapter,
    zone_sizes: &std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>>,
) -> Result<Vec<DeviceInfo>, String> {
    if adapter.is_connected() {
        refresh_devices(adapter, zone_sizes).await
    } else {
        connect_and_prepare(adapter, zone_sizes).await
    }
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
    mut on_devices: impl FnMut(Vec<DeviceInfo>) -> D,
    mut on_link: impl FnMut(LinkState) -> L,
    mut rescans: Option<tokio::sync::mpsc::Receiver<Rescan>>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) where
    D: std::future::Future<Output = ()>,
    L: std::future::Future<Output = ()>,
{
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
        let sizes = zone_sizes.read().await.clone();
        let (result, cut) = within(waiting.iter().map(|r| r.budget).max(), async {
            let mut guard = adapter.lock().await;
            connect_and_prepare(guard.as_mut(), &sizes).await
        })
        .await;
        if cut {
            // The attempt was dropped mid-way, possibly between the socket and the handshake: a
            // link that says "connected" with no protocol version would be taken for a good one.
            let _ = adapter.lock().await.disconnect().await;
        }
        match result {
            Ok(devices) => {
                tracing::info!(count = devices.len(), "connected to OpenRGB");
                let mut last = device_key(&devices);
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
                            let sizes = zone_sizes.read().await.clone();
                            let (result, _) = within(Some(rescan.budget), async {
                                let mut guard = adapter.lock().await;
                                rescan_devices(guard.as_mut(), &sizes).await
                            })
                            .await;
                            if let Ok(devices) = &result {
                                last = device_key(devices);
                                on_devices(devices.clone()).await;
                            }
                            let _ = rescan.reply.send(result);
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
                    if !ticks.is_multiple_of(10) {
                        continue;
                    }
                    let refreshed = {
                        let mut guard = adapter.lock().await;
                        refresh_devices(guard.as_mut(), &zone_sizes.read().await.clone()).await
                    };
                    match refreshed {
                        Ok(devices) if device_key(&devices) != last => {
                            tracing::info!(count = devices.len(), "the OpenRGB device list changed");
                            last = device_key(&devices);
                            on_devices(devices).await;
                        }
                        Ok(_) => {}
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
                let delay = if from_rescan {
                    backoff.current()
                } else {
                    let (delay, next_backoff) = backoff.advanced();
                    backoff = next_backoff;
                    delay
                };
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
}
