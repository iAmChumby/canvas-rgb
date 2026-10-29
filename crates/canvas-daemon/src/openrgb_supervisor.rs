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
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    let mut backoff = Backoff::new();
    loop {
        if *stop.borrow() {
            return;
        }
        // Every attempt is "connecting"; `Down` only follows a failure.
        on_link(LinkState::Connecting);
        let sizes = zone_sizes.read().await.clone();
        let result = {
            let mut guard = adapter.lock().await;
            connect_and_prepare(guard.as_mut(), &sizes).await
        };
        match result {
            Ok(devices) => {
                tracing::info!(count = devices.len(), "connected to OpenRGB");
                let mut last = device_key(&devices);
                on_devices(devices);
                backoff = backoff.reset();
                // Wait for the connection to drop before trying again. Every 5 s, re-read the
                // device list: OpenRGB may still be detecting when we connect (seen live: 0
                // devices at connect, never refreshed), and devices can come and go.
                let mut ticks = 0u32;
                loop {
                    if *stop.borrow() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let connected = {
                        let guard = adapter.lock().await;
                        guard.is_connected()
                    };
                    if !connected {
                        tracing::info!("OpenRGB went away; reconnecting");
                        on_link(LinkState::Connecting);
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
                            on_devices(devices);
                        }
                        Ok(_) => {}
                        Err(e) => tracing::debug!(error = %e, "could not re-read the OpenRGB devices"),
                    }
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "OpenRGB is not reachable yet");
                on_link(LinkState::Down);
                let (delay, next_backoff) = backoff.advanced();
                backoff = next_backoff;
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    _ = stop.changed() => return,
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
