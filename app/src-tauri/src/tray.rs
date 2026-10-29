//! The tray icon, its menu, tooltip and warning icon, and the poll that keeps them current.
//!
//! One background thread polls the daemon every `POLL_INTERVAL`, and every action calls
//! `refresh` too. A refresh rebuilds the menu only when the snapshot changed, so an open menu
//! is not replaced under the user's pointer by a timer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use canvas_proto::api::{
    routes, ActivateEffect, Conflict, EffectsResponse, FanProfileName, PatchOutput, Power, Status,
};
use canvas_proto::fans::{FansStatus, PIPE_LOST_WARN_MS};

use canvas_proto::consts;

use crate::{api, commands, editor, log, onboard};

pub const TRAY_ID: &str = "main";
const POLL_INTERVAL: Duration = Duration::from_secs(5);

const TOOLTIP_OFFLINE: &str = "canvas-rgb daemon not running";
const TOOLTIP_FAN_SERVICE_DOWN: &str = "Fan service not running: fans are on BIOS";
const TOOLTIP_SIGNALRGB: &str = "SignalRGB is running";

/// What the menu and tooltip show. Equal snapshots mean nothing visible changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The daemon answered `GET /status`.
    pub daemon: bool,
    /// Lighting power is running (not paused).
    pub running: bool,
    pub active_effect: Option<String>,
    /// Favorite effects as `(id, name)`. `None` when the effects list could not be read.
    pub favorites: Option<Vec<(String, String)>>,
    /// The fan profile menu data. `None` when the fan service did not answer (503).
    pub fans: Option<FanMenu>,
    /// The daemon has lost the fan service for at least `PIPE_LOST_WARN_MS` (G4).
    pub fan_service_down: bool,
    pub signalrgb: bool,
    /// G7: the editor should open once for the "size your ARGB headers" panel.
    pub show_zone_panel: bool,
    /// The user reached Done in the onboarding. Offline snapshots say `true`: no answer, no open.
    pub onboarding_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanMenu {
    pub active: String,
    pub profiles: Vec<String>,
}

impl Snapshot {
    fn offline() -> Self {
        Snapshot {
            daemon: false,
            running: false,
            active_effect: None,
            favorites: None,
            fans: None,
            fan_service_down: false,
            signalrgb: false,
            show_zone_panel: false,
            onboarding_complete: true,
        }
    }
}

/// Managed state: the last snapshot applied, and whether the G7 panel has opened the editor.
#[derive(Default)]
pub struct TrayState {
    last: Mutex<Option<Snapshot>>,
    zone_panel_opened: AtomicBool,
    /// The window was opened for the onboarding in this tray run (by `--onboard` or a poll).
    onboarding_opened: AtomicBool,
}

/// Creates the tray icon. The daemon need not be running: the tray starts offline and
/// the poll fills it in.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    app.manage(TrayState::default());
    let menu = build_menu(app, &Snapshot::offline())?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon_for(false)?)
        .tooltip(TOOLTIP_OFFLINE)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event: MenuEvent| on_menu_event(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                editor::open(tray.app_handle(), None);
            }
        })
        .build(app)?;
    Ok(())
}

/// Starts the background poll. Call once, after `build`.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || loop {
        refresh(&app);
        std::thread::sleep(POLL_INTERVAL);
    });
}

/// Polls the daemon and applies the result to the tray if it changed. Blocking: call it off
/// the UI thread.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    let snap = poll();
    let state = app.state::<TrayState>();
    let changed = {
        let mut last = state.last.lock().unwrap_or_else(|p| p.into_inner());
        if last.as_ref() == Some(&snap) {
            false
        } else {
            *last = Some(snap.clone());
            true
        }
    };
    if changed {
        let handle = app.clone();
        let snap_for_main = snap.clone();
        if let Err(e) = app.run_on_main_thread(move || apply(&handle, &snap_for_main)) {
            log::append(&format!("could not update the tray: {e}"));
        }
    }
    // The old "size your ARGB headers" panel stays off until the onboarding is done, so the
    // two never open the window twice.
    if snap.show_zone_panel
        && snap.onboarding_complete
        && !state.zone_panel_opened.swap(true, Ordering::SeqCst)
    {
        editor::open(app, None);
    }
    auto_open_for_onboarding(app, &snap);
}

/// Opens the editor on Layout for the onboarding, now. Used for `--onboard`, which does not
/// count toward the auto-open limit. It marks this run as opened, so the poll does not open
/// the window a second time.
pub fn open_onboarding<R: Runtime>(app: &AppHandle<R>) {
    app.state::<TrayState>().onboarding_opened.store(true, Ordering::SeqCst);
    editor::open(app, Some("layout"));
}

/// The poll path: the first successful poll that reports `onboarding_complete == false` opens
/// the window once per tray run, while the tray's counter is below the limit.
fn auto_open_for_onboarding<R: Runtime>(app: &AppHandle<R>, snap: &Snapshot) {
    let state = app.state::<TrayState>();
    let dir = onboard::tray_data_dir();
    let opened = state.onboarding_opened.load(Ordering::SeqCst);
    let count = onboard::auto_open_count(&dir);
    if !onboard::should_auto_open(snap.daemon, snap.onboarding_complete, opened, count) {
        return;
    }
    if state.onboarding_opened.swap(true, Ordering::SeqCst) {
        return;
    }
    match onboard::bump_auto_open_count(&dir) {
        Ok(n) => log::append(&format!(
            "onboarding: the poll opened the window ({n} of {})",
            onboard::AUTO_OPEN_LIMIT
        )),
        Err(e) => log::append(&format!("onboarding: could not save the open counter: {e}")),
    }
    editor::open(app, Some("layout"));
}

fn poll() -> Snapshot {
    // The status route needs no token, but its answer drives the menu and tooltip: only a
    // listener that passes the peer check (canvas-rgbd.exe, same user) may supply it.
    if crate::peer::check(consts::API_PORT).is_err() {
        return Snapshot::offline();
    }
    let client = api::client();
    let status: Status = match client.status() {
        Ok(status) => status,
        Err(_) => return Snapshot::offline(),
    };

    let favorites = match client.get::<EffectsResponse>(routes::EFFECTS) {
        Ok(effects) => Some(
            effects
                .effects
                .into_iter()
                .filter(|e| e.favorite)
                .map(|e| (e.id, e.name))
                .collect(),
        ),
        Err(_) => None,
    };

    let profile = client.get::<FanProfileName>(routes::FAN_PROFILE);
    let fans_list = client.get::<FansStatus>(routes::FANS);
    let fans = match (profile, fans_list) {
        (Ok(profile), Ok(fans)) => Some(FanMenu {
            active: profile.name,
            profiles: fans.profiles,
        }),
        _ => None,
    };

    Snapshot {
        daemon: true,
        running: status.power == Power::Running,
        active_effect: status.active_effect.clone(),
        favorites,
        fans,
        fan_service_down: status.service_down_ms.is_some_and(|ms| ms >= PIPE_LOST_WARN_MS),
        signalrgb: status
            .conflicts
            .iter()
            .any(|c| matches!(c, Conflict::SignalRgb { process, service } if *process || *service)),
        show_zone_panel: status.show_zone_panel,
        onboarding_complete: status.onboarding_complete,
    }
}

fn apply<R: Runtime>(app: &AppHandle<R>, snap: &Snapshot) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(app, snap) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => log::append(&format!("could not build the tray menu: {e}")),
    }
    let _ = tray.set_tooltip(Some(tooltip(snap)));
    match icon_for(snap.fan_service_down) {
        Ok(icon) => {
            let _ = tray.set_icon(Some(icon));
        }
        Err(e) => log::append(&format!("could not load the tray icon: {e}")),
    }
}

fn tooltip(snap: &Snapshot) -> String {
    if !snap.daemon {
        return TOOLTIP_OFFLINE.to_string();
    }
    let mut warnings = Vec::new();
    if snap.fan_service_down {
        warnings.push(TOOLTIP_FAN_SERVICE_DOWN);
    }
    if snap.signalrgb {
        warnings.push(TOOLTIP_SIGNALRGB);
    }
    if warnings.is_empty() {
        if snap.running {
            "canvas-rgb: lighting on".to_string()
        } else {
            "canvas-rgb: lighting paused".to_string()
        }
    } else {
        warnings.join("; ")
    }
}

fn icon_for(warning: bool) -> tauri::Result<Image<'static>> {
    if warning {
        Image::from_bytes(include_bytes!("../icons/tray-warning.png"))
    } else {
        Image::from_bytes(include_bytes!("../icons/tray-normal.png"))
    }
}

fn build_menu<R: Runtime>(app: &AppHandle<R>, snap: &Snapshot) -> tauri::Result<Menu<R>> {
    let (power_id, power_text) = if snap.running || !snap.daemon {
        ("power:paused", "Pause lighting")
    } else {
        ("power:running", "Resume lighting")
    };

    let items: Vec<Box<dyn IsMenuItem<R>>> = vec![
        Box::new(MenuItem::with_id(app, power_id, power_text, snap.daemon, None::<&str>)?),
        Box::new(effects_submenu(app, snap)?),
        Box::new(fans_submenu(app, snap)?),
        Box::new(MenuItem::with_id(app, "fans", "Fans…", true, None::<&str>)?),
        Box::new(PredefinedMenuItem::separator(app)?),
        Box::new(MenuItem::with_id(app, "editor", "Open editor", true, None::<&str>)?),
        Box::new(MenuItem::with_id(app, "logs", "Open logs folder", true, None::<&str>)?),
        Box::new(PredefinedMenuItem::separator(app)?),
        Box::new(MenuItem::with_id(app, "quit", "Quit canvas-rgb", true, None::<&str>)?),
    ];
    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|item| item.as_ref()).collect();
    Menu::with_items(app, &refs)
}

fn effects_submenu<R: Runtime>(app: &AppHandle<R>, snap: &Snapshot) -> tauri::Result<Submenu<R>> {
    let mut items: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
    if !snap.daemon {
        items.push(Box::new(MenuItem::with_id(app, "noop", "Daemon not running", false, None::<&str>)?));
    } else {
        match &snap.favorites {
            None => items.push(Box::new(MenuItem::with_id(app, "noop", "Effects unavailable", false, None::<&str>)?)),
            Some(favorites) if favorites.is_empty() => items.push(Box::new(MenuItem::with_id(
                app,
                "noop",
                "No favorites yet (star one in the editor)",
                false,
                None::<&str>,
            )?)),
            Some(favorites) => {
                for (id, name) in favorites {
                    let checked = snap.active_effect.as_deref() == Some(id.as_str());
                    items.push(Box::new(CheckMenuItem::with_id(
                        app,
                        format!("effect:{id}"),
                        name,
                        true,
                        checked,
                        None::<&str>,
                    )?));
                }
            }
        }
    }
    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|item| item.as_ref()).collect();
    Submenu::with_items(app, "Effects", snap.daemon, &refs)
}

fn fans_submenu<R: Runtime>(app: &AppHandle<R>, snap: &Snapshot) -> tauri::Result<Submenu<R>> {
    let mut items: Vec<Box<dyn IsMenuItem<R>>> = Vec::new();
    let title = if !snap.daemon {
        "Fan profile"
    } else if snap.fans.is_none() {
        "Fan profile (service not running)"
    } else {
        "Fan profile"
    };
    match &snap.fans {
        Some(fans) if !fans.profiles.is_empty() => {
            for name in &fans.profiles {
                items.push(Box::new(CheckMenuItem::with_id(
                    app,
                    format!("profile:{name}"),
                    name,
                    true,
                    *name == fans.active,
                    None::<&str>,
                )?));
            }
        }
        Some(_) => items.push(Box::new(MenuItem::with_id(app, "noop", "No profiles", false, None::<&str>)?)),
        None => {
            let text = if snap.daemon { "Fan service not running" } else { "Daemon not running" };
            items.push(Box::new(MenuItem::with_id(app, "noop", text, false, None::<&str>)?));
        }
    }
    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|item| item.as_ref()).collect();
    Submenu::with_items(app, title, snap.daemon && snap.fans.is_some(), &refs)
}

fn on_menu_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    if let Some(power) = id.strip_prefix("power:") {
        let power = if power == "paused" { Power::Paused } else { Power::Running };
        act(app, "set lighting power", move |client| {
            client
                .patch::<_, serde_json::Value>(routes::OUTPUT, &PatchOutput { power: Some(power), brightness: None })
                .map(drop)
        });
    } else if let Some(effect) = id.strip_prefix("effect:") {
        let path = routes::EFFECT_ACTIVATE.replace("{id}", effect);
        act(app, "activate effect", move |client| {
            client
                .post::<_, serde_json::Value>(&path, &ActivateEffect::default())
                .map(drop)
        });
    } else if let Some(name) = id.strip_prefix("profile:") {
        let body = FanProfileName { name: name.to_string() };
        act(app, "switch fan profile", move |client| {
            client
                .patch::<_, serde_json::Value>(routes::FAN_PROFILE, &body)
                .map(drop)
        });
    } else {
        match id {
            "editor" => editor::open(app, None),
            "fans" => editor::open(app, Some("fans")),
            "logs" => {
                if let Err(e) = commands::open_logs(app) {
                    log::append(&e);
                }
            }
            "quit" => quit(app),
            _ => {}
        }
    }
}

/// Runs a daemon call off the UI thread, then refreshes the tray and tells the editor.
fn act<R: Runtime, F>(app: &AppHandle<R>, what: &'static str, call: F)
where
    F: FnOnce(&api::Client) -> Result<(), api::ApiError> + Send + 'static,
{
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = call(api::client()) {
            log::append(&format!("{what} failed: {e}"));
        }
        refresh(&app);
        editor::notify_state_changed(&app);
    });
}

/// "Quit": the daemon applies `on_quit`, tells the fan service it is quitting on purpose,
/// and exits. Then the tray exits, whether or not the daemon answered.
fn quit<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    std::thread::spawn(move || {
        let body = serde_json::json!({});
        if let Err(e) = api::client().post::<_, serde_json::Value>(routes::QUIT, &body) {
            log::append(&format!("quit: the daemon did not confirm the quit: {e}"));
        }
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || handle.exit(0));
    });
}
