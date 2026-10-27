//! The numeric displays (the GAMDIAS CHIONE pump-head display): what each one shows and the
//! task that writes it.
//!
//! A display is driven only while it has a saved config in `displays.json` (spine: `routes::DISPLAY`).
//! The adapter enumerates with `discover`, which never opens a device. The task connects to and
//! writes only a display that has a config, because the vendor app may own the others.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use canvas_proto::device::DeviceId;
use canvas_proto::display::{DisplayConfig, DisplayInfo, DisplayMode, DisplaySource, DisplayUnit, DisplayValue};
use canvas_proto::fans::{FansStatus, TempSource};
use canvas_proto::traits::{AdapterError, NumericDisplayAdapter};

use crate::state::App;

/// The file the display configs live in, next to `layouts.json` and `zones.json`.
pub const DISPLAYS_FILE: &str = "displays.json";
pub const DISPLAYS_SCHEMA: u32 = 1;

/// After an adapter error the task waits this long, then doubles the wait each time, up to the cap.
pub const BACKOFF_FIRST: Duration = Duration::from_secs(1);
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// A configured display that the enumeration does not find: enumerate again after this long.
/// Nothing is opened in the meantime.
pub const ABSENT_POLL: Duration = Duration::from_secs(5);
/// No display has a saved config: the task sleeps this long, or until a PUT or DELETE wakes it.
pub const IDLE_POLL: Duration = Duration::from_secs(60);
/// Used when an adapter reports a zero write interval.
const FALLBACK_INTERVAL: Duration = Duration::from_millis(250);

/// `displays.json`: the saved display configs, one per device.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplaysFile {
    pub schema_version: u32,
    #[serde(default)]
    pub displays: Vec<DisplayConfig>,
}

impl Default for DisplaysFile {
    fn default() -> Self {
        Self { schema_version: DISPLAYS_SCHEMA, displays: Vec::new() }
    }
}

/// Writes `displays.json` in `home`, atomically.
pub fn save(home: &Path, file: &DisplaysFile) -> std::io::Result<()> {
    crate::storage::write_json(&home.join(DISPLAYS_FILE), file)
}

/// The adapter and what the task knows about it. One adapter serves every display; the
/// CHIONE is one device, and the trait's `connect` takes no device.
pub struct DisplayHub {
    /// Held for every enumerate, connect, write and disconnect. A DELETE that waits for this
    /// lock cannot race a write: once it holds the lock, no write is in flight.
    pub adapter: tokio::sync::Mutex<Box<dyn NumericDisplayAdapter>>,
    /// What the task and the routes share.
    pub runtime: tokio::sync::RwLock<Runtime>,
    /// Bumped by every config change. The task drops its retry pacing when this moves.
    generation: AtomicU64,
    /// Wakes the task after a config change.
    pub wake: tokio::sync::Notify,
}

/// Runtime state. Nothing here is saved: after a restart, API values are blank until PUT again.
#[derive(Debug, Default)]
pub struct Runtime {
    /// What the last enumeration found.
    pub found: Vec<DisplayInfo>,
    /// Whether the adapter is connected, as the task last left it.
    pub connected: bool,
    /// The last value written per display. A blank write is stored with `value: None`.
    pub shown: BTreeMap<DeviceId, DisplayValue>,
    /// The last value PUT to each display's value route (`None` blanks it).
    pub api: BTreeMap<DeviceId, Option<f32>>,
    /// Displays whose refusal streak has been logged. Cleared by the next accepted write.
    pub refusing: BTreeSet<DeviceId>,
    /// How many refusal warnings have been logged in total. Tests read it to check "logged once".
    pub refusal_warnings: u32,
    /// Retry pacing: no adapter call before this instant.
    pub retry_at: Option<Instant>,
    /// The last backoff wait after an adapter error.
    pub backoff: Option<Duration>,
    /// The generation the pacing was last reset for.
    seen_generation: u64,
}

impl DisplayHub {
    pub fn new(adapter: Box<dyn NumericDisplayAdapter>) -> Self {
        Self {
            adapter: tokio::sync::Mutex::new(adapter),
            runtime: tokio::sync::RwLock::new(Runtime::default()),
            generation: AtomicU64::new(0),
            wake: tokio::sync::Notify::new(),
        }
    }

    /// Called after a PUT or DELETE: the task retries now rather than after a backoff.
    pub fn changed(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.wake.notify_one();
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }
}

/// An adapter that has no displays. The daemon's default until the CHIONE is wired in, and the
/// `App::new` default for the tests that do not use a display.
pub struct NoDisplays;

#[async_trait]
impl NumericDisplayAdapter for NoDisplays {
    fn name(&self) -> &'static str {
        "none"
    }

    async fn connect(&mut self) -> Result<(), AdapterError> {
        Err(AdapterError::NotConnected)
    }

    async fn disconnect(&mut self) -> Result<(), AdapterError> {
        Ok(())
    }

    fn is_connected(&self) -> bool {
        false
    }

    async fn discover(&mut self) -> Result<Vec<DisplayInfo>, AdapterError> {
        Ok(Vec::new())
    }

    async fn write_value(&mut self, _device: &DeviceId, _value: &DisplayValue) -> Result<(), AdapterError> {
        Err(AdapterError::NotConnected)
    }

    fn write_interval_ms(&self) -> u32 {
        250
    }
}

/// What a display's source can read from: the fan service's status (sensors and headers) and the
/// last API value for that display.
pub struct Inputs<'a> {
    pub fans: Option<&'a FansStatus>,
    pub api: Option<f32>,
}

fn celsius(inputs: &Inputs<'_>, source: &TempSource) -> Option<f32> {
    inputs.fans?.temps.iter().find(|t| &t.source == source)?.celsius
}

fn rpm(inputs: &Inputs<'_>, header: &str) -> Option<f32> {
    let status = inputs.fans?.headers.iter().find(|h| h.id.as_str() == header)?;
    status.rpm.map(|r| r as f32)
}

/// Rounds to `decimals` places. Half rounds away from zero, so 108.5 shows as 109.
pub fn round_to(value: f32, decimals: u8) -> f32 {
    let scale = 10f32.powi(i32::from(decimals));
    (value * scale).round() / scale
}

/// The value a config shows now, before the adapter sees it. `value: None` is blank.
///
/// Sensor temperatures are read in °C and converted here when the config says °F. Fixed and API
/// values are shown as given, in the config's unit. The blink flag compares the shown (rounded)
/// value with `blink_above`, at or above.
pub fn display_value(config: &DisplayConfig, inputs: &Inputs<'_>) -> DisplayValue {
    let (mode, raw, from_sensor) = match &config.source {
        DisplaySource::CpuTemp => (DisplayMode::Temperature, celsius(inputs, &TempSource::Cpu), true),
        DisplaySource::GpuTemp => (DisplayMode::Temperature, celsius(inputs, &TempSource::Gpu), true),
        DisplaySource::BoardTemp { sensor } => (
            DisplayMode::Temperature,
            celsius(inputs, &TempSource::Board(sensor.clone())),
            true,
        ),
        DisplaySource::FanRpm { header } => (DisplayMode::FanRpm, rpm(inputs, header), false),
        DisplaySource::Fixed { value } => (DisplayMode::Temperature, Some(*value), false),
        DisplaySource::Api => (DisplayMode::Temperature, inputs.api, false),
    };
    let converted = raw.map(|c| {
        if from_sensor && config.unit == DisplayUnit::Fahrenheit {
            c * 9.0 / 5.0 + 32.0
        } else {
            c
        }
    });
    let shown = converted.map(|v| round_to(v, config.decimals));
    let blink = match (config.blink_above, shown) {
        (Some(at), Some(v)) => v >= at,
        _ => false,
    };
    DisplayValue { value: shown, decimals: config.decimals, unit: config.unit, mode, blink }
}

/// Checks a config PUT to `/displays/{path_id}` before it is saved.
pub fn validate(path_id: &str, config: &DisplayConfig) -> Result<(), String> {
    if config.device.0 != path_id {
        return Err(format!(
            "the device in the body ({}) does not match the path ({path_id})",
            config.device.0
        ));
    }
    if config.decimals > 1 {
        return Err(format!("decimals must be 0 or 1, got {}", config.decimals));
    }
    match &config.source {
        DisplaySource::FanRpm { header } => {
            if config.decimals != 0 {
                return Err("a fan RPM is shown with 0 decimals".into());
            }
            if header.trim().is_empty() {
                return Err("the fan header name is empty".into());
            }
        }
        DisplaySource::BoardTemp { sensor } if sensor.trim().is_empty() => {
            return Err("the board sensor name is empty".into());
        }
        DisplaySource::Fixed { value } if !value.is_finite() => {
            return Err("the fixed value must be a finite number".into());
        }
        _ => {}
    }
    if config.blink_above.is_some_and(|at| !at.is_finite()) {
        return Err("blink_above must be a finite number".into());
    }
    Ok(())
}

fn write_interval(ms: u32) -> Duration {
    if ms == 0 {
        FALLBACK_INTERVAL
    } else {
        Duration::from_millis(u64::from(ms))
    }
}

/// Writes one display's value.
///
/// An adapter that refuses a value keeps its connection, because a refusal writes nothing. For
/// that case the display shows blank and the refusal is logged once per streak. A failed write
/// leaves the adapter disconnected; the error goes back to the caller.
async fn write_one(
    adapter: &mut dyn NumericDisplayAdapter,
    runtime: &mut Runtime,
    device: &DeviceId,
    value: DisplayValue,
) -> Result<(), AdapterError> {
    match adapter.write_value(device, &value).await {
        Ok(()) => {
            accepted(runtime, device, value);
            Ok(())
        }
        Err(error) if adapter.is_connected() => {
            if runtime.refusing.insert(device.clone()) {
                runtime.refusal_warnings += 1;
                tracing::warn!(
                    device = %device,
                    value = ?value.value,
                    error = %error,
                    "the display refused the value; showing blank"
                );
            }
            // The blank write is what the display shows now. It does not end the streak: the
            // refused value comes back next pass, and that is not a new refusal to log.
            let blank = DisplayValue { value: None, ..value };
            adapter.write_value(device, &blank).await?;
            runtime.shown.insert(device.clone(), blank);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn accepted(runtime: &mut Runtime, device: &DeviceId, value: DisplayValue) {
    runtime.refusing.remove(device);
    runtime.backoff = None;
    runtime.shown.insert(device.clone(), value);
}

/// Closes the adapter and sets the next retry: 1 s after the first error, then doubling to 30 s.
async fn fail(
    hub: &DisplayHub,
    adapter: &mut dyn NumericDisplayAdapter,
    now: Instant,
    what: &str,
    error: AdapterError,
) -> Duration {
    let _ = adapter.disconnect().await;
    let mut runtime = hub.runtime.write().await;
    let wait = match runtime.backoff {
        None => BACKOFF_FIRST,
        Some(previous) => (previous * 2).min(BACKOFF_MAX),
    };
    runtime.backoff = Some(wait);
    runtime.retry_at = Some(now + wait);
    runtime.connected = false;
    runtime.shown.clear();
    tracing::warn!(
        error = %error,
        retry_in_ms = wait.as_millis() as u64,
        "{what}; the display is closed and will be retried"
    );
    wait
}

/// One pass of the display task. Returns how long to wait before the next pass.
///
/// With no saved config it does nothing but sleep: it does not enumerate or open anything. With a
/// config for a display the enumeration finds, it connects and writes every such display at the
/// adapter's interval. Adapter errors close the device and back off.
pub async fn tick(app: &App, now: Instant) -> Duration {
    let hub = &app.displays;
    let generation = hub.generation();
    // Lock order: adapter, then store, then runtime. The config is read under the adapter lock,
    // so a DELETE that has removed its config and then takes the adapter lock sees every write
    // from before its removal finished.
    let mut adapter = hub.adapter.lock().await;
    let configs = app.store.read().await.displays.displays.clone();
    {
        let mut runtime = hub.runtime.write().await;
        if generation != runtime.seen_generation {
            runtime.seen_generation = generation;
            runtime.retry_at = None;
            runtime.backoff = None;
        }
    }

    if configs.is_empty() {
        if adapter.is_connected() {
            let _ = adapter.disconnect().await;
            tracing::info!("no display has a saved config; the display is closed");
        }
        hub.runtime.write().await.connected = false;
        return IDLE_POLL;
    }

    if let Some(at) = hub.runtime.read().await.retry_at {
        if now < at {
            return at - now;
        }
    }

    if !adapter.is_connected() {
        let found = match adapter.discover().await {
            Ok(found) => found,
            Err(error) => return fail(hub, &mut **adapter, now, "could not enumerate the displays", error).await,
        };
        let wanted = configs.iter().any(|c| found.iter().any(|f| f.id == c.device));
        {
            let mut runtime = hub.runtime.write().await;
            runtime.found = found;
        }
        if !wanted {
            // A display is here with no saved config (the vendor app may own it): leave it alone.
            // A configured display that is not here yet: look again later, opening nothing.
            let mut runtime = hub.runtime.write().await;
            runtime.retry_at = Some(now + ABSENT_POLL);
            return ABSENT_POLL;
        }
        if let Err(error) = adapter.connect().await {
            return fail(hub, &mut **adapter, now, "could not connect to the display", error).await;
        }
        hub.runtime.write().await.connected = true;
        tracing::info!("the display is connected");
    }

    let interval = write_interval(adapter.write_interval_ms());
    let fans = app.fans.read().await.clone();
    let mut failure = None;
    {
        let mut runtime = hub.runtime.write().await;
        let found: Vec<DeviceId> = runtime.found.iter().map(|f| f.id.clone()).collect();
        for config in configs.iter().filter(|c| found.contains(&c.device)) {
            let api = runtime.api.get(&config.device).copied().flatten();
            let inputs = Inputs { fans: fans.as_ref(), api };
            let value = display_value(config, &inputs);
            if let Err(error) = write_one(&mut **adapter, &mut runtime, &config.device, value).await {
                failure = Some(error);
                break;
            }
        }
    }
    match failure {
        Some(error) => fail(hub, &mut **adapter, now, "the display write failed", error).await,
        None => interval,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::fans::{FanHeaderStatus, FanMode, TempReading};

    fn config(source: DisplaySource, unit: DisplayUnit, decimals: u8, blink_above: Option<f32>) -> DisplayConfig {
        DisplayConfig {
            device: DeviceId("chione:1b80:b53c".into()),
            source,
            unit,
            decimals,
            blink_above,
        }
    }

    fn status(temps: Vec<(TempSource, Option<f32>)>, headers: Vec<(&str, Option<u32>)>) -> FansStatus {
        FansStatus {
            chip: None,
            hardware_watchdog: None,
            headers: headers
                .into_iter()
                .map(|(id, rpm)| FanHeaderStatus {
                    id: id.to_string(),
                    label: id.to_string(),
                    rpm,
                    pwm_percent: None,
                    mode: FanMode::Bios,
                    on_bios: true,
                    override_reason: None,
                })
                .collect(),
            temps: temps
                .into_iter()
                .map(|(source, celsius)| TempReading { label: String::new(), source, celsius })
                .collect(),
            active_profile: "Default".into(),
            profiles: vec![],
            floor_percent: 30,
        }
    }

    fn close(a: Option<f32>, b: f32) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-3)
    }

    #[test]
    fn celsius_is_shown_as_read_with_the_chosen_decimals() {
        let fans = status(vec![(TempSource::Cpu, Some(42.5))], vec![]);
        let inputs = Inputs { fans: Some(&fans), api: None };
        let value = display_value(&config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 1, None), &inputs);
        assert!(close(value.value, 42.5), "{value:?}");
        assert_eq!(value.mode, DisplayMode::Temperature);
        assert_eq!(value.unit, DisplayUnit::Celsius);
        assert!(!value.blink);
    }

    #[test]
    fn fahrenheit_is_converted_before_rounding() {
        let fans = status(vec![(TempSource::Cpu, Some(37.4))], vec![]);
        let inputs = Inputs { fans: Some(&fans), api: None };
        let whole = display_value(&config(DisplaySource::CpuTemp, DisplayUnit::Fahrenheit, 0, None), &inputs);
        assert!(close(whole.value, 99.0), "37.4 C is 99.32 F, shown as 99: {whole:?}");
        let tenth = display_value(&config(DisplaySource::CpuTemp, DisplayUnit::Fahrenheit, 1, None), &inputs);
        assert!(close(tenth.value, 99.3), "{tenth:?}");
        assert_eq!(tenth.unit, DisplayUnit::Fahrenheit);
    }

    #[test]
    fn rounding_is_half_away_from_zero() {
        assert_eq!(round_to(108.5, 0), 109.0);
        assert_eq!(round_to(108.49, 0), 108.0);
        assert!(close(Some(round_to(42.25, 1)), 42.3));
    }

    #[test]
    fn blink_is_at_or_above_the_shown_value() {
        let fans = status(vec![(TempSource::Cpu, Some(37.8))], vec![]);
        let inputs = Inputs { fans: Some(&fans), api: None };
        // 37.8 C is 100.04 F, which shows as 100: at blink_above 100 it blinks.
        let at = display_value(&config(DisplaySource::CpuTemp, DisplayUnit::Fahrenheit, 0, Some(100.0)), &inputs);
        assert!(at.blink, "{at:?}");
        let below = display_value(&config(DisplaySource::CpuTemp, DisplayUnit::Fahrenheit, 0, Some(100.5)), &inputs);
        assert!(!below.blink, "{below:?}");
    }

    #[test]
    fn a_missing_reading_is_blank_and_never_blinks() {
        let fans = status(vec![], vec![]);
        let inputs = Inputs { fans: Some(&fans), api: None };
        let value = display_value(&config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 1, Some(0.0)), &inputs);
        assert_eq!(value.value, None);
        assert!(!value.blink);
        let no_service = display_value(
            &config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 1, None),
            &Inputs { fans: None, api: None },
        );
        assert_eq!(no_service.value, None);
    }

    #[test]
    fn gpu_reads_the_gpu_temperature_or_is_blank() {
        let with_gpu = status(vec![(TempSource::Gpu, Some(61.0))], vec![]);
        let value = display_value(
            &config(DisplaySource::GpuTemp, DisplayUnit::Celsius, 0, None),
            &Inputs { fans: Some(&with_gpu), api: None },
        );
        assert!(close(value.value, 61.0), "{value:?}");
        let without = status(vec![(TempSource::Cpu, Some(40.0))], vec![]);
        let blank = display_value(
            &config(DisplaySource::GpuTemp, DisplayUnit::Celsius, 0, None),
            &Inputs { fans: Some(&without), api: None },
        );
        assert_eq!(blank.value, None);
    }

    #[test]
    fn a_board_sensor_reads_its_own_name() {
        let fans = status(vec![(TempSource::Board("board_temp1".into()), Some(33.0))], vec![]);
        let value = display_value(
            &config(DisplaySource::BoardTemp { sensor: "board_temp1".into() }, DisplayUnit::Celsius, 1, None),
            &Inputs { fans: Some(&fans), api: None },
        );
        assert!(close(value.value, 33.0), "{value:?}");
    }

    #[test]
    fn a_fan_rpm_reads_the_header_in_rpm_mode_without_conversion() {
        let fans = status(vec![], vec![("pump_fan", Some(2400)), ("cpu_fan", None)]);
        let inputs = Inputs { fans: Some(&fans), api: None };
        let value = display_value(
            &config(DisplaySource::FanRpm { header: "pump_fan".into() }, DisplayUnit::Fahrenheit, 0, None),
            &inputs,
        );
        assert!(close(value.value, 2400.0), "{value:?}");
        assert_eq!(value.mode, DisplayMode::FanRpm);
        let no_rpm = display_value(
            &config(DisplaySource::FanRpm { header: "cpu_fan".into() }, DisplayUnit::Celsius, 0, None),
            &inputs,
        );
        assert_eq!(no_rpm.value, None);
    }

    #[test]
    fn fixed_and_api_values_are_shown_as_given() {
        let inputs = Inputs { fans: None, api: Some(50.0) };
        let fixed = display_value(
            &config(DisplaySource::Fixed { value: 50.0 }, DisplayUnit::Fahrenheit, 0, None),
            &inputs,
        );
        assert!(close(fixed.value, 50.0), "a fixed value is not converted: {fixed:?}");
        let api = display_value(&config(DisplaySource::Api, DisplayUnit::Fahrenheit, 0, None), &inputs);
        assert!(close(api.value, 50.0), "{api:?}");
        let blank = display_value(&config(DisplaySource::Api, DisplayUnit::Celsius, 0, None), &Inputs { fans: None, api: None });
        assert_eq!(blank.value, None);
    }

    #[test]
    fn validation_refuses_a_mismatched_device_and_bad_decimals() {
        let base = config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 1, None);
        assert!(validate("chione:1b80:b53c", &base).is_ok());
        assert!(validate("chione:0000:0000", &base).is_err());
        let two = config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 2, None);
        assert!(validate("chione:1b80:b53c", &two).is_err());
    }

    #[test]
    fn validation_allows_fan_rpm_with_zero_decimals_only() {
        let rpm = |decimals| config(DisplaySource::FanRpm { header: "pump_fan".into() }, DisplayUnit::Celsius, decimals, None);
        assert!(validate("chione:1b80:b53c", &rpm(0)).is_ok());
        assert!(validate("chione:1b80:b53c", &rpm(1)).is_err());
    }

    #[test]
    fn validation_refuses_empty_names_and_non_finite_numbers() {
        let empty_header = config(DisplaySource::FanRpm { header: " ".into() }, DisplayUnit::Celsius, 0, None);
        assert!(validate("chione:1b80:b53c", &empty_header).is_err());
        let empty_sensor = config(DisplaySource::BoardTemp { sensor: String::new() }, DisplayUnit::Celsius, 0, None);
        assert!(validate("chione:1b80:b53c", &empty_sensor).is_err());
        let nan_fixed = config(DisplaySource::Fixed { value: f32::NAN }, DisplayUnit::Celsius, 0, None);
        assert!(validate("chione:1b80:b53c", &nan_fixed).is_err());
        let nan_blink = config(DisplaySource::CpuTemp, DisplayUnit::Celsius, 0, Some(f32::INFINITY));
        assert!(validate("chione:1b80:b53c", &nan_blink).is_err());
    }

    #[test]
    fn a_zero_write_interval_falls_back_to_250_ms() {
        assert_eq!(write_interval(0), Duration::from_millis(250));
        assert_eq!(write_interval(250), Duration::from_millis(250));
    }

    #[test]
    fn the_displays_file_round_trips_through_json() {
        let file = DisplaysFile {
            schema_version: DISPLAYS_SCHEMA,
            displays: vec![config(DisplaySource::BoardTemp { sensor: "board_temp0".into() }, DisplayUnit::Fahrenheit, 1, Some(90.0))],
        };
        let text = serde_json::to_string(&file).expect("serializes");
        let back: DisplaysFile = serde_json::from_str(&text).expect("parses");
        assert_eq!(back, file);
        let empty: DisplaysFile = serde_json::from_str(r#"{"schema_version":1}"#).expect("a missing list is empty");
        assert!(empty.displays.is_empty());
    }
}
