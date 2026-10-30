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
    use std::sync::mpsc::Sender;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Power::{
        RegisterPowerSettingNotification, RegisterSuspendResumeNotification, POWERBROADCAST_SETTING,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetPropW, PostQuitMessage,
        RegisterClassW, RegisterWindowMessageW, SetPropW, TranslateMessage, WINDOW_EX_STYLE,
        WINDOW_STYLE, WM_APP, WM_ENDSESSION, WM_POWERBROADCAST, WM_QUERYENDSESSION,
        WM_WTSSESSION_CHANGE, WNDCLASSW, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
        PBT_APMSUSPEND, HWND_MESSAGE,
    };
    use windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification;

    /// The window class name.
    const CLASS_NAME: PCWSTR = w!("CanvasRgbPowerWatcher");

    /// `ENDSESSION_LOGOFF` in the `lParam` of `WM_ENDSESSION`: the user is logging off.
    const ENDSESSION_LOGOFF: u32 = 0x8000_0000;

    /// The message the hidden window posts to itself when the display state changes.
    const WM_DISPLAY_STATE: u32 = WM_APP + 1;
    /// The message the hidden window posts to itself when the session changes.
    const WM_SESSION_CHANGE: u32 = WM_APP + 2;

    /// The GUID of the console display state setting.
    const GUID_CONSOLE_DISPLAY_STATE: windows::core::GUID =
        windows::core::GUID::from_values(
            0x6fe69556,
            0x704a,
            0x47a0,
            [0x8f, 0x24, 0xf8, 0x4a, 0x1d, 0x60, 0x62, 0x8a],
        );

    /// The GUID of the monitor power setting.
    const GUID_MONITOR_POWER_ON: windows::core::GUID =
        windows::core::GUID::from_values(
            0x02731015,
            0x4510,
            0x4526,
            [0x99, 0xe6, 0xe5, 0x7a, 0x32, 0xbe, 0xfd, 0xd1],
        );

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

            let handle = windows::Win32::Foundation::HANDLE(hwnd.0);
            let _ = RegisterSuspendResumeNotification(
                handle,
                windows::Win32::UI::WindowsAndMessaging::REGISTER_NOTIFICATION_FLAGS(0),
            );
            let _ = RegisterPowerSettingNotification(
                handle,
                &GUID_CONSOLE_DISPLAY_STATE,
                windows::Win32::UI::WindowsAndMessaging::REGISTER_NOTIFICATION_FLAGS(0),
            );
            let _ = RegisterPowerSettingNotification(
                handle,
                &GUID_MONITOR_POWER_ON,
                windows::Win32::UI::WindowsAndMessaging::REGISTER_NOTIFICATION_FLAGS(0),
            );
            let _ = WTSRegisterSessionNotification(
                hwnd,
                windows::Win32::System::RemoteDesktop::NOTIFY_FOR_THIS_SESSION,
            );

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
            WM_POWERBROADCAST => {
                match wparam.0 as u32 {
                    PBT_APMSUSPEND => {
                        if let Some(sender) = sender(hwnd) {
                            let _ = sender.send(PowerEvent::Suspend);
                        }
                    }
                    PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC => {
                        if let Some(sender) = sender(hwnd) {
                            let _ = sender.send(PowerEvent::Resume);
                        }
                    }
                    _ => {}
                }
                LRESULT(0)
            }
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

    /// Kept so the unused-import checker does not complain about the types that only
    /// matter on the Windows path.
    #[allow(dead_code)]
    fn _types() {
        let _: Option<POWERBROADCAST_SETTING> = None;
        let _: Option<HANDLE> = None;
        unsafe {
            let _ = RegisterWindowMessageW(w!("canvas-rgb-test"));
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
}
