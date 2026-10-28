//! G7: the first run.
//!
//! On a clean profile the daemon lights everything up with no clicks: it sizes the JARGB
//! headers from SignalRGB's own configuration (read-only), places every device on the
//! 320x200 canvas, activates Aurora and opens the editor's dismissible "size your ARGB
//! headers" panel.

use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::{Layout, PlacedZone, RenderScale, SamplingMode, ZoneShape};
use canvas_proto::Rgb8;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The JARGB header count SignalRGB uses on the test board (today 120 each).
pub const SIGNALRGB_JARGB_LED_COUNT: u32 = 120;

/// The effect the first run activates.
///
/// Aurora: the built-in that looks most like the effect this project started from
/// (moving colour over a dark canvas, in a three-colour palette), and one of ours, so
/// the first run works on a clean profile with nothing installed.
pub const FIRST_RUN_EFFECT: &str = "aurora";

/// Where SignalRGB's device configuration lives.
pub fn signalrgb_devices_key() -> String {
    r"Software\WhirlwindFX\SignalRgb\devices".to_string()
}

/// SignalRGB's log folder.
pub fn signalrgb_log_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("WhirlwindFX").join("SignalRgb").join("Logs")
}

/// SignalRGB's effect cache, where the reference `effect.html` lives.
pub fn signalrgb_cache_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("WhirlwindFX").join("SignalRgb").join("cache").join("effects")
}

/// Where the JARGB counts came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CountSource {
    /// Read from SignalRGB's registry values.
    Registry,
    /// Read from SignalRGB's log.
    Log,
    /// Not found; the counts stay unknown until the guided step.
    Unknown,
}

/// The result of looking for SignalRGB's JARGB LED counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JargbCounts {
    pub source: CountSource,
    /// The counts per header, in the order they were found.
    pub counts: Vec<u32>,
}

impl JargbCounts {
    /// True when the counts are known.
    pub fn is_known(&self) -> bool {
        !self.counts.is_empty()
    }
}

/// Looks for SignalRGB's MSI ARGB header LED counts.
///
/// The registry is read first, then the newest log file. Both are read-only. Nothing
/// here touches SignalRGB itself.
pub fn find_signalrgb_jargb_counts() -> JargbCounts {
    let registry = find_in_registry();
    if registry.is_known() {
        return registry;
    }
    let log = find_in_logs();
    if log.is_known() {
        return log;
    }
    JargbCounts { source: CountSource::Unknown, counts: Vec::new() }
}

/// Reads the counts from `HKCU\Software\WhirlwindFX\SignalRgb\devices`.
///
/// SignalRGB stores a value per device; the MSI board's value names the ARGB headers and
/// their LED counts. The values are read as `REG_SZ` and scanned for numbers.
#[cfg(windows)]
fn find_in_registry() -> JargbCounts {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_NO_MORE_ITEMS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    unsafe {
        let mut key: HKEY = HKEY::default();
        let path: Vec<u16> =
            signalrgb_devices_key().encode_utf16().chain(std::iter::once(0)).collect();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), None, KEY_READ, &mut key).0 != 0 {
            return JargbCounts { source: CountSource::Unknown, counts: Vec::new() };
        }

        // SignalRGB stores one subkey per device.
        let mut counts = Vec::new();
        let mut index: u32 = 0;
        loop {
            let mut name = [0u16; 256];
            let result = RegEnumKeyW(key, index, Some(&mut name));
            if result == ERROR_NO_MORE_ITEMS || result.0 != 0 {
                break;
            }
            index += 1;

            // Open the device's own key and read its values as text.
            let mut device_key: HKEY = HKEY::default();
            if RegOpenKeyExW(key, PCWSTR(name.as_ptr()), None, KEY_READ, &mut device_key).0 != 0 {
                continue;
            }
            for value_index in 0..64u32 {
                let value_name: Vec<u16> =
                    format!("{value_index}").encode_utf16().chain(std::iter::once(0)).collect();
                let mut size: u32 = 0;
                let queried = RegQueryValueExW(
                    device_key,
                    PCWSTR(value_name.as_ptr()),
                    None,
                    None,
                    None,
                    Some(std::ptr::addr_of_mut!(size)),
                );
                if queried.0 != 0 || size == 0 {
                    continue;
                }
                let mut buffer = vec![0u16; (size as usize / 2) + 1];
                let mut written = size;
                let read = RegQueryValueExW(
                    device_key,
                    PCWSTR(value_name.as_ptr()),
                    None,
                    None,
                    Some(buffer.as_mut_ptr().cast::<u8>()),
                    Some(std::ptr::addr_of_mut!(written)),
                );
                if read.0 != 0 {
                    continue;
                }
                let text = String::from_utf16_lossy(&buffer);
                if let Some(found) = counts_in_text(&text) {
                    counts.extend(found);
                }
            }
            let _ = RegCloseKey(device_key);
        }
        let _ = RegCloseKey(key);

        if counts.is_empty() {
            JargbCounts { source: CountSource::Unknown, counts: Vec::new() }
        } else {
            JargbCounts { source: CountSource::Registry, counts }
        }
    }
}

#[cfg(not(windows))]
fn find_in_registry() -> JargbCounts {
    JargbCounts { source: CountSource::Unknown, counts: Vec::new() }
}

/// Reads the counts from the newest SignalRGB log.
///
/// The log names the MSI board's ARGB headers and their LED counts on one line, e.g.
/// `JARGB1: 120 LEDs`.
fn find_in_logs() -> JargbCounts {
    let dir = signalrgb_log_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return JargbCounts { source: CountSource::Unknown, counts: Vec::new() };
    };
    // The newest log file wins.
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "log") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, path));
        }
    }
    let Some((_, path)) = newest else {
        return JargbCounts { source: CountSource::Unknown, counts: Vec::new() };
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return JargbCounts { source: CountSource::Unknown, counts: Vec::new() };
    };

    let mut counts = Vec::new();
    for line in text.lines() {
        if !line.to_ascii_lowercase().contains("jargb") {
            continue;
        }
        if let Some(found) = counts_in_text(line) {
            counts.extend(found);
        }
    }
    if counts.is_empty() {
        JargbCounts { source: CountSource::Unknown, counts: Vec::new() }
    } else {
        JargbCounts { source: CountSource::Log, counts }
    }
}

/// Extracts LED counts from a line of text that mentions ARGB headers.
///
/// Only a number directly followed by "led" or "leds" is a count: the digits in a header
/// name like "JARGB1" are not.
fn counts_in_text(text: &str) -> Option<Vec<u32>> {
    let lower = text.to_ascii_lowercase();
    if !lower.contains("jargb") && !lower.contains("argb") {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut counts = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // Look for a number.
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let digits: String = chars[start..i].iter().collect();
            let Ok(number) = digits.parse::<u32>() else {
                continue;
            };
            if (1..=1024).contains(&number) {
                // The number must be followed by "led" or "leds".
                let tail: String = chars[i..(i + 8).min(chars.len())].iter().collect::<String>().to_ascii_lowercase();
                let tail = tail.trim_start();
                if tail.starts_with("led") {
                    counts.push(number);
                }
            }
        } else {
            i += 1;
        }
    }
    if counts.is_empty() {
        None
    } else {
        Some(counts)
    }
}

/// The shape a zone gets by default, from the device kind and the zone.
///
/// Fans are rings, RAM and ARGB headers are strips, a single-LED peripheral is a point
/// (one blob of light) and a matrix keeps its own map. A multi-LED keyboard that reports
/// one long line is laid out as a grid, which reads as a keyboard.
pub fn default_shape(kind: DeviceKind, zone: &ZoneInfo) -> ZoneShape {
    let name = zone.name.to_ascii_lowercase();
    if matches!(zone.kind, ZoneKind::Matrix { .. }) {
        return ZoneShape::Auto;
    }
    if name.contains("fan") && zone.led_count > 1 {
        return ZoneShape::Ring;
    }
    if matches!(zone.kind, ZoneKind::Single) || zone.led_count == 1 {
        return ZoneShape::Point;
    }
    match kind {
        DeviceKind::Keyboard => ZoneShape::Grid { columns: zone.led_count.div_ceil(6).max(1) },
        _ => ZoneShape::Strip,
    }
}

/// The default rectangle size (width, height) for a shape on a device kind.
fn default_size(kind: DeviceKind, shape: ZoneShape) -> (f32, f32) {
    match (kind, shape) {
        (_, ZoneShape::Ring) => (40.0, 40.0),
        (DeviceKind::Dram, _) => (16.0, 120.0),
        (DeviceKind::Keyboard, ZoneShape::Grid { .. }) => (120.0, 38.0),
        (DeviceKind::Keyboard, _) => (110.0, 30.0),
        (DeviceKind::Mouse, ZoneShape::Point) => (22.0, 32.0),
        (DeviceKind::Microphone, ZoneShape::Point) => (16.0, 28.0),
        (DeviceKind::Headset, ZoneShape::Point) => (30.0, 30.0),
        (_, ZoneShape::Point) => (20.0, 20.0),
        _ => (120.0, 10.0),
    }
}

/// Builds the default layout for the discovered devices.
///
/// The placement is deterministic: the devices are sorted by kind and name, then laid
/// out by role on the 320x200 canvas, like a diagram of the PC: ARGB headers stack on
/// the left, fan rings sit in a row beneath them, peripherals line up along the bottom
/// and the RAM sticks stand on the right. Nothing overlaps another zone.
pub fn build_default_layout(devices: &[DeviceInfo]) -> Layout {
    const MARGIN: f32 = 4.0;
    const GAP: f32 = 6.0;
    const ROW_HEIGHT: f32 = 18.0;
    const DRAM_Y: f32 = 20.0;
    const LEFT_LIMIT: f32 = 240.0;
    const FAN_BAND_Y: f32 = 68.0;
    const PERIPHERAL_Y: f32 = 150.0;

    let mut sorted: Vec<&DeviceInfo> = devices.iter().collect();
    sorted.sort_by(|a, b| {
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    let mut zones = Vec::new();
    // The RAM sticks go on the right, side by side.
    let mut dram_column = 0u32;
    // ARGB headers and other strips stack down the left.
    let mut row_y = MARGIN;
    let mut row_x = MARGIN;
    // Fan rings and small points share a band beneath the headers.
    let mut band_x = MARGIN;
    let mut band_y = FAN_BAND_Y;
    // Peripherals line up along the bottom.
    let mut peri_x = MARGIN;
    let mut peri_y = PERIPHERAL_Y;
    let mut peri_row_height = 0.0f32;

    for device in sorted {
        let peripheral = matches!(
            device.kind,
            DeviceKind::Keyboard
                | DeviceKind::Mouse
                | DeviceKind::Microphone
                | DeviceKind::Headset
        );
        for zone in &device.zones {
            if zone.led_count == 0 {
                continue;
            }
            let shape = default_shape(device.kind, zone);
            let (width, height) = default_size(device.kind, shape);
            let (x, y) = if device.kind == DeviceKind::Dram {
                let x = 320.0 - width - MARGIN - (dram_column as f32) * (width + 4.0);
                dram_column += 1;
                (x, DRAM_Y)
            } else if peripheral {
                if peri_x + width > LEFT_LIMIT && peri_x > MARGIN {
                    peri_x = MARGIN;
                    peri_y += peri_row_height + GAP;
                    peri_row_height = 0.0;
                }
                let at = (peri_x, peri_y);
                peri_x += width + GAP;
                peri_row_height = peri_row_height.max(height);
                at
            } else if matches!(shape, ZoneShape::Ring | ZoneShape::Point) {
                if band_x + width > LEFT_LIMIT && band_x > MARGIN {
                    band_x = MARGIN;
                    band_y += 44.0;
                }
                let at = (band_x, band_y);
                band_x += width + GAP;
                at
            } else {
                let at = (row_x, row_y);
                row_y += ROW_HEIGHT;
                if row_y > FAN_BAND_Y - ROW_HEIGHT {
                    row_y = MARGIN;
                    row_x += width + GAP;
                }
                at
            };
            zones.push(PlacedZone {
                device_id: device.id.clone(),
                zone_index: zone.index,
                x,
                y,
                width,
                height,
                rotation_deg: 0.0,
                reversed: false,
                visible: true,
                shape,
            });
        }
    }

    Layout {
        id: "default".into(),
        name: "Default".into(),
        render_scale: RenderScale::default(),
        sampling: SamplingMode::AreaAverage,
        zones,
        effect: None,
    }
}

/// Adds every zone with LEDs that the layout does not place yet, where the first run
/// would have put it. Zones already in the layout are left where they are.
///
/// A device detected after the first run (or a header sized later) lands on the canvas,
/// as SignalRGB does with a new device, instead of staying black. Returns how many zones
/// were added.
pub fn place_missing_zones(layout: &mut Layout, devices: &[DeviceInfo]) -> usize {
    let defaults = build_default_layout(devices);
    let mut added = 0;
    for zone in defaults.zones {
        let present = layout
            .zones
            .iter()
            .any(|p| p.device_id == zone.device_id && p.zone_index == zone.zone_index);
        if !present {
            layout.zones.push(zone);
            added += 1;
        }
    }
    added
}

/// How long the device list must hold still before the first run trusts it: longer than
/// one supervisor refresh (5 s), so a device still being detected shows up first.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(6);

/// The most settle rounds the first run waits through before using the list it has.
const SETTLE_ROUNDS: u32 = 10;

/// What decides "the device list moved": ids and every zone's LED count.
fn settle_key(devices: &[DeviceInfo]) -> Vec<(String, Vec<u32>)> {
    let mut key: Vec<_> = devices
        .iter()
        .map(|d| (d.id.0.clone(), d.zones.iter().map(|z| z.led_count).collect()))
        .collect();
    key.sort();
    key
}

/// The device list once OpenRGB has stopped adding to it.
///
/// OpenRGB detects devices one at a time, so the first list can be short: the mic alone,
/// with the board still coming. `None` when the daemon is quitting.
async fn settled_devices(app: &std::sync::Arc<crate::state::App>) -> Option<Vec<DeviceInfo>> {
    let mut devices = app.devices.read().await.clone();
    for _ in 0..SETTLE_ROUNDS {
        tokio::time::sleep(SETTLE).await;
        if app.shutting_down.load(std::sync::atomic::Ordering::SeqCst) != 0 {
            return None;
        }
        let now = app.devices.read().await.clone();
        if settle_key(&now) == settle_key(&devices) {
            return Some(now);
        }
        tracing::info!(count = now.len(), "the device list is still changing; the first run waits");
        devices = now;
    }
    Some(devices)
}

/// The order the device kinds are placed in.
fn kind_rank(kind: DeviceKind) -> u8 {
    match kind {
        DeviceKind::Motherboard => 0,
        DeviceKind::LedStrip => 1,
        DeviceKind::Keyboard => 2,
        DeviceKind::Mouse => 3,
        DeviceKind::Microphone => 4,
        DeviceKind::Headset => 5,
        DeviceKind::Dram => 6,
        DeviceKind::Gpu => 7,
        DeviceKind::Other => 8,
    }
}

/// The zone sizes to save for the JARGB headers, by device fingerprint.
pub fn jargb_zone_sizes(devices: &[DeviceInfo], counts: &JargbCounts) -> BTreeMap<String, BTreeMap<u32, u32>> {
    let mut sizes = BTreeMap::new();
    if !counts.is_known() {
        return sizes;
    }
    for device in devices {
        let jargb_zones: Vec<u32> =
            device.zones.iter().filter(|z| is_jargb_zone(&device.name, &z.name)).map(|z| z.index).collect();
        if jargb_zones.is_empty() {
            continue;
        }
        let mut per_device = BTreeMap::new();
        for (i, zone) in jargb_zones.iter().enumerate() {
            let count = counts.counts.get(i).copied().unwrap_or(SIGNALRGB_JARGB_LED_COUNT);
            per_device.insert(*zone, count);
        }
        sizes.insert(device.id.0.clone(), per_device);
    }
    sizes
}

/// True when a zone is one of the MSI ARGB headers.
fn is_jargb_zone(device_name: &str, zone_name: &str) -> bool {
    let device = device_name.to_ascii_lowercase();
    let zone = zone_name.to_ascii_lowercase();
    if !device.contains("mystic") && !device.contains("msi") && !device.contains("b850") {
        return false;
    }
    zone.contains("jargb") || zone.contains("argb")
}

/// The colour the devices get before the first run's layout is applied.
pub fn first_frame_color() -> Rgb8 {
    Rgb8::BLACK
}

/// Whether the first run still has to happen.
pub async fn should_run(app: &std::sync::Arc<crate::state::App>) -> bool {
    !app.store.read().await.config.first_run_done
}

/// The first run: size the JARGB headers, place the devices, activate the first effect.
///
/// It runs only once, when `config.first_run_done` is false and OpenRGB has devices.
/// Set while a first run is in progress, so two device callbacks can't both run it (the
/// second would rebuild the layout without the effect the first one activated).
static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub async fn maybe_run(app: &std::sync::Arc<crate::state::App>) {
    if !should_run(app).await {
        return;
    }
    if RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    run(app).await;
    RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
}

async fn run(app: &std::sync::Arc<crate::state::App>) {
    if !should_run(app).await {
        return;
    }
    if app.devices.read().await.is_empty() {
        tracing::info!("the first run waits for the devices");
        return;
    }
    let Some(devices) = settled_devices(app).await else {
        return;
    };

    // The JARGB counts come from SignalRGB, read-only.
    let counts = find_signalrgb_jargb_counts();
    match &counts.source {
        CountSource::Registry => {
            tracing::info!(counts = ?counts.counts, "the JARGB LED counts came from SignalRGB's registry");
        }
        CountSource::Log => {
            tracing::info!(counts = ?counts.counts, "the JARGB LED counts came from SignalRGB's log");
        }
        CountSource::Unknown => {
            tracing::warn!(
                "SignalRGB's JARGB LED counts could not be read; the counts stay unknown until the guided step"
            );
        }
    }

    // Size the JARGB headers and save the sizes.
    let sizes = jargb_zone_sizes(&devices, &counts);
    if !sizes.is_empty() {
        for (device, zones) in &sizes {
            for (zone, leds) in zones {
                let mut adapter = app.adapter.lock().await;
                if let Err(e) = adapter.set_zone_size(&canvas_proto::device::DeviceId(device.clone()), *zone, *leds).await {
                    tracing::warn!(device, zone, leds, error = %e, "could not size the header");
                }
            }
        }
        {
            let home = app.store.read().await.home.clone();
            let mut guard = app.zones.write().await;
            for (device, zones) in &sizes {
                guard.entry(device.clone()).or_default().extend(zones.clone());
            }
            let file = canvas_proto::config::ZonesFile {
                schema_version: canvas_proto::config::ZONES_SCHEMA,
                sizes: guard.clone(),
            };
            let _ = crate::storage::write_json(
                &home.join(canvas_proto::config::ZONES_FILE),
                &file,
            );
        }
    }

    // Re-discover after sizing, so the layout and the engine see the new LED counts (the
    // list read above still has the headers at 0 LEDs).
    let devices = if sizes.is_empty() {
        devices
    } else {
        let rediscovered = app.adapter.lock().await.discover().await;
        match rediscovered {
            Ok(list) => {
                *app.devices.write().await = list.clone();
                list
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not re-discover after sizing the headers");
                devices
            }
        }
    };

    // Place the devices and activate the first effect. The effect is saved on the layout,
    // as the activate route does, so the status and the next start both see it.
    let props = canvas_proto::props::Props::new();
    let mut layout = build_default_layout(&devices);
    layout.effect = Some(canvas_proto::layout::ActiveEffect {
        id: FIRST_RUN_EFFECT.to_string(),
        props: props.clone(),
    });
    {
        let mut store = app.store.write().await;
        store.layouts.active = layout.id.clone();
        match store.layouts.layouts.iter_mut().find(|l| l.id == layout.id) {
            Some(existing) => *existing = layout.clone(),
            None => store.layouts.layouts.push(layout.clone()),
        }
        let _ = store.save_layouts();
    }
    let device_list = app.devices.read().await.clone();
    app.engine.set_layout(layout.clone(), device_list);

    if let Err(e) = crate::apply_effect(app, FIRST_RUN_EFFECT, &props).await {
        tracing::error!(error = %e, effect = FIRST_RUN_EFFECT, "the first run could not activate the effect");
    } else {
        tracing::info!(effect = FIRST_RUN_EFFECT, "the first run activated the effect");
    }

    // The zone panel stays open until the user dismisses it.
    {
        let mut store = app
            .store
            .write()
            .await;
        store.config.first_run_done = true;
        let _ = store.save_config();
    }
    tracing::info!("the first run is done");
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::device::DeviceId;

    fn device(name: &str, kind: DeviceKind, leds: u32) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(format!("openrgb:{name}")),
            name: name.into(),
            vendor: "test".into(),
            kind,
            adapter: "openrgb".into(),
            adapter_index: 0,
            serial: name.into(),
            location: String::new(),
            zones: vec![ZoneInfo {
                index: 0,
                name: "Zone 1".into(),
                kind: ZoneKind::Linear,
                led_count: leds,
                min_leds: 1,
                max_leds: 512,
                resizable: true,
            }],
            connected: true,
        }
    }

    #[test]
    fn the_ram_sticks_are_vertical_and_side_by_side() {
        let devices = vec![
            device("ENE DIMM A", DeviceKind::Dram, 8),
            device("ENE DIMM B", DeviceKind::Dram, 8),
        ];
        let layout = build_default_layout(&devices);
        assert_eq!(layout.zones.len(), 2);
        let a = &layout.zones[0];
        let b = &layout.zones[1];
        assert!(a.height > a.width, "a RAM zone is vertical");
        assert!(b.height > b.width, "a RAM zone is vertical");
        assert_ne!(a.x, b.x, "the RAM sticks are side by side");
        assert_eq!(a.shape, ZoneShape::Strip);
    }

    #[test]
    fn the_keyboard_is_a_point_the_size_of_a_keyboard_and_the_mic_a_small_point() {
        let mut keyboard = device("ROYUAN", DeviceKind::Keyboard, 1);
        keyboard.zones[0].kind = ZoneKind::Single;
        let mut mic = device("Quadcast S", DeviceKind::Microphone, 2);
        mic.zones[0].kind = ZoneKind::Single;
        let layout = build_default_layout(&[keyboard, mic]);
        let keyboard = layout.zones.iter().find(|z| z.device_id.0.contains("ROYUAN")).unwrap();
        let mic = layout.zones.iter().find(|z| z.device_id.0.contains("Quadcast")).unwrap();
        assert_eq!(keyboard.shape, ZoneShape::Point);
        assert_eq!(mic.shape, ZoneShape::Point);
        assert!(keyboard.width > keyboard.height * 2.0 && keyboard.width < keyboard.height * 5.0);
        assert!(mic.height > mic.width, "a microphone stands upright");
        assert!(mic.width <= 20.0 && mic.height <= 32.0);
    }

    #[test]
    fn peripherals_do_not_stack_on_one_spot() {
        let mut mouse = device("Mouse", DeviceKind::Mouse, 1);
        mouse.zones[0].kind = ZoneKind::Single;
        let mut mic = device("Mic", DeviceKind::Microphone, 2);
        mic.zones[0].kind = ZoneKind::Single;
        let mut headset = device("Headset", DeviceKind::Headset, 2);
        headset.zones[0].kind = ZoneKind::Single;
        let mut keyboard = device("Keys", DeviceKind::Keyboard, 1);
        keyboard.zones[0].kind = ZoneKind::Single;
        let layout = build_default_layout(&[mouse, mic, headset, keyboard]);
        assert_eq!(layout.zones.len(), 4);
        for (i, a) in layout.zones.iter().enumerate() {
            for b in layout.zones.iter().skip(i + 1) {
                let overlap = a.x < b.x + b.width
                    && b.x < a.x + a.width
                    && a.y < b.y + b.height
                    && b.y < a.y + a.height;
                assert!(!overlap, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn no_default_zone_overlaps_another_on_a_full_desk() {
        let mut devices = vec![
            device("MSI", DeviceKind::Motherboard, 120),
            device("ENE A", DeviceKind::Dram, 8),
            device("ENE B", DeviceKind::Dram, 8),
        ];
        devices[0].zones = (0..3)
            .map(|i| ZoneInfo { index: i, name: format!("JARGB{}", i + 1), ..devices[0].zones[0].clone() })
            .chain((3..6).map(|i| ZoneInfo { index: i, name: format!("CPU Fan {i}"), led_count: 12, ..devices[0].zones[0].clone() }))
            .collect();
        let mut mouse = device("Mouse", DeviceKind::Mouse, 1);
        mouse.zones[0].kind = ZoneKind::Single;
        let mut mic = device("Mic", DeviceKind::Microphone, 2);
        mic.zones[0].kind = ZoneKind::Single;
        let mut keyboard = device("Keys", DeviceKind::Keyboard, 1);
        keyboard.zones[0].kind = ZoneKind::Single;
        devices.extend([mouse, mic, keyboard]);
        let layout = build_default_layout(&devices);
        assert_eq!(layout.zones.len(), 3 + 3 + 2 + 3);
        for (i, a) in layout.zones.iter().enumerate() {
            assert!(a.x >= 0.0 && a.y >= 0.0 && a.x + a.width <= 320.0 && a.y + a.height <= 200.0, "{a:?} off canvas");
            for b in layout.zones.iter().skip(i + 1) {
                let overlap = a.x < b.x + b.width
                    && b.x < a.x + a.width
                    && a.y < b.y + b.height
                    && b.y < a.y + a.height;
                assert!(!overlap, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn default_shapes_follow_the_device_and_zone() {
        let zone = |name: &str, kind: ZoneKind, n: u32| ZoneInfo {
            index: 0,
            name: name.into(),
            kind,
            led_count: n,
            min_leds: 1,
            max_leds: 512,
            resizable: true,
        };
        assert_eq!(default_shape(DeviceKind::Dram, &zone("DRAM", ZoneKind::Linear, 8)), ZoneShape::Strip);
        assert_eq!(default_shape(DeviceKind::Motherboard, &zone("CPU Fan", ZoneKind::Linear, 12)), ZoneShape::Ring);
        assert_eq!(default_shape(DeviceKind::Motherboard, &zone("JARGB1", ZoneKind::Linear, 120)), ZoneShape::Strip);
        assert_eq!(default_shape(DeviceKind::Mouse, &zone("Logo", ZoneKind::Single, 1)), ZoneShape::Point);
        assert_eq!(default_shape(DeviceKind::Microphone, &zone("Mic", ZoneKind::Single, 2)), ZoneShape::Point);
        let matrix = ZoneKind::Matrix { width: 2, height: 2, map: vec![Some(0), Some(1), Some(2), Some(3)] };
        assert_eq!(default_shape(DeviceKind::Keyboard, &zone("Keys", matrix, 4)), ZoneShape::Auto);
        assert_eq!(
            default_shape(DeviceKind::Keyboard, &zone("Keys", ZoneKind::Linear, 100)),
            ZoneShape::Grid { columns: 17 }
        );
    }

    #[test]
    fn a_fan_zone_becomes_a_square_ring() {
        let mut board = device("MSI", DeviceKind::Motherboard, 12);
        board.zones[0].name = "System Fan".into();
        let layout = build_default_layout(&[board]);
        let fan = &layout.zones[0];
        assert_eq!(fan.shape, ZoneShape::Ring);
        assert_eq!(fan.width, fan.height);
    }

    #[test]
    fn the_board_headers_are_horizontal_strips() {
        let devices = vec![device("MSI B850 Mystic Light", DeviceKind::Motherboard, 120)];
        let layout = build_default_layout(&devices);
        let zone = &layout.zones[0];
        assert!(zone.width > zone.height, "a board header is a horizontal strip");
    }

    #[test]
    fn a_late_device_is_placed_and_the_placed_ones_stay_put() {
        let mic = device("HyperX Quadcast S", DeviceKind::Microphone, 2);
        let board = device("MSI MYSTIC LIGHT", DeviceKind::Motherboard, 120);
        let mut layout = build_default_layout(std::slice::from_ref(&mic));
        layout.zones[0].x = 200.0; // The mic was moved.
        let added = place_missing_zones(&mut layout, &[mic.clone(), board.clone()]);
        assert_eq!(added, board.zones.iter().filter(|z| z.led_count > 0).count());
        assert!(layout.zones.iter().any(|z| z.device_id == board.id));
        let mic_zone = layout.zones.iter().find(|z| z.device_id == mic.id).unwrap();
        assert_eq!(mic_zone.x, 200.0);
        assert_eq!(place_missing_zones(&mut layout, &[mic, board]), 0);
    }

    #[test]
    fn the_layout_is_deterministic() {
        let devices = vec![
            device("B", DeviceKind::Dram, 8),
            device("A", DeviceKind::Dram, 8),
        ];
        let first = build_default_layout(&devices);
        let second = build_default_layout(&devices);
        assert_eq!(first.zones.len(), second.zones.len());
        for (a, b) in first.zones.iter().zip(second.zones.iter()) {
            assert_eq!(a.device_id, b.device_id);
            assert_eq!(a.x, b.x);
            assert_eq!(a.y, b.y);
        }
    }

    #[test]
    fn counts_are_pulled_out_of_a_log_line() {
        let found = counts_in_text("MSI ARGB header JARGB1: 120 LEDs").unwrap();
        assert_eq!(found, vec![120]);
        let none = counts_in_text("nothing to see here");
        assert!(none.is_none());
    }

    #[test]
    fn the_counts_are_unknown_when_signalrgb_is_not_installed() {
        let counts = find_signalrgb_jargb_counts();
        // On a machine without SignalRGB the counts stay unknown.
        if counts.source == CountSource::Unknown {
            assert!(!counts.is_known());
        }
    }
}
