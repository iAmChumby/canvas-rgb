//! On-disk config schemas. Every file carries `schema_version`. Writes go to a temp file
//! in the same directory and are renamed over the target. Before a migration and on every
//! daemon start, a snapshot goes to `backups\` (last 10 kept, G10).
//!
//! User files live in `%APPDATA%\canvas-rgb\`. `fans.toml` lives in
//! `%ProgramData%\canvas-rgb\` and is written only by the service, on the daemon's
//! request over the pipe.

use crate::api::Power;
use crate::color::Rgb8;
use crate::fans::{FanProfile, DEFAULT_FLOOR_PERCENT};
use crate::layout::Layout;
use crate::props::{Preset, Props};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

pub const CONFIG_FILE: &str = "config.toml";
pub const LAYOUTS_FILE: &str = "layouts.json";
pub const ZONES_FILE: &str = "zones.json";
pub const EFFECTS_FILE: &str = "effects.json";
pub const FANS_FILE: &str = "fans.toml";
pub const BACKUPS_DIR: &str = "backups";
pub const BACKUPS_KEPT: usize = 10;

pub const CONFIG_SCHEMA: u32 = 1;
pub const LAYOUTS_SCHEMA: u32 = 1;
pub const ZONES_SCHEMA: u32 = 1;
pub const EFFECTS_SCHEMA: u32 = 1;
pub const FANS_SCHEMA: u32 = 1;
pub const EXPORT_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum OnQuit {
    /// G3 default.
    #[default]
    Black,
    LeaveLastFrame,
}

/// Settings the editor's Settings page edits (`GET/PATCH /api/v1/settings`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Settings {
    /// 10..=60
    pub fps: u32,
    /// G2: off by default.
    pub black_on_lock: bool,
    /// G3.
    pub on_quit: OnQuit,
    /// G5: the service holds this until the daemon connects. `None` = black.
    pub boot_color: Option<Rgb8>,
    pub brightness: u8,
    pub power: Power,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fps: 30,
            black_on_lock: false,
            on_quit: OnQuit::Black,
            boot_color: None,
            brightness: 100,
            power: Power::Running,
        }
    }
}

/// `config.toml`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigFile {
    pub schema_version: u32,
    pub api_port: u16,
    pub openrgb_port: u16,
    /// G7: false until first run has auto-placed devices and activated the first-run effect.
    pub first_run_done: bool,
    /// G7: the "size your ARGB headers" panel was dismissed.
    pub zone_panel_dismissed: bool,
    pub settings: Settings,
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA,
            api_port: crate::consts::API_PORT,
            openrgb_port: crate::consts::OPENRGB_PORT,
            first_run_done: false,
            zone_panel_dismissed: false,
            settings: Settings::default(),
        }
    }
}

/// `layouts.json`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LayoutsFile {
    pub schema_version: u32,
    pub active: String,
    pub layouts: Vec<Layout>,
}

/// `zones.json`: LED counts by device fingerprint, then zone index. Re-applied on every
/// OpenRGB connect, because OpenRGB does not persist SDK resizes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ZonesFile {
    pub schema_version: u32,
    pub sizes: BTreeMap<String, BTreeMap<u32, u32>>,
}

/// `effects.json`
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectsFile {
    pub schema_version: u32,
    /// Last props used per effect id.
    pub saved: BTreeMap<String, Props>,
    pub presets: BTreeMap<String, Vec<Preset>>,
    pub favorites: Vec<String>,
}

/// `fans.toml` (in `%ProgramData%\canvas-rgb\`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FansFile {
    pub schema_version: u32,
    pub floor_percent: u8,
    pub active_profile: String,
    pub profiles: Vec<FanProfile>,
    /// G5 boot color, mirrored here so the service has it with nobody logged in.
    pub boot_color: Option<Rgb8>,
}

impl Default for FansFile {
    fn default() -> Self {
        Self {
            schema_version: FANS_SCHEMA,
            floor_percent: DEFAULT_FLOOR_PERCENT,
            active_profile: "Default".into(),
            profiles: vec![
                FanProfile { name: "Default".into(), headers: BTreeMap::new() },
                FanProfile { name: "Quiet".into(), headers: BTreeMap::new() },
            ],
            boot_color: None,
        }
    }
}

/// `canvas-rgb config export/import` and the editor's Export/Import (G10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExportBundle {
    pub schema_version: u32,
    pub exported_at: String,
    pub app_version: String,
    pub settings: Settings,
    pub layouts: LayoutsFile,
    pub zones: ZonesFile,
    pub effects: EffectsFile,
    pub fans: FansFile,
    /// The numeric displays' configs (`displays.json`). Absent in bundles from before displays;
    /// import replaces the saved display configs with these, like the other files.
    #[serde(default)]
    pub displays: Vec<crate::display::DisplayConfig>,
}
