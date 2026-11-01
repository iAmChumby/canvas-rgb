//! The elevated OpenRGB server, supervised by the service (spec: "OpenRGB server process").
//!
//! The server binds 127.0.0.1 and starts with `--noautoconnect` and its own `--config` dir.
//! It restarts when it exits. Once it accepts a connection and no daemon is connected, the
//! service writes the boot color to every device (G5), and holds it until the daemon connects.

use crate::daemon::DaemonBeforeStop;
use crate::process::{ChildProcess, ProcessSpawner};
use canvas_proto::pipe::OpenRgbServerState;
use canvas_proto::traits::DeviceAdapter;
use canvas_proto::{DeviceId, Rgb8};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The loopback address the server binds (spec: "Nothing listens on a non-loopback address").
pub const SERVER_HOST: &str = "127.0.0.1";

/// The count of detected devices must hold still this long before the boot color is written.
pub const DETECTION_SETTLE: Duration = Duration::from_millis(750);

/// The longest the boot color waits for OpenRGB to find a device (the issue's log shows the first
/// controller at about 1.2 s and detection done at about 2.1 s).
pub const DETECTION_CAP: Duration = Duration::from_secs(5);

/// After a color is first written, the service keeps watching this long (until the daemon takes
/// over, if sooner) and paints any controller that OpenRGB announces late.
pub const WATCH_WINDOW: Duration = Duration::from_secs(15);

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
    /// The boot-color paint in progress: waits for detection, writes, then watches for late
    /// controllers. Reset with each new OpenRGB session until the first write.
    boot_job: Option<PaintJob>,
    /// Black after the daemon's restart limit. It runs the same wait-and-watch as the boot color.
    black_job: Option<PaintJob>,
    starts: u32,
}

/// One color painted onto every device OpenRGB reports, once detection has settled, and then onto
/// each controller that appears late. Shared by the boot color and the restart-limit black, so
/// neither can write to "whoever is listed right now" and stop (class `cause:unchecked-state`, #18).
pub(crate) struct PaintJob {
    /// Completes the log sentences: "<label> written to N devices".
    label: &'static str,
    color: Rgb8,
    /// When the wait began (set by the first step, or by the request).
    started: Option<Instant>,
    last_count: usize,
    last_change: Option<Instant>,
    /// The devices already painted.
    pub(crate) painted: HashSet<DeviceId>,
    /// When the first write succeeded; the watch window runs from here.
    first_write: Option<Instant>,
    warned: bool,
}

impl PaintJob {
    pub(crate) fn new(label: &'static str, color: Rgb8, started: Option<Instant>) -> Self {
        Self { label, color, started, last_count: 0, last_change: None, painted: HashSet::new(), first_write: None, warned: false }
    }

    /// The machine woke: the job's clocks start again from here.
    ///
    /// `Instant` may have counted the sleep (the standard library does not say), so the watch
    /// window must not end at the wake: a controller that OpenRGB announces after the wake
    /// still gets the color (#29, class `cause:sleep-state`).
    pub(crate) fn refresh(&mut self, now: Instant) {
        self.started = Some(now);
        self.last_change = Some(now);
        if self.first_write.is_some() {
            self.first_write = Some(now);
        }
    }

    /// One pass. Returns true when the job is over (the watch window has passed).
    pub(crate) async fn step(&mut self, now: Instant, session: &mut dyn DeviceAdapter) -> bool {
        let started = *self.started.get_or_insert(now);
        if self.first_write.is_some_and(|first| now.saturating_duration_since(first) >= WATCH_WINDOW) {
            self.report_window_end(session).await;
            return true;
        }
        let devices = match session.discover().await {
            Ok(devices) => devices,
            Err(e) => {
                tracing::warn!(error = %e, "{} not written yet", self.label);
                return false;
            }
        };
        let count = devices.len();
        let last_change = match self.last_change {
            Some(at) if count == self.last_count => at,
            _ => {
                self.last_count = count;
                self.last_change = Some(now);
                now
            }
        };
        if self.first_write.is_none() {
            let settled = count > 0 && now.saturating_duration_since(last_change) >= DETECTION_SETTLE;
            if settled {
                match self.paint_unpainted(session, &devices).await {
                    Ok(_) if !self.painted.is_empty() => {
                        self.first_write = Some(now);
                        log_device_write(&format!("{} written to", self.label), self.painted.len());
                        return false;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "{} not written yet", self.label);
                        return false;
                    }
                }
            }
            if now.saturating_duration_since(started) >= DETECTION_CAP && !self.warned {
                self.warned = true;
                self.warn_unsettled(count);
            }
            return false;
        }
        // Written once: a controller announced later is painted as soon as it shows up.
        match self.paint_unpainted(session, &devices).await {
            Ok(0) => {}
            Ok(late) => tracing::info!(devices = late, "{} also written to {late} devices that appeared late", self.label),
            Err(e) => tracing::warn!(error = %e, "{} not written to a late device yet", self.label),
        }
        false
    }

    /// The watch window ended: say how many devices OpenRGB lists that never got the color. A
    /// controller announced after this is not painted, and the log must say if any were left.
    async fn report_window_end(&self, session: &mut dyn DeviceAdapter) {
        let window = WATCH_WINDOW.as_secs();
        match session.discover().await {
            Ok(devices) => {
                let unpainted = devices.iter().filter(|d| d.led_count() > 0 && !self.painted.contains(&d.id)).count();
                if unpainted == 0 {
                    tracing::info!(
                        devices = self.painted.len(),
                        "{} watch ended after {window} s: all {} devices were painted",
                        self.label,
                        self.painted.len()
                    );
                } else {
                    tracing::warn!(
                        unpainted,
                        painted = self.painted.len(),
                        "{} watch ended after {window} s: {unpainted} devices are still unpainted; a controller OpenRGB announces after {window} s is not painted",
                        self.label
                    );
                }
            }
            Err(e) => tracing::warn!(
                error = %e,
                "{} watch ended after {window} s; could not read which devices are still unpainted",
                self.label
            ),
        }
    }

    /// The cap passed with nothing written: say how many devices OpenRGB reports, settled or not.
    fn warn_unsettled(&self, count: usize) {
        if count == 0 {
            log_device_write(&format!("{} not written: OpenRGB has found", self.label), 0);
        } else {
            tracing::warn!(
                devices = count,
                "{} not written: OpenRGB has found {count} devices, but the count had not settled within {} s, so nothing was written",
                self.label,
                DETECTION_CAP.as_secs()
            );
        }
    }

    /// Writes the color to each device not painted yet that has LEDs. Returns how many it wrote.
    async fn paint_unpainted(
        &mut self,
        session: &mut dyn DeviceAdapter,
        devices: &[canvas_proto::DeviceInfo],
    ) -> Result<usize, canvas_proto::AdapterError> {
        let mut wrote = 0;
        for device in devices {
            if device.led_count() == 0 || self.painted.contains(&device.id) {
                continue;
            }
            session.write_frame(&device.id, &vec![self.color; device.led_count() as usize]).await?;
            self.painted.insert(device.id.clone());
            wrote += 1;
        }
        Ok(wrote)
    }
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
            boot_job: None,
            black_job: None,
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
                self.boot_job = None;
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
    /// `daemon` is what the daemon is doing now: it is told nothing, so it keeps streaming (#81).
    pub fn restart(&mut self, reason: &str, daemon: DaemonBeforeStop) {
        tracing::info!(reason, "OpenRGB restart requested");
        if let Some(mut child) = self.child.take() {
            if let Err(e) = kill_server(child.as_mut(), &format!("restart: {reason}"), daemon) {
                tracing::warn!(error = %e, "could not kill the OpenRGB server");
            }
        } else {
            tracing::info!(reason, "OpenRGB restart requested with no server process running");
        }
        self.session = None;
        self.state = OpenRgbServerState::Crashed;
    }

    /// Stops the server when the service stops. `daemon` is the daemon stop that came before it.
    pub fn stop(&mut self, reason: &str, daemon: DaemonBeforeStop) {
        if let Some(mut child) = self.child.take() {
            if let Err(e) = kill_server(child.as_mut(), reason, daemon) {
                tracing::warn!(error = %e, "could not kill the OpenRGB server on stop");
            }
        } else {
            tracing::info!(reason, "no OpenRGB server process to stop");
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
                    let pid = child.id();
                    self.child = None;
                    self.session = None;
                    self.state = OpenRgbServerState::Crashed;
                    tracing::info!(
                        pid,
                        reason = "exit",
                        code = %crate::process::describe_exit_code(code),
                        "OpenRGB server (pid {pid}) exited on its own with code {}, not stopped by the service; restarting",
                        crate::process::describe_exit_code(code)
                    );
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
                // A new connection restarts the wait for detection, until something is written.
                if !self.boot_written {
                    self.boot_job = None;
                }
                if let Some(job) = self.black_job.as_mut().filter(|job| job.first_write.is_none()) {
                    *job = PaintJob::new(job.label, job.color, job.started);
                }
                self.state = OpenRgbServerState::Running;
            }
        }
        if self.black_job.is_some() {
            // The daemon has been given up on: black wins over the boot color.
            self.step_black(now).await;
        } else if !daemon_connected && (!self.boot_written || self.boot_job.is_some()) {
            self.step_boot(now, boot.unwrap_or(Rgb8::BLACK)).await;
        }
    }

    /// The daemon hit its restart limit: paint every device black once OpenRGB has found them, and
    /// keep watching for late ones. The boot color is over, and black replaces it.
    pub fn request_black(&mut self, now: Instant) {
        if self.black_job.is_some() {
            return;
        }
        self.boot_written = true;
        self.boot_job = None;
        self.black_job = Some(PaintJob::new("black after the daemon restart limit", Rgb8::BLACK, Some(now)));
    }

    /// Whether a restart-limit black is still being painted or watched.
    pub fn black_pending(&self) -> bool {
        self.black_job.is_some()
    }

    /// Whether the boot color is still being painted or watched.
    #[cfg(test)]
    pub(crate) fn boot_pending(&self) -> bool {
        self.boot_job.is_some()
    }

    /// The machine woke: the color job's clocks start again from here.
    ///
    /// The job watches for the controllers OpenRGB announces late, and `Instant` may have
    /// counted the sleep (the standard library does not say), so the watch window must not end
    /// at the wake (#29, class `cause:sleep-state`).
    pub fn on_resume(&mut self, now: Instant) {
        if let Some(job) = self.boot_job.as_mut() {
            job.refresh(now);
        }
        if let Some(job) = self.black_job.as_mut() {
            job.refresh(now);
        }
    }

    async fn step_black(&mut self, now: Instant) {
        let (Some(job), Some(session)) = (self.black_job.as_mut(), self.session.as_mut()) else {
            // No OpenRGB connection to paint through: say so once the cap passes.
            if let Some(job) = self.black_job.as_mut() {
                let waited = now.saturating_duration_since(job.started.unwrap_or(now));
                if waited >= DETECTION_CAP && !job.warned {
                    job.warned = true;
                    tracing::warn!("{} not written: no OpenRGB connection after {} s", job.label, DETECTION_CAP.as_secs());
                }
            }
            return;
        };
        if job.step(now, session.as_mut()).await {
            self.black_job = None;
        }
    }

    /// Paints the boot color once OpenRGB has finished detecting: the device count must be above
    /// zero and unchanged for [`DETECTION_SETTLE`]. After [`DETECTION_CAP`] with nothing written it
    /// logs the count it sees and keeps watching. After the first write it keeps watching for
    /// [`WATCH_WINDOW`] (or until the daemon connects) and paints controllers announced late.
    async fn step_boot(&mut self, now: Instant, color: Rgb8) {
        let Some(session) = self.session.as_mut() else { return };
        let job = self.boot_job.get_or_insert_with(|| PaintJob::new("boot color", color, None));
        let finished = job.step(now, session.as_mut()).await;
        if job.first_write.is_some() {
            self.boot_written = true;
        }
        if finished {
            self.boot_job = None;
        }
    }
}

/// Ends the OpenRGB server, and says first which process, why, how, and what the daemon had done
/// before it (#81). Every kill of the server goes through here, so the next stick that goes dark
/// after a stop leaves the order and the gap on record. The kill itself is unchanged:
/// `TerminateProcess`, no drain, no wait.
fn kill_server(child: &mut dyn ChildProcess, reason: &str, daemon: DaemonBeforeStop) -> std::io::Result<()> {
    let pid = child.id();
    let daemon_text = daemon.describe(Instant::now());
    tracing::warn!(
        pid,
        reason,
        how = "kill",
        daemon = %daemon_text,
        "stopping the OpenRGB server (pid {pid}): {reason}; how: kill, no graceful close, not waited on; {daemon_text}"
    );
    child.kill()
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
