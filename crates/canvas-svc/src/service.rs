//! The service core: owns the fan controller, the OpenRGB supervisor and the daemon
//! supervisor, runs the 1 Hz tick, and answers pipe requests.

use crate::daemon::{DaemonAction, DaemonSupervisor};
use crate::fans::{FanController, FanError};
use crate::openrgb::OpenRgbSupervisor;
use canvas_proto::api::error_codes;
use canvas_proto::pipe::{PipeRequest, PipeResponse, PROTOCOL_VERSION};
use std::time::Instant;

pub struct Core {
    pub fans: FanController,
    pub openrgb: OpenRgbSupervisor,
    pub daemon: DaemonSupervisor,
}

fn error(code: &str, message: impl Into<String>) -> PipeResponse {
    PipeResponse::Error { code: code.into(), message: message.into() }
}

fn fan_error(e: FanError) -> PipeResponse {
    match e {
        FanError::Validation(m) => error(error_codes::VALIDATION, m),
        FanError::NotFound(m) => error(error_codes::NOT_FOUND, m),
        FanError::Unavailable(m) => error(error_codes::SERVICE_UNAVAILABLE, m),
        FanError::Conflict(m) => error(error_codes::CONFLICT, m),
        FanError::Store(s) => error(error_codes::INTERNAL, s.to_string()),
    }
}

fn ok_or(result: Result<(), FanError>) -> PipeResponse {
    match result {
        Ok(()) => PipeResponse::Ok,
        Err(e) => fan_error(e),
    }
}

impl Core {
    pub fn new(fans: FanController, openrgb: OpenRgbSupervisor, daemon: DaemonSupervisor) -> Self {
        Self { fans, openrgb, daemon }
    }

    /// One pass per second. The fan loop and the OpenRGB supervisor run whether or not a
    /// daemon or tray is connected.
    pub async fn tick(&mut self, now: Instant) {
        self.fans.tick(now);
        if self.daemon.tick(now) == DaemonAction::WriteBlack {
            // Not "whoever is listed right now": the same wait for detection as the boot color.
            self.openrgb.request_black(now);
        }
        let connected = self.daemon.is_connected(now);
        let boot = self.fans.boot_color();
        self.openrgb.tick(now, connected, boot).await;
    }

    /// Writes BIOS on every header and stops the OpenRGB server. Called on stop and shutdown.
    pub fn shutdown(&mut self, reason: &str) {
        self.fans.restore_all(reason);
        self.daemon.stop();
        self.openrgb.stop();
    }

    /// Answers one request. Every fan value is validated before anything is written.
    pub fn handle(&mut self, request: PipeRequest, now: Instant) -> PipeResponse {
        match request {
            PipeRequest::Hello { protocol, .. } => {
                if protocol != PROTOCOL_VERSION {
                    return error(
                        error_codes::VALIDATION,
                        format!("protocol {protocol} is not supported; the service speaks {PROTOCOL_VERSION}"),
                    );
                }
                PipeResponse::Hello { protocol: PROTOCOL_VERSION, service_version: env!("CARGO_PKG_VERSION").into() }
            }
            PipeRequest::Heartbeat => {
                self.daemon.on_heartbeat(now);
                PipeResponse::Ok
            }
            PipeRequest::FanStatus => PipeResponse::FanStatus { status: self.fans.status() },
            PipeRequest::SetHeaderMode { header, mode } => ok_or(self.fans.set_header_mode(&header, mode, now)),
            PipeRequest::SetProfile { name } => ok_or(self.fans.set_profile(&name, now)),
            PipeRequest::PutProfile { profile } => ok_or(self.fans.put_profile(profile, now)),
            PipeRequest::DeleteProfile { name } => ok_or(self.fans.delete_profile(&name)),
            PipeRequest::PutFanConfig { config } => ok_or(self.fans.put_config(config, now)),
            PipeRequest::OpenRgbState => PipeResponse::OpenRgb {
                state: self.openrgb.state(),
                port: self.openrgb.port(),
            },
            PipeRequest::RestartOpenRgb => {
                self.openrgb.restart("requested over the pipe");
                PipeResponse::Ok
            }
            PipeRequest::SetBootColor { color } => ok_or(self.fans.set_boot_color(color)),
            PipeRequest::FailSensor { source, failed } => {
                self.fans.set_sensor_failed(source, failed);
                PipeResponse::Ok
            }
            PipeRequest::DaemonQuitting => {
                self.daemon.on_quitting();
                PipeResponse::Ok
            }
        }
    }
}
