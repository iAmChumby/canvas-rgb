//! The elevated OpenRGB server, supervised by the service (spec: "OpenRGB server process").
//!
//! The server binds 127.0.0.1 and starts with `--noautoconnect` and its own `--config` dir.
//! It restarts when it exits. Once it accepts a connection and no daemon is connected, the
//! service writes the boot color to every device (G5), and holds it until the daemon connects.

use crate::process::{ChildProcess, ProcessSpawner};
use canvas_proto::pipe::OpenRgbServerState;
use canvas_proto::traits::DeviceAdapter;
use canvas_proto::Rgb8;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The loopback address the server binds (spec: "Nothing listens on a non-loopback address").
pub const SERVER_HOST: &str = "127.0.0.1";

/// The count of detected devices must hold still this long before the boot color is written.
pub const DETECTION_SETTLE: Duration = Duration::from_millis(750);

/// The longest the boot color waits for OpenRGB to find a device (the issue's log shows the first
/// controller at about 1.2 s and detection done at about 2.1 s).
pub const DETECTION_CAP: Duration = Duration::from_secs(5);

/// Creates the adapter the service uses to talk to the server. Tests point it at a fake server.
pub type AdapterFactory = Box<dyn FnMut() -> Box<dyn DeviceAdapter> + Send>;

/// Arguments for `OpenRGB.exe` (spec, OpenRGB section).
pub fn server_args(port: u16, config_dir: &Path) -> Vec<String> {
    vec![
        "--server".into(),
        "--server-host".into(),
        SERVER_HOST.into(),
        "--server-port".into(),
        port.to_string(),
        "--noautoconnect".into(),
        "--config".into(),
        config_dir.display().to_string(),
    ]
}

/// True when `exe` lives under `root` (the service's own install directory). Both paths are
/// compared canonically when they exist, so a junction or symlink out of the directory fails.
pub fn exe_is_under(exe: &Path, root: &Path) -> bool {
    if !exe.starts_with(root) {
        return false;
    }
    match (std::fs::canonicalize(root), std::fs::canonicalize(exe)) {
        (Ok(root), Ok(exe)) => exe.starts_with(root),
        // The exe does not exist yet: the lexical check is all there is.
        (Ok(_), Err(_)) => true,
        _ => false,
    }
}

pub struct OpenRgbSupervisor {
    exe: PathBuf,
    port: u16,
    config_dir: PathBuf,
    install_root: PathBuf,
    spawner: Box<dyn ProcessSpawner>,
    adapter: AdapterFactory,
    child: Option<Box<dyn ChildProcess>>,
    session: Option<Box<dyn DeviceAdapter>>,
    state: OpenRgbServerState,
    /// Set when the service must not run OpenRGB at all (machine home not secured).
    disabled: Option<String>,
    /// Set once the boot color has been written for the current server run.
    boot_written: bool,
    /// The device count seen while waiting for detection to finish (reset with each session).
    detection: Option<Detection>,
    starts: u32,
}

/// What the boot-color wait has seen so far.
struct Detection {
    started: Instant,
    last_count: usize,
    last_change: Instant,
    warned_empty: bool,
}

impl OpenRgbSupervisor {
    pub fn new(
        exe: PathBuf,
        port: u16,
        config_dir: PathBuf,
        install_root: PathBuf,
        spawner: Box<dyn ProcessSpawner>,
        adapter: AdapterFactory,
    ) -> Self {
        Self {
            exe,
            port,
            config_dir,
            install_root,
            spawner,
            adapter,
            child: None,
            session: None,
            state: OpenRgbServerState::NotStarted,
            disabled: None,
            boot_written: false,
            detection: None,
            starts: 0,
        }
    }

    /// Permanently disables launching (e.g. the machine home could not be secured).
    pub fn disable(&mut self, reason: &str) {
        tracing::error!(reason, "OpenRGB disabled; it will not be launched");
        self.disabled = Some(reason.to_string());
        self.state = OpenRgbServerState::NotStarted;
    }

    pub fn state(&self) -> OpenRgbServerState {
        self.state
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether the boot color has been written for the current server run.
    pub fn boot_written(&self) -> bool {
        self.boot_written
    }

    /// Number of times the server process has been started (first start included).
    pub fn starts(&self) -> u32 {
        self.starts
    }

    fn start(&mut self, reason: &str) {
        if self.disabled.is_some() {
            return;
        }
        if !exe_is_under(&self.exe, &self.install_root) {
            self.disable(&format!("{} is outside the install directory {}", self.exe.display(), self.install_root.display()));
            return;
        }
        let args = server_args(self.port, &self.config_dir);
        match self.spawner.spawn(&self.exe, &args) {
            Ok(child) => {
                self.child = Some(child);
                self.session = None;
                self.boot_written = false;
                self.detection = None;
                self.state = OpenRgbServerState::Starting;
                self.starts += 1;
                tracing::info!(reason, port = self.port, "OpenRGB server started");
            }
            Err(e) => {
                self.child = None;
                self.state = OpenRgbServerState::Crashed;
                tracing::error!(reason, error = %e, "OpenRGB server did not start");
            }
        }
    }

    /// Restarts the server on request (`RestartOpenRgb`). The next tick starts it again.
    pub fn restart(&mut self, reason: &str) {
        tracing::info!(reason, "OpenRGB restart requested");
        if let Some(mut child) = self.child.take() {
            if let Err(e) = child.kill() {
                tracing::warn!(error = %e, "could not kill the OpenRGB server");
            }
        }
        self.session = None;
        self.state = OpenRgbServerState::Crashed;
    }

    /// Stops the server when the service stops.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Err(e) = child.kill() {
                tracing::warn!(error = %e, "could not kill the OpenRGB server on stop");
            }
        }
        self.session = None;
        self.state = OpenRgbServerState::NotStarted;
    }

    /// One pass per second: keep the process alive, connect when it is ready, and paint the
    /// boot color when nobody else is driving the devices.
    pub async fn tick(&mut self, now: Instant, daemon_connected: bool, boot: Option<Rgb8>) {
        if self.disabled.is_some() {
            return;
        }
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(code)) => {
                    self.child = None;
                    self.session = None;
                    self.state = OpenRgbServerState::Crashed;
                    tracing::info!(code = %crate::process::describe_exit_code(code), "OpenRGB server exited; restarting");
                }
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, "could not query the OpenRGB server"),
            }
        }
        if self.child.is_none() {
            self.start("not running");
            if self.child.is_none() {
                return;
            }
        }
        if let Some(session) = self.session.as_ref() {
            if !session.is_connected() {
                tracing::info!("OpenRGB connection dropped; reconnecting");
                self.session = None;
                self.state = OpenRgbServerState::Starting;
            }
        }
        if self.session.is_none() {
            let mut adapter = (self.adapter)();
            if adapter.connect().await.is_ok() {
                self.session = Some(adapter);
                self.detection = None;
                self.state = OpenRgbServerState::Running;
            }
        }
        if !daemon_connected && !self.boot_written {
            self.write_boot_color(now, boot.unwrap_or(Rgb8::BLACK)).await;
        }
    }

    /// Paints the boot color once OpenRGB has finished detecting: the device count must be above
    /// zero and unchanged for [`DETECTION_SETTLE`]. After [`DETECTION_CAP`] with no device it logs
    /// that nothing was painted and keeps watching; a device that shows up later is still painted.
    async fn write_boot_color(&mut self, now: Instant, color: Rgb8) {
        let Some(session) = self.session.as_mut() else { return };
        let devices = match session.discover().await {
            Ok(devices) => devices,
            Err(e) => {
                tracing::warn!(error = %e, "boot color not written yet");
                return;
            }
        };
        let count = devices.len();
        let seen = self.detection.get_or_insert(Detection {
            started: now,
            last_count: count,
            last_change: now,
            warned_empty: false,
        });
        if count != seen.last_count {
            seen.last_count = count;
            seen.last_change = now;
        }
        let waited = now.saturating_duration_since(seen.started);
        let settled = count > 0 && now.saturating_duration_since(seen.last_change) >= DETECTION_SETTLE;
        if !settled {
            if count == 0 && waited >= DETECTION_CAP && !seen.warned_empty {
                seen.warned_empty = true;
                log_device_write("boot color not written: OpenRGB has found", 0);
            }
            return;
        }
        match paint_devices(session.as_mut(), &devices, color).await {
            Ok(written) => {
                self.boot_written = true;
                log_device_write("boot color written to", written);
            }
            Err(e) => tracing::warn!(error = %e, "boot color not written yet"),
        }
    }

    /// Writes one color to every device now (the service's black-on-stop path).
    pub async fn write_color_all(&mut self, color: Rgb8) -> Result<usize, String> {
        match self.session.as_mut() {
            Some(session) => paint(session.as_mut(), color).await.map_err(|e| e.to_string()),
            None => Err("OpenRGB is not connected".into()),
        }
    }
}

/// Writes `color` to every LED of every device the adapter reports.
pub async fn paint(adapter: &mut dyn DeviceAdapter, color: Rgb8) -> Result<usize, canvas_proto::AdapterError> {
    let devices = adapter.discover().await?;
    paint_devices(adapter, &devices, color).await
}

/// Writes `color` to every LED of each of `devices`. Returns how many devices it wrote to.
async fn paint_devices(
    adapter: &mut dyn DeviceAdapter,
    devices: &[canvas_proto::DeviceInfo],
    color: Rgb8,
) -> Result<usize, canvas_proto::AdapterError> {
    for device in devices {
        let colors = vec![color; device.led_count() as usize];
        adapter.write_frame(&device.id, &colors).await?;
    }
    Ok(devices.len())
}

/// Logs how many devices a step touched: INFO for some, WARN for none. A step that reached no
/// device did nothing, and the log must say so instead of reading as a success (class
/// `cause:unchecked-state`, #18). `what` completes the sentence "<what> N devices".
pub fn log_device_write(what: &str, devices: usize) {
    if devices == 0 {
        tracing::warn!(devices, "{what} 0 devices: nothing was written");
    } else {
        tracing::info!(devices, "{what} {devices} devices");
    }
}
