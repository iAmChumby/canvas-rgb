//! `DevicesView`, after `app/web/src/views/DevicesView.tsx` and
//! `app/web/src/views/editor/IdentifyWalker.tsx`.
//!
//! The port keeps the React view's rules (the checklist is
//! `docs/gpui-parity/devices.md`):
//! - the list order: disconnected devices first, then by kind rank; the
//!   daemon's order stands inside a rank;
//! - the row accent follows the sorted position (`--a1`..`--a5` cycling);
//! - the tag from the last `0x...` run in the location when two devices share
//!   a name;
//! - one-zone devices draw the strip and count in their own row; multi-zone
//!   devices draw a header row plus one row per zone;
//! - the identify walker: `POST .../identify` on open and on each LED step,
//!   resent every 2 s while open, `stop: true` on close, on a zone change and
//!   after a length write; `PATCH .../zones/{zone}` with `{led_count}` on
//!   "This is the last LED";
//! - load states: loading placeholder, error with Try again (pending-guarded,
//!   React's `retryDevices`), empty with Try again (unguarded,
//!   `refreshDevices`), stale notice over live rows, and the OpenRGB notice.
//!
//! Known React bugs, copied, not fixed here (#230: Try again never rescans,
//! #233: the daemon ignores the rescan flag, #231): the error panel's Try
//! again is a plain `GET /api/v1/devices` with no `?rescan=1`, exactly as
//! React does.
//!
//! The React demo-mode states (`?state=...`) are demo-backend behavior and
//! have no GPUI counterpart: the GPUI editor always talks to a real (here,
//! fake) daemon over HTTP.

use std::time::Duration;

use canvas_proto::api::{DevicesResponse, IdentifyRequest, LinkState, PatchZone, Status};
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use gpui::{Context, IntoElement, Window, div, prelude::*, px};

use crate::client::DaemonClient;
use crate::components::button::{Variant, button, button_icon, button_sm, icon_button};
use crate::components::empty_state::empty_state;
use crate::components::number_field::number_field;
use crate::components::panel::{state_dot, tracked_title};
use crate::theme::{Theme, TokenRadius};

// ---------------------------------------------------------------------------
// Logic, ported from DevicesView.tsx and covered by the tests below.

/// The read order of the kinds: the board first, then the memory, the card,
/// the hubs, the strips and the peripherals. Inside a group the daemon's
/// order stands (React `KIND_RANK`).
pub fn kind_rank(kind: DeviceKind) -> u32 {
    match kind {
        DeviceKind::Motherboard => 0,
        DeviceKind::Dram => 1,
        DeviceKind::Gpu => 2,
        DeviceKind::Other => 3,
        DeviceKind::LedStrip => 4,
        DeviceKind::Keyboard => 5,
        DeviceKind::Mouse => 5,
        DeviceKind::Microphone => 5,
        DeviceKind::Headset => 5,
    }
}

/// The kind words the list draws (React `KIND_LABEL`).
pub fn kind_label(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Motherboard => "Motherboard",
        DeviceKind::Dram => "DRAM",
        DeviceKind::Keyboard => "Keyboard",
        DeviceKind::Mouse => "Mouse",
        DeviceKind::Microphone => "Microphone",
        DeviceKind::Headset => "Headset",
        DeviceKind::LedStrip => "LED strip",
        DeviceKind::Gpu => "GPU",
        DeviceKind::Other => "Other",
    }
}

/// The Phosphor icon asset for a device kind. The headset shares the
/// microphone's icon, as React does.
pub fn kind_icon(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Motherboard => "Cpu",
        DeviceKind::Dram => "Memory",
        DeviceKind::Keyboard => "Keyboard",
        DeviceKind::Mouse => "Mouse",
        DeviceKind::Microphone => "Microphone",
        DeviceKind::Headset => "Microphone",
        DeviceKind::LedStrip => "Lightbulb",
        DeviceKind::Gpu => "GraphicsCard",
        DeviceKind::Other => "Plugs",
    }
}

/// The one detail that tells two devices apart when the daemon gives them
/// the same name: the address in their location. The last `0x...` hex run
/// wins (React `deviceTag`).
pub fn device_tag(location: &str) -> Option<String> {
    let mut tag = None;
    let bytes = location.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
            let mut j = i + 2;
            while j < bytes.len() && (bytes[j] as char).is_ascii_hexdigit() {
                j += 1;
            }
            if j > i + 2 {
                tag = Some(location[i..j].to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    tag
}

/// The list in the order it is read: what is not there first, then by kind.
/// The sort is on a copy; the daemon's order stands inside a rank (React
/// sorts a spread copy).
pub fn sort_devices(devices: &[DeviceInfo]) -> Vec<DeviceInfo> {
    let mut sorted = devices.to_vec();
    sorted.sort_by(|a, b| {
        if a.connected != b.connected {
            if a.connected {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            }
        } else {
            kind_rank(a.kind).cmp(&kind_rank(b.kind))
        }
    });
    sorted
}

/// The accent the row wears: `ACCENTS[index % 5]` by position in the sorted
/// list, so the colour follows the position, not the device.
pub fn accent_for(t: &Theme, index: usize) -> gpui::Rgba {
    match index % 5 {
        0 => Theme::color(t.a1),
        1 => Theme::color(t.a2),
        2 => Theme::color(t.a3),
        3 => Theme::color(t.a4),
        _ => Theme::color(t.a5),
    }
}

/// The sub-line: kind, vendor, adapter, address, and the tag when there is
/// one. The same text is the row's tooltip in React.
pub fn sub_line(d: &DeviceInfo, tag: Option<&str>) -> String {
    let mut s = format!(
        "{} · {} · {} #{}",
        kind_label(d.kind),
        d.vendor,
        d.adapter,
        d.adapter_index
    );
    if let Some(tag) = tag {
        s.push_str(&format!(" · {tag}"));
    }
    s
}

/// The device name: `d.name`, with ` <tag>` appended only when another
/// device has the same name and this one has a tag.
pub fn display_name(d: &DeviceInfo, same_name: bool, tag: Option<&str>) -> String {
    match (same_name, tag) {
        (true, Some(tag)) => format!("{} {tag}", d.name),
        _ => d.name.clone(),
    }
}

/// The shape word under a zone's name: `matrix <w> × <h>`, `linear` or
/// `single` (React `zoneShape`).
pub fn zone_shape(zone: &ZoneInfo) -> String {
    match &zone.kind {
        ZoneKind::Matrix { width, height, .. } => format!("matrix {width} × {height}"),
        ZoneKind::Linear => "linear".to_string(),
        ZoneKind::Single => "single".to_string(),
    }
}

/// "LED" for one LED, "LEDs" otherwise (0 reads "0 LEDs").
pub fn led_word(led_count: u32) -> &'static str {
    if led_count == 1 { "LED" } else { "LEDs" }
}

/// How many devices are lit: devices with `connected: true`.
pub fn connected_count(devices: &[DeviceInfo]) -> usize {
    devices.iter().filter(|d| d.connected).count()
}

/// The LED total the strip states: only connected devices contribute their
/// zones' `led_count`.
pub fn lit_count(devices: &[DeviceInfo]) -> u32 {
    devices
        .iter()
        .filter(|d| d.connected)
        .flat_map(|d| d.zones.iter())
        .map(|z| z.led_count)
        .sum()
}

/// The strip's context line: `<connected>/<total> connected · <lit> LEDs`,
/// a fixed plural "LEDs" even for one LED.
pub fn context_text(devices: Option<&[DeviceInfo]>) -> String {
    let devices = devices.unwrap_or(&[]);
    format!(
        "{}/{} connected · {} LEDs",
        connected_count(devices),
        devices.len(),
        lit_count(devices)
    )
}

/// How many placeholder cards the loading frame draws: `total ??
/// listed ?? (counted > 0 ? counted : 3)` (React `DevicesView`).
pub fn loading_cards(total: Option<u32>, listed: Option<usize>, counted: Option<u32>) -> usize {
    if let Some(total) = total {
        return total as usize;
    }
    if let Some(listed) = listed {
        return listed;
    }
    let counted = counted.unwrap_or(0);
    if counted > 0 {
        counted as usize
    } else {
        3
    }
}

/// Which frame the page draws: error when no list was ever read and no
/// retry is pending; loading while the first read is in flight or a Try
/// again is running; empty for an empty list; ready otherwise (React
/// `LoadFrame` `state` in `DevicesView`).
pub fn frame_state(has_list: bool, list_empty: bool, error_set: bool, retrying: bool) -> &'static str {
    if !has_list {
        if error_set && !retrying { "error" } else { "loading" }
    } else if list_empty {
        "empty"
    } else {
        "ready"
    }
}

/// How many zone rows placeholder card `i` draws: `ZONE_RHYTHM[i % 12]` with
/// the rhythm `[1,1,1,4,1,1,1,1,1,1,1,1]` (card 3 has four zone rows).
pub fn placeholder_zones(card: usize) -> usize {
    const ZONE_RHYTHM: [usize; 12] = [1, 1, 1, 4, 1, 1, 1, 1, 1, 1, 1, 1];
    ZONE_RHYTHM[card % ZONE_RHYTHM.len()]
}

/// How the desktop device row splits its room between the sub-line (`flex:
/// 0 1 300px`) and the strip (`flex: 1 1 200px`): proportional shrink
/// (shrink × basis), and the strip grows into free space past the sub's 300.
/// The fixed items are the kind box (32), six 12 px gaps, the name (220),
/// the count, the state and the Size slot (104). Returns (sub_w, strip_w).
/// Measured against the React captures: at 1024 the sub is 187 and the strip
/// box 125 (the dots and the count land on React's pixels); at 1440, 300
/// and 292.
pub fn device_split(row_w: f32, count_w: f32, state_w: f32) -> (f32, f32) {
    let fixed = 32.0 + 72.0 + 220.0 + 104.0 + count_w + state_w;
    let free = (row_w - fixed).max(0.0);
    if free >= 500.0 {
        (300.0, free - 300.0)
    } else {
        let shrink = 500.0 - free;
        let sub = (300.0 - shrink * 0.6).clamp(96.0, 300.0);
        let strip = (200.0 - shrink * 0.4).clamp(64.0, 200.0);
        // If one item floors out, the other takes what is left.
        if sub + strip > free {
            let sub = (free - strip).clamp(96.0, 300.0);
            (sub, (free - sub).clamp(64.0, 200.0))
        } else {
            (sub, strip)
        }
    }
}

/// A card's height in the two-column split: its header row, one row per
/// zone (multi-zone devices), the zone-empty panel or the walker when open.
/// The CSS `columns` rule breaks between cards to balance the columns, so
/// the GPUI list uses the same heights for its split.
pub fn card_height(device: &DeviceInfo, row_h: f32, walker_here: bool) -> f32 {
    let mut h = row_h;
    if device.zones.is_empty() {
        h += 220.0;
    } else if device.zones.len() > 1 {
        h += device.zones.len() as f32 * row_h;
    }
    if walker_here {
        h += 140.0;
    }
    h
}

/// Where the two columns split this list: the break that leaves the most
/// balanced columns (React's `columns: 2` balancing), breaking between
/// cards only.
pub fn balanced_split(heights: &[f32]) -> usize {
    let total: f32 = heights.iter().sum();
    let mut best = heights.len().saturating_sub(1).max(1);
    let mut best_diff = f32::INFINITY;
    let mut acc = 0.0f32;
    for (i, h) in heights.iter().enumerate() {
        acc += h;
        let diff = (total - 2.0 * acc).abs();
        if diff < best_diff {
            best_diff = diff;
            best = i + 1;
        }
    }
    best.min(heights.len().saturating_sub(1)).max(1).min(heights.len())
}

/// The zone row's strip width (React `.rd-zone`: only the strip is flexible,
/// basis 200 with a 64 floor): everything the row's room leaves it.
pub fn zone_strip_width(row_w: f32, count_w: f32) -> f32 {
    let fixed = 60.0 + 116.0 + 36.0 + 104.0 + 16.0 + count_w;
    (row_w - fixed).max(64.0)
}

/// The most dots or bar the strip draws: `Math.min(led_count, 240)`, while
/// the count text reads the real `led_count`.
pub fn drawn_count(led_count: u32) -> u32 {
    led_count.min(240)
}

/// How a zone's strip is drawn: nothing for a single LED, a dashed line for
/// zero LEDs, one dot per LED up to 24, a lit bar beyond that, a grid for a
/// matrix.
#[derive(Debug, Clone, PartialEq)]
pub enum StripMode {
    Single,
    Dashed,
    Dots { step: f32, radius: f32 },
    Bar,
    Matrix { cols: u32, rows: u32 },
}

/// The strip's drawing for one zone (React `ZoneStrip`).
pub fn strip_mode(zone: &ZoneInfo) -> StripMode {
    match &zone.kind {
        ZoneKind::Matrix { width, height, .. } => StripMode::Matrix {
            cols: *width,
            rows: *height,
        },
        ZoneKind::Single => StripMode::Single,
        ZoneKind::Linear => {
            let n = drawn_count(zone.led_count) as f32;
            if n <= 0.0 {
                StripMode::Dashed
            } else if n > 24.0 {
                StripMode::Bar
            } else {
                let step = 200.0 / n;
                let radius = 4.0_f32.min(1.8_f32.max(step * 0.3));
                StripMode::Dots { step, radius }
            }
        }
    }
}

/// The walker target the `?walk=<device id>` deep link opens: the device's
/// first resizable zone that already reports LEDs, else its first resizable
/// zone (React `DevicesView` `fromUrl`). The GPUI editor has no URL, so the
/// deep link itself is not ported; the rule is kept and tested (the checklist
/// item stays unticked).
#[allow(dead_code)]
pub fn walk_target_for(devices: &[DeviceInfo], device_id: &str) -> Option<(String, u32)> {
    let d = devices.iter().find(|x| x.id.0 == device_id)?;
    let z = d
        .zones
        .iter()
        .find(|x| x.resizable && x.led_count > 0)
        .or_else(|| d.zones.iter().find(|x| x.resizable))?;
    Some((d.id.0.clone(), z.index))
}

/// The walker is on the last LED when `led` equals `max_leds - 1` and the
/// first at 0. The ceiling never drops below 0 even for a zone with
/// `max_leds` 0.
pub fn walker_ceiling(zone: &ZoneInfo) -> u32 {
    zone.max_leds.saturating_sub(1)
}

/// Clamp an LED step into `0..=ceiling` (React `setLed`).
pub fn clamp_led(led: i32, zone: &ZoneInfo) -> u32 {
    led.clamp(0, walker_ceiling(zone) as i32) as u32
}

/// Join the walker's two error lines: the length write's message first, then
/// the identify light's, with one space when both are set.
pub fn walker_error(length: Option<&str>, identify: Option<&str>) -> Option<String> {
    match (length, identify) {
        (Some(l), Some(i)) => Some(format!("{l} {i}")),
        (Some(l), None) => Some(l.to_string()),
        (None, Some(i)) => Some(i.to_string()),
        (None, None) => None,
    }
}

/// The note after a successful length write: "<zone> is now <n> LEDs."
pub fn walker_note(zone: &ZoneInfo, led: u32) -> String {
    format!("{} is now {} LEDs.", zone.name, led + 1)
}

/// The walker's resend interval: the walker asks again every 2 s while open
/// (React `RESEND_MS`).
pub const RESEND_MS: u64 = 2000;

// ---------------------------------------------------------------------------
// The view.

/// The walker target: a device id and a zone index within it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkTarget {
    pub device_id: String,
    pub zone_index: u32,
}

/// The walker's own state for the zone it walks on.
#[derive(Debug, Clone)]
pub struct WalkerState {
    pub led: u32,
    pub identify_error: Option<String>,
    pub length_error: Option<String>,
    pub note: Option<String>,
    /// True once the stop has been sent for this opening: no second stop, no
    /// resend (React `stopped` ref).
    pub stopped: bool,
    /// A token for the resend loop: only the loop of the current opening
    /// resends.
    pub generation: u32,
}

impl WalkerState {
    pub fn new(generation: u32) -> Self {
        WalkerState {
            led: 0,
            identify_error: None,
            length_error: None,
            note: None,
            stopped: false,
            generation,
        }
    }
}

/// The Devices page: the daemon's list, in the order it is read, with the
/// state banners the React page draws.
pub struct DevicesView {
    pub theme: Theme,
    pub devices: Option<Vec<DeviceInfo>>,
    pub devices_error: Option<String>,
    pub retrying: bool,
    pub total: Option<u32>,
    pub counted: Option<u32>,
    pub openrgb: LinkState,
    /// The walker target from a Size press. There is no URL in the GPUI
    /// editor, so no `?walk=` deep link: the target is only ever set here.
    pub walk: Option<WalkTarget>,
    pub walker: WalkerState,
    pub generation: u32,
    /// The error panel's Details disclosure (`<details>`, closed by default).
    pub details_open: bool,
}

impl DevicesView {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        let mut view = DevicesView {
            theme,
            devices: None,
            devices_error: None,
            retrying: false,
            total: None,
            counted: None,
            openrgb: LinkState::Connected,
            walk: None,
            walker: WalkerState::new(0),
            generation: 0,
            details_open: false,
        };
        view.load(cx);
        view
    }

    fn client() -> DaemonClient {
        DaemonClient::new()
    }

    /// `GET /api/v1/devices` and `GET /api/v1/status` (for the strip totals
    /// and the OpenRGB notice). Runs off the UI thread; the result lands on
    /// the next render. A failure with no list read yet is the error frame;
    /// a failure over a live list is the stale notice (React `LoadFrame`).
    fn load(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let devices = client.get("/api/v1/devices");
            let status = client.get("/api/v1/status");
            (devices, status)
        });
        cx.spawn(async move |view, cx| {
            let (devices, status) = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.retrying = false;
                if let Ok(body) = status {
                    if let Ok(parsed) = serde_json::from_str::<Status>(&body) {
                        this.total = Some(parsed.devices_total);
                        this.counted = Some(parsed.devices_connected);
                        this.openrgb = parsed.openrgb;
                    }
                }
                match devices {
                    Ok(body) => {
                        if let Ok(parsed) = serde_json::from_str::<DevicesResponse>(&body) {
                            this.devices = Some(parsed.devices);
                            this.devices_error = None;
                        }
                    }
                    Err(e) => {
                        this.devices_error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
            // The next read, 4 s later (React polls `devices` every 4 s).
            let timer = cx.background_executor().timer(Duration::from_secs(4));
            timer.await;
            let _ = view.update(cx, |this, cx| {
                this.load(cx);
            });
        })
        .detach();
    }

    /// The error panel's Try again: back to loading while it runs, one GET
    /// per press (React `retryDevices`, the pending guard). A plain GET with
    /// no `?rescan=1` (#230 copied).
    fn retry(&mut self, cx: &mut Context<Self>) {
        if self.retrying {
            return;
        }
        self.retrying = true;
        self.devices_error = None;
        cx.notify();
        self.load(cx);
    }

    /// The empty and zone-empty Try again: a plain refresh that never sets
    /// the pending marker (React `refreshDevices`).
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    /// The walker target's device and zone, read from the current list. The
    /// walker is not drawn while either has left the list, but the target is
    /// kept, so it comes back with them.
    fn walker_device_zone(&self) -> Option<(&DeviceInfo, &ZoneInfo)> {
        let walk = self.walk.as_ref()?;
        let device = self
            .devices
            .as_ref()?
            .iter()
            .find(|d| d.id.0 == walk.device_id)?;
        let zone = device.zones.iter().find(|z| z.index == walk.zone_index)?;
        Some((device, zone))
    }

    /// One identify POST for the LED the walker is on (React `light`).
    fn light(&mut self, device_id: String, zone_index: u32, led: u32, generation: u32, cx: &mut Context<Self>) {
        let req = IdentifyRequest {
            zone: zone_index,
            led,
            count: None,
            stop: false,
        };
        let body = serde_json::to_string(&req).unwrap_or_default();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.post_json(&format!("/api/v1/devices/{device_id}/identify"), &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                // A resend from an old opening must not write into the new
                // one (React drops a stale answer on unmount).
                if this.walker.generation != generation {
                    return;
                }
                match result {
                    Ok(_) => this.walker.identify_error = None,
                    Err(e) => this.walker.identify_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The stop POST: `{zone, led: 0, stop: true}`, once per exit; its own
    /// errors are ignored (React `stop`).
    fn stop(&mut self, device_id: String, zone_index: u32, cx: &mut Context<Self>) {
        let req = IdentifyRequest {
            zone: zone_index,
            led: 0,
            count: None,
            stop: true,
        };
        let body = serde_json::to_string(&req).unwrap_or_default();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.post_json(&format!("/api/v1/devices/{device_id}/identify"), &body)
        });
        cx.spawn(async move |view, cx| {
            let _ = bg.await;
            let _ = view.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// A Size press: open the walker on that zone. Re-pressing the open
    /// zone keeps it open (no toggle); pressing another zone moves it and
    /// the LED returns to 0 (React remounts with a new key).
    fn open_walk(&mut self, target: WalkTarget, cx: &mut Context<Self>) {
        if self.walk.as_ref() == Some(&target) {
            return;
        }
        if let Some(old) = self.walk.take() {
            self.stop(old.device_id.clone(), old.zone_index, cx);
        }
        self.generation += 1;
        let generation = self.generation;
        self.walk = Some(target.clone());
        self.walker = WalkerState::new(generation);
        cx.notify();
        self.light(target.device_id.clone(), target.zone_index, 0, generation, cx);
        self.resend(target.clone(), generation, cx);
    }

    /// The walker's 2 s resend: the same identify request again while the
    /// walker stays open (React's `RESEND_MS` interval).
    fn resend(&mut self, target: WalkTarget, generation: u32, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            loop {
                let timer = cx.background_executor().timer(Duration::from_millis(RESEND_MS));
                timer.await;
                let keep = view
                    .update(cx, |this, cx| {
                        let open = this.walk.as_ref() == Some(&target)
                            && this.walker.generation == generation
                            && !this.walker.stopped
                            && this.walker_device_zone().is_some();
                        if open {
                            this.light(target.device_id.clone(), target.zone_index, this.walker.led, generation, cx);
                        }
                        open
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    /// Close the walker: send the stop, then clear the target and unmount
    /// (React `exit`).
    fn close_walk(&mut self, cx: &mut Context<Self>) {
        if let Some(target) = self.walk.take() {
            self.stop(target.device_id.clone(), target.zone_index, cx);
        }
        self.walker.stopped = true;
        cx.notify();
    }

    /// Step the walker: one LED down or up, floored at 0 and ceiled at
    /// `max_leds - 1` (React Prev/Next).
    fn step_led(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(target) = self.walk.clone() else {
            return;
        };
        let Some(zone) = self
            .devices
            .as_ref()
            .and_then(|d| d.iter().find(|x| x.id.0 == target.device_id))
            .and_then(|d| d.zones.iter().find(|z| z.index == target.zone_index))
            .cloned()
        else {
            return;
        };
        let next = clamp_led(self.walker.led as i32 + delta, &zone);
        self.set_led(next, cx);
    }

    /// Move the walker to an LED and light it.
    fn set_led(&mut self, led: u32, cx: &mut Context<Self>) {
        let Some(target) = self.walk.clone() else {
            return;
        };
        let generation = self.walker.generation;
        self.walker.led = led;
        cx.notify();
        self.light(target.device_id, target.zone_index, led, generation, cx);
    }

    /// "This is the last LED": PATCH the zone to one past the lit LED,
    /// refresh the list, send the stop, then set the note (React
    /// `setLength`). The write never clamps to `min_leds`.
    fn set_length(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.walk.clone() else {
            return;
        };
        let led = self.walker.led;
        self.walker.length_error = None;
        self.walker.note = None;
        cx.notify();
        let patch = PatchZone { led_count: led + 1 };
        let body = serde_json::to_string(&patch).unwrap_or_default();
        let bg_target = target.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.patch_json(&format!("/api/v1/devices/{}/zones/{}", bg_target.device_id, bg_target.zone_index), &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.stop(target.device_id.clone(), target.zone_index, cx);
                        this.walker.stopped = true;
                        this.walker.note = Some(walker_note(
                            this.walker_device_zone().map(|(_, z)| z).unwrap_or(&ZoneInfo {
                                index: target.zone_index,
                                name: String::new(),
                                kind: ZoneKind::Single,
                                led_count: 0,
                                min_leds: 0,
                                max_leds: 0,
                                resizable: true,
                            }),
                            led,
                        ));
                        this.load(cx);
                    }
                    Err(e) => {
                        this.walker.length_error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The strip's title slot: the page's name and its one-line context
    /// (React `PageTitle`). The context is hidden on a phone by the shell,
    /// and needs a loaded list.
    pub fn strip_title(&self) -> (String, Option<String>) {
        let context = self.devices.as_deref().map(|d| context_text(Some(d)));
        ("Devices".to_string(), context)
    }

    // -- elements ---------------------------------------------------------

    /// A zone's LED strip (React `ZoneStrip`): a canvas in the device's
    /// accent, painted from the SVG viewBox with `preserveAspectRatio=
    /// xMidYMid meet` (uniform scale, centred). `box_h` is the element's
    /// height (22 on a desktop row; the aspect height on a phone device row;
    /// 16 on a phone zone row); the element's width is `box_w` when set, else
    /// it takes the row's width.
    fn zone_strip(&self, zone: &ZoneInfo, tone: gpui::Rgba, box_w: Option<f32>, box_h: f32, grow: bool) -> gpui::AnyElement {
        let zone = zone.clone();
        let mode = strip_mode(&zone);
        if let StripMode::Single = mode {
            // A single LED has no strip to draw, but the row keeps the
            // strip's box (React renders an empty `span.zone-strip-svg`,
            // which still takes its `flex: 0 0 64px`/strip width), so the
            // count lands where a drawn strip would put it.
            return match box_w {
                Some(w) => div().w(px(w)).h(px(box_h)).into_any_element(),
                None => div().w_full().h(px(box_h)).into_any_element(),
            };
        }
        let canvas = gpui::canvas(
            move |_, _, _| {},
            move |bounds, _, window, _| {
                let w: f32 = bounds.size.width.into();
                let h: f32 = bounds.size.height.into();
                if w <= 0.0 || h <= 0.0 {
                    return;
                }
                let palette = tone;
                // The viewBox and its meet scale: a linear strip is 200 × 22,
                // a matrix is its grid. Above the viewBox's 1:1 the painting
                // stays put (React's dots are 8 px); below it scales down.
                let (vbw, vbh) = match &mode {
                    StripMode::Matrix { cols, rows } => (*cols as f32, *rows as f32),
                    _ => (200.0, 22.0),
                };
                let s = (w / vbw).min(h / vbh);
                let ox = (w - vbw * s) / 2.0;
                let oy = (h - vbh * s) / 2.0;
                let x = |units: f32| ox + units * s;
                let y = |units: f32| oy + units * s;
                let dot = |window: &mut Window, cx: f32, cy: f32, r: f32, alpha: f32| {
                    let c = gpui::Rgba { a: alpha, ..palette };
                    let mut quad = gpui::fill(
                        gpui::Bounds {
                            origin: gpui::point(bounds.origin.x + gpui::px(cx - r), bounds.origin.y + gpui::px(cy - r)),
                            size: gpui::size(gpui::px(2.0 * r), gpui::px(2.0 * r)),
                        },
                        c,
                    );
                    quad.corner_radii.top_left = gpui::px(r);
                    quad.corner_radii.top_right = gpui::px(r);
                    quad.corner_radii.bottom_right = gpui::px(r);
                    quad.corner_radii.bottom_left = gpui::px(r);
                    window.paint_quad(quad);
                };
                // The pill is a filled path, not a quad: a quad snaps to whole
                // pixels, so a pill 4.5 px tall painted its edge rows at full
                // opacity where the browser's are partial coverage.
                let bar = |window: &mut Window, units_y: f32, units_bh: f32, radius: f32, alpha: f32| {
                    let (x0, y0) = (x(0.0), y(units_y));
                    let (w, h, r) = (200.0 * s, units_bh * s, radius * s);
                    let r = r.min(h / 2.0).min(w / 2.0);
                    let mut builder = gpui::PathBuilder::fill();
                    let at = |px_x: f32, px_y: f32| {
                        gpui::point(bounds.origin.x + gpui::px(px_x), bounds.origin.y + gpui::px(px_y))
                    };
                    // Corners as 8-segment arcs: (centre, start angle).
                    let corners = [
                        (x0 + w - r, y0 + r, -std::f32::consts::FRAC_PI_2),
                        (x0 + w - r, y0 + h - r, 0.0),
                        (x0 + r, y0 + h - r, std::f32::consts::FRAC_PI_2),
                        (x0 + r, y0 + r, std::f32::consts::PI),
                    ];
                    let mut first = true;
                    for (cx, cy, a0) in corners {
                        for i in 0..=8 {
                            let a = a0 + std::f32::consts::FRAC_PI_2 * (i as f32 / 8.0);
                            let p = at(cx + r * a.cos(), cy + r * a.sin());
                            if first {
                                builder.move_to(p);
                                first = false;
                            } else {
                                builder.line_to(p);
                            }
                        }
                    }
                    builder.close();
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, gpui::Rgba { a: alpha, ..palette });
                    }
                };
                match mode {
                    StripMode::Dashed => {
                        // The "no LEDs yet" line: dashes 40..160 at y 11, 1.5
                        // units thick, opacity 0.45. A quad snaps to whole
                        // pixels, so a line a pixel thick that straddles two
                        // rows drew both at full opacity (390 px: 71,88,112
                        // where React's rows read 40,50,63); a filled path
                        // takes the fractional coverage the browser's does.
                        let mut builder = gpui::PathBuilder::fill();
                        let top = y(11.0) - 0.75 * s;
                        let bottom = y(11.0) + 0.75 * s;
                        let mut units = 40.0f32;
                        while units < 160.0 {
                            let dash_units = (4.0f32).min(160.0 - units);
                            let left = bounds.origin.x + gpui::px(x(units));
                            let right = bounds.origin.x + gpui::px(x(units) + dash_units * s);
                            let (t, b) = (bounds.origin.y + gpui::px(top), bounds.origin.y + gpui::px(bottom));
                            builder.move_to(gpui::point(left, t));
                            builder.line_to(gpui::point(right, t));
                            builder.line_to(gpui::point(right, b));
                            builder.line_to(gpui::point(left, b));
                            builder.close();
                            units += 8.0;
                        }
                        if let Ok(path) = builder.build() {
                            window.paint_path(path, gpui::Rgba { a: 0.45, ..palette });
                        }
                    }
                    StripMode::Bar => {
                        bar(window, 4.0, 14.0, 5.0, 0.16);
                        bar(window, 6.5, 9.0, 4.5, 0.5);
                        bar(window, 9.5, 3.0, 1.5, 1.0);
                    }
                    StripMode::Dots { step, radius } => {
                        let r = radius * s;
                        bar(window, 4.0, 14.0, 5.0, 0.16);
                        let n = drawn_count(zone.led_count);
                        for i in 0..n {
                            let cx = x((i as f32 + 0.5) * step);
                            dot(window, cx, y(11.0), r, 1.0);
                        }
                    }
                    StripMode::Matrix { cols, rows } => {
                        if let ZoneKind::Matrix { map, .. } = &zone.kind {
                            for row in 0..rows {
                                for col in 0..cols {
                                    let idx = (row * cols + col) as usize;
                                    let lit = map.get(idx).is_some_and(|cell| cell.is_some());
                                    let cx = x(col as f32 + 0.5);
                                    let cy = y(row as f32 + 0.5);
                                    if lit {
                                        dot(window, cx, cy, 0.36 * s, 1.0);
                                    } else {
                                        dot(window, cx, cy, 0.12 * s, 0.35);
                                    }
                                }
                            }
                        }
                    }
                    StripMode::Single => {}
                }
            },
        )
        .w_full()
        .h_full();
        if let Some(width) = box_w {
            div().w(px(width)).h(px(box_h)).child(canvas).into_any_element()
        } else if grow {
            // The strip takes the row's leftover room (React
            // `flex: 1 1 200px` — basis 200 with grow and shrink 1). GPUI's
            // `flex_1` (basis 0) let Yoga pin the strip at its 64 minimum and
            // shrink the sub instead (measured at 1024: the strip landed
            // 64 wide at +40 px, React's is ~134).
            div()
                .flex_grow()
                .flex_shrink()
                .flex_basis(px(200.0))
                .min_w(px(64.0))
                .h(px(box_h))
                .child(canvas)
                .into_any_element()
        } else {
            div().w_full().h(px(box_h)).child(canvas).into_any_element()
        }
    }

    /// The Size key for one zone: primary while the walker is on it
    /// (React `Button size="sm"`).
    fn size_key(&self, device_id: &str, zone_index: u32, cx: &mut Context<Self>) -> gpui::AnyElement {
        let on = self
            .walk
            .as_ref()
            .is_some_and(|w| w.device_id == device_id && w.zone_index == zone_index);
        let entity = cx.entity();
        let target = WalkTarget {
            device_id: device_id.to_string(),
            zone_index,
        };
        div()
            .child(button_sm(
                &self.theme,
                &format!("size-{device_id}-{zone_index}"),
                "Size",
                if on { Variant::Primary } else { Variant::Default },
                false,
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.open_walk(target.clone(), cx));
                },
            ))
            .into_any_element()
    }

    /// One device's header row (React `.rd-dev-row`).
    #[allow(clippy::too_many_arguments)]
    fn device_row(
        &self,
        window: &Window,
        device: &DeviceInfo,
        accent: gpui::Rgba,
        same_name: bool,
        phone: bool,
        row_h: f32,
        content_w: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = &self.theme;
        let tag = device_tag(&device.location);
        let sub = sub_line(device, tag.as_deref());
        let name = display_name(device, same_name, tag.as_deref());
        let one = if device.zones.len() == 1 { Some(device.zones[0].clone()) } else { None };
        let state_color = if device.connected { Theme::color(t.ok) } else { Theme::color(t.down) };
        let state_text = if device.connected { Theme::color(t.dim) } else { Theme::color(t.down) };
        let state = div()
            .flex()
            .items_center()
            .gap_2()
            .flex_none()
            .text_size(px(14.0))
            .line_height(gpui::relative(1.45))
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(state_text)
            .child(div().size(px(8.0)).rounded_full().flex_none().bg(state_color))
            .child(if device.connected { "connected" } else { "disconnected" }.to_string());
        if phone {
            // The phone device row wraps to two lines (React `flex-wrap`
            // with `order`): kind, name and state on the first; sub, strip,
            // count and Size on the second. GPUI has no flex order, so the
            // items are laid out in the same visual order with the names
            // shrunk by the flex, and the strip is 64 wide with the aspect
            // height of its viewBox (React `height: auto`).
            let mut row = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(4.0))
                .px(px(16.0))
                .py(px(4.0));
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(24.0))
                    .h(px(24.0))
                    .r_md()
                    .bg(Theme::color(t.raised))
                    .child(
                        gpui::svg()
                            .path(format!("icons/{}.svg", kind_icon(device.kind)))
                            .size(px(18.0))
                            .flex_none()
                            .text_color(accent),
                    ),
            );
            row = row.child(
                div()
                    .flex_initial()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(15.0))
                    .line_height(gpui::relative(1.45))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child(name.clone()),
            );
            row = row.child(div().w(px(100.0)).flex_none().child(state));
            row = row.child(
                div()
                    .flex_initial()
                    .w(px(150.0))
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(crate::shell::ellipsize(window, &sub, 150.0, 14.0, gpui::FontWeight::NORMAL, crate::fonts::MONO_FAMILY)),
            );
            if let Some(zone) = &one {
                // The phone strip is 64 wide with the height its viewBox
                // gives it (a linear strip: 64 × 22/200; a matrix: 64 ×
                // rows/cols).
                let box_h = match &zone.kind {
                    ZoneKind::Matrix { width, height, .. } => 64.0 * (*height as f32) / (*width as f32),
                    _ => 64.0 * 22.0 / 200.0,
                };
                row = row
                    .child(self.zone_strip(zone, accent, Some(64.0), box_h, false))
                    .child(
                        crate::fonts::with_tabular_figures(div())
                            .flex_none()
                            .w(px(59.0))
                            .text_size(px(15.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(format!("{} {}", zone.led_count, led_word(zone.led_count))),
                    );
            }
            // React draws the phone's Size slot only for a resizable zone. An empty
            // `ml_auto` slot on the wrapped row swallowed the row's 4 px gaps
            // (the strip and the count sat 4 and 8 px left of React's).
            if let Some(zone) = one.as_ref().filter(|z| z.resizable) {
                row = row.child(div().flex_none().flex().justify_end().ml_auto().child(self.size_key(&device.id.0, zone.index, cx)));
            }
            return row.into_any_element();
        }
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .h(px(row_h))
            .px(px(16.0));
        row = row.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(32.0))
                .h(px(32.0))
                .r_md()
                .bg(Theme::color(t.raised))
                .child(
                    gpui::svg()
                        .path(format!("icons/{}.svg", kind_icon(device.kind)))
                        .size(px(18.0))
                        .flex_none()
                        .text_color(accent),
               ),
        );
        // The desktop row's room, split the way React's flex splits it
        // (measured against the captures: see `device_split`). The
        // single-zone count is mono 14 at desktop (React
        // `.zone-strip-count` at `@media (min-width: 721px)`: `--fm` 14
        // tabular; no font-weight, so the body's 400). The count box's
        // min-width is 56, not the media
        // rule's 96: the `.rd-devices` rules (three classes) out-specify
        // the media rule's one class, so 56 wins (measured at 1024: with
        // 96 the "3 LEDs" strip sat 20px left of React's, 418-618 vs
        // 438-638; with 56 it lands on React's pixels).
        let count_for_split = one
            .as_ref()
            .map(|z| {
                let s = format!("{} {}", z.led_count, led_word(z.led_count));
                crate::shell::text_width(window, &s, 14.0, gpui::FontWeight::NORMAL, crate::fonts::MONO_FAMILY).max(56.0)
            })
            .unwrap_or(56.0);
        let state_word = if device.connected { "connected" } else { "disconnected" };
        let state_w = 16.0
            + crate::shell::text_width(window, state_word, 14.0, gpui::FontWeight::NORMAL, crate::fonts::UI_FAMILY);
        // A header row (no strip, no count) gives its sub-line the
        // space React's flex gives it: kind 32, four 12px gaps (48),
        // name 220, state, Size slot 104, inside the row's
        // `content_w - 32` (16px row padding each side); the sub is
        // `flex: 0 1 300px` min 96, so 300 unless the room shrinks it.
        let (sub_w, strip_w) = if one.is_none() {
            let available = (content_w - 32.0) - (32.0 + 48.0 + 220.0 + 104.0 + state_w);
            (available.clamp(96.0, 300.0), 0.0)
        } else {
            device_split(content_w - 32.0, count_for_split, state_w)
        };
        row = row.child(
            div()
                .flex_none()
                .w(px(220.0))
                .min_w(px(0.0))
                .truncate()
                .text_size(px(15.0))
                .line_height(gpui::relative(1.45))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child(crate::shell::ellipsize(window, &name, 220.0, 15.0, gpui::FontWeight::SEMIBOLD, crate::fonts::UI_FAMILY)),
        );
        row = row.child(
            div()
                .flex_none()
                .w(px(sub_w))
                .min_w(px(0.0))
                .truncate()
                .text_size(px(14.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.mute))
                .child(crate::shell::ellipsize(window, &sub, sub_w, 14.0, gpui::FontWeight::NORMAL, crate::fonts::MONO_FAMILY)),
        );
        if let Some(zone) = &one {
            row = row.child(self.zone_strip(zone, accent, Some(strip_w), 22.0, false));
            row = row.child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_end()
                    .flex_none()
                    .w(px(count_for_split))
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.45))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .whitespace_nowrap()
                    .child(format!("{} {}", zone.led_count, led_word(zone.led_count))),
            );
        }
        row = row.child(
            div()
                .flex_none()
                .w(px(state_w))
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(14.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(state_text)
                .child(div().size(px(8.0)).rounded_full().flex_none().bg(state_color))
                .child(state_word.to_string()),
        );
        let size_slot = div()
            .flex_none()
            .min_w(px(104.0))
            .flex()
            .justify_end();
        row = row.child(match &one {
            Some(zone) if zone.resizable => size_slot.child(self.size_key(&device.id.0, zone.index, cx)),
            _ => size_slot,
        });
        row.into_any_element()
    }

    /// One zone row of a multi-zone device (React `.rd-zone`).
    #[allow(clippy::too_many_arguments)]
    fn zone_row(
        &self,
        window: &Window,
        device: &DeviceInfo,
        zone: &ZoneInfo,
        accent: gpui::Rgba,
        phone: bool,
        row_h: f32,
        content_w: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = &self.theme;
        let count_main = format!("{} {}", zone.led_count, led_word(zone.led_count));
        let count_range = if zone.resizable {
            Some(format!(" · {}–{}", zone.min_leds, zone.max_leds))
        } else {
            None
        };
        // The count face follows React's breakpoint: at
        // `@media (min-width: 721px)` `.zone-strip-count` is mono 14
        // tabular right-aligned (no font-weight, so the body's 400);
        // under it (a phone viewport, the grid) it inherits the body
        // face, UI 15, in an auto column of the same box. Measured at
        // desktop in mono 14 normal, on the phone in UI 15 normal
        // (count regions at 1024/1440 read 27/7/7/7 px closer with
        // normal than with semibold). The box min is 56 at
        // desktop: the `.rd-devices` rules' three classes out-specify
        // the media rule's one, so the media rule's 96 never wins
        // (measured at 1024: 96 pulled the "3 LEDs" strip 20px left).
        let (m_size, m_family, m_weight, m_min) = if phone {
            (15.0, crate::fonts::UI_FAMILY, gpui::FontWeight::NORMAL, 0.0)
        } else {
            (14.0, crate::fonts::MONO_FAMILY, gpui::FontWeight::NORMAL, 56.0)
        };
        let count_w = {
            let mut w = crate::shell::text_width(window, &count_main, m_size, m_weight, m_family);
            if let Some(range) = &count_range {
                w += crate::shell::text_width(window, range, m_size, m_weight, m_family);
            }
            w.max(m_min)
        };
        // `zone_strip_width` already carries the row's 60/16 padding
        // and the fixed columns, so it takes the row's full width:
        // passing `content_w - 32` made the strip 32px too narrow and
        // pulled the count and Size keys 32px left (measured at
        // 1024: Size 904-952 vs React's 936-984, count end 835 vs
        // 868, JRGB dashes at 431 vs 447).
        let strip_w = zone_strip_width(content_w, count_w);
        let (c_size, c_family, c_weight) = if phone {
            (15.0, crate::fonts::UI_FAMILY, gpui::FontWeight::NORMAL)
        } else {
            (14.0, crate::fonts::MONO_FAMILY, gpui::FontWeight::NORMAL)
        };
        let count = {
            let mut c = div()
                .flex()
                .items_baseline()
                .flex_none()
                .w(px(count_w))
                .justify_end()
                .text_size(px(c_size))
                .line_height(gpui::relative(1.45))
                .font_weight(c_weight)
                .font_family(c_family)
                .whitespace_nowrap()
                .child(
                    crate::fonts::with_tabular_figures(div())
                        .font_weight(c_weight)
                        .font_family(c_family)
                        .text_color(Theme::color(t.ink))
                        .child(count_main),
                );
            if let Some(range) = count_range {
                c = c.child(
                    div()
                        .font_weight(c_weight)
                        .font_family(c_family)
                        .text_color(Theme::color(t.mute))
                        .child(range),
                );
            }
            c
        };
        let size_slot = div()
            .flex_none()
            .min_w(px(104.0))
            .flex()
            .justify_end();
        let size = if zone.resizable {
            size_slot.child(self.size_key(&device.id.0, zone.index, cx))
        } else {
            size_slot
        };
        if phone {
            // The phone zone row: a grid (React `.rd-zone` at `≤640px`):
            // name, count and Size on one line, the drawing 100% wide and
            // 16 px tall below. The name cell is the `1fr` column (`flex_1`
            // on the name box pushes the count to its column, left-aligned
            // in it); the Size slot keeps the desktop `min-width: 104px`
            // right-aligned key (only the device row resets it to 0 on a
            // phone), so the count ends at the slot's left minus the
            // `--s2` (8 px) gap. Vertical padding is `--s1` (4), right
            // `--s4` (16). Padding is `--s6` (24) on the left like the
            // stylesheet.
            //
            // React's `.zone-strip-name` keeps `justify-content: center`
            // from the wide rule (the phone rule only turns the box into a
            // row), so the name sits in the middle of its `1fr` cell, not
            // at a fixed inset: "JARGB1" reads at x 60 in a 95 px cell and
            // "JRGB1" at x 89 in a 142 px one (measured at 390 with
            // `-Row 138` and `-Row 306`).
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .pl(px(24.0))
                .pr(px(16.0))
                .py(px(4.0))
                .border_t_1()
                .border_color(Theme::color(t.line))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex()
                                .items_baseline()
                                .justify_center()
                                .line_height(gpui::relative(1.25))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(15.0))
                                        .font_family(crate::fonts::UI_FAMILY)
                                        .text_color(Theme::color(t.ink))
                                        .child(zone.name.clone()),
                                ),
                        )
                        .child(count)
                        .child(size),
                )
                .child(self.zone_strip(zone, accent, None, 16.0, false))
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap_3()
                .h(px(row_h))
                .pl(px(60.0))
                .pr_4()
                .border_t_1()
                .border_color(Theme::color(t.line))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .flex_none()
                        .w(px(116.0))
                        .min_w(px(116.0))
                        .line_height(gpui::relative(1.25))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(15.0))
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child(crate::shell::ellipsize(window, &zone.name, 116.0, 15.0, gpui::FontWeight::NORMAL, crate::fonts::UI_FAMILY)),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(14.0))
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child(crate::shell::ellipsize(window, &zone_shape(zone), 116.0, 14.0, gpui::FontWeight::NORMAL, crate::fonts::UI_FAMILY)),
                        ),
                )
                .child(self.zone_strip(zone, accent, Some(strip_w), 22.0, false))
                .child(count)
                .child(size)
                .into_any_element()
        }
    }

    /// The zone-empty row (React `.zone-empty` with `EmptyState`).
    fn zone_empty(&self, device: &DeviceInfo, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &self.theme;
        let entity = cx.entity();
        let try_again = button_icon(
            t,
            &format!("{}-try-again", device.id.0),
            "Try again",
            Variant::Default,
            false,
            true,
            "ArrowClockwise",
            "",
            16.0,
            move |_, _, cx| {
                entity.update(cx, |this, cx| this.refresh(cx));
            },
        );
        div()
            .px_4()
            .pt_2()
            .pb_3()
            .border_t_1()
            .border_color(Theme::color(t.line))
            .child(empty_state(
                t,
                &format!("{}-empty-zones", device.id.0),
                "Memory",
                &format!("{} reports no zones", device.name),
                "OpenRGB lists this device but no zones for it yet. Try again once it is lit.",
                Some(try_again),
            ))
            .into_any_element()
    }

    /// A bare icon key with an 18 px glyph (React `IconButton` with
    /// `size={18}`): the shared `icon_button` draws 16 px, so the walker
    /// draws its own.
    fn walker_icon_button(
        &self,
        id: &str,
        icon: &str,
        disabled: bool,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> gpui::Stateful<gpui::Div> {
        let t = &self.theme;
        let el = div()
            .id(gpui::SharedString::from(id.to_string()))
            .flex()
            .items_center()
            .justify_center()
            .size(px(32.0))
            .r_md()
            .text_color(Theme::color(t.dim))
            .child(gpui::svg().path(format!("icons/{icon}.svg")).size(px(18.0)).text_color(Theme::color(t.dim)));
        if disabled { el.opacity(0.4) } else { el.cursor_pointer().on_click(on_click) }
    }

    /// The identify walker (React `IdentifyWalker` in `.rd-walk`): the title,
    /// the LED readout, the "Set to" line, the keys, "This is the last LED",
    /// the range, the note and the error.
    fn walker(&self, _device: &DeviceInfo, zone: &ZoneInfo, phone: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &self.theme;
        let led = self.walker.led;
        let max_index = walker_ceiling(zone);
        let entity = cx.entity();
        let close_entity = entity.clone();
        let close = icon_button(
            t,
            "walker-close",
            "X",
            "",
            false,
            false,
            false,
            move |_, _, cx| {
                close_entity.update(cx, |this, cx| {
                    this.close_walk(cx);
                });
            },
        );
        // The readout: the big LED number and "of <max> LEDs" (fixed plural).
        let read = div()
            .flex()
            .items_baseline()
            .gap_2()
            .flex_none()
            .font_family(crate::fonts::MONO_FAMILY)
            .child(
                div()
                    .text_size(px(28.0))
                    .line_height(gpui::relative(1.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child(format!("{}", led + 1)),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(format!("of {} LEDs", zone.max_leds)),
            );
        let set_to = div()
            .flex()
            .items_baseline()
            .flex_none()
            .whitespace_nowrap()
            .text_size(px(14.0))
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.mute))
            .child("Set to ")
            .child(
                div()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .child(format!("{}", zone.led_count)),
            );
        let prev_entity = entity.clone();
        let prev = self.walker_icon_button("walker-prev", "ArrowLeft", led == 0, move |_, _, cx| {
            prev_entity.update(cx, |this, cx| this.step_led(-1, cx));
        });
        let next_entity = entity.clone();
        let next = self.walker_icon_button("walker-next", "ArrowRight", led >= max_index, move |_, _, cx| {
            next_entity.update(cx, |this, cx| this.step_led(1, cx));
        });
        let num_entity = entity.clone();
        let num = number_field(
            t,
            "walker-led-number",
            (led + 1) as f32,
            1.0,
            max_index as f32 + 1.0,
            1.0,
            None,
            move |v, _, cx| {
                num_entity.update(cx, |this, cx| {
                    let next = (v as i32 - 1).clamp(0, max_index as i32) as u32;
                    this.set_led(next, cx);
                });
            },
        );
        let keys = div()
            .flex()
            .items_center()
            .gap_2()
            .flex_none()
            .child(prev)
            .child(next)
            .child(div().w(px(64.0)).child(num));
        let last_entity = entity.clone();
        let last = button(
            t,
            "walker-last",
            "This is the last LED",
            Variant::Primary,
            false,
            move |_, _, cx| {
                last_entity.update(cx, |this, cx| this.set_length(cx));
            },
        );
        let range = div()
            .flex_none()
            .whitespace_nowrap()
            .text_size(px(14.0))
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.mute))
            .child(format!("{}–{}", zone.min_leds, zone.max_leds));
        let walker_title = || {
            tracked_title(
                &format!("IDENTIFY {}", zone.name.to_uppercase()),
                14.0,
                0.08,
                gpui::FontWeight::MEDIUM,
                crate::fonts::MONO_FAMILY,
                Theme::color(t.mute),
            )
        };
        let head = div()
            .flex()
            .items_center()
            .gap_3()
            .flex_none()
            .child(walker_title());
        let note = self.walker.note.clone().map(|n| {
            state_dot(t, "ok", &n).into_any_element()
        });
        let error = walker_error(self.walker.length_error.as_deref(), self.walker.identify_error.as_deref()).map(|e| {
            div()
                .text_size(px(14.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.down))
                .child(e)
                .into_any_element()
        });
        let mut tail = div().flex().flex_col().gap_1();
        if let Some(n) = note {
            tail = tail.child(n);
        }
        if let Some(e) = error {
            tail = tail.child(e);
        }
        let body = if phone {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(walker_title())
                        .child(close),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(read)
                        .child(keys),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(set_to)
                        .child(last),
                )
                .child(range)
                .child(tail)
        } else {
            div()
                .flex()
                .items_center()
                .gap_3()
                .flex_1()
                .min_w(px(0.0))
                .child(head)
                .child(read)
                .child(set_to)
                .child(keys)
                .child(last)
                .child(range)
                .child(tail)
                .child(close)
        };
        div()
            .border_t_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.raised))
            .p_2()
            .child(body)
            .into_any_element()
    }

    /// One device card (React `section.rd-dev`).
    /// One device card (React `section.rd-dev`).
    #[allow(clippy::too_many_arguments)]
    fn card(
        &self,
        window: &Window,
        device: &DeviceInfo,
        accent: gpui::Rgba,
        same_name: bool,
        phone: bool,
        row_h: f32,
        content_w: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = &self.theme;
        let mut card = div()
            .id(gpui::SharedString::from(format!("device-{}", device.id.0)))
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(Theme::color(t.line));
        card = card.child(self.device_row(window, device, accent, same_name, phone, row_h, content_w, cx));
        if device.zones.is_empty() {
            card = card.child(self.zone_empty(device, cx));
        }
        if device.zones.len() > 1 {
            for zone in &device.zones {
                card = card.child(self.zone_row(window, device, zone, accent, phone, row_h, content_w, cx));
            }
        }
        if let Some((wdev, wzone)) = self.walker_device_zone() {
            if wdev.id.0 == device.id.0 {
                card = card.child(self.walker(wdev, wzone, phone, cx));
            }
        }
        card.into_any_element()
    }

    /// The loading frame's placeholder (React `DeviceListShape`).
    fn loading_shape(&self, phone: bool) -> gpui::AnyElement {
        let t = &self.theme;
        let cards = loading_cards(self.total, self.devices.as_ref().map(|d| d.len()), self.counted);
        let skel = |w: f32, h: f32| {
            div().w(px(w)).h(px(h)).r_sm().bg(Theme::color(t.raised))
        };
        let mut list = div().flex().flex_col().gap_4().flex_1().min_h(px(0.0));
        for i in 0..cards {
            let zones = placeholder_zones(i);
            let mut card = div()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(Theme::color(t.line))
                .min_h(px(if phone { 206.0 } else { 140.0 }));
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .h(px(44.0))
                    .px_4()
                    .child(skel(32.0, 32.0))
                    .child(skel(200.0, 14.0))
                    .child(skel(180.0, 12.0))
                    .child(div().flex_1())
                    .child(skel(64.0, 14.0)),
            );
            if zones > 1 {
                for _ in 0..zones {
                    card = card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .h(px(44.0))
                            .pl(px(60.0))
                            .pr_4()
                            .border_t_1()
                            .border_color(Theme::color(t.line))
                            .child(skel(100.0, 14.0))
                            .child(div().flex_1().h(px(22.0)))
                            .child(skel(64.0, 14.0)),
                    );
                }
            }
            list = list.child(card);
        }
        list.into_any_element()
    }
}

impl Render for DevicesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let win_w = f32::from(window.window_bounds().get_bounds().size.width);
        let win_h = f32::from(window.window_bounds().get_bounds().size.height);
        let rail_w = if win_w <= 720.0 {
            0.0
        } else if win_w < 1280.0 {
            64.0
        } else {
            200.0
        };
        let content_w = win_w - rail_w - if win_w <= 720.0 { 32.0 } else { 48.0 };
        let phone = content_w <= 640.0;
        // The short-window rule (`@container page (max-height: 700px)`):
        // the container is `.content`, measured on its content box, which is
        // the window minus the strip (55) and the content padding (16 + 24).
        // Measured: at 1024×768 React's rows are 36 px (the page is 673);
        // at 1280+ they are 44.
        let row_h = if win_h - 95.0 <= 700.0 { 36.0 } else { 44.0 };
        let mut page = div().flex().flex_col().gap_4().flex_1().min_h(px(0.0));
        // The stale notice over the rows (React `rule rule-warn` with Details).
        if let (Some(_devices), Some(err)) = (&self.devices, &self.devices_error) {
            let entity = cx.entity();
            let details_open = self.details_open;
            let detail = div()
                .id(gpui::SharedString::from("devices-stale-details"))
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(14.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.mute))
                .child("Details")
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.details_open = !this.details_open;
                        cx.notify();
                    });
                });
            let detail = if details_open {
                detail.child(
                    div()
                        .text_size(px(14.0))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(err.clone()),
                )
            } else {
                detail
            };
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .r_md()
                    .border_1()
                    .border_color(Theme::color(t.warn))
                    .bg(Theme::color(t.warn_wash))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.dim))
                            .child("The device list may be out of date."),
                    )
                    .child(detail),
            );
        }
        if self.openrgb != LinkState::Connected {
            page = page.child(
                div()
                    .p_2()
                    .r_md()
                    .border_1()
                    .border_color(Theme::color(t.warn))
                    .bg(Theme::color(t.warn_wash))
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child("OpenRGB is not connected."),
            );
        }
        let state = frame_state(
            self.devices.is_some(),
            self.devices.as_ref().is_some_and(|d| d.is_empty()),
            self.devices_error.is_some(),
            self.retrying,
        );
        match state {
            "loading" => {
                page = page.child(self.loading_shape(phone));
            }
            "error" => {
                let entity = cx.entity();
                let retry = button_sm(
                    t,
                    "devices-error-try-again",
                    "Try again",
                    Variant::Default,
                    self.retrying,
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| this.retry(cx));
                    },
                );
                let entity2 = cx.entity();
                let details_open = self.details_open;
                let detail = div()
                    .id(gpui::SharedString::from("devices-error-details"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .mt_3()
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("Details")
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        entity2.update(cx, |this, cx| {
                            this.details_open = !this.details_open;
                            cx.notify();
                        });
                    });
                let detail = if details_open {
                    detail.child(
                        div()
                            .text_size(px(14.0))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child(self.devices_error.clone().unwrap_or_default()),
                    )
                } else {
                    detail
                };
                page = page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .p_4()
                        .r_lg()
                        .border_1()
                        .border_color(Theme::color(t.line))
                        .bg(Theme::color(t.panel))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .line_height(gpui::relative(1.25))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child("The devices could not be read"),
                        )
                        .child(
                            div()
                                .text_size(px(14.0))
                                .line_height(gpui::relative(1.45))
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.down))
                                .child("canvas-rgb could not read the devices. The lights are unaffected."),
                        )
                        .child(detail)
                        .child(retry),
                );
            }
            "empty" => {
                let entity = cx.entity();
                let try_again = button_icon(
                    t,
                    "devices-empty-try-again",
                    "Try again",
                    Variant::Default,
                    false,
                    true,
                    "ArrowClockwise",
                    "",
                    16.0,
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| this.refresh(cx));
                    },
                );
                page = page.child(
                    div()
                        .p_4()
                        .r_lg()
                        .border_1()
                        .border_color(Theme::color(t.line))
                        .bg(Theme::color(t.panel))
                        .child(empty_state(
                            t,
                            "devices-empty",
                            "Memory",
                            "No device found",
                            "OpenRGB reports no lights. Plug one in or start OpenRGB, then try again.",
                            Some(try_again),
                        )),
                );
            }
            _ => {
                let devices = self.devices.clone().unwrap_or_default();
                let sorted = sort_devices(&devices);
                let names: Vec<String> = sorted.iter().map(|d| d.name.clone()).collect();
                let wide_two = !phone && content_w >= 1500.0;
                // Build the cards into the right container.
                let cards: Vec<gpui::AnyElement> = sorted
                    .iter()
                    .enumerate()
                    .map(|(i, device)| {
                        let same_name = names.iter().filter(|n| *n == &device.name).count() > 1;
                        let accent = accent_for(t, i);
                        // In the two-column rule each card lays out against
                        // half the content width (the columns' `560px` minimum
                        // is wider in practice), so the row split below uses
                        // the column's width, not the page's.
                        let col_w = if wide_two { (content_w - 16.0) / 2.0 } else { content_w };
                        self.card(window, device, accent, same_name, phone, row_h, col_w, cx)
                    })
                    .collect();
                let mut list = div()
                    .id(gpui::SharedString::from("devices-list"))
                    .flex()
                    .flex_1()
                    .min_h(px(0.0));
                if wide_two {
                    // The wide rule: the list becomes two columns (`columns: 2
                    // 560px`), a device never splitting across them. The
                    // columns are balanced on card heights, filled in card
                    // order (measured at 1920: the MSI card alone in the left
                    // column, the four single-row cards stacked on the right).
                    let heights: Vec<f32> = sorted
                        .iter()
                        .map(|d| {
                            let walk = self
                                .walker_device_zone()
                                .is_some_and(|(wdev, _)| wdev.id.0 == d.id.0);
                            card_height(d, row_h, walk)
                        })
                        .collect();
                    let split = balanced_split(&heights);
                    let col_w = (content_w - 16.0) / 2.0;
                    let cols = div().flex().gap_x(px(16.0));
                    let mut col1 = div().flex().flex_col().w(px(col_w)).flex_none();
                    let mut col2 = div().flex().flex_col().w(px(col_w)).flex_none();
                    for (i, card) in cards.into_iter().enumerate() {
                        if i < split {
                            col1 = col1.child(card);
                        } else {
                            col2 = col2.child(card);
                        }
                    }
                    list = list.child(cols.child(col1).child(col2));
                } else {
                    list = list.flex_col();
                    if !phone {
                        list = list.overflow_y_scroll();
                    }
                    for card in cards {
                        list = list.child(card);
                    }
                }
                page = page.child(list);
            }
        }
        page
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::DeviceId;

    fn zone(index: u32, name: &str, kind: ZoneKind, led_count: u32, min_leds: u32, max_leds: u32, resizable: bool) -> ZoneInfo {
        ZoneInfo { index, name: name.into(), kind, led_count, min_leds, max_leds, resizable }
    }

    fn device(id: &str, name: &str, kind: DeviceKind, connected: bool, location: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(id.into()),
            name: name.into(),
            vendor: "V".into(),
            kind,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: String::new(),
            location: location.into(),
            zones,
            connected,
        }
    }

    fn linear(led: u32) -> ZoneInfo {
        zone(0, "Z", ZoneKind::Linear, led, 0, 240, true)
    }

    #[test]
    fn disconnected_first_then_by_kind_rank_stable() {
        let devices = vec![
            device("a", "Mouse", DeviceKind::Mouse, true, "loc", vec![linear(1)]),
            device("b", "Board", DeviceKind::Motherboard, true, "loc", vec![linear(1)]),
            device("c", "Old RAM", DeviceKind::Dram, false, "loc", vec![linear(1)]),
            device("d", "RAM", DeviceKind::Dram, true, "loc", vec![linear(1)]),
            device("e", "Old board", DeviceKind::Motherboard, false, "loc", vec![linear(1)]),
        ];
        let sorted = sort_devices(&devices);
        let names: Vec<&str> = sorted.iter().map(|d| d.name.as_str()).collect();
        // Disconnected first, then by kind rank inside each group too
        // (React sorts connected first, then the rank); the daemon's order
        // stands only inside a rank.
        assert_eq!(names, vec!["Old board", "Old RAM", "Board", "RAM", "Mouse"]);
        // the daemon's order stands inside a rank: the two dram devices keep
        // their input order (c before d is across the connected split, so use
        // two connected drams here).
        let drams = vec![
            device("x", "RAM2", DeviceKind::Dram, true, "loc", vec![linear(1)]),
            device("y", "RAM1", DeviceKind::Dram, true, "loc", vec![linear(1)]),
        ];
        let sorted = sort_devices(&drams);
        assert_eq!(sorted[0].name, "RAM2");
        assert_eq!(sorted[1].name, "RAM1");
    }

    #[test]
    fn kind_ranks_match_react() {
        assert_eq!(kind_rank(DeviceKind::Motherboard), 0);
        assert_eq!(kind_rank(DeviceKind::Dram), 1);
        assert_eq!(kind_rank(DeviceKind::Gpu), 2);
        assert_eq!(kind_rank(DeviceKind::Other), 3);
        assert_eq!(kind_rank(DeviceKind::LedStrip), 4);
        for kind in [DeviceKind::Keyboard, DeviceKind::Mouse, DeviceKind::Microphone, DeviceKind::Headset] {
            assert_eq!(kind_rank(kind), 5);
        }
    }

    #[test]
    fn kind_labels_and_icons_cover_every_kind() {
        let cases = [
            (DeviceKind::Motherboard, "Motherboard", "Cpu"),
            (DeviceKind::Dram, "DRAM", "Memory"),
            (DeviceKind::Keyboard, "Keyboard", "Keyboard"),
            (DeviceKind::Mouse, "Mouse", "Mouse"),
            (DeviceKind::Microphone, "Microphone", "Microphone"),
            (DeviceKind::Headset, "Headset", "Microphone"),
            (DeviceKind::LedStrip, "LED strip", "Lightbulb"),
            (DeviceKind::Gpu, "GPU", "GraphicsCard"),
            (DeviceKind::Other, "Other", "Plugs"),
        ];
        for (kind, label, icon) in cases {
            assert_eq!(kind_label(kind), label);
            assert_eq!(kind_icon(kind), icon);
        }
    }

    #[test]
    fn accents_cycle_every_five_by_position() {
        let t = Theme::dark();
        assert_eq!(accent_for(&t, 0), Theme::color(t.a1));
        assert_eq!(accent_for(&t, 4), Theme::color(t.a5));
        assert_eq!(accent_for(&t, 5), Theme::color(t.a1));
        assert_eq!(accent_for(&t, 12), Theme::color(t.a3));
    }

    #[test]
    fn tag_takes_the_last_hex_run_in_the_location() {
        assert_eq!(
            device_tag("I2C: PawnIO SMBus PIIX4 0, address 0x71").as_deref(),
            Some("0x71")
        );
        assert_eq!(device_tag("HID: #vid_1 and 0xAB").as_deref(), Some("0xAB"));
        assert_eq!(device_tag("no hex here"), None);
        assert_eq!(device_tag("0x1 0x2").as_deref(), Some("0x2"));
    }

    #[test]
    fn name_gets_a_tag_only_when_another_device_shares_it() {
        let d = device("id", "ENE DRAM", DeviceKind::Dram, true, "addr 0x71", vec![]);
        assert_eq!(display_name(&d, true, Some("0x71")), "ENE DRAM 0x71");
        assert_eq!(display_name(&d, false, Some("0x71")), "ENE DRAM");
        assert_eq!(display_name(&d, true, None), "ENE DRAM");
    }

    #[test]
    fn sub_line_is_kind_vendor_adapter_and_tag() {
        let d = device("id", "ENE DRAM", DeviceKind::Dram, true, "loc", vec![]);
        assert_eq!(sub_line(&d, Some("0x71")), "DRAM · V · openrgb #0 · 0x71");
        assert_eq!(sub_line(&d, None), "DRAM · V · openrgb #0");
    }

    #[test]
    fn shape_words_cover_each_zone_kind() {
        assert_eq!(zone_shape(&linear(1)), "linear");
        let single = zone(0, "Z", ZoneKind::Single, 1, 1, 1, false);
        assert_eq!(zone_shape(&single), "single");
        let matrix = zone(0, "Z", ZoneKind::Matrix { width: 8, height: 4, map: vec![] }, 1, 1, 1, false);
        assert_eq!(zone_shape(&matrix), "matrix 8 × 4");
    }

    #[test]
    fn led_word_is_singular_only_for_one() {
        assert_eq!(led_word(0), "LEDs");
        assert_eq!(led_word(1), "LED");
        assert_eq!(led_word(2), "LEDs");
    }

    #[test]
    fn counts_only_feed_from_connected_devices() {
        let devices = vec![
            device("a", "A", DeviceKind::Dram, true, "loc", vec![zone(0, "Z", ZoneKind::Linear, 8, 8, 8, false)]),
            device("b", "B", DeviceKind::Dram, false, "loc", vec![zone(0, "Z", ZoneKind::Linear, 8, 8, 8, false)]),
        ];
        assert_eq!(connected_count(&devices), 1);
        assert_eq!(lit_count(&devices), 8);
        assert_eq!(context_text(Some(&devices)), "1/2 connected · 8 LEDs");
        assert_eq!(context_text(None), "0/0 connected · 0 LEDs");
    }

    #[test]
    fn a_single_led_still_reads_leds_in_the_context() {
        let devices = vec![device("a", "A", DeviceKind::Dram, true, "loc", vec![zone(0, "Z", ZoneKind::Single, 1, 1, 1, false)])];
        assert_eq!(context_text(Some(&devices)), "1/1 connected · 1 LEDs");
    }

    #[test]
    fn loading_cards_follow_total_then_listed_then_counted() {
        assert_eq!(loading_cards(Some(12), Some(5), Some(3)), 12);
        assert_eq!(loading_cards(Some(0), Some(5), Some(3)), 0);
        assert_eq!(loading_cards(None, Some(5), Some(3)), 5);
        assert_eq!(loading_cards(None, None, Some(3)), 3);
        assert_eq!(loading_cards(None, None, Some(0)), 3);
        assert_eq!(loading_cards(None, None, None), 3);
    }

    #[test]
    fn placeholder_rhythm_gives_card_three_four_rows() {
        assert_eq!(placeholder_zones(0), 1);
        assert_eq!(placeholder_zones(3), 4);
        assert_eq!(placeholder_zones(15), 4);
        assert_eq!(placeholder_zones(12), 1);
    }

    #[test]
    fn strip_draws_at_most_240_but_the_text_reads_the_real_count() {
        assert_eq!(drawn_count(300), 240);
        assert_eq!(drawn_count(30), 30);
        assert_eq!(drawn_count(0), 0);
    }

    #[test]
    fn strip_modes_cover_each_zone_kind_and_size() {
        let single = zone(0, "Z", ZoneKind::Single, 2, 2, 2, false);
        assert_eq!(strip_mode(&single), StripMode::Single);
        let empty = zone(0, "Z", ZoneKind::Linear, 0, 0, 240, true);
        assert_eq!(strip_mode(&empty), StripMode::Dashed);
        let dots = zone(0, "Z", ZoneKind::Linear, 8, 8, 8, false);
        match strip_mode(&dots) {
            StripMode::Dots { step, radius } => {
                assert!((step - 25.0).abs() < 0.001);
                assert!(radius > 0.0 && radius <= 4.0);
            }
            other => panic!("expected dots, got {other:?}"),
        }
        // a single dot is at the radius cap 4.0 (step 200, step * 0.3 = 60)
        let one = zone(0, "Z", ZoneKind::Linear, 1, 1, 1, false);
        match strip_mode(&one) {
            StripMode::Dots { radius, .. } => assert!((radius - 4.0).abs() < 0.001),
            other => panic!("expected dots, got {other:?}"),
        }
        let bar = zone(0, "Z", ZoneKind::Linear, 30, 0, 120, true);
        assert_eq!(strip_mode(&bar), StripMode::Bar);
        let capped = zone(0, "Z", ZoneKind::Linear, 300, 0, 300, true);
        assert_eq!(strip_mode(&capped), StripMode::Bar);
        let matrix = zone(0, "Z", ZoneKind::Matrix { width: 8, height: 4, map: vec![] }, 1, 1, 1, false);
        assert_eq!(strip_mode(&matrix), StripMode::Matrix { cols: 8, rows: 4 });
    }

    #[test]
    fn walk_target_prefers_the_first_resizable_zone_with_leds() {
        let devices = vec![device(
            "openrgb:board",
            "MSI",
            DeviceKind::Motherboard,
            true,
            "loc",
            vec![
                zone(0, "JAF", ZoneKind::Linear, 0, 0, 240, true),
                zone(1, "JARGB 1", ZoneKind::Linear, 120, 0, 240, true),
            ],
        )];
        assert_eq!(
            walk_target_for(&devices, "openrgb:board"),
            Some(("openrgb:board".to_string(), 1))
        );
        assert_eq!(walk_target_for(&devices, "openrgb:missing"), None);
    }

    #[test]
    fn walk_target_falls_back_to_the_first_resizable_zone() {
        let devices = vec![device(
            "openrgb:strip",
            "Strip",
            DeviceKind::LedStrip,
            true,
            "loc",
            vec![
                zone(0, "Fixed", ZoneKind::Linear, 10, 10, 10, false),
                zone(1, "Channel 1", ZoneKind::Linear, 0, 0, 120, true),
            ],
        )];
        assert_eq!(
            walk_target_for(&devices, "openrgb:strip"),
            Some(("openrgb:strip".to_string(), 1))
        );
    }

    #[test]
    fn walker_steps_floor_at_zero_and_ceiling_at_max_minus_one() {
        let z = zone(0, "JARGB", ZoneKind::Linear, 120, 0, 240, true);
        assert_eq!(walker_ceiling(&z), 239);
        assert_eq!(clamp_led(-1, &z), 0);
        assert_eq!(clamp_led(9999, &z), 239);
        assert_eq!(clamp_led(5, &z), 5);
        let zero = zone(0, "Z", ZoneKind::Linear, 0, 0, 0, true);
        assert_eq!(walker_ceiling(&zero), 0);
        assert_eq!(clamp_led(10, &zero), 0);
    }

    #[test]
    fn walker_errors_join_with_one_space_and_clear_alone() {
        assert_eq!(walker_error(None, None), None);
        assert_eq!(walker_error(Some("len"), None).as_deref(), Some("len"));
        assert_eq!(walker_error(None, Some("id")).as_deref(), Some("id"));
        assert_eq!(walker_error(Some("len"), Some("id")).as_deref(), Some("len id"));
    }

    #[test]
    fn walker_note_names_the_zone_and_the_new_count() {
        let z = zone(0, "JARGB 1", ZoneKind::Linear, 120, 0, 240, true);
        assert_eq!(walker_note(&z, 119), "JARGB 1 is now 120 LEDs.");
    }

    #[test]
    fn resend_interval_is_two_seconds() {
        assert_eq!(RESEND_MS, 2000);
    }

    #[test]
    fn balanced_split_puts_the_tall_card_alone() {
        // The 1920 capture: 264 + 4×44 = 440; [264][176] beats [308][132].
        let heights = [264.0, 44.0, 44.0, 44.0, 44.0];
        assert_eq!(balanced_split(&heights), 1);
    }

    #[test]
    fn card_height_counts_header_and_zones() {
        let multi = device(
            "id",
            "MSI",
            DeviceKind::Motherboard,
            true,
            "loc",
            vec![
                zone(0, "A", ZoneKind::Linear, 0, 0, 240, true),
                zone(1, "B", ZoneKind::Linear, 0, 0, 240, true),
            ],
        );
        // Two zones: header + 2 rows (44 + 88), no walker.
        assert_eq!(card_height(&multi, 44.0, false), 132.0);
        let single = device("id", "S", DeviceKind::Dram, true, "loc", vec![linear(1)]);
        assert_eq!(card_height(&single, 44.0, false), 44.0);
        assert_eq!(card_height(&single, 44.0, true), 184.0);
    }

    #[test]
    fn frame_state_follows_the_loadframe_rules() {
        // No list read, error set, no retry: the error panel.
        assert_eq!(frame_state(false, false, true, false), "error");
        // A Try again in flight puts the frame back to loading.
        assert_eq!(frame_state(false, false, true, true), "loading");
        // First read still in flight.
        assert_eq!(frame_state(false, false, false, false), "loading");
        // Empty list, then ready.
        assert_eq!(frame_state(true, true, false, false), "empty");
        assert_eq!(frame_state(true, false, true, false), "ready");
        assert_eq!(frame_state(true, false, false, false), "ready");
    }

    #[test]
    fn device_split_grows_the_strip_past_the_sub() {
        // 1440: the sub is unshrunk at 300, the strip takes the rest.
        let (sub, strip) = device_split(1160.0, 50.0, 84.0);
        assert_eq!(sub, 300.0);
        assert!((strip - (1160.0 - (32.0 + 72.0 + 220.0 + 104.0 + 50.0 + 84.0) - 300.0)).abs() < 0.01);
    }

    #[test]
    fn device_split_shrinks_proportionally_at_1024() {
        // 1024: sub 190.8, strip 127.2, count lands on React's 716.
        let (sub, strip) = device_split(880.0, 50.0, 84.0);
        assert!((sub - 190.8).abs() < 0.5, "sub {sub}");
        assert!((strip - 127.2).abs() < 0.5, "strip {strip}");
    }

    #[test]
    fn zone_strip_takes_the_row_leftover_above_its_floor() {
        // 1440 zone row: everything the room leaves the strip.
        let w = zone_strip_width(1180.0, 105.0);
        assert!((w - (1180.0 - (60.0 + 116.0 + 36.0 + 104.0 + 16.0) - 105.0)).abs() < 0.01);
        assert_eq!(zone_strip_width(300.0, 100.0), 64.0);
    }

    #[test]
    fn identify_and_patch_bodies_match_the_spine_shapes() {
        let id = IdentifyRequest { zone: 1, led: 5, count: None, stop: false };
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            r#"{"zone":1,"led":5,"stop":false}"#
        );
        let stop = IdentifyRequest { zone: 1, led: 0, count: None, stop: true };
        assert_eq!(
            serde_json::to_string(&stop).unwrap(),
            r#"{"zone":1,"led":0,"stop":true}"#
        );
        let patch = PatchZone { led_count: 120 };
        assert_eq!(serde_json::to_string(&patch).unwrap(), r#"{"led_count":120}"#);
    }
}
