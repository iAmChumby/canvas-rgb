//! Windows service plumbing (`windows-service`). The SCM calls `service_main`, which registers
//! the control handler and runs the core. Stop, Shutdown and PreShutdown all end the core loop,
//! which restores BIOS before the service reports Stopped. Power events (suspend, resume) go to
//! the fan controller through a channel; see `fans::RESUME_GRACE`. Session changes are logged
//! with their reason and their time (#6); they change no behaviour yet.

use crate::app;
use crate::service::PowerEvent;
use canvas_proto::consts::SERVICE_NAME;
use std::ffi::OsString;
use std::time::Duration;
use windows_service::service::{
    PowerEventParam, ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    ServiceType, SessionChangeParam,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

define_windows_service!(ffi_service_main, service_main);

/// Hands the process to the SCM. Only the service manager may call this.
pub fn run_under_scm() -> Result<(), windows_service::Error> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        tracing::error!(error = %e, "service failed");
    }
}

fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}

/// Days since 1970-01-01 as (year, month, day) in the proleptic Gregorian calendar (Howard
/// Hinnant's algorithm, as in the daemon's log).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// The current UTC time as `YYYY-MM-DDTHH:MM:SS.mmmZ`: the time a session-change notification
/// arrived, on the line itself (#6).
fn utc_stamp() -> String {
    let (secs, millis) = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or((0i64, 0u32), |d| (d.as_secs() as i64, d.subsec_millis()));
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let day = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        day / 3600,
        (day % 3600) / 60,
        day % 60
    )
}

/// The fields of the session-change log line: the reason, the session it is about, and the time
/// it arrived (#6). The line changes no behaviour: it settles whether a logoff arrives before
/// the daemon is ended.
fn session_change_fields(param: &SessionChangeParam) -> (String, u32, String) {
    (
        format!("{:?}", param.reason),
        param.notification.session_id,
        utc_stamp(),
    )
}

fn run_service() -> Result<(), Box<dyn std::error::Error>> {
    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let (power_tx, power_rx) = tokio::sync::mpsc::unbounded_channel::<PowerEvent>();
    let handler = move |control: ServiceControl| -> ServiceControlHandlerResult {
        match control {
            ServiceControl::PowerEvent(param) => {
                // Issue #29: every notification is logged, so the first real wake shows what arrived.
                tracing::info!(?param, "service control: power event");
                let event = match param {
                    PowerEventParam::Suspend => Some(PowerEvent::Suspend),
                    PowerEventParam::ResumeAutomatic
                    | PowerEventParam::ResumeSuspend
                    | PowerEventParam::ResumeCritical => Some(PowerEvent::Resume),
                    _ => None,
                };
                if let Some(event) = event {
                    if power_tx.send(event).is_err() {
                        tracing::warn!(?event, "power event dropped: the service loop is gone");
                    }
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
                tracing::info!(?control, "service control: stopping; BIOS restore first");
                let _ = stop_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::SessionChange(param) => {
                // Issue #6: every notification is logged with its reason, its session and the
                // time it arrived. It changes no behaviour yet.
                let (reason, session, time) = session_change_fields(&param);
                tracing::info!(reason, session, time, "service control: session change");
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let handle = service_control_handler::register(SERVICE_NAME, handler)?;
    let accept = ServiceControlAccept::STOP
        | ServiceControlAccept::SHUTDOWN
        | ServiceControlAccept::PRESHUTDOWN
        | ServiceControlAccept::POWER_EVENT
        | ServiceControlAccept::SESSION_CHANGE;
    handle.set_service_status(status(ServiceState::Running, accept))?;

    let boot = app::take_boot().ok_or("the machine home was not secured before the SCM dispatch")?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let result = runtime.block_on(app::run_scm(boot, async move {
        let _ = stop_rx.recv().await;
    }, power_rx));

    handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()))?;
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_service::service::{SessionChangeReason, SessionNotification};

    /// The line for a session change names the reason and carries the time it arrived (#6).
    #[test]
    fn a_session_change_notification_is_logged_with_its_reason_and_its_time() {
        let param = SessionChangeParam {
            reason: SessionChangeReason::SessionLogoff,
            notification: SessionNotification {
                size: 0,
                session_id: 3,
            },
        };
        let (reason, session, time) = session_change_fields(&param);
        assert_eq!(reason, "SessionLogoff");
        assert_eq!(session, 3);
        assert!(time.ends_with('Z'), "{time}");
        assert_eq!(time.len(), 24, "{time}");
        // The time is the current UTC day.
        let today = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() / 86_400) as i64;
        let (y, m, d) = civil_from_days(today);
        assert!(time.starts_with(&format!("{y:04}-{m:02}-{d:02}")), "{time}");
    }
}
