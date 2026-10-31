//! G2: sleep, shutdown, lock and monitors-off.
//!
//! - Suspend and shutdown: write black, then park. On resume the active effect comes
//!   back (the OpenRGB connection is made first).
//! - Lock and monitors-off: leave the lighting as it is, unless the
//!   "go black on lock / monitors off" setting is on (it is off by default).
//!
//! The Windows notifications arrive on their own thread, because the message loop must
//! run on the thread that created the window.

use canvas_proto::api::Power;
use std::sync::Arc;

use crate::state::App;

/// What the daemon should do for a power event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    /// The machine is suspending.
    Suspend,
    /// The machine is resuming.
    Resume,
    /// The machine is shutting down or logging off.
    Shutdown,
    /// The session was locked.
    Lock,
    /// The session was unlocked.
    Unlock,
    /// The monitors were turned off.
    MonitorsOff,
    /// The monitors were turned on.
    MonitorsOn,
}

/// What the daemon did about a power event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    /// Nothing happened.
    None,
    /// The lighting went black and the render loop parked.
    BlackAndPark,
    /// The lighting went black but the render loop kept its place.
    Black,
    /// The previous power state was restored.
    Restored,
}

/// The state the daemon keeps across a power event.
pub struct PowerWatcher {
    /// The power state before the event, so a resume can restore it.
    previous: Power,
    /// Whether the daemon is handling a suspend right now.
    suspended: bool,
}

impl PowerWatcher {
    pub fn new() -> Self {
        Self { previous: Power::Running, suspended: false }
    }

    /// The power state before the last suspend.
    pub fn previous(&self) -> Power {
        self.previous
    }

    /// Whether a suspend is in progress.
    pub fn suspended(&self) -> bool {
        self.suspended
    }
}

impl Default for PowerWatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// What a resume resets outside the engine: the OpenRGB adapters forget which controllers are in
/// custom mode. Returns how many adapters were told.
pub fn on_resume() -> usize {
    canvas_adapters::note_resume()
}

/// `PBT_POWERSETTINGCHANGE` in the `Type` of a power notification.
pub const PBT_POWERSETTINGCHANGE: u32 = 0x8013;
/// `PBT_APMSUSPEND`: the system is suspending.
pub const PBT_APMSUSPEND: u32 = 0x4;
/// `PBT_APMRESUMESUSPEND`: resumed by user input (sent after the automatic one).
pub const PBT_APMRESUMESUSPEND: u32 = 0x7;
/// `PBT_APMRESUMEAUTOMATIC`: sent on every resume.
pub const PBT_APMRESUMEAUTOMATIC: u32 = 0x12;

/// `GUID_CONSOLE_DISPLAY_STATE` in its in-memory layout: data 0 off, 1 on, 2 dimmed.
pub const GUID_CONSOLE_DISPLAY_STATE: [u8; 16] = guid_bytes(0x6fe69556_704a_47a0_8f24_f84a1d60628a);
/// `GUID_MONITOR_POWER_ON` in its in-memory layout: data 0 off, otherwise on.
pub const GUID_MONITOR_POWER_ON: [u8; 16] = guid_bytes(0x02731015_4510_4526_99e6_e57a32befdd1);

/// A GUID as the bytes Windows lays out in memory (Data1, Data2 and Data3 little endian), so the
/// pure classifier needs no Windows types.
const fn guid_bytes(value: u128) -> [u8; 16] {
    let b = value.to_be_bytes();
    [b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]
}

/// Turns one power notification (`Type` and, for a setting change, the setting's GUID bytes and
/// its first data word) into the event the daemon acts on. Pure, so it is tested on any platform.
pub fn classify_power_notification(kind: u32, setting: Option<([u8; 16], u32)>) -> Option<PowerEvent> {
    match kind {
        PBT_APMSUSPEND => Some(PowerEvent::Suspend),
        PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC => Some(PowerEvent::Resume),
        PBT_POWERSETTINGCHANGE => {
            let (guid, data) = setting?;
            if guid == GUID_CONSOLE_DISPLAY_STATE {
                // 2 (dimmed) leaves the lighting alone.
                match data {
                    0 => Some(PowerEvent::MonitorsOff),
                    1 => Some(PowerEvent::MonitorsOn),
                    _ => None,
                }
            } else if guid == GUID_MONITOR_POWER_ON {
                Some(if data == 0 { PowerEvent::MonitorsOff } else { PowerEvent::MonitorsOn })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Applies a power event to the daemon.
///
/// Returns what the daemon did, so the caller can log it.
pub async fn handle(app: &Arc<App>, watcher: &mut PowerWatcher, event: PowerEvent) -> PowerAction {
    let black_on_lock = app.store.read().await.config.settings.black_on_lock;

    match event {
        PowerEvent::Suspend => {
            watcher.previous = app.store.read().await.config.settings.power;
            watcher.suspended = true;
            app.identify.stop();
            app.engine.write_solid(canvas_proto::Rgb8::BLACK);
            app.engine.set_power(Power::Paused);
            tracing::info!(reason = "suspend", "wrote black and parked");
            PowerAction::BlackAndPark
        }
        PowerEvent::Shutdown => {
            watcher.suspended = true;
            app.identify.stop();
            app.engine.write_solid(canvas_proto::Rgb8::BLACK);
            app.engine.set_power(Power::Paused);
            tracing::info!(reason = "shutdown", "wrote black and parked");
            PowerAction::BlackAndPark
        }
        PowerEvent::Resume => {
            watcher.suspended = false;
            // The OpenRGB connection is made first, then the effect comes back.
            tracing::info!(reason = "resume", "restoring the output");
            // A sleep cuts power to some controllers (the ENE DRAM sticks fall back to their
            // built-in rainbow), so each one is switched to custom mode again on its next frame (#5).
            let adapters = on_resume();
            tracing::info!(adapters, "the OpenRGB controllers will re-enter custom mode");
            app.engine.set_power(watcher.previous);
            PowerAction::Restored
        }
        PowerEvent::Lock | PowerEvent::MonitorsOff => {
            if black_on_lock {
                app.identify.stop();
                app.engine.write_solid(canvas_proto::Rgb8::BLACK);
                tracing::info!(reason = ?event, "wrote black because the setting is on");
                PowerAction::Black
            } else {
                tracing::info!(reason = ?event, "left the lighting as it is");
                PowerAction::None
            }
        }
        PowerEvent::Unlock | PowerEvent::MonitorsOn => {
            tracing::info!(reason = ?event, "nothing to do");
            PowerAction::None
        }
    }
}

/// The Windows side of the watcher: a hidden message-only window that receives the
/// power, session and display notifications.
///
/// The window lives on its own thread, because a message loop must run on the thread
/// that created the window.
#[cfg(windows)]
pub mod windows_watcher {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Power::{
        RegisterPowerSettingNotification, RegisterSuspendResumeNotification,
        DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, POWERBROADCAST_SETTING,
    };
    use windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetPropW, PostQuitMessage,
        RegisterClassW, SetPropW, TranslateMessage, DEVICE_NOTIFY_CALLBACK, HWND_MESSAGE,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_ENDSESSION, WM_QUERYENDSESSION,
        WM_WTSSESSION_CHANGE, WNDCLASSW,
    };

    /// The window class name.
    const CLASS_NAME: PCWSTR = w!("CanvasRgbPowerWatcher");

    /// `ENDSESSION_LOGOFF` in the `lParam` of `WM_ENDSESSION`: the user is logging off.
    const ENDSESSION_LOGOFF: u32 = 0x8000_0000;

    /// The message the hidden window posts to itself when the display state changes.
    const WM_DISPLAY_STATE: u32 = WM_APP + 1;
    /// The message the hidden window posts to itself when the session changes.
    const WM_SESSION_CHANGE: u32 = WM_APP + 2;

    /// Starts the watcher thread. Every event is sent on the channel.
    pub fn start(events: Sender<PowerEvent>) -> std::io::Result<()> {
        std::thread::Builder::new()
            .name("canvas-rgb-power".into())
            .spawn(move || {
                if let Err(e) = run(events) {
                    tracing::warn!(error = %e, "the power watcher stopped");
                }
            })
            .map(|_| ())
    }

    /// The message loop.
    fn run(events: Sender<PowerEvent>) -> Result<(), String> {
        unsafe {
            let instance = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                .unwrap_or_default();
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance.into(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err("could not register the power watcher window class".into());
            }
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS_NAME,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;

            match WTSRegisterSessionNotification(
                hwnd,
                windows::Win32::System::RemoteDesktop::NOTIFY_FOR_THIS_SESSION,
            ) {
                Ok(()) => tracing::info!("registered for session notifications (lock, unlock)"),
                Err(e) => tracing::warn!(error = %e, "WTSRegisterSessionNotification failed; lock and unlock will not arrive"),
            }

            // Sleep, wake and display state do not come through this window (see
            // `register_power_notifications`).
            register_power_notifications(events.clone());

            // The pointer to the sender, so the window procedure can use it.
            let sender = Box::new(events);
            let sender_ptr = Box::into_raw(sender);
            let _ = SetPropW(
                hwnd,
                w!("canvas-rgb-events"),
                Some(windows::Win32::Foundation::HANDLE(sender_ptr as *mut core::ffi::c_void)),
            );

            let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            PostQuitMessage(0);
            Ok(())
        }
    }

    /// Whether the first power notification has been logged.
    static FIRST_NOTIFICATION: AtomicBool = AtomicBool::new(false);

    const CONSOLE_DISPLAY_STATE: windows::core::GUID =
        windows::core::GUID::from_u128(0x6fe69556_704a_47a0_8f24_f84a1d60628a);
    const MONITOR_POWER_ON: windows::core::GUID =
        windows::core::GUID::from_u128(0x02731015_4510_4526_99e6_e57a32befdd1);

    /// Registers for sleep, wake and display-state notifications with
    /// `DEVICE_NOTIFY_CALLBACK`: Windows calls a function on one of its own threads, so no
    /// window is involved.
    ///
    /// Why not the message-only window (#5): Microsoft documents that a message-only window "does
    /// not receive broadcast messages" (Window Features, Message-Only Windows), and the daemon
    /// logged no suspend or resume across real sleeps while the same window did receive lock and
    /// unlock. `RegisterSuspendResumeNotification` and `RegisterPowerSettingNotification` both
    /// accept `DEVICE_NOTIFY_CALLBACK` with a `DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS`, the form for a
    /// process that has no top-level window. The registrations live as long as the process, so
    /// they are never unregistered. Every result is logged.
    fn register_power_notifications(events: Sender<PowerEvent>) {
        // The sender and the parameters live as long as the process.
        let context: &'static Sender<PowerEvent> = Box::leak(Box::new(events));
        let params: &'static mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS =
            Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
                Callback: Some(power_callback),
                Context: context as *const Sender<PowerEvent> as *mut core::ffi::c_void,
            }));
        let recipient = HANDLE(params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS as *mut core::ffi::c_void);
        unsafe {
            match RegisterSuspendResumeNotification(recipient, DEVICE_NOTIFY_CALLBACK) {
                Ok(handle) => tracing::info!(registration_null = handle.0 == 0, "registered for suspend and resume notifications"),
                Err(e) => tracing::warn!(error = %e, "RegisterSuspendResumeNotification failed; sleep and wake will not reach the daemon"),
            }
            for (name, guid) in [("console display state", CONSOLE_DISPLAY_STATE), ("monitor power", MONITOR_POWER_ON)] {
                match RegisterPowerSettingNotification(recipient, &guid, DEVICE_NOTIFY_CALLBACK) {
                    Ok(handle) => tracing::info!(setting = name, registration_null = handle.0 == 0, "registered for a power setting"),
                    Err(e) => tracing::warn!(setting = name, error = %e, "RegisterPowerSettingNotification failed"),
                }
            }
        }
    }

    /// The power callback (`DeviceNotifyCallbackRoutine`), called on a system thread.
    unsafe extern "system" fn power_callback(
        context: *const core::ffi::c_void,
        kind: u32,
        setting: *const core::ffi::c_void,
    ) -> u32 {
        let setting_data = if kind == super::PBT_POWERSETTINGCHANGE && !setting.is_null() {
            let broadcast = &*(setting as *const POWERBROADCAST_SETTING);
            let data = if broadcast.DataLength >= 4 {
                u32::from_le_bytes(std::ptr::read_unaligned(broadcast.Data.as_ptr() as *const [u8; 4]))
            } else {
                u32::MAX
            };
            let g = broadcast.PowerSetting;
            let mut bytes = [0u8; 16];
            bytes[..4].copy_from_slice(&g.data1.to_le_bytes());
            bytes[4..6].copy_from_slice(&g.data2.to_le_bytes());
            bytes[6..8].copy_from_slice(&g.data3.to_le_bytes());
            bytes[8..].copy_from_slice(&g.data4);
            Some((bytes, data))
        } else {
            None
        };
        if !FIRST_NOTIFICATION.swap(true, Ordering::SeqCst) {
            tracing::info!(kind, "the first power notification arrived");
        }
        let event = classify_power_notification(kind, setting_data);
        tracing::debug!(kind, ?event, "a power notification");
        if let (Some(event), false) = (event, context.is_null()) {
            let sender = &*(context as *const Sender<PowerEvent>);
            if sender.send(event).is_err() {
                tracing::warn!(?event, "the power event could not be delivered; the handler is gone");
            }
        }
        0
    }

    /// Reads the sender back out of the window's properties.
    unsafe fn sender(hwnd: HWND) -> Option<&'static Sender<PowerEvent>> {
        let ptr = GetPropW(hwnd, w!("canvas-rgb-events"));
        if ptr.0.is_null() {
            return None;
        }
        Some(&*(ptr.0 as *const Sender<PowerEvent>))
    }

    /// The window procedure.
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_QUERYENDSESSION => {
                tracing::warn!(logoff = lparam.0 as u32 & ENDSESSION_LOGOFF != 0, "WM_QUERYENDSESSION: Windows asks whether the session may end");
                if let Some(sender) = sender(hwnd) {
                    let _ = sender.send(PowerEvent::Shutdown);
                }
                LRESULT(1)
            }
            WM_ENDSESSION => {
                tracing::warn!(
                    session_ending = wparam.0 != 0,
                    logoff = lparam.0 as u32 & ENDSESSION_LOGOFF != 0,
                    "WM_ENDSESSION received"
                );
                if let Some(sender) = sender(hwnd) {
                    let _ = sender.send(PowerEvent::Shutdown);
                }
                // wParam is TRUE when the session really ends (FALSE: it was cancelled). Windows
                // ends this process once the handler returns, so the closing line is written and
                // flushed now (#19).
                if wparam.0 != 0 {
                    crate::exit_log::end_session_exit(lparam.0 as u32 & ENDSESSION_LOGOFF != 0);
                }
                LRESULT(0)
            }
            WM_WTSSESSION_CHANGE => {
                match wparam.0 as u32 {
                    0x7 => {
                        // WTS_SESSION_LOCK
                        if let Some(sender) = sender(hwnd) {
                            let _ = sender.send(PowerEvent::Lock);
                        }
                    }
                    0x8 => {
                        // WTS_SESSION_UNLOCK
                        if let Some(sender) = sender(hwnd) {
                            let _ = sender.send(PowerEvent::Unlock);
                        }
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DISPLAY_STATE | WM_SESSION_CHANGE => {
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watcher_starts_running() {
        let watcher = PowerWatcher::new();
        assert_eq!(watcher.previous(), Power::Running);
        assert!(!watcher.suspended());
    }

    #[test]
    fn suspend_and_both_resume_notifications_are_classified() {
        assert_eq!(classify_power_notification(PBT_APMSUSPEND, None), Some(PowerEvent::Suspend));
        assert_eq!(classify_power_notification(PBT_APMRESUMEAUTOMATIC, None), Some(PowerEvent::Resume));
        assert_eq!(classify_power_notification(PBT_APMRESUMESUSPEND, None), Some(PowerEvent::Resume));
        assert_eq!(classify_power_notification(0xA, None), None);
    }

    #[test]
    fn display_state_changes_are_classified() {
        let display = |data| classify_power_notification(PBT_POWERSETTINGCHANGE, Some((GUID_CONSOLE_DISPLAY_STATE, data)));
        assert_eq!(display(0), Some(PowerEvent::MonitorsOff));
        assert_eq!(display(1), Some(PowerEvent::MonitorsOn));
        assert_eq!(display(2), None, "dimmed leaves the lighting alone");
        let monitor = |data| classify_power_notification(PBT_POWERSETTINGCHANGE, Some((GUID_MONITOR_POWER_ON, data)));
        assert_eq!(monitor(0), Some(PowerEvent::MonitorsOff));
        assert_eq!(monitor(1), Some(PowerEvent::MonitorsOn));
        assert_eq!(classify_power_notification(PBT_POWERSETTINGCHANGE, Some(([0; 16], 1))), None);
    }

    #[test]
    fn guid_bytes_match_the_windows_layout() {
        // {6fe69556-704a-47a0-8f24-f84a1d60628a}: Data1..3 little endian, Data4 as written.
        assert_eq!(GUID_CONSOLE_DISPLAY_STATE[..4], [0x56, 0x95, 0xe6, 0x6f]);
        assert_eq!(GUID_CONSOLE_DISPLAY_STATE[4..8], [0x4a, 0x70, 0xa0, 0x47]);
        assert_eq!(GUID_CONSOLE_DISPLAY_STATE[8..], [0x8f, 0x24, 0xf8, 0x4a, 0x1d, 0x60, 0x62, 0x8a]);
    }
}
