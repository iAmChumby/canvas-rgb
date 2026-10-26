//! Loopback HTTP API on 127.0.0.1:9430 under `/api/v1`.
//!
//! Auth (G9): every request needs `Authorization: Bearer <token>` except `GET /api/v1/status`.
//! A request with an `Origin` header not in `consts::API_ALLOWED_ORIGINS` gets 403 `forbidden_origin`; a `Host` that is not
//! `127.0.0.1:9430` or `localhost:9430` gets 403 `bad_host`.

use crate::device::DeviceInfo;
use crate::fans::{FanMode, FanProfile, FansStatus};
use crate::layout::{Layout, RenderScale, SamplingMode};
use crate::props::{InputNeeds, Preset, Props, PropertySchema};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod routes {
    pub const PREFIX: &str = "/api/v1";
    pub const STATUS: &str = "/api/v1/status";
    pub const OUTPUT: &str = "/api/v1/output";
    pub const RENDER: &str = "/api/v1/render";
    pub const DEVICES: &str = "/api/v1/devices";
    /// `PATCH` with [`super::PatchZone`].
    pub const DEVICE_ZONE: &str = "/api/v1/devices/{id}/zones/{zone}";
    /// `POST` with [`super::IdentifyRequest`].
    pub const DEVICE_IDENTIFY: &str = "/api/v1/devices/{id}/identify";
    pub const EFFECTS: &str = "/api/v1/effects";
    pub const EFFECT: &str = "/api/v1/effects/{id}";
    pub const EFFECT_ACTIVATE: &str = "/api/v1/effects/{id}/activate";
    pub const EFFECT_ACTIVE_PROPS: &str = "/api/v1/effects/active/props";
    /// `PUT` with `{ "favorite": bool }`.
    pub const EFFECT_FAVORITE: &str = "/api/v1/effects/{id}/favorite";
    pub const LAYOUTS: &str = "/api/v1/layouts";
    pub const LAYOUT: &str = "/api/v1/layouts/{id}";
    /// `PUT` with [`super::SetActiveLayout`].
    pub const LAYOUT_ACTIVE: &str = "/api/v1/layouts/active";
    pub const FANS: &str = "/api/v1/fans";
    pub const FAN_HEADER: &str = "/api/v1/fans/headers/{h}";
    pub const FAN_PROFILE: &str = "/api/v1/fans/profile";
    /// `PUT` a [`crate::fans::FanProfile`] to create or replace it.
    pub const FAN_PROFILES: &str = "/api/v1/fans/profiles/{name}";
    /// `GET` / `PUT` a [`crate::config::Settings`].
    pub const SETTINGS: &str = "/api/v1/settings";
    /// `POST`, no body: dismisses G7's "size your ARGB headers" panel.
    pub const DISMISS_ZONE_PANEL: &str = "/api/v1/first-run/dismiss-zone-panel";
    pub const CONFIG_EXPORT: &str = "/api/v1/config/export";
    pub const CONFIG_IMPORT: &str = "/api/v1/config/import";
    /// Diagnostics: current colors as the adapter reads them back from the hardware.
    pub const DIAG_READBACK: &str = "/api/v1/diag/readback";
    /// WebSocket. Binary messages: `u16 LE width, u16 LE height, width*height*3 RGB bytes`,
    /// at most [`crate::consts::PREVIEW_MAX_FPS`].
    pub const PREVIEW: &str = "/api/v1/preview";
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiError {
    /// One of [`error_codes`].
    pub error: String,
    pub message: String,
}

pub mod error_codes {
    /// 401
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// 403
    pub const FORBIDDEN_ORIGIN: &str = "forbidden_origin";
    /// 403
    pub const BAD_HOST: &str = "bad_host";
    /// 404
    pub const NOT_FOUND: &str = "not_found";
    /// 422, e.g. a fan value below the floor
    pub const VALIDATION: &str = "validation";
    /// 503, e.g. the service pipe is down
    pub const SERVICE_UNAVAILABLE: &str = "service_unavailable";
    /// 500
    pub const INTERNAL: &str = "internal";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Power {
    #[default]
    Running,
    /// Writes black once, then stops the render loop.
    Paused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Conflict {
    /// SignalRgb.exe or the SignalRgbService service is running.
    SignalRgb { process: bool, service: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Connected,
    Connecting,
    Down,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Status {
    pub version: String,
    pub power: Power,
    pub brightness: u8,
    pub fps_target: u32,
    /// Measured over the last second.
    pub fps_measured: f32,
    pub frame_time_ms: f32,
    /// Monotonic count of frames rendered since start. Stops growing when parked or paused.
    #[ts(type = "number")]
    pub frames_rendered: u64,
    /// True when the loop is parked (no device sinks, no preview subscriber).
    pub parked: bool,
    pub devices_connected: u32,
    pub preview_subscribers: u32,
    pub active_layout: Option<String>,
    pub active_effect: Option<String>,
    pub openrgb: LinkState,
    pub service: LinkState,
    /// Milliseconds the service pipe has been down, if it is (G4 banner after 10 s).
    #[ts(type = "number | null")]
    pub service_down_ms: Option<u64>,
    pub conflicts: Vec<Conflict>,
    /// G7: the editor shows the dismissible "size your ARGB headers" panel.
    pub show_zone_panel: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OutputState {
    pub power: Power,
    /// 0..100
    pub brightness: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PatchOutput {
    pub power: Option<Power>,
    pub brightness: Option<u8>,
}

/// `GET /render`: fps is global; scale and sampling are the active layout's.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RenderState {
    /// 10..=60
    pub fps: u32,
    pub scale: RenderScale,
    pub sampling: SamplingMode,
}

/// `PATCH /render`: fps changes live; scale and sampling apply to the active layout.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PatchRender {
    pub fps: Option<u32>,
    pub scale: Option<RenderScale>,
    pub sampling: Option<SamplingMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DevicesResponse {
    pub devices: Vec<DeviceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PatchZone {
    pub led_count: u32,
}

/// Lights one white LED (everything else on the device black) until another identify or
/// `stop: true`. The editor walks `led` along the zone.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IdentifyRequest {
    pub zone: u32,
    pub led: u32,
    #[serde(default)]
    pub stop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub needs: InputNeeds,
    pub favorite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectsResponse {
    pub effects: Vec<EffectSummary>,
    pub active: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectDetail {
    pub schema: PropertySchema,
    pub favorite: bool,
    /// User presets saved for this effect (built-ins are in `schema.presets`).
    pub user_presets: Vec<Preset>,
    /// Current values if this effect is active, else the saved values or defaults.
    pub props: Props,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActivateEffect {
    /// Overrides on top of the saved values or defaults.
    #[serde(default)]
    pub props: Props,
    pub preset: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SetFavorite {
    pub favorite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LayoutsResponse {
    pub active: String,
    pub layouts: Vec<Layout>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SetActiveLayout {
    pub id: String,
}

/// `PATCH /fans/headers/{h}` body is a [`FanMode`].
pub type PatchFanHeader = FanMode;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FanProfileName {
    pub name: String,
}

/// `GET /fans` when the service is down returns 503; otherwise this.
pub type FansResponse = FansStatus;

pub type PutFanProfile = FanProfile;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReadbackDevice {
    pub device_id: String,
    pub name: String,
    pub colors: Vec<crate::color::Rgb8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReadbackResponse {
    pub devices: Vec<ReadbackDevice>,
}
