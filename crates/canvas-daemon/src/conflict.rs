//! The SignalRGB conflict check.
//!
//! Every 5 s the daemon looks for `SignalRgb.exe` and for the `SignalRgbService` service.
//! Either one running means SignalRGB is still driving the hardware, which the editor
//! shows as a banner and `canvas-rgb status` prints as a CONFLICT line.

use canvas_proto::api::Conflict;
use canvas_proto::consts::{SIGNALRGB_PROCESS, SIGNALRGB_SERVICE};

/// How often the check runs.
pub const CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Looks for the SignalRGB process by name.
///
/// Uses the ToolHelp snapshot API, so it sees other users' processes too and needs no
/// extra privileges.
#[cfg(windows)]
pub fn signalrgb_process_running() -> bool {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_err() {
            let _ = windows::Win32::Foundation::CloseHandle(snapshot);
            return false;
        }
        let target = SIGNALRGB_PROCESS.to_uppercase();
        let mut found = false;
        loop {
            let name: String = entry.szExeFile.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
            if name.eq_ignore_ascii_case(&target) {
                found = true;
                break;
            }
            if Process32NextW(snapshot, &mut entry).is_err() {
                break;
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
        found
    }
}

#[cfg(not(windows))]
pub fn signalrgb_process_running() -> bool {
    false
}

/// Whether the `SignalRgbService` service exists and is running.
#[cfg(windows)]
pub fn signalrgb_service_running() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx, SC_MANAGER_CONNECT,
        SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS, SC_STATUS_PROCESS_INFO,
    };

    unsafe {
        let manager = match OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) {
            Ok(m) => m,
            Err(_) => return false,
        };
        let name: Vec<u16> = SIGNALRGB_SERVICE.encode_utf16().chain(std::iter::once(0)).collect();
        let service = match OpenServiceW(manager, PCWSTR(name.as_ptr()), SERVICE_QUERY_STATUS) {
            Ok(s) => s,
            Err(_) => {
                let _ = CloseServiceHandle(manager);
                return false;
            }
        };
        let mut status = SERVICE_STATUS::default();
        let mut bytes: u32 = 0;
        let buffer: &mut [u8] = std::slice::from_raw_parts_mut(
            std::ptr::addr_of_mut!(status).cast::<u8>(),
            std::mem::size_of::<SERVICE_STATUS>(),
        );
        let ok = QueryServiceStatusEx(service, SC_STATUS_PROCESS_INFO, Some(buffer), &mut bytes);
        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(manager);
        if ok.is_err() {
            return false;
        }
        status.dwCurrentState == SERVICE_RUNNING
    }
}

#[cfg(not(windows))]
pub fn signalrgb_service_running() -> bool {
    false
}

/// The conflicts to report right now. Empty when SignalRGB is not running.
pub fn detect() -> Vec<Conflict> {
    let process = signalrgb_process_running();
    let service = signalrgb_service_running();
    if process || service {
        vec![Conflict::SignalRgb { process, service }]
    } else {
        Vec::new()
    }
}

/// A one-line description for the CLI.
pub fn describe(conflicts: &[Conflict]) -> Vec<String> {
    conflicts
        .iter()
        .map(|c| match c {
            Conflict::SignalRgb { process, service } => {
                let mut parts = Vec::new();
                if *process {
                    parts.push(SIGNALRGB_PROCESS.to_string());
                }
                if *service {
                    parts.push(format!("{SIGNALRGB_SERVICE} service"));
                }
                format!("SignalRGB is running ({}): it will fight canvas-rgb for the devices", parts.join(", "))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conflict_check_reports_what_it_finds() {
        // SignalRGB may or may not be running on this machine: the check must report
        // what it sees, either way.
        let conflicts = detect();
        for conflict in &conflicts {
            match conflict {
                Conflict::SignalRgb { process, service } => {
                    assert!(
                        *process || *service,
                        "a conflict is reported only when something runs"
                    );
                }
            }
        }
        // The description is one line per conflict.
        assert_eq!(describe(&conflicts).len(), conflicts.len());
    }

    #[test]
    fn conflicts_describe_both_sources() {
        let conflicts = vec![Conflict::SignalRgb { process: true, service: true }];
        let text = describe(&conflicts);
        assert_eq!(text.len(), 1);
        assert!(text[0].contains("SignalRgb.exe"));
        assert!(text[0].contains("SignalRgbService"));
    }
}
