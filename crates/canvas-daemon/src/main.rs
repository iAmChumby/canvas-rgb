//! `canvas-rgbd.exe`: the canvas-rgb daemon.
//!
//! It runs as the logged-on user, non-elevated, at logon. It owns the API on 127.0.0.1:9430, the
//! render engine, the OpenRGB client (and in milestone 1 the OpenRGB server itself),
//! the screen and audio capture, the storage, the logs, the first run, the sleep
//! handling, the SignalRGB conflict check and the pipe client to the service.

use canvas_daemon::exit_log::StopReason;
use canvas_daemon::state::App;
use canvas_proto::api::LinkState;
use canvas_proto::traits::{DeviceAdapter, EngineSettings};
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn main() -> std::process::ExitCode {
    // The daemon never runs elevated. This comes before the log: the logger creates and prunes
    // files in a user-writable folder, and an elevated process must not touch one. A refused
    // start is reported on stderr (a console-started admin run has one).
    if let Err(e) = canvas_daemon::elevated::ElevatedError::check() {
        eprintln!("error: {e}");
        return std::process::ExitCode::from(1);
    }

    // The logs go to a rolling daily file (G8). They start before anything else can fail, so
    // every later refused start leaves a line (#19).
    if let Err(e) = canvas_daemon::logging::init("canvas-rgbd") {
        eprintln!("warning: could not open the log file: {e}");
    }
    canvas_daemon::exit_log::install_panic_hook();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: could not start the tokio runtime: {e}");
            return finish(Exit::new(1, format!("could not start the tokio runtime: {e}")));
        }
    };

    let exit = runtime.block_on(daemon_main());
    // A task still blocked must not hold the daemon up (G3): the runtime gets a short
    // grace, then it is dropped without waiting for blocked workers.
    runtime.shutdown_timeout(RUNTIME_GRACE);
    finish(exit)
}

/// Why `main` is ending the process and with which code. Every path out of `main` builds one,
/// and [`finish`] writes it to the log (#19).
struct Exit {
    code: i32,
    reason: String,
}

impl Exit {
    fn new(code: i32, reason: impl Into<String>) -> Self {
        Self { code, reason: reason.into() }
    }
}

/// Logs the exit, flushes the log file, and ends the process.
fn finish(exit: Exit) -> std::process::ExitCode {
    canvas_daemon::exit_log::log_exit(&exit.reason, exit.code);
    canvas_daemon::logging::close();
    std::process::exit(exit.code)
}

/// The quit path's bounds (G3). The whole path ends well inside `QUIT_DEADLINE`.
const QUIT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);
const ON_QUIT_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);
const BLACK_GRACE: std::time::Duration = std::time::Duration::from_millis(250);
const SERVICE_NOTICE_LIMIT: std::time::Duration = std::time::Duration::from_millis(300);
const ENGINE_STOP_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);
const RUNTIME_GRACE: std::time::Duration = std::time::Duration::from_millis(200);

/// The daemon's main loop. Returns the process exit code.
async fn daemon_main() -> Exit {
    // A snapshot before anything is changed (G10).
    if let Err(e) = canvas_daemon::storage::snapshot(&canvas_daemon::paths::home(), "startup") {
        tracing::warn!(error = %e, "could not take the startup snapshot");
    }

    // The API token.
    let token = match canvas_daemon::token::load_or_create(&canvas_daemon::paths::token_file()) {
        Ok(token) => token,
        Err(e) => {
            tracing::error!(error = %e, "could not create the API token");
            return Exit::new(1, format!("could not create the API token: {e}; exiting"));
        }
    };

    let mut store = canvas_daemon::storage::Store::load();

    // Poison is not a built-in any more: it is a user HTML effect in
    // `%APPDATA%\canvas-rgb\effects\`. The saved ids move to it, once, so the saved
    // setup comes back exactly as it was. The old case forms move to a case
    // landmark in the same pass.
    if canvas_daemon::migration::migrate_store(&mut store) {
        tracing::info!(
            "the saved setup was migrated: the old effect ids and the old case forms moved to the new ones"
        );
        if let Err(e) = store.save_layouts() {
            tracing::warn!(error = %e, "could not save the migrated layouts");
        }
        if let Err(e) = store.save_effects() {
            tracing::warn!(error = %e, "could not save the migrated effects");
        }
    }

    let settings = store.config.settings.clone();

    // The engine.
    let engine = canvas_core::spawn(EngineSettings {
        fps: settings.fps,
        power: settings.power,
        brightness: settings.brightness,
    });

    // The OpenRGB adapter, on the port the config names, with the native adapters beside it.
    let adapter = canvas_daemon::composite::CompositeAdapter::new(
        canvas_adapters::openrgb::OpenRgbAdapter::new(
            canvas_proto::consts::OPENRGB_HOST,
            store.config.openrgb_port,
        ),
    );

    // The numeric display (the CHIONE pump head). It only enumerates until a display has a saved
    // config, so starting it opens nothing on a PC without one.
    let display_adapter = Box::new(canvas_adapters::chione::ChioneDisplay::new())
        as Box<dyn canvas_proto::traits::NumericDisplayAdapter>;

    // The peripherals: the mouse and the controller. They are enumerated, never opened, until
    // the owner says so in `peripherals.json` (see `canvas_daemon::peripherals`).
    let peripherals: Vec<Box<dyn canvas_daemon::peripherals::Peripheral>> = vec![
        Box::new(canvas_adapters::attack_shark_x6::AttackSharkX6::new()),
        Box::new(canvas_adapters::eightbitdo_ultimate2::EightBitDoUltimate2::new()),
    ];

    let (quit_tx, mut quit_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let app = Arc::new(
        App::with_displays(
            store,
            engine.clone(),
            Box::new(adapter) as Box<dyn DeviceAdapter>,
            token,
            canvas_daemon::version(),
            quit_tx,
            display_adapter,
        )
        .with_peripherals(peripherals),
    );

    // The peripherals the owner allowed are opened; the others stay closed.
    canvas_daemon::peripherals::apply_saved(&app).await;

    // The zone sizes from disk.
    {
        let zones = load_zone_sizes();
        let mut guard = app.zones.write().await;
        *guard = zones;
    }

    tracing::info!(version = %canvas_daemon::version(), "canvas-rgbd is starting");

    // The user HTML effects are listed before the API answers; the watcher keeps them current.
    app.user_effects.rescan(&app.effects_dir);

    // The effect saved on the active layout runs again, so a restart does not leave the
    // lights black until something activates an effect.
    let saved_effect = app.active_layout().await.and_then(|layout| layout.effect);
    if let Some(effect) = saved_effect {
        match canvas_daemon::apply_effect(&app, &effect.id, &effect.props).await {
            Ok(()) => tracing::info!(effect = %effect.id, "the saved effect was restored"),
            Err(e) => {
                tracing::warn!(effect = %effect.id, error = %e, "the saved effect could not be restored");
                if effect.id == canvas_daemon::migration::USER_POISON_ID {
                    tracing::warn!(
                        path = %canvas_daemon::paths::effects_dir().join(canvas_daemon::migration::USER_POISON_FILE).display(),
                        "the user effect file is missing; install it to run this effect"
                    );
                }
            }
        }
    }

    // The background tasks.
    spawn_openrgb_supervisor(app.clone());
    spawn_pipe_client(app.clone());
    spawn_fan_poller(app.clone());
    spawn_display_task(app.clone());
    spawn_conflict_check(app.clone());
    spawn_power_watcher(app.clone());
    spawn_first_run(app.clone());
    spawn_user_effects_watch(app.clone());

    // The API.
    let router = canvas_daemon::api::router(app.clone());
    let address = format!(
        "{}:{}",
        canvas_proto::consts::API_HOST,
        app.store.read().await.config.api_port
    );
    let listener = match tokio::net::TcpListener::bind(&address).await {
        Ok(listener) => listener,
        Err(e) => {
            let holders = port_holders(app.store.read().await.config.api_port);
            tracing::error!(error = %e, address = %address, holders = %holders, "could not bind the API; exiting");
            return Exit::new(1, format!("could not bind the API on {address} ({e}); {holders}; exiting"));
        }
    };
    tracing::info!(address = %address, "the API is listening");

    let server = tokio::spawn(async move {
        let result = axum::serve(listener, router).await;
        if let Err(e) = result {
            tracing::error!(error = %e, "the API stopped");
        }
    });

    // Wait for the quit signal: Ctrl+C, console close/logoff/shutdown, or the API's quit route.
    let reason = tokio::select! {
        reason = console_stop() => reason,
        Some(()) = quit_rx.recv() => StopReason::Api,
    };
    canvas_daemon::exit_log::log_stop_requested(reason);

    // The process ends within QUIT_DEADLINE of this point, whatever is still running.
    arm_exit_watchdog(QUIT_DEADLINE);

    quit_sequence(&app).await;
    server.abort();

    tracing::info!("canvas-rgbd has stopped");
    Exit::new(0, format!("stopped: {}", reason.describe()))
}


/// Who holds `port`, for the line logged when the API cannot bind it: "the port 9430 is held by
/// pid 4242 (canvas-rgbd.exe)", or why that could not be found out.
fn port_holders(port: u16) -> String {
    match canvas_daemon::peer::listener_pids(port) {
        Some(pids) if pids.is_empty() => format!("port {port} has no listener now (it was taken and released)"),
        Some(pids) => {
            let named: Vec<String> = pids
                .iter()
                .map(|pid| match canvas_daemon::peer::image_name(*pid) {
                    Some(image) => format!("pid {pid} ({image})"),
                    None => format!("pid {pid}"),
                })
                .collect();
            format!("port {port} is held by {}", named.join(", "))
        }
        None => format!("could not read who listens on port {port}"),
    }
}

/// The quit path (G3), in order. Each step has a bound, so no step holds the daemon up.
async fn quit_sequence(app: &Arc<App>) {
    app.shutting_down.store(1, Ordering::SeqCst);

    // 1. `on_quit`: black, or the last frame left in place.
    if tokio::time::timeout(ON_QUIT_LIMIT, write_on_quit(app)).await.is_err() {
        tracing::warn!("the on-quit write did not finish in time");
    }

    // 2. Tell the service, if it is up.
    let notice = canvas_daemon::pipe_client::notify_quitting(app);
    if tokio::time::timeout(SERVICE_NOTICE_LIMIT, notice).await.is_err() {
        tracing::warn!("the service was not told about the quit in time");
    }

    // 3. Stop the render loop. Its thread is joined on a blocking thread, so a stuck
    // join cannot hold the daemon.
    let engine = app.engine.clone();
    let stop = tokio::task::spawn_blocking(move || engine.shutdown());
    if tokio::time::timeout(ENGINE_STOP_LIMIT, stop).await.is_err() {
        tracing::warn!("the render loop did not stop in time");
    }

    // 4. The OpenRGB server the daemon started.
    kill_openrgb_server(app);
}

/// `on_quit`: black, or the last frame left in place. Black is queued to the device
/// tasks, which write it on their own; the grace gives them time to take it before the
/// process ends.
async fn write_on_quit(app: &Arc<App>) {
    let on_quit = app.store.read().await.config.settings.on_quit;
    match on_quit {
        canvas_proto::config::OnQuit::Black => {
            app.identify.stop();
            // The engine skips a sink with no LEDs, so only the devices it writes are counted.
            let targets = canvas_daemon::solid_write_targets(&app.devices.read().await);
            app.engine.write_solid(canvas_proto::Rgb8::BLACK);
            canvas_daemon::logging::log_device_write("queued black for", targets);
            tokio::time::sleep(BLACK_GRACE).await;
        }
        canvas_proto::config::OnQuit::LeaveLastFrame => {
            tracing::info!(reason = "quit", "leaving the last frame on the devices");
        }
    }
}

/// Stops the OpenRGB server the daemon started (milestone 1). Nothing is stopped when the
/// service owns the server or something else already answered the port.
fn kill_openrgb_server(app: &App) {
    let mut slot = app.openrgb_child.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(mut server) = slot.take() {
        server.kill();
        tracing::info!(reason = "quit", "stopped the OpenRGB server the daemon started");
    }
}

/// The exit code of the watchdog's forced exit: the quit was requested, so it is not a failure.
const WATCHDOG_EXIT_CODE: i32 = 0;

/// The backstop: after `after`, the process ends whatever is still running.
fn arm_exit_watchdog(after: std::time::Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        tracing::warn!("the quit deadline passed; ending the process");
        canvas_daemon::exit_log::log_stop_exit(StopReason::QuitDeadline, WATCHDOG_EXIT_CODE);
        // Drain the log writer so the closing line is in the file before the process ends.
        canvas_daemon::logging::close();
        std::process::exit(WATCHDOG_EXIT_CODE);
    });
}

/// Waits for Ctrl+C or a console control event and says which one: close, logoff or shutdown
/// (the three the system sends a console process before it ends it), or Ctrl+Break.
async fn console_stop() -> StopReason {
    #[cfg(windows)]
    {
        use tokio::signal::windows as sig;
        // A signal that cannot be registered never fires; the others still do.
        use canvas_daemon::exit_log::{
            CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
        };
        // Each branch resolves to the reason its Win32 event type maps to.
        macro_rules! watch {
            ($register:expr, $name:literal, $ctrl_type:expr) => {
                async {
                    match $register {
                        Ok(mut signal) => {
                            signal.recv().await;
                            StopReason::from_ctrl_type($ctrl_type).unwrap_or(StopReason::CtrlC)
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, event = $name, "a console control event cannot be watched");
                            std::future::pending::<StopReason>().await
                        }
                    }
                }
            };
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => StopReason::from_ctrl_type(canvas_daemon::exit_log::CTRL_C_EVENT).unwrap_or(StopReason::CtrlC),
            reason = watch!(sig::ctrl_close(), "CTRL_CLOSE_EVENT", CTRL_CLOSE_EVENT) => reason,
            reason = watch!(sig::ctrl_logoff(), "CTRL_LOGOFF_EVENT", CTRL_LOGOFF_EVENT) => reason,
            reason = watch!(sig::ctrl_shutdown(), "CTRL_SHUTDOWN_EVENT", CTRL_SHUTDOWN_EVENT) => reason,
            reason = watch!(sig::ctrl_break(), "CTRL_BREAK_EVENT", CTRL_BREAK_EVENT) => reason,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = tokio::signal::ctrl_c().await;
        StopReason::CtrlC
    }
}

/// Reads the zone sizes from `zones.json`.
fn load_zone_sizes() -> std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u32>> {
    let path = canvas_daemon::paths::zones_file();
    match canvas_daemon::storage::read_parsed::<canvas_proto::config::ZonesFile>(
        &path,
        canvas_proto::config::ZONES_SCHEMA,
        "zones.json",
    ) {
        Ok(Some(file)) => file.sizes,
        Ok(None) => std::collections::BTreeMap::new(),
        Err(e) => {
            tracing::error!(error = %e, "could not read the zone sizes");
            std::collections::BTreeMap::new()
        }
    }
}

/// Gives every connected device a frame sink in the engine: a bounded(1) channel into a
/// writer task that sends each frame to the adapter. Devices that went away lose theirs.
/// Without this the engine has no sinks, stays parked, and nothing reaches the lights.
async fn sync_device_sinks(
    app: &Arc<App>,
    devices: &[canvas_proto::DeviceInfo],
    sinks: &tokio::sync::Mutex<std::collections::HashMap<canvas_proto::DeviceId, tokio::task::JoinHandle<()>>>,
) {
    let mut sinks = sinks.lock().await;
    let live: std::collections::HashSet<_> = devices
        .iter()
        .filter(|d| d.connected)
        .filter(|d| {
            let driven = canvas_daemon::is_driven(&d.id);
            if !driven && !sinks.contains_key(&d.id) {
                tracing::info!(device = %d.id, "this device is never driven; it gets no frames");
            }
            driven
        })
        .map(|d| d.id.clone())
        .collect();
    sinks.retain(|id, writer| {
        if live.contains(id) {
            return true;
        }
        app.engine.remove_sink(id);
        writer.abort();
        tracing::info!(device = %id, "device gone; its frame sink is removed");
        false
    });
    for id in live {
        if sinks.contains_key(&id) {
            continue;
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<canvas_proto::Rgb8>>(1);
        let adapter = app.adapter.clone();
        let writer_id = id.clone();
        let writer = tokio::spawn(async move {
            let mut failures = 0u32;
            while let Some(frame) = rx.recv().await {
                let result = adapter.lock().await.write_frame(&writer_id, &frame).await;
                match result {
                    Ok(()) => failures = 0,
                    Err(e) => {
                        failures += 1;
                        // Log the first failure and then every 300th, not every frame.
                        if failures == 1 || failures.is_multiple_of(300) {
                            tracing::warn!(device = %writer_id, error = %e, failures, "frame write failed");
                        }
                    }
                }
            }
        });
        app.engine.add_sink(id.clone(), Arc::new(canvas_daemon::state::DeviceSink::with_gate(id.clone(), tx, app.identify.clone())));
        tracing::info!(device = %id, "device frame sink added");
        sinks.insert(id, writer);
    }
}

/// Keeps the OpenRGB connection alive and re-applies the zone sizes.
fn spawn_openrgb_supervisor(app: Arc<App>) {
    let adapter = app.adapter.clone();
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let app_for_supervisor = app.clone();
    let zones = app_for_supervisor.zones.clone();
    // Devices that have a frame sink registered with the engine.
    let sinks: Arc<tokio::sync::Mutex<std::collections::HashMap<canvas_proto::DeviceId, tokio::task::JoinHandle<()>>>> =
        Arc::default();
    tokio::spawn(async move {
        // OpenRGB is starting: the UI shows "waking up", not "broken".
        *app_for_supervisor.openrgb.write().await = LinkState::Connecting;
        // Milestone 1: the daemon starts the OpenRGB server itself when the service does
        // not own it. The app keeps it, and the quit path stops it.
        let port = app_for_supervisor.store.read().await.config.openrgb_port;
        if let Some(server) =
            canvas_daemon::openrgb_supervisor::ensure_openrgb_running(port).await
        {
            *app_for_supervisor
                .openrgb_child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(server);
        }
        let _restarts = canvas_daemon::openrgb_supervisor::RestartBudget::default();

        canvas_daemon::openrgb_supervisor::supervise(
            adapter,
            // The zone sizes are shared with the app, so a resize shows up here.
            zones,
            {
                let app = app_for_supervisor.clone();
                let sinks = sinks.clone();
                move |devices| {
                    let app = app.clone();
                    let sinks = sinks.clone();
                    // The device list and the layout are updated on the runtime, because
                    // the callback is not async.
                    tokio::spawn(async move {
                        {
                            let mut list = app.devices.write().await;
                            *list = devices.clone();
                        }
                        {
                            let mut state = app.openrgb.write().await;
                            *state = LinkState::Connected;
                        }
                        // After the first run, a zone that appears (a late device, a
                        // header that got its LEDs) is placed instead of staying black.
                        {
                            let mut store = app.store.write().await;
                            if store.config.first_run_done {
                                let added = store.active_layout_mut().map_or(0, |layout| {
                                    canvas_daemon::first_run::place_missing_zones(layout, &devices)
                                });
                                if added > 0 {
                                    tracing::info!(added, "new zones were placed on the canvas");
                                    if let Err(e) = store.save_layouts() {
                                        tracing::warn!(error = %e, "could not save the layouts");
                                    }
                                }
                            }
                        }
                        // The engine needs the new devices and the zone sizes.
                        if let Some(layout) = app.active_layout().await {
                            let device_list = app.devices.read().await.clone();
                            app.engine.set_layout(layout, device_list);
                        }
                        sync_device_sinks(&app, &devices, &sinks).await;
                        // The first run waits for the devices.
                        canvas_daemon::first_run::maybe_run(&app).await;
                    });
                }
            },
            {
                let app = app_for_supervisor.clone();
                // The link state follows every attempt. `Connected` is set with the devices.
                move |link| {
                    if link == LinkState::Connected {
                        return;
                    }
                    let app = app.clone();
                    tokio::spawn(async move {
                        *app.openrgb.write().await = link;
                    });
                }
            },
            stop_rx,
        )
        .await;
    });
    // The supervisor stops when the daemon quits.
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if app.shutting_down.load(Ordering::SeqCst) != 0 {
                let _ = stop_tx.send(true);
                return;
            }
        }
    });
}

/// Keeps the pipe client connected.
fn spawn_pipe_client(app: Arc<App>) {
    tokio::spawn(async move {
        let mut down_since = std::time::Instant::now();
        loop {
            if let Ok(mut client) = canvas_daemon::pipe_client::PipeClient::connect().await {
                let hello = canvas_proto::pipe::PipeRequest::Hello {
                    protocol: canvas_proto::pipe::PROTOCOL_VERSION,
                    daemon_pid: std::process::id(),
                };
                match client.request(&hello).await {
                        Ok(canvas_proto::pipe::PipeResponse::Hello { protocol, service_version }) => {
                            tracing::info!(protocol, service_version, "the service is up");
                            app.service_down_ms.store(0, Ordering::SeqCst);
                            down_since = std::time::Instant::now();
                            // The heartbeat goes every second.
                            loop {
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    canvas_proto::consts::HEARTBEAT_INTERVAL_MS,
                                ))
                                .await;
                                if client.request(&canvas_proto::pipe::PipeRequest::Heartbeat).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(other) => {
                            tracing::warn!(response = ?other, "the service answered the hello differently");
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "the service did not answer the hello");
                        }
                    }
            }

            // The service is down.
            let down = down_since.elapsed().as_millis() as u64;
            app.service_down_ms.store(down.max(1), Ordering::SeqCst);
            tokio::time::sleep(canvas_daemon::pipe_client::RECONNECT_INTERVAL).await;
        }
    });
}

/// Polls the fan status from the service, so `/fans` has something to show.
fn spawn_fan_poller(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            if app.shutting_down.load(Ordering::SeqCst) != 0 {
                return;
            }
            if let Ok(mut client) = canvas_daemon::pipe_client::PipeClient::connect().await {
                let hello = canvas_proto::pipe::PipeRequest::Hello {
                    protocol: canvas_proto::pipe::PROTOCOL_VERSION,
                    daemon_pid: std::process::id(),
                };
                if client.request(&hello).await.is_ok() {
                    if let Ok(canvas_proto::pipe::PipeResponse::FanStatus { status }) =
                        client.request(&canvas_proto::pipe::PipeRequest::FanStatus).await
                    {
                        let mut fans = app.fans.write().await;
                        *fans = Some(status);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

/// Drives the numeric displays: writes each configured display's value at its own rate, and
/// backs off after an adapter error. Idle until a display has a saved config.
fn spawn_display_task(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            if app.shutting_down.load(Ordering::SeqCst) != 0 {
                return;
            }
            let wait = canvas_daemon::displays::tick(&app, std::time::Instant::now()).await;
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = app.displays.wake.notified() => {}
            }
        }
    });
}

/// Rescans the user effects folder every few seconds, so an added or edited HTML effect
/// shows up without a restart.
fn spawn_user_effects_watch(app: Arc<App>) {
    tokio::spawn(async move {
        let dir = app.effects_dir.clone();
        loop {
            tokio::time::sleep(canvas_daemon::user_effects::RESCAN_INTERVAL).await;
            let user = app.user_effects.clone();
            let dir = dir.clone();
            // Parsing an effect can take a while; keep it off the async workers.
            let _ = tokio::task::spawn_blocking(move || user.rescan(&dir)).await;
        }
    });
}

/// Checks for SignalRGB every 5 s.
fn spawn_conflict_check(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            let conflicts = canvas_daemon::conflict::detect();
            {
                let mut guard = app.conflicts.write().await;
                *guard = conflicts.clone();
            }
            for line in canvas_daemon::conflict::describe(&conflicts) {
                tracing::warn!("{line}");
            }
            tokio::time::sleep(canvas_daemon::conflict::CHECK_INTERVAL).await;
        }
    });
}

/// Watches for sleep, shutdown, lock and monitors-off.
fn spawn_power_watcher(app: Arc<App>) {
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(windows)]
    {
        if let Err(e) = canvas_daemon::power::windows_watcher::start(tx) {
            tracing::warn!(error = %e, "the power watcher could not start");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = tx;
    }
    // The events come over a std channel that blocks in `recv`. It runs on a blocking
    // thread: a runtime worker parked in `recv` could never be released at quit.
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let mut watcher = canvas_daemon::power::PowerWatcher::new();
        while let Ok(event) = rx.recv() {
            let action = handle.block_on(canvas_daemon::power::handle(&app, &mut watcher, event));
            tracing::info!(?event, ?action, "a power event was handled");
        }
    });
}

/// Runs the first run once the devices are there.
fn spawn_first_run(app: Arc<App>) {
    tokio::spawn(async move {
        // Give the OpenRGB supervisor a moment to discover the devices.
        for _ in 0..30 {
            if app.shutting_down.load(Ordering::SeqCst) != 0 {
                return;
            }
            if !app.devices.read().await.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        canvas_daemon::first_run::maybe_run(&app).await;
    });
}
