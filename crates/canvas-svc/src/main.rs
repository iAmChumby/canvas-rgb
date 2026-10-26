//! canvas-rgb-svc.exe. With no arguments it runs under the SCM. Flags:
//! `--console` runs the same logic in the foreground (Ctrl+C restores BIOS and exits);
//! `--restore-bios` hands every header back to BIOS and exits (the uninstaller calls it);
//! `--confirm-channel-map <name>` records that the named channel map was checked against
//! LibreHardwareMonitor on this board, which unlocks fan writes.
//!
//! Every start secures the machine home (`%ProgramData%\canvas-rgb`) before anything is read,
//! logged, or launched.

use canvas_svc::secure::windows_acl;
use canvas_svc::{app, logging, paths};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let home = paths::machine_home();

    if args.iter().any(|a| a == "--restore-bios") {
        let _guard = logging::init(&paths::logs_dir(&home), true);
        return match app::restore_bios_production(&home) {
            Ok(count) => {
                tracing::info!(headers = count, "BIOS control restored");
                ExitCode::SUCCESS
            }
            Err(e) => {
                tracing::error!(error = %e, "could not restore BIOS control");
                ExitCode::FAILURE
            }
        };
    }

    if let Some(pos) = args.iter().position(|a| a == "--confirm-channel-map") {
        let _guard = logging::init(&paths::logs_dir(&home), true);
        let Some(name) = args.get(pos + 1) else {
            eprintln!("canvas-rgb-svc: --confirm-channel-map needs a map name, e.g. msi_alt1");
            return ExitCode::from(2);
        };
        let boot = app::secure_boot(&home, windows_acl());
        return match app::confirm_channel_map(&boot, name) {
            Ok(()) => {
                tracing::info!(map = %name, "channel map confirmed in chip.toml; fan writes are now allowed");
                ExitCode::SUCCESS
            }
            Err(e) => {
                tracing::error!(error = %e, "channel map not confirmed");
                ExitCode::FAILURE
            }
        };
    }

    let console = args.iter().any(|a| a == "--console");
    // Secure the home before the log directory is created inside it.
    let boot = app::secure_boot(&home, windows_acl());
    let _guard = match logging::init(&paths::logs_dir(&home), console || !boot.secured()) {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("canvas-rgb-svc: cannot open the log: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(error) = boot.secure_error() {
        tracing::error!(error, "machine home is not secured; the service will hold every header on BIOS");
    }
    install_panic_hook(home.clone());

    let result = if console {
        run_console(boot)
    } else {
        app::stash_boot(boot);
        canvas_svc::winsvc::run_under_scm().map_err(|e| std::io::Error::other(e.to_string()))
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "service exited with an error");
            ExitCode::FAILURE
        }
    }
}

fn run_console(boot: app::Boot) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(app::run_console(boot))
}

/// A panic must not leave headers in manual mode: restore BIOS from the saved state first.
fn install_panic_hook(home: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "service panicked; restoring BIOS control");
        if let Err(e) = app::restore_bios_production(&home) {
            tracing::error!(error = %e, "panic path could not restore BIOS control");
        }
        default(info);
    }));
}
