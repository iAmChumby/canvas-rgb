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
use std::time::Instant;

/// The loopback address the server binds (spec: "Nothing listens on a non-loopback address").
pub const SERVER_HOST: &str = "127.0.0.1";

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
    starts: u32,
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
    pub async fn tick(&mut self, _now: Instant, daemon_connected: bool, boot: Option<Rgb8>) {
        if self.disabled.is_some() {
            return;
        }
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(code)) => {
                    self.child = None;
                    self.session = None;
                    self.state = OpenRgbServerState::Crashed;
                    tracing::info!(code, "OpenRGB server exited; restarting");
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
                self.state = OpenRgbServerState::Running;
            }
        }
        if !daemon_connected && !self.boot_written {
            if let Some(session) = self.session.as_mut() {
                match paint(session.as_mut(), boot.unwrap_or(Rgb8::BLACK)).await {
                    Ok(devices) => {
                        self.boot_written = true;
                        tracing::info!(devices, "boot color written to devices");
                    }
                    Err(e) => tracing::warn!(error = %e, "boot color not written yet"),
                }
            }
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
    for device in &devices {
        let colors = vec![color; device.led_count() as usize];
        adapter.write_frame(&device.id, &colors).await?;
    }
    Ok(devices.len())
}
