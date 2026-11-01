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
    /// `GET`: a short looping preview of the effect at its default props, rendered headless by
    /// the daemon (no devices touched). Binary: `u16 LE width, u16 LE height, u16 LE frames`, then
    /// frames x width x height x 3 RGB bytes. 64x40, 24 frames at 12 fps. Cached per effect id
    /// until the effect set changes.
    pub const EFFECT_THUMBNAIL: &str = "/api/v1/effects/{id}/thumbnail";
    /// `PUT` with `{ "favorite": bool }`.
    pub const EFFECT_FAVORITE: &str = "/api/v1/effects/{id}/favorite";
    /// `POST` with [`super::ImportEffect`], answers [`super::ImportEffectResponse`]. The daemon
    /// parses the file before it writes it; a file that does not parse is refused with the
    /// parser's message and nothing is written. `DELETE` on [`EFFECT`] removes a user effect's
    /// file (built-ins refuse). Its presets go with it unless the query says
    /// `?keep_presets=true` ([`super::DeleteEffectQuery`]).
    pub const EFFECTS_IMPORT: &str = "/api/v1/effects/import";
    /// `POST` with [`super::PresetCreate`]: saves a user preset for the effect. Answers
    /// [`super::PresetsResponse`]. 409 when the name equals, ignoring ASCII case, a built-in or
    /// user preset of that effect; 404 for an unknown effect; 422 for an empty name, one over 40
    /// characters, one with a control, bidirectional or other invisible format character, or
    /// props over the caps (64 keys, 8 KiB). Keys the effect does not have are dropped and numbers
    /// are clamped to the schema's range.
    pub const EFFECT_PRESETS: &str = "/api/v1/effects/{id}/presets";
    /// `PUT` with [`super::PresetReplace`] overwrites and/or renames one user preset; `DELETE`
    /// removes it. Both answer [`super::PresetsResponse`]. 404 for a missing preset or effect;
    /// 409 when `expected` is not what is stored (changed in another window) or `rename_to`
    /// clashes; 422 for a bad name or props over the caps, as for [`EFFECT_PRESETS`].
    pub const EFFECT_PRESET: &str = "/api/v1/effects/{id}/presets/{name}";
    /// `GET`: the HTML source of an `html:` effect, user or stock, and the file name Save
    /// would use ([`super::EffectSource`]). A Rust built-in has no source and answers 404.
    pub const EFFECT_SOURCE: &str = "/api/v1/effects/{id}/source";
    /// `POST` with [`super::EffectPreviewRequest`]: the draft HTML and the editor's props. The
    /// daemon parses and renders the draft headless (no devices touched) and answers
    /// [`super::EffectPreviewResponse`]: the schema, the runtime's errors, the check warnings,
    /// a `source_hash`, and the 64x40, 24-frame loop rendered with those props. The same body
    /// cap as [`EFFECTS_IMPORT`]. A draft whose top-level script never ends is stopped by the
    /// runtime's deadline and answers 422; the API keeps answering while it runs. At most two
    /// previews render at once; a request that finds both busy answers 503
    /// `service_unavailable` with `Retry-After: 1`.
    pub const EFFECT_PREVIEW: &str = "/api/v1/effects/preview";
    /// `GET` the draft on the lights ([`super::DraftStatus`]); `PUT` with
    /// [`super::DraftRequest`] starts it or renews its lease (and updates the props); `DELETE`
    /// stops it. The editor PUTs every [`crate::consts::DRAFT_HEARTBEAT`] while the draft runs;
    /// the slot ends after [`crate::consts::DRAFT_LEASE`] with no PUT. Nothing about a draft is
    /// saved: the previous effect comes back on Stop, on expiry, or when the daemon restarts.
    /// An `activate` or a layout switch ends the draft without restoring anything, because the
    /// owner chose something else. A bad PUT (one that does not parse) leaves a running draft
    /// alone. Same body cap as [`EFFECTS_IMPORT`].
    pub const EFFECT_DRAFT: &str = "/api/v1/effects/draft";
    pub const LAYOUTS: &str = "/api/v1/layouts";
    pub const LAYOUT: &str = "/api/v1/layouts/{id}";
    /// `PUT` with [`super::SetActiveLayout`].
    pub const LAYOUT_ACTIVE: &str = "/api/v1/layouts/active";
    pub const FANS: &str = "/api/v1/fans";
    pub const FAN_HEADER: &str = "/api/v1/fans/headers/{h}";
    pub const FAN_PROFILE: &str = "/api/v1/fans/profile";
    /// `PUT` a [`crate::fans::FanProfile`] to create or replace it. `DELETE` removes it
    /// (409 `conflict` for the active profile or the last remaining one).
    pub const FAN_PROFILES: &str = "/api/v1/fans/profiles/{name}";
    /// `GET`: every numeric display with its config and the value it shows now ([`super::DisplayStatus`]).
    pub const DISPLAYS: &str = "/api/v1/displays";
    /// `PUT` a [`crate::display::DisplayConfig`] for one display (its `device` must match `{id}`).
    /// A display is driven only while it has a saved config: the daemon never opens or writes one
    /// it was not told to (the vendor app may own it). `DELETE` removes the config, stops writing
    /// and closes the device so the vendor app can take it back.
    pub const DISPLAY: &str = "/api/v1/displays/{id}";
    /// `PUT` with [`super::SetDisplayValue`]: the number a display with `DisplaySource::Api` shows.
    pub const DISPLAY_VALUE: &str = "/api/v1/displays/{id}/value";
    /// `GET`: every peripheral the adapters see ([`crate::peripheral::PeripheralsResponse`]).
    pub const PERIPHERALS: &str = "/api/v1/peripherals";
    /// `GET`: one peripheral's [`crate::peripheral::PeripheralDetail`] (a fresh read from the
    /// device). `PATCH` with [`crate::props::Props`]: write those keys; refused while config writes
    /// are off (see [`PERIPHERAL_ACCESS`]), and for unknown or read-only keys, with nothing sent.
    pub const PERIPHERAL: &str = "/api/v1/peripherals/{id}";
    /// `PUT` with [`crate::peripheral::PeripheralAccess`]: turn reads and config writes on or off
    /// for one device, saved. `writes: true` with `reads: false` is a validation error.
    pub const PERIPHERAL_ACCESS: &str = "/api/v1/peripherals/{id}/access";
    /// `GET`: the [`crate::peripheral::ReadProgress`] of the read of this device that is running now
    /// (`stage: "idle"` when none is). Answers at once, even while `GET` on [`PERIPHERAL`] waits for
    /// the device; the editor polls it to say what a long read is doing.
    pub const PERIPHERAL_READ: &str = "/api/v1/peripherals/{id}/read";
    /// `GET` / `PUT` a [`crate::config::Settings`].
    pub const SETTINGS: &str = "/api/v1/settings";
    /// `POST`, no body: dismisses G7's "size your ARGB headers" panel.
    pub const DISMISS_ZONE_PANEL: &str = "/api/v1/first-run/dismiss-zone-panel";
    /// `POST`, no body: the owner reached the end of the onboarding. Sets
    /// `Status.onboarding_complete`, which stays true.
    pub const ONBOARDING_COMPLETE: &str = "/api/v1/onboarding/complete";
    /// `POST` with [`super::OnboardingSession`]: the onboarding holds the lights. While a session
    /// is active every LED is black except what identify lights. The page resends `active: true`
    /// every ~2 s; the daemon ends the session after 5 s without one, or on `active: false`.
    pub const ONBOARDING_SESSION: &str = "/api/v1/onboarding/session";
    pub const CONFIG_EXPORT: &str = "/api/v1/config/export";
    pub const CONFIG_IMPORT: &str = "/api/v1/config/import";
    /// `POST`, no body: the daemon applies `on_quit` (G3: black by default), tells the
    /// service it is quitting on purpose (`PipeRequest::DaemonQuitting`), and exits.
    pub const QUIT: &str = "/api/v1/quit";
    /// `POST` [`InstallUpdateRequest`]: the daemon forwards it to the service as
    /// `PipeRequest::InstallUpdate` (#43). `202` [`InstallUpdateResponse`] once setup runs;
    /// `422 validation` for a refused file, signature or version; `409 conflict` while an install
    /// runs; `503 service_unavailable` when the service is down (the tray then falls back to the
    /// permission prompt).
    pub const UPDATE_INSTALL: &str = "/api/v1/update/install";
    /// Diagnostics: current colors as the adapter reads them back from the hardware.
    pub const DIAG_READBACK: &str = "/api/v1/diag/readback";
    /// WebSocket. Binary messages: `u16 LE width, u16 LE height, width*height*3 RGB bytes`,
    /// at most [`crate::consts::PREVIEW_MAX_FPS`].
    pub const PREVIEW: &str = "/api/v1/preview";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
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
    /// 409, e.g. deleting the active fan profile
    pub const CONFLICT: &str = "conflict";
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
    /// SignalRgb.exe or the SignalRgb.Service service is running.
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
    /// How many devices the daemon knows about, connected or not. The editor draws one card per
    /// device while the list is on its way, so it needs the total and not only the connected
    /// count. A daemon that does not send it (an older one) reports none.
    #[serde(default)]
    pub devices_total: u32,
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
    /// False until the owner finishes the first-run onboarding (maps their PC). While false,
    /// the tray opens the window by itself (a few times) onto the onboarding.
    #[serde(default)]
    pub onboarding_complete: bool,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OnboardingSession {
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PatchZone {
    pub led_count: u32,
}

/// Body of `POST /api/v1/update/install` (#43).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InstallUpdateRequest {
    /// Absolute path of the installer, readable by the calling user.
    pub installer: String,
    /// The text of the installer's `.sig` file (base64 minisign signature, as the release ships it).
    pub signature: String,
}

/// `202` answer of `POST /api/v1/update/install`: setup is running for this signed version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InstallUpdateResponse {
    pub version: String,
}

/// Shows the owner where a light is. While an identify is active the engine renders nothing
/// else: every LED on every device is black except the target, so the target is the only
/// light in the room. The target is `count` LEDs from `led` (default 1) in a bright colour.
/// It lasts until another identify, `stop: true`, or a few seconds with no request, so a
/// closed window can never leave the PC dark.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IdentifyRequest {
    pub zone: u32,
    pub led: u32,
    /// How many LEDs from `led` to light (a whole fan). `None` is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub count: Option<u32>,
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
    /// User effect files in the effects folder that did not parse, so an author sees why.
    pub failed: Vec<FailedEffect>,
}

/// A user effect file that did not parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FailedEffect {
    /// The file name inside the effects folder, e.g. `"my-effect.html"`.
    pub file: String,
    /// The parser's message.
    pub error: String,
}

/// `POST /effects/import`: a SignalRGB-format HTML effect to add to the effects folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImportEffect {
    /// The file name to save it as: letters, digits, space, `-`, `_` and `.`, ending in
    /// `.html`. No folders. The effect's id becomes `html:` + the stem.
    pub file_name: String,
    /// The file's contents, at most [`crate::consts::EFFECT_IMPORT_MAX_BYTES`].
    pub html: String,
    /// Overwrite a user effect of the same name. Without it, a clash is a conflict.
    pub replace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImportEffectResponse {
    pub effect: EffectSummary,
    /// True when an existing file was overwritten.
    pub replaced: bool,
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

/// A new user preset: props only, never a copy of the effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PresetCreate {
    pub name: String,
    pub props: Props,
}

/// Overwrite and/or rename a user preset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PresetReplace {
    /// The new name, or none to keep it.
    #[serde(default)]
    pub rename_to: Option<String>,
    pub props: Props,
    /// The props the client last saw for this preset. A mismatch is a conflict, so an edit made
    /// in another window is never overwritten silently.
    pub expected: Props,
}

/// Query of `DELETE /api/v1/effects/{id}`: what happens to the user presets of the effect that
/// is removed. Left out, they are deleted with it; with `keep_presets=true` they stay in
/// `effects.json`, so importing the same file again brings them back.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeleteEffectQuery {
    #[serde(default)]
    pub keep_presets: bool,
}

/// The effect's user presets after a change, in their stored order.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PresetsResponse {
    pub user_presets: Vec<Preset>,
}

/// `GET /effects/{id}/source`: the HTML the editor opens, and the file name Save writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectSource {
    pub source: String,
    /// The name `POST /effects/import` would use with `replace: true`: the file that is
    /// there for a user effect (its own case), or `<stem>.html` for a stock effect.
    pub file_name: String,
}

/// `POST /effects/preview`: the draft's HTML and the props the editor has set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectPreviewRequest {
    /// The draft, at most [`crate::consts::EFFECT_IMPORT_MAX_BYTES`].
    pub html: String,
    /// The props the preview is rendered with, laid under the effect's tile recipe.
    pub props: Props,
}

/// The preview of a draft: its schema, what the runtime reported, and its loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EffectPreviewResponse {
    /// The draft's schema. `id` is `draft:` + [`EffectPreviewResponse::source_hash`]: a draft is
    /// never a saved effect.
    pub schema: PropertySchema,
    /// The hash of the exact HTML this preview was rendered from. The editor keeps the last good
    /// loop when a later preview fails and labels it with this, so a stale loop is never
    /// mistaken for the current draft.
    pub source_hash: String,
    /// What the runtime reported while the draft was parsed and drawn. A preview with errors still
    /// carries the loop: the last good frame stays drawn, so the editor can show it while it
    /// says what failed.
    pub errors: Vec<String>,
    /// The check's warnings: a picture that does not move, one that is one flat colour, and
    /// anything the file declared that the loader ignored.
    pub warnings: Vec<String>,
    /// The 64x40, 24-frame loop rendered with the editor's props, base64. Decoded it is the
    /// thumbnail format: `u16 LE width, u16 LE height, u16 LE frames`, then
    /// frames x width x height x 3 RGB bytes (see [`crate::api::routes::EFFECT_THUMBNAIL`]).
    pub thumbnail: String,
}

/// `PUT /effects/draft`: the draft to put on the lights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DraftRequest {
    /// The draft, at most [`crate::consts::EFFECT_IMPORT_MAX_BYTES`]. Sent on every beat.
    pub html: String,
    /// The props the draft runs with.
    pub props: Props,
}

/// The draft on the lights, as `GET`/`PUT`/`DELETE /effects/draft` answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DraftStatus {
    /// True while a draft is on the lights.
    pub active: bool,
    /// The draft effect's id: `draft:` + `source_hash`. It is never a saved effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source_hash: Option<String>,
    /// The props the draft is running with. Empty when no draft runs.
    #[serde(default)]
    pub props: Props,
    /// Milliseconds left in the lease; 0 when no draft runs. A PUT renews it.
    #[ts(type = "number")]
    pub lease_remaining_ms: u64,
    /// How often to PUT to keep the draft alive; [`crate::consts::DRAFT_HEARTBEAT`].
    #[ts(type = "number")]
    pub heartbeat_ms: u64,
    /// How long the draft survives without a PUT; [`crate::consts::DRAFT_LEASE`].
    #[ts(type = "number")]
    pub lease_ms: u64,
    /// A sentence about the ending, when it needs saying: the prior effect could not be
    /// restored (its file was deleted mid-draft), so the lights are black. Absent when the
    /// prior, or black for a layout with no effect, went on normally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ActivateEffect {
    /// Overrides on top of the saved values or defaults (or on top of the preset).
    #[serde(default)]
    pub props: Props,
    /// A built-in preset (`schema.presets`) or a user preset, by name. A preset's props sit on
    /// the defaults, not on the saved values, so two presets never leak into each other.
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

/// One display as `GET /displays` reports it: what it is, how it is set, and what it shows now
/// (`shown: None` when it is blank or not written yet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DisplayStatus {
    pub info: crate::display::DisplayInfo,
    pub config: crate::display::DisplayConfig,
    pub shown: Option<crate::display::DisplayValue>,
}

/// Body of `PUT /displays/{id}/value`. `value: None` blanks the display.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SetDisplayValue {
    pub value: Option<f32>,
}
