//! `canvas-rgbd.exe`: the canvas-rgb daemon.
//!
//! It runs as Luke, non-elevated, at logon. It owns the API on 127.0.0.1:9430, the
//! render engine, the OpenRGB client (and in milestone 1 the OpenRGB server itself),
//! the screen and audio capture, the storage, the logs, the first run, the sleep
//! handling, the SignalRGB conflict check and the pipe client to the service.

use canvas_daemon::state::App;
use canvas_proto::api::LinkState;
use canvas_proto::traits::{DeviceAdapter, EngineSettings};
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn main() -> std::process::ExitCode {
    // The daemon never runs elevated.
    if let Err(e) = canvas_daemon::elevated::ElevatedError::check() {
        eprintln!("error: {e}");
        return std::process::ExitCode::from(1);
    }

    // The logs go to a rolling daily file (G8).
    let _log_guard = match canvas_daemon::logging::init("canvas-rgbd") {
        Ok(guard) => Some(guard),
        Err(e) => {
            eprintln!("warning: could not open the log file: {e}");
            None
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: could not start the tokio runtime: {e}");
            return std::process::ExitCode::from(1);
        }
    };

    let code = runtime.block_on(daemon_main());
    // A task still blocked must not hold the daemon up (G3): the runtime gets a short
    // grace, then it is dropped without waiting for blocked workers.
    runtime.shutdown_timeout(RUNTIME_GRACE);
    // Dropping the guard flushes the log file before the process ends.
    drop(_log_guard);
    std::process::exit(code)
}

/// The quit path's bounds (G3). The whole path ends well inside `QUIT_DEADLINE`.
const QUIT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);
const ON_QUIT_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);
const BLACK_GRACE: std::time::Duration = std::time::Duration::from_millis(250);
const SERVICE_NOTICE_LIMIT: std::time::Duration = std::time::Duration::from_millis(300);
const ENGINE_STOP_LIMIT: std::time::Duration = std::time::Duration::from_millis(500);
const RUNTIME_GRACE: std::time::Duration = std::time::Duration::from_millis(200);

/// The daemon's main loop. Returns the process exit code.
async fn daemon_main() -> i32 {
    // A snapshot before anything is changed (G10).
    if let Err(e) = canvas_daemon::storage::snapshot(&canvas_daemon::paths::home(), "startup") {
        tracing::warn!(error = %e, "could not take the startup snapshot");
    }

    // The API token.
    let token = match canvas_daemon::token::load_or_create(&canvas_daemon::paths::token_file()) {
        Ok(token) => token,
        Err(e) => {
            tracing::error!(error = %e, "could not create the API token");
            return 1;
        }
    };

    let store = canvas_daemon::storage::Store::load();
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

    let (quit_tx, mut quit_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let app = Arc::new(App::new(
        store,
        engine.clone(),
        Box::new(adapter) as Box<dyn DeviceAdapter>,
        token,
        canvas_daemon::version(),
        quit_tx,
    ));

    // The zone sizes from disk.
    {
        let zones = load_zone_sizes();
        let mut guard = app.zones.write().await;
        *guard = zones;
    }

    tracing::info!(version = %canvas_daemon::version(), "canvas-rgbd is starting");

    // The user HTML effects are listed before the API answers; the watcher keeps them current.
    app.user_effects.rescan(&canvas_daemon::paths::effects_dir());

    // The effect saved on the active layout runs again, so a restart does not leave the
    // lights black until something activates an effect.
    let saved_effect = app.active_layout().await.and_then(|layout| layout.effect);
    if let Some(effect) = saved_effect {
        match canvas_daemon::apply_effect(&app, &effect.id, &effect.props).await {
            Ok(()) => tracing::info!(effect = %effect.id, "the saved effect was restored"),
            Err(e) => tracing::warn!(effect = %effect.id, error = %e, "the saved effect could not be restored"),
        }
    }

    // The background tasks.
    spawn_openrgb_supervisor(app.clone());
    spawn_pipe_client(app.clone());
    spawn_fan_poller(app.clone());
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
            tracing::error!(error = %e, address = %address, "could not bind the API");
            return 1;
        }
    };
    tracing::info!(address = %address, "the API is listening");

    let server = tokio::spawn(async move {
        let result = axum::serve(listener, router).await;
        if let Err(e) = result {
            tracing::error!(error = %e, "the API stopped");
        }
    });

    // Wait for the quit signal: Ctrl+C, console close, or the API's quit route.
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!(reason = "ctrl-c", "the daemon was asked to stop");
        }
        _ = console_close() => {
            tracing::info!(reason = "console-close", "the daemon was asked to stop");
        }
        Some(()) = quit_rx.recv() => {
            tracing::info!(reason = "api", "the daemon was asked to stop");
        }
    }

    // The process ends within QUIT_DEADLINE of this point, whatever is still running.
    arm_exit_watchdog(QUIT_DEADLINE);

    quit_sequence(&app).await;
    server.abort();

    tracing::info!("canvas-rgbd has stopped");
    0
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
            app.engine.write_solid(canvas_proto::Rgb8::BLACK);
            tracing::info!(reason = "quit", "wrote black to every device");
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

/// The backstop: after `after`, the process ends whatever is still running.
fn arm_exit_watchdog(after: std::time::Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        tracing::warn!("the quit deadline passed; ending the process");
        std::process::exit(0);
    });
}

/// Console close, logoff and shutdown: the same quit as Ctrl+C.
async fn console_close() {
    #[cfg(windows)]
    {
        if let Ok(mut close) = tokio::signal::windows::ctrl_close() {
            close.recv().await;
            return;
        }
    }
    std::future::pending::<()>().await
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
        app.engine.add_sink(id.clone(), Arc::new(canvas_daemon::state::DeviceSink::new(id.clone(), tx)));
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

/// Rescans the user effects folder every few seconds, so an added or edited HTML effect
/// shows up without a restart.
fn spawn_user_effects_watch(app: Arc<App>) {
    tokio::spawn(async move {
        let dir = canvas_daemon::paths::effects_dir();
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
