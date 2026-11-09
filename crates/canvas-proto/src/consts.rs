//! Ports, paths, names and limits shared across processes.

use std::path::PathBuf;
use std::time::Duration;

pub const APP_NAME: &str = "canvas-rgb";
pub const API_HOST: &str = "127.0.0.1";
pub const API_PORT: u16 = 9430;
/// Hosts the API accepts in the `Host` header (G9, DNS rebinding).
pub const API_ALLOWED_HOSTS: &[&str] = &["127.0.0.1:9430", "localhost:9430"];
/// G9: the only `Origin` values the API accepts (the Tauri editor's WebView2 origin).
/// Any other `Origin` gets 403. The API answers CORS preflight for these only.
pub const API_ALLOWED_ORIGINS: &[&str] = &["http://tauri.localhost", "https://tauri.localhost", "tauri://localhost"];
/// The preview WebSocket cannot carry headers from a browser; it takes `?token=`.
pub const PREVIEW_TOKEN_QUERY: &str = "token";
pub const OPENRGB_HOST: &str = "127.0.0.1";
pub const OPENRGB_PORT: u16 = 6742;
/// OpenRGB SDK client name canvas-rgb registers with.
pub const OPENRGB_CLIENT_NAME: &str = "canvas-rgb";

pub const FPS_MIN: u32 = 10;
pub const FPS_MAX: u32 = 60;
pub const FPS_DEFAULT: u32 = 30;
pub const PREVIEW_MAX_FPS: u32 = 15;
/// The largest user HTML effect `POST /effects/import` accepts, in bytes.
pub const EFFECT_IMPORT_MAX_BYTES: usize = 1024 * 1024;
/// How often the editor sends its draft to `PUT /effects/draft` while the draft is on the
/// lights. A beat is a lease renewal: a PUT at least this often keeps the draft running.
pub const DRAFT_HEARTBEAT: Duration = Duration::from_secs(5);
/// How long a draft keeps the lights after the last PUT. Three missed beats: the draft ends
/// and the previous effect comes back, so a closed, crashed or throttled editor can leave
/// the lights on a draft for at most this long.
pub const DRAFT_LEASE: Duration = Duration::from_secs(15);
/// Issue #166: an audio test lease ends this long after its last renewal (each poll renews it).
pub const AUDIO_TEST_LEASE: Duration = Duration::from_secs(3);
/// Issue #166: how often the editor polls the audio test (10 Hz).
pub const AUDIO_TEST_POLL_MS: u64 = 100;
/// Issue #166 / #188: the dB floor for audio levels; a level at or below it reads 0.
pub const AUDIO_DB_FLOOR: f32 = -60.0;
/// The id prefix of a draft effect (`draft:` + its source hash). A draft is never a saved
/// effect: no route that saves an effect or a layout stores one, and `find_schema` has no id
/// that starts with this.
pub const DRAFT_ID_PREFIX: &str = "draft:";
/// Screen Ambience capture rate cap.
pub const SCREEN_CAPTURE_MAX_FPS: u32 = 30;

pub const SERVICE_NAME: &str = "canvas-rgb-svc";
pub const SERVICE_DISPLAY_NAME: &str = "canvas-rgb fan and lighting service";
pub const DAEMON_EXE: &str = "canvas-rgbd.exe";
pub const TRAY_EXE: &str = "canvas-rgb.exe";
/// The CLI binary. Installed with a `canvas-rgb.cmd` shim on PATH, since the tray owns
/// the name `canvas-rgb.exe`.
pub const CLI_EXE: &str = "canvas-rgb-cli.exe";
pub const SERVICE_EXE: &str = "canvas-rgb-svc.exe";

/// G3: the service restarts a crashed daemon at most this many times in the window.
pub const DAEMON_RESTART_LIMIT: u32 = 3;
pub const DAEMON_RESTART_WINDOW_S: u64 = 300;
pub const HEARTBEAT_INTERVAL_MS: u64 = 1000;

/// G8 log retention.
pub const LOG_KEEP_DAYS: u32 = 7;
pub const LOG_MAX_BYTES_PER_DAY: u64 = 10 * 1024 * 1024;

pub const TOKEN_FILE: &str = "api-token";
/// G9: 256-bit token, stored as 64 lowercase hex characters.
pub const TOKEN_BYTES: usize = 32;

/// Processes and services that mean SignalRGB is running (conflict banner).
pub const SIGNALRGB_PROCESS: &str = "SignalRgb.exe";
/// The service name on this PC (`Get-Service SignalRgb.Service`; the exe is SignalRgbService.exe).
pub const SIGNALRGB_SERVICE: &str = "SignalRgb.Service";

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `%APPDATA%\canvas-rgb\`: config, layouts, zones, effects, api-token, backups.
pub fn user_config_dir() -> PathBuf {
    env_dir("APPDATA").join(APP_NAME)
}

/// `%LOCALAPPDATA%\canvas-rgb\logs\`: daemon and tray logs (G8).
pub fn user_log_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join(APP_NAME).join("logs")
}

/// `%ProgramData%\canvas-rgb\`: fans.toml, OpenRGB config dir, service state.
pub fn machine_data_dir() -> PathBuf {
    env_dir("ProgramData").join(APP_NAME)
}

/// `%ProgramData%\canvas-rgb\logs\`: service logs (G8).
pub fn machine_log_dir() -> PathBuf {
    machine_data_dir().join("logs")
}

/// `%ProgramData%\canvas-rgb\updates\`: SYSTEM and Administrators only. The service stages an
/// installer here before it checks the signature and runs it (#43).
pub fn update_staging_dir() -> PathBuf {
    machine_data_dir().join("updates")
}

/// The release signing key (minisign public key, as `tauri signer generate` writes it). The
/// private half is the `UPDATE_SIGNING_KEY` Actions secret. The tray's updater and the service's
/// install path (#43) both trust this key and only this key.
pub const UPDATE_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVFMzExRTFDOTg2QjI2MTMKUldRVEptdVlIQjR4N2hFYUs3Q1YvdFliZlA3NzdQVGRXakU4SWtTaUFPLzAwcndCYWx1ZlZxaTcK";

/// The Inno Setup switch that marks a run started by the service as SYSTEM (#43). Setup then
/// skips every step that runs as the person who started it (the per-user script and the tray
/// relaunch): the service does those as the console user once setup ends.
pub const SETUP_SERVICE_UPDATE_SWITCH: &str = "/SERVICEUPDATE";

/// The switches the service starts setup with. The log path is added after them.
pub const SERVICE_SETUP_ARGS: &str = "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE /SERVICEUPDATE";

/// `%ProgramData%\canvas-rgb\openrgb\`: the `--config` dir the OpenRGB server runs with.
pub fn openrgb_config_dir() -> PathBuf {
    machine_data_dir().join("openrgb")
}

/// `%APPDATA%\canvas-rgb\effects\`: user HTML effects (`*.html`, id = file stem).
pub fn user_effects_dir() -> PathBuf {
    user_config_dir().join("effects")
}

pub fn token_path() -> PathBuf {
    user_config_dir().join(TOKEN_FILE)
}

pub fn api_base_url() -> String {
    format!("http://{API_HOST}:{}", api_port())
}

/// Debug builds only: a test API port for a daemon on the fakes (#199), so it never collides
/// with the owner's real daemon on [`API_PORT`]. Release builds ignore it.
pub const TEST_API_PORT_ENV: &str = "CANVAS_RGB_TEST_API_PORT";

/// The test port from [`TEST_API_PORT_ENV`], in debug builds only, when it is a valid non-zero
/// port other than [`API_PORT`]. Always `None` in a release build.
pub fn debug_test_port() -> Option<u16> {
    #[cfg(debug_assertions)]
    {
        parse_test_port(std::env::var(TEST_API_PORT_ENV).ok().as_deref())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

/// Reads a test port value: a non-zero `u16` other than [`API_PORT`].
pub fn parse_test_port(value: Option<&str>) -> Option<u16> {
    value
        .and_then(|v| v.trim().parse::<u16>().ok())
        .filter(|p| *p != 0 && *p != API_PORT)
}

/// The port clients connect to: [`API_PORT`], or the debug test port when one is set.
pub fn api_port() -> u16 {
    port_with(debug_test_port())
}

/// [`api_port`] for a given test port (pure, so both build kinds are testable).
pub fn port_with(test_port: Option<u16>) -> u16 {
    test_port.unwrap_or(API_PORT)
}

/// The Host-header guard (G9, DNS rebinding): [`API_ALLOWED_HOSTS`], plus `127.0.0.1:<p>` and
/// `localhost:<p>` for the debug test port only. A release build accepts 9430 only.
pub fn api_host_allowed(host: &str) -> bool {
    host_allowed_with(host, debug_test_port())
}

/// [`api_host_allowed`] for a given test port (pure, so both build kinds are testable).
pub fn host_allowed_with(host: &str, test_port: Option<u16>) -> bool {
    if API_ALLOWED_HOSTS.contains(&host) {
        return true;
    }
    match test_port {
        Some(p) => host == format!("127.0.0.1:{p}") || host == format!("localhost:{p}"),
        None => false,
    }
}

/// The experimental update channel's feed (#203).
pub const EXPERIMENTAL_FEED_URL: &str =
    "https://github.com/iAmChumby/canvas-rgb/releases/download/experimental/latest.json";


/// The 12 native effects, by stable id (spec, effects line 1; White from #50).
pub const EFFECT_IDS: [&str; 12] = [
    "solid",
    "white",
    "gradient",
    "rainbow_wave",
    "spiral",
    "breathing",
    "color_cycle",
    "aurora",
    "rain",
    "fire",
    "screen_ambience",
    "audio_spectrum",
];

#[cfg(test)]
mod api_port_tests {
    use super::*;

    #[test]
    fn without_a_test_port_only_9430_is_allowed() {
        assert_eq!(port_with(None), 9430);
        assert!(host_allowed_with("127.0.0.1:9430", None));
        assert!(host_allowed_with("localhost:9430", None));
        assert!(!host_allowed_with("127.0.0.1:19430", None));
        assert!(!host_allowed_with("evil.example:9430", None));
    }

    #[test]
    fn a_test_port_adds_only_its_own_loopback_hosts() {
        assert_eq!(port_with(Some(19430)), 19430);
        assert!(host_allowed_with("127.0.0.1:19430", Some(19430)));
        assert!(host_allowed_with("localhost:19430", Some(19430)));
        assert!(host_allowed_with("127.0.0.1:9430", Some(19430)));
        assert!(!host_allowed_with("127.0.0.1:19431", Some(19430)));
        assert!(!host_allowed_with("evil.example:19430", Some(19430)));
    }

    #[test]
    fn a_bad_test_port_value_is_ignored() {
        assert_eq!(parse_test_port(None), None);
        assert_eq!(parse_test_port(Some("")), None);
        assert_eq!(parse_test_port(Some("0")), None);
        assert_eq!(parse_test_port(Some("9430")), None);
        assert_eq!(parse_test_port(Some("70000")), None);
        assert_eq!(parse_test_port(Some("abc")), None);
        assert_eq!(parse_test_port(Some(" 19430 ")), Some(19430));
    }

    /// Runs in release test builds (the `rust-release` CI job): the env var is ignored.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_release_build_ignores_the_test_port() {
        std::env::set_var(TEST_API_PORT_ENV, "19430");
        assert_eq!(debug_test_port(), None);
        assert_eq!(api_port(), 9430);
        assert!(!api_host_allowed("127.0.0.1:19430"));
        std::env::remove_var(TEST_API_PORT_ENV);
    }
}
