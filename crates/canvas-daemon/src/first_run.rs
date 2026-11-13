//! G7: the first run.
//!
//! On a clean profile the daemon lights everything up with no clicks: it sizes the JARGB
//! headers from SignalRGB's own configuration (read-only), places every device on the
//! 320x200 canvas, activates Aurora and opens the editor's dismissible "size your ARGB
//! headers" panel.

use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::ZoneShape;
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

/// What a case slot holds.
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

/// Sizes the JARGB headers, saves the sizes, and makes the lights that come of it join the
/// first-run scene.
///
/// The zones the sizing gives LEDs are lights now: they join the hardware map, and the first-run
/// scene (one group holding every light) takes them, so the owner's headers are lit and not
/// dark. The devices are read again after the sizing, so the counts are the new ones.
pub async fn size_headers(
    app: &std::sync::Arc<crate::state::App>,
    devices: &[DeviceInfo],
    counts: &JargbCounts,
) -> Vec<DeviceInfo> {
    let sizes = jargb_zone_sizes(devices, counts);
    if sizes.is_empty() {
        return devices.to_vec();
    }
    for (device, zones) in &sizes {
        for (zone, leds) in zones {
            let mut adapter = app.adapter.lock().await;
            if let Err(e) = adapter
                .set_zone_size(&canvas_proto::device::DeviceId(device.clone()), *zone, *leds)
                .await
            {
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
        crate::logging::warn_if_err(
            "the first run could not save the zone sizes",
            crate::storage::write_json(&home.join(canvas_proto::config::ZONES_FILE), &file),
        );
    }

    // Read the devices again, so the scene and the engine see the new LED counts (the list
    // read above still has the headers at 0 LEDs).
    let devices = match app.adapter.lock().await.discover().await {
        Ok(list) => {
            *app.devices.write().await = list.clone();
            list
        }
        Err(e) => {
            tracing::warn!(error = %e, "could not re-discover after sizing the headers");
            devices.to_vec()
        }
    };

    // The lights that come of the sizing join the hardware map, and the first-run scene takes
    // them: they were skipped when the scene was made, because the headers had no LEDs then.
    {
        let mut store = app.store.write().await;
        let added = store.sync_lights_from(&devices);
        if added > 0 {
            tracing::info!(added, "the sized headers are lights now");
        }
        // A migration that is still held (the layouts place a device that has not arrived)
        // runs now, with the devices the sizing left.
        store.finish_scenes(&devices);
        if store.refresh_first_run_scene() {
            tracing::info!("the first run's scene holds every light");
            crate::logging::warn_if_err(
                "the first run could not save the scenes",
                store.save_scenes(),
            );
        }
    }
    devices
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

/// Gives the flag back when the first run ends for any reason: its own return, an error, or a
/// panic in the task. Clearing it after the await alone leaves the flag set for the life of the
/// daemon when the task does not reach the end (docs/bug-classes.md, `cause:state-token`).
struct RunningGuard;

impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

pub async fn maybe_run(app: &std::sync::Arc<crate::state::App>) {
    if !should_run(app).await {
        return;
    }
    if RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let _running = RunningGuard;
    run(app).await;
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

    // Size the JARGB headers, save the sizes, and make the lights that come of it join the
    // first-run scene.
    let _devices = size_headers(app, &devices, &counts).await;

    // The scene: the store made it from the first device list (on a first run the migration
    // through `migrate(None, devices)`), and it is already on disk. Nothing is written here:
    // writing it twice would fight the migration the store owns.
    //
    // The default scene plays the first run's effect, so applying the scene puts the lights
    // on the way the owner expects.
    let replace = crate::draft::hold_for_replace(app).await;
    crate::scenes::apply_active_ungated(app, false).await;
    // The scene is on the lights: the takeover is over, and the guard owes nothing.
    replace.settle();
    {
        let store = app.store.read().await;
        let scene = crate::scenes::active_scene(&store.scenes);
        tracing::info!(scene = %scene.name, groups = scene.groups.len(), effect = FIRST_RUN_EFFECT, "the first run applied the default scene");
    }

    // The zone panel stays open until the user dismisses it.
    {
        let mut store = app
            .store
            .write()
            .await;
        store.config.first_run_done = true;
        crate::logging::warn_if_err("the first run could not save first_run_done; it will run again", store.save_config());
    }
    tracing::info!("the first run is done");
}

#[cfg(test)]
mod tests {
    use super::*;

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


