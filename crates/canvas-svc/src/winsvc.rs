//! Windows service plumbing (`windows-service`). The SCM calls `service_main`, which registers
//! the control handler and runs the core. Stop, Shutdown and PreShutdown all end the core loop,
//! which restores BIOS before the service reports Stopped.

use crate::app;
use canvas_proto::consts::SERVICE_NAME;
use std::ffi::OsString;
use std::time::Duration;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
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

fn run_service() -> Result<(), Box<dyn std::error::Error>> {
    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let handler = move |control: ServiceControl| -> ServiceControlHandlerResult {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
                tracing::info!(?control, "service control: stopping; BIOS restore first");
                let _ = stop_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let handle = service_control_handler::register(SERVICE_NAME, handler)?;
    let accept = ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN | ServiceControlAccept::PRESHUTDOWN;
    handle.set_service_status(status(ServiceState::Running, accept))?;

    let boot = app::take_boot().ok_or("the machine home was not secured before the SCM dispatch")?;
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let result = runtime.block_on(app::run_scm(boot, async move {
        let _ = stop_rx.recv().await;
    }));

    handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()))?;
    result.map_err(Into::into)
}
