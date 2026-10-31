//! Assembles the service and runs it.
//!
//! Order (each step only after the previous one):
//! 1. `secure_boot`: the machine home gets its protected owner and DACL. Nothing else touches it first.
//! 2. `build_core`: reads config and opens the chip only when the home is secured; launches
//!    OpenRGB only when secured and when the exe is inside this install directory.
//! 3. `run`: claims the pipe, starts the 1 Hz loop, and restores BIOS on stop.
//!
//! `Boot` can only be made by `secure_boot`, so a caller cannot reach `build_core` without the
//! secure step having run.

use crate::chip::FanChip;
use crate::daemon::{DaemonSupervisor, SessionLauncher};
use crate::fans::FanController;
use crate::hardware;
use crate::openrgb::{AdapterFactory, OpenRgbSupervisor};
use crate::paths;
use crate::pipe_server;
use crate::process::{ProcessSpawner, StdSpawner};
use crate::secure::{windows_acl, SecureDir};
use crate::service::{Core, PowerEvent};
use crate::store::{self, ChipState};
use crate::update_install::{Finish, UpdateInstaller};
use canvas_adapters::openrgb::OpenRgbAdapter;
use canvas_proto::config::FansFile;
use canvas_proto::consts::{DAEMON_EXE, OPENRGB_HOST, OPENRGB_PORT};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::windows::named_pipe::NamedPipeServer;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::Mutex as AsyncMutex;

/// The result of securing the machine home. Only [`secure_boot`] makes one. It keeps the
/// [`SecureDir`] so that every file read later can be checked for its owner.
pub struct Boot {
    home: PathBuf,
    secure_error: Option<String>,
    trust: Box<dyn SecureDir>,
}

impl Boot {
    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn secured(&self) -> bool {
        self.secure_error.is_none()
    }

    pub fn secure_error(&self) -> Option<&str> {
        self.secure_error.as_deref()
    }

    /// True when `path` may be read: the home is secured and the path's owner is SYSTEM or
    /// Administrators, and it is not a reparse point.
    pub fn trusted(&self, path: &Path) -> bool {
        self.secured() && self.trust.trusted(path)
    }
}

/// Step 1: secures the machine home. A failure is recorded, not panicked on.
pub fn secure_boot(home: &Path, mut secure: Box<dyn SecureDir>) -> Boot {
    let secure_error = secure.secure(home).err().map(|e| format!("{}: {e}", home.display()));
    Boot { home: home.to_path_buf(), secure_error, trust: secure }
}

/// Reads permission for one file: `false` (treated as missing) when it is not trusted.
fn trusted_file(boot: &Boot, path: &Path) -> bool {
    if boot.trusted(path) {
        return true;
    }
    if path.exists() {
        tracing::error!(path = %path.display(), "file is not owned by SYSTEM or Administrators, or is a reparse point; treating it as missing");
    }
    false
}

/// Injected process and session seams, plus where the install directory is.
pub struct Deps {
    pub spawner: Box<dyn ProcessSpawner>,
    pub launcher: Box<dyn SessionLauncher>,
    pub adapter: AdapterFactory,
    pub install_dir: PathBuf,
    /// The update installer. `None` builds the production one (when the home is secured).
    pub updates: Option<Arc<UpdateInstaller>>,
}

/// The production dependencies: real processes, the user session launcher, and the loopback
/// OpenRGB adapter. Install directory is the service exe's directory.
pub fn production_deps() -> Deps {
    Deps {
        spawner: Box::new(StdSpawner),
        launcher: Box::new(crate::daemon::WtsLauncher),
        adapter: Box::new(|| Box::new(OpenRgbAdapter::new(OPENRGB_HOST, OPENRGB_PORT))),
        install_dir: install_dir(),
        updates: None,
    }
}

/// The directory the service exe lives in. The daemon and `openrgb\OpenRGB.exe` sit beside it.
pub fn install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Step 2: builds the core. Reads and spawns happen only when the boot is secured.
pub fn build_core(boot: &Boot, deps: Deps) -> Core {
    let home = boot.home();
    let now = Instant::now();
    let (chip, chip_error, temps, file, confirmed) = if boot.secured() {
        let hardware = hardware::open();
        let chip_path = paths::chip_path(home);
        let chip_state = if trusted_file(boot, &chip_path) {
            match store::load_chip_state(&chip_path) {
                Ok(state) => state,
                Err(e) => {
                    tracing::error!(error = %e, "chip.toml is unreadable; the channel map is unconfirmed");
                    ChipState::unconfirmed()
                }
            }
        } else {
            ChipState::unconfirmed()
        };
        let fans_path = paths::fans_path(home);
        let file = if trusted_file(boot, &fans_path) {
            match store::load_fans(&fans_path) {
                Ok(Some(file)) => file,
                Ok(None) => FansFile::default(),
                Err(e) => {
                    tracing::error!(error = %e, "fans.toml is unreadable; starting from defaults");
                    FansFile::default()
                }
            }
        } else {
            FansFile::default()
        };
        (hardware.chip, hardware.chip_error, hardware.temps, file, chip_state.confirmed_map)
    } else {
        let reason = boot.secure_error().unwrap_or("unknown").to_string();
        tracing::error!(
            reason = %reason,
            "machine home could not be secured: no config read, no chip access, no OpenRGB; every header stays on BIOS"
        );
        let chip: Option<Box<dyn FanChip>> = None;
        (chip, Some(format!("machine home not secured: {reason}")), std::collections::BTreeMap::new(), FansFile::default(), None)
    };

    let mut fans = FanController::new(home.to_path_buf(), chip, chip_error, temps, file, confirmed);
    if boot.secured() {
        fans.set_state_trusted(trusted_file(boot, &paths::bios_state_path(home)));
        fans.startup(now);
    }

    let config_dir = paths::openrgb_config_dir(home);
    let mut openrgb = OpenRgbSupervisor::new(
        deps.install_dir.join("openrgb").join("OpenRGB.exe"),
        OPENRGB_PORT,
        config_dir.clone(),
        deps.install_dir.clone(),
        deps.spawner,
        deps.adapter,
    );
    if !boot.secured() {
        openrgb.disable("machine home not secured");
    } else if !boot.trusted(&config_dir) {
        openrgb.disable("the OpenRGB config directory is not owned by SYSTEM or Administrators");
    }

    let mut daemon = DaemonSupervisor::new(deps.install_dir.join(DAEMON_EXE), deps.launcher);
    daemon.start(now);
    let mut core = Core::new(fans, openrgb, daemon);
    if boot.secured() {
        core.updates = Some(deps.updates.unwrap_or_else(|| Arc::new(UpdateInstaller::production(home.to_path_buf(), deps.install_dir.clone()))));
    }
    core
}

/// Step 3: claims the pipe, then runs until `stop` completes, then restores BIOS. `power`
/// carries the SCM's suspend and resume notifications.
pub async fn run(
    boot: Boot,
    deps: Deps,
    stop: impl Future<Output = ()>,
    power: UnboundedReceiver<PowerEvent>,
) -> io::Result<()> {
    // The pipe is claimed before anything else, so a squatter fails the service loudly.
    let name = pipe_server::pipe_name();
    let first = pipe_server::create_instance(&name, true)?;
    let core = Arc::new(AsyncMutex::new(build_core(&boot, deps)));
    run_core(core, name, first, stop, power).await
}

/// Starts the thread that finishes a pending update (#43): the new service finishes an update once
/// setup has ended. On its own thread, so the service is up and answering while it waits. `None` when
/// the service has no installer (the home is not secured).
pub fn start_update_resume(updates: Option<Arc<UpdateInstaller>>) -> Option<std::thread::JoinHandle<Option<Finish>>> {
    let updates = updates?;
    match std::thread::Builder::new().name("update-resume".into()).spawn(move || updates.resume_pending()) {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::error!(error = %e, "could not start the pending-update thread");
            None
        }
    }
}

/// The part of [`run`] after the core is built: serves the pipe, resumes a pending update, runs the
/// 1 Hz loop, and restores BIOS on stop.
pub async fn run_core(
    core: Arc<AsyncMutex<Core>>,
    name: String,
    first: NamedPipeServer,
    stop: impl Future<Output = ()>,
    mut power: UnboundedReceiver<PowerEvent>,
) -> io::Result<()> {
    let pipe_core = Arc::clone(&core);
    tokio::spawn(async move {
        if let Err(e) = pipe_server::serve(pipe_core, name, first, pipe_server::win_peers()).await {
            tracing::error!(error = %e, "pipe server stopped");
        }
    });
    let updates = core.lock().await.updates.clone();
    start_update_resume(updates);
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = ticker.tick() => core.lock().await.tick(Instant::now()).await,
            Some(event) = power.recv() => {
                tracing::info!(?event, "power event");
                core.lock().await.on_power(event, Instant::now());
            }
            _ = &mut stop => break,
        }
    }
    core.lock().await.shutdown("service stop");
    tracing::info!("service stopped; BIOS restored on every header");
    Ok(())
}

static PENDING_BOOT: Mutex<Option<Boot>> = Mutex::new(None);

/// Hands the secured boot to the SCM entry point, which cannot take arguments.
pub fn stash_boot(boot: Boot) {
    *PENDING_BOOT.lock().unwrap_or_else(|p| p.into_inner()) = Some(boot);
}

pub fn take_boot() -> Option<Boot> {
    PENDING_BOOT.lock().unwrap_or_else(|p| p.into_inner()).take()
}

/// `--confirm-channel-map <name>`: records the confirmation in `chip.toml` when `name` is the
/// map that is detected now. Reads the chip only; writes no fan register.
pub fn confirm_channel_map(boot: &Boot, name: &str) -> Result<(), String> {
    if !boot.secured() {
        return Err(format!("refusing to write chip.toml: {}", boot.secure_error().unwrap_or("unsecured")));
    }
    let chip = hardware::open_chip()?;
    check_confirmation(chip.map_name(), name)?;
    store::save_chip_state(&paths::chip_path(boot.home()), &ChipState::confirmed(name)).map_err(|e| e.to_string())
}

/// The confirmation must name exactly the map that is detected.
pub fn check_confirmation(detected: &str, requested: &str) -> Result<(), String> {
    if detected == requested {
        Ok(())
    } else {
        Err(format!("detected channel map is {detected}, not {requested}; nothing saved"))
    }
}

/// `--restore-bios` and the panic hook. Secures the home first, then restores BIOS from the
/// saved state. If securing fails, the restore still runs: it only writes the fan-mode
/// register to auto, and never spawns or drives a fan.
pub fn restore_bios_now(home: &Path, secure: Box<dyn SecureDir>) -> io::Result<usize> {
    let boot = secure_boot(home, secure);
    if !boot.secured() {
        tracing::error!(error = ?boot.secure_error(), "restoring BIOS without a secured home (auto-only writes)");
    }
    let mut chip = hardware::open_chip().map_err(io::Error::other)?;
    if boot.secured() {
        FanController::restore_from_file(home, chip.as_mut())
    } else {
        // The home may be a junction: no file operation there, chip writes only.
        FanController::restore_chip_only(chip.as_mut())
    }
}

/// Production restore, with the real ACL step.
pub fn restore_bios_production(home: &Path) -> io::Result<usize> {
    restore_bios_now(home, windows_acl())
}

/// Used by the console runner (`--console`).
pub async fn run_console(boot: Boot) -> io::Result<()> {
    // The console runner gets no power events; the sender stays alive until the run ends.
    let (_power_tx, power_rx) = tokio::sync::mpsc::unbounded_channel();
    run(
        boot,
        production_deps(),
        async {
            let _ = tokio::signal::ctrl_c().await;
        },
        power_rx,
    )
    .await
}

/// Used by the SCM entry point.
pub async fn run_scm(
    boot: Boot,
    stop: impl Future<Output = ()>,
    power: UnboundedReceiver<PowerEvent>,
) -> io::Result<()> {
    run(boot, production_deps(), stop, power).await
}
