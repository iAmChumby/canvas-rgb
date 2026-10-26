//! Ports, paths, names and limits shared across processes.

use std::path::PathBuf;

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
pub const SIGNALRGB_SERVICE: &str = "SignalRgbService";

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

/// `%ProgramData%\canvas-rgb\openrgb\`: the `--config` dir the OpenRGB server runs with.
pub fn openrgb_config_dir() -> PathBuf {
    machine_data_dir().join("openrgb")
}

pub fn token_path() -> PathBuf {
    user_config_dir().join(TOKEN_FILE)
}

pub fn api_base_url() -> String {
    format!("http://{API_HOST}:{API_PORT}")
}

/// The 12 native effects, by stable id (spec, effects line 1).
pub const EFFECT_IDS: [&str; 12] = [
    "solid",
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
    "poison",
];
