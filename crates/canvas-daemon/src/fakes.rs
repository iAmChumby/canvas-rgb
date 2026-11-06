//! Debug-build-only fakes mode (#199): a test daemon that talks only to fakes.
//!
//! With `--fakes` (debug builds only) the daemon:
//!
//! - binds the spine's test API port ([`canvas_proto::consts::api_port`], set by
//!   `CANVAS_RGB_TEST_API_PORT`), never a config port that could be 9430;
//! - requires `CANVAS_RGB_HOME` to point at a folder that is not the real
//!   `%APPDATA%\canvas-rgb` (and not the real `%ProgramData%\canvas-rgb`), and
//!   refuses to start otherwise;
//! - never opens the service pipe `\\.\pipe\canvas-rgb-svc`: fans come from an
//!   in-process [`FakePipeLink`] / [`fake_fans_status`];
//! - never launches or connects to the real OpenRGB on 6742: the adapter talks
//!   to the fake OpenRGB server on [`FAKE_OPENRGB_PORT`] (or
//!   `CANVAS_RGB_FAKE_OPENRGB_PORT`);
//! - never opens HID devices (mouse, controller, ROYUAN keyboard, CHIONE
//!   display): the peripheral and display adapters are empty/`NoDisplays` and
//!   the composite's keyboard is disabled;
//! - never captures the screen or audio, never runs the power watcher, the
//!   SignalRGB conflict check, or the first run's SignalRGB reads;
//! - touches no path outside its home: every path already follows
//!   `CANVAS_RGB_HOME` (see [`crate::paths`]), which this mode requires.
//!
//! NVML is never loaded by the daemon itself: it lives in `canvas-svc`
//! (`crates/canvas-svc/src/temps.rs`), reached only through the service pipe,
//! which fakes mode never opens. Gating the pipe gates NVML.
//!
//! A release build rejects `--fakes` (see [`parse_fakes_mode`]): the flag is
//! not known to a release daemon at all.

use std::path::{Path, PathBuf};

use canvas_proto::fans::{FanHeaderStatus, FanMode, FansStatus, TempReading, TempSource};
use canvas_proto::pipe::{PipeRequest, PipeResponse};

/// The flag that turns fakes mode on. Debug builds only.
pub const FAKES_FLAG: &str = "--fakes";

/// The environment variable that names the fake OpenRGB port.
pub const FAKE_OPENRGB_PORT_ENV: &str = "CANVAS_RGB_FAKE_OPENRGB_PORT";

/// The default fake OpenRGB port: the real 6742 plus 10000, so it can never be
/// the real OpenRGB port.
pub const FAKE_OPENRGB_PORT: u16 = 16742;

/// Parses the fakes flag out of the process arguments.
///
/// Debug builds: `Ok(true)` when `--fakes` is present. Release builds: the
/// flag is unknown, so its presence is an error and the daemon must refuse to
/// start.
pub fn parse_fakes_mode(args: &[String]) -> Result<bool, String> {
    let requested = args.iter().any(|a| a == FAKES_FLAG);
    #[cfg(debug_assertions)]
    {
        Ok(requested)
    }
    #[cfg(not(debug_assertions))]
    {
        if requested {
            Err(format!("unknown argument {FAKES_FLAG}: fakes mode is a debug-build-only tool"))
        } else {
            Ok(false)
        }
    }
}

/// Whether fakes mode may run in this build at all.
pub fn fakes_supported() -> bool {
    cfg!(debug_assertions)
}

/// The fake OpenRGB port from an environment value, or [`FAKE_OPENRGB_PORT`]
/// when it is missing/invalid. Pure, so both build kinds are testable. The
/// real OpenRGB port (6742), the real API port (9430) and the test API port
/// (19430) are never accepted: a fake must never sit on a real port.
pub fn fake_openrgb_port_from(value: Option<&str>) -> u16 {
    value
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|p| {
            *p != 0
                && *p != canvas_proto::consts::OPENRGB_PORT
                && *p != canvas_proto::consts::API_PORT
                && *p != 19430
        })
        .unwrap_or(FAKE_OPENRGB_PORT)
}

/// The fake OpenRGB port for this process.
pub fn fake_openrgb_port() -> u16 {
    fake_openrgb_port_from(std::env::var(FAKE_OPENRGB_PORT_ENV).ok().as_deref())
}

/// Normalises a path for comparison: absolute, with `.`/`..` collapsed
/// textually and a trailing slash removed. No filesystem access, so a missing
/// folder still compares.
pub fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(path)
    };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when `home` names the real user data folder (or sits inside it is not
/// relevant: equality after normalisation is the refusal, plus the machine
/// folder). Compared case-insensitively on Windows paths, as the filesystem
/// is case-insensitive there.
pub fn is_real_home(home: &Path, real_home: &Path, machine_home: &Path) -> bool {
    let home = normalize(home);
    let same = |a: &Path, b: &Path| {
        let (a, b) = (normalize(a), normalize(b));
        a == b
            || a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    };
    same(&home, real_home) || same(&home, machine_home)
}

/// Validates the home a fakes daemon runs with: `CANVAS_RGB_HOME` must be set
/// to a non-empty folder that is not the real user or machine data folder.
/// Returns the home to use. Pure over its inputs, so it is unit-testable.
pub fn validate_fakes_home(
    home_env: Option<&Path>,
    real_home: &Path,
    machine_home: &Path,
) -> Result<PathBuf, String> {
    let Some(home) = home_env else {
        return Err(
            "fakes mode requires CANVAS_RGB_HOME to be set to a test folder (not the real %APPDATA%\\canvas-rgb)".into(),
        );
    };
    if home.as_os_str().is_empty() {
        return Err("fakes mode requires CANVAS_RGB_HOME to be set to a test folder".into());
    }
    if is_real_home(home, real_home, machine_home) {
        return Err(format!(
            "fakes mode refuses to run with the real data folder {}: set CANVAS_RGB_HOME to a test folder",
            home.display()
        ));
    }
    Ok(home.to_path_buf())
}

/// Validates this process's home for fakes mode, reading the real folders
/// from the spine.
pub fn validate_process_home() -> Result<PathBuf, String> {
    let home_env = std::env::var_os(crate::paths::HOME_ENV).map(PathBuf::from);
    validate_fakes_home(
        home_env.as_deref(),
        &canvas_proto::consts::user_config_dir(),
        &canvas_proto::consts::machine_data_dir(),
    )
}

// --- Gates -----------------------------------------------------------------
//
// Each predicate answers "may fakes mode touch the real thing?". Every one is
// `false` in fakes mode and `true` otherwise, so a unit test proves the gate
// and `main.rs` has no other place to decide.

/// May the daemon open the real service pipe?
pub fn real_pipe_enabled(fakes: bool) -> bool {
    !fakes
}

/// May the daemon launch the real OpenRGB server (or ask the service whether
/// it owns it)?
pub fn real_openrgb_launch_enabled(fakes: bool) -> bool {
    !fakes
}

/// May the daemon enumerate or open HID peripherals (mouse, controller)?
pub fn real_peripherals_enabled(fakes: bool) -> bool {
    !fakes
}

/// May the composite open the ROYUAN keyboard over HID?
pub fn real_keyboard_enabled(fakes: bool) -> bool {
    !fakes
}

/// May the daemon enumerate or open the CHIONE numeric display over HID?
pub fn real_display_enabled(fakes: bool) -> bool {
    !fakes
}

/// May the daemon capture the screen (Desktop Duplication) or audio (WASAPI)?
pub fn real_capture_enabled(fakes: bool) -> bool {
    !fakes
}

/// May the daemon run the power watcher (window, session/display notifications)?
pub fn real_power_watcher_enabled(fakes: bool) -> bool {
    !fakes
}

/// May the daemon run the SignalRGB conflict check (process/service snapshots)?
pub fn real_conflict_check_enabled(fakes: bool) -> bool {
    !fakes
}

/// May the first run read SignalRGB's registry and logs?
pub fn real_first_run_enabled(fakes: bool) -> bool {
    !fakes
}

// --- Fake fans --------------------------------------------------------------

/// The fake fan status: a fake NCT6687D-R with two headers and CPU/GPU/board
/// temperatures, so the Fans view shows curves and readings. The GPU
/// temperature is a fixed fake value, never an NVML read.
pub fn fake_fans_status() -> FansStatus {
    let header = |id: &str, label: &str, rpm: u32, pwm: u8, mode: FanMode| FanHeaderStatus {
        id: id.to_string(),
        label: label.to_string(),
        rpm: Some(rpm),
        pwm_percent: Some(pwm),
        mode,
        on_bios: false,
        override_reason: None,
        available: true,
    };
    FansStatus {
        chip: Some("NCT6687D-R @ 0xA20 (fake)".into()),
        hardware_watchdog: Some(false),
        headers: vec![
            header(
                "cpu_fan",
                "CPU fan",
                1200,
                40,
                FanMode::Curve {
                    source: TempSource::Cpu,
                    points: vec![
                        canvas_proto::fans::CurvePoint { temp_c: 30.0, percent: 30 },
                        canvas_proto::fans::CurvePoint { temp_c: 70.0, percent: 100 },
                    ],
                },
            ),
            header("sys_fan1", "System fan 1", 900, 50, FanMode::Bios),
        ],
        temps: vec![
            TempReading { source: TempSource::Cpu, label: "CPU (fake)".into(), celsius: Some(42.5) },
            TempReading { source: TempSource::Gpu, label: "GPU (fake)".into(), celsius: Some(55.0) },
            TempReading {
                source: TempSource::Board("board_temp1".into()),
                label: "Board (fake)".into(),
                celsius: Some(33.0),
            },
        ],
        active_profile: "Default".into(),
        profiles: vec!["Default".into(), "Quiet".into()],
        floor_percent: canvas_proto::fans::DEFAULT_FLOOR_PERCENT,
    }
}

/// An in-process fake of the service pipe: answers `Hello`, `FanStatus` and
/// `OpenRgbState` from the fakes, and `Ok` for everything else. Never opens
/// `\\.\pipe\canvas-rgb-svc`.
pub struct FakePipeLink;

#[async_trait::async_trait]
impl crate::pipe_client::PipeLink for FakePipeLink {
    async fn exchange(&self, request: &PipeRequest) -> Result<PipeResponse, crate::pipe_client::PipeError> {
        match request {
            PipeRequest::Hello { .. } => Ok(PipeResponse::Hello {
                protocol: canvas_proto::pipe::PROTOCOL_VERSION,
                service_version: "fake".into(),
            }),
            PipeRequest::FanStatus => Ok(PipeResponse::FanStatus { status: fake_fans_status() }),
            PipeRequest::OpenRgbState => Ok(PipeResponse::OpenRgb {
                state: canvas_proto::pipe::OpenRgbServerState::Running,
                port: fake_openrgb_port(),
            }),
            _ => Ok(PipeResponse::Ok),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn fakes_mode_skips_every_real_touchpoint() {
        // Each gate: fakes mode must not touch the real thing; normal mode must.
        assert!(!real_pipe_enabled(true), "fakes mode never opens the service pipe");
        assert!(real_pipe_enabled(false));
        assert!(!real_openrgb_launch_enabled(true), "fakes mode never launches the real OpenRGB");
        assert!(real_openrgb_launch_enabled(false));
        assert!(!real_peripherals_enabled(true), "fakes mode never opens HID peripherals");
        assert!(real_peripherals_enabled(false));
        assert!(!real_keyboard_enabled(true), "fakes mode never opens the ROYUAN keyboard");
        assert!(real_keyboard_enabled(false));
        assert!(!real_display_enabled(true), "fakes mode never opens the CHIONE display");
        assert!(real_display_enabled(false));
        assert!(!real_capture_enabled(true), "fakes mode never captures screen or audio");
        assert!(real_capture_enabled(false));
        assert!(!real_power_watcher_enabled(true), "fakes mode never runs the power watcher");
        assert!(real_power_watcher_enabled(false));
        assert!(!real_conflict_check_enabled(true), "fakes mode never checks for SignalRGB");
        assert!(real_conflict_check_enabled(false));
        assert!(!real_first_run_enabled(true), "fakes mode never reads SignalRGB's registry/logs");
        assert!(real_first_run_enabled(false));
    }

    #[test]
    fn the_fakes_flag_is_parsed_and_unknown_in_release() {
        let args = vec!["canvas-rgbd".to_string(), "--fakes".to_string()];
        let parsed = parse_fakes_mode(&args);
        #[cfg(debug_assertions)]
        assert_eq!(parsed, Ok(true));
        #[cfg(not(debug_assertions))]
        assert!(parsed.is_err(), "a release build must reject --fakes");
        assert_eq!(parse_fakes_mode(&["canvas-rgbd".to_string()]), Ok(false));
    }

    #[test]
    fn fakes_support_matches_the_build_kind() {
        assert_eq!(fakes_supported(), cfg!(debug_assertions));
    }

    #[test]
    fn a_fake_openrgb_port_is_never_a_real_port() {
        assert_eq!(fake_openrgb_port_from(None), FAKE_OPENRGB_PORT);
        assert_eq!(fake_openrgb_port_from(Some("16742")), 16742);
        assert_eq!(fake_openrgb_port_from(Some("6742")), FAKE_OPENRGB_PORT, "never the real OpenRGB port");
        assert_eq!(fake_openrgb_port_from(Some("9430")), FAKE_OPENRGB_PORT, "never the real API port");
        assert_eq!(fake_openrgb_port_from(Some("19430")), FAKE_OPENRGB_PORT, "never the test API port");
        assert_eq!(fake_openrgb_port_from(Some("0")), FAKE_OPENRGB_PORT);
        assert_eq!(fake_openrgb_port_from(Some("abc")), FAKE_OPENRGB_PORT);
        assert_ne!(FAKE_OPENRGB_PORT, canvas_proto::consts::OPENRGB_PORT);
    }

    #[test]
    fn the_real_home_is_refused_and_a_test_home_is_accepted() {
        let real = PathBuf::from(r"C:\test-user\AppData\Roaming\canvas-rgb");
        let machine = PathBuf::from(r"C:\ProgramData\canvas-rgb");
        assert!(validate_fakes_home(None, &real, &machine).is_err(), "no home env: refuse");
        assert!(validate_fakes_home(Some(Path::new("")), &real, &machine).is_err(), "empty home: refuse");
        assert!(validate_fakes_home(Some(&real), &real, &machine).is_err(), "the real home: refuse");
        assert!(
            validate_fakes_home(Some(Path::new(r"C:\test-user\AppData\Roaming\canvas-rgb\")), &real, &machine)
                .is_err(),
            "the real home with a trailing slash: refuse"
        );
        assert!(validate_fakes_home(Some(&machine), &real, &machine).is_err(), "the real machine dir: refuse");
        let test = PathBuf::from(r"C:\work\canvas-rgb\.build\fake-home");
        assert_eq!(validate_fakes_home(Some(&test), &real, &machine), Ok(test));
    }

    #[test]
    fn path_normalisation_collapses_dots() {
        let a = normalize(Path::new(r"C:\a\b\..\c"));
        let b = normalize(Path::new(r"C:\a\c"));
        assert_eq!(a, b);
        assert!(is_real_home(Path::new(r"C:\a\c"), Path::new(r"C:\a\c"), Path::new(r"C:\m")));
        assert!(!is_real_home(Path::new(r"C:\a\c"), Path::new(r"C:\a\d"), Path::new(r"C:\m")));
    }

    #[test]
    fn the_fake_fans_show_curves_and_readings() {
        let status = fake_fans_status();
        assert!(status.chip.as_deref().unwrap().contains("fake"));
        assert!(!status.headers.is_empty());
        assert!(status.headers.iter().any(|h| matches!(h.mode, FanMode::Curve { .. })));
        assert!(status.headers.iter().all(|h| h.rpm.is_some()));
        assert!(status.temps.iter().all(|t| t.celsius.is_some()));
        assert_eq!(status.profiles, vec!["Default", "Quiet"]);
    }

    #[tokio::test]
    async fn the_fake_pipe_answers_without_touching_the_real_pipe() {
        use crate::pipe_client::PipeLink;
        let link = FakePipeLink;
        let hello = link
            .exchange(&PipeRequest::Hello { protocol: 1, daemon_pid: 1 })
            .await
            .expect("the fake pipe answers hello");
        assert!(matches!(hello, PipeResponse::Hello { .. }));
        let fans = link.exchange(&PipeRequest::FanStatus).await.expect("the fake pipe answers fans");
        match fans {
            PipeResponse::FanStatus { status } => assert!(!status.headers.is_empty()),
            other => panic!("expected fan status, got {other:?}"),
        }
        let openrgb = link.exchange(&PipeRequest::OpenRgbState).await.expect("the fake pipe answers openrgb");
        match openrgb {
            PipeResponse::OpenRgb { port, .. } => assert_eq!(port, fake_openrgb_port()),
            other => panic!("expected openrgb state, got {other:?}"),
        }
    }
}
