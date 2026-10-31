//! The fan controller: the 1 Hz curve loop, per-header safety, the BIOS save/restore, and
//! the profile operations the pipe exposes.
//!
//! Rules this module enforces (spec and grill choices G4, the safety body):
//! - Before the first manual write to a header, its BIOS state is saved to `bios-state.json`.
//! - Every exit path restores BIOS: `restore_all`, called on stop and at startup.
//! - A failed write, a failed read, a readback mismatch, or a curve source missing for
//!   `SENSOR_LOSS_TIMEOUT_MS` hands that header back to BIOS. A handed-back header stays on
//!   BIOS until the user applies it again.
//! - Every target is clamped by `runtime_floor` at write time, so no path drives a header below it.
//! - Startup restores BIOS on every header before any profile is applied.

use crate::chip::FanChip;
use crate::paths;
use crate::store::{self, BiosState, SavedHeader, StoreError};
use crate::temps::TempReader;
use canvas_proto::config::{FansFile, FANS_SCHEMA};
use canvas_proto::fans::{
    floor_for, runtime_floor, validate_mode, validate_profile, CurvePoint, FanHeaderStatus, FanMode, FanProfile,
    FanValidationError, FansStatus, TempReading, TempSource, SENSOR_LOSS_TIMEOUT_MS,
};
use canvas_proto::Rgb8;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Readback tolerance for a written duty, in percentage points.
pub const READBACK_TOLERANCE: u8 = 2;
/// How long a new duty may take to show in the chip's PWM readback before it is a mismatch.
pub const READBACK_SETTLE: Duration = Duration::from_secs(3);
/// Readback polls while a duty settles.
const READBACK_POLL: Duration = Duration::from_millis(100);

/// Time constant of the exponential average (falling side only; a rise snaps the average up) that
/// decides a curve's step down, in seconds. A 1 Hz
/// Ryzen `Tctl` jumps about 4 C between reads under light load (issue #17); the average turns
/// those spikes into a drift of about a degree.
pub const CURVE_SMOOTHING_TAU_S: f32 = 4.0;
/// Step down only after the smoothed temperature fell this many degrees below the temperature that
/// raised the duty. Fan Control's curve "Hysteresis" is the same idea: the "minimum temperature
/// difference for a change to occur" (https://getfancontrol.com/docs/). 5 C is above the 4 C
/// spikes of issue #17.
pub const CURVE_STEP_DOWN_DROP_C: f32 = 5.0;
/// Or step down after the duty has been held this long and the smoothed temperature asks for less
/// (Fan Control's "Response time", the minimum time for a change to occur, doubled on a decrease
/// by its Auto curve). Keeps a slow cool-down from leaving the fan high for good.
pub const CURVE_STEP_DOWN_HOLD: Duration = Duration::from_secs(60);
/// A gap between two curve updates longer than this (a missing sensor, a sleep) restarts the
/// smoothing from the current reading.
const CURVE_RESEED: Duration = Duration::from_secs(10);

/// hardware_watchdog as settled from public sources (see the report): neither the nct6687d
/// driver nor LibreHardwareMonitor's NCT677X handling reaches a chip watchdog register.
pub const HARDWARE_WATCHDOG: Option<bool> = Some(false);

#[derive(Debug, thiserror::Error)]
pub enum FanError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("fan chip unavailable: {0}")]
    Unavailable(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Store(#[from] StoreError),
}

impl From<FanValidationError> for FanError {
    fn from(e: FanValidationError) -> Self {
        Self::Validation(e.to_string())
    }
}

/// Per-header state that smooths a curve and holds its output steady (issue #17). Step up is
/// immediate: the raw reading is mapped through the curve, and any higher duty is taken at once.
/// Step down needs the smoothed temperature to fall `CURVE_STEP_DOWN_DROP_C` below the temperature
/// that set the held duty, or `CURVE_STEP_DOWN_HOLD` to pass.
#[derive(Default)]
struct CurveState {
    smoothed: Option<f32>,
    last_at: Option<Instant>,
    held: Option<u8>,
    /// The temperature that set `held`.
    anchor: f32,
    changed_at: Option<Instant>,
}

impl CurveState {
    fn step(&mut self, points: &[CurvePoint], temp: f32, now: Instant) -> u8 {
        let dt = self.last_at.map(|t| now.saturating_duration_since(t));
        self.last_at = Some(now);
        match (self.smoothed, dt) {
            (Some(prev), Some(dt)) if dt <= CURVE_RESEED => {
                let dt = dt.as_secs_f32();
                self.smoothed = Some(if temp >= prev { temp } else { prev + (temp - prev) * dt / (CURVE_SMOOTHING_TAU_S + dt) });
            }
            _ => {
                self.smoothed = Some(temp);
                self.held = None;
            }
        }
        let smoothed = self.smoothed.unwrap_or(temp);
        let raw = curve_percent(points, temp);
        let Some(held) = self.held else {
            self.set(raw, temp, now);
            return raw;
        };
        if raw > held {
            self.set(raw, temp, now);
            return raw;
        }
        let calm = curve_percent(points, smoothed);
        let waited = self.changed_at.is_none_or(|t| now.saturating_duration_since(t) >= CURVE_STEP_DOWN_HOLD);
        if calm < held && (smoothed <= self.anchor - CURVE_STEP_DOWN_DROP_C || waited) {
            self.set(calm, smoothed, now);
            return calm;
        }
        held
    }

    fn set(&mut self, percent: u8, anchor: f32, now: Instant) {
        self.held = Some(percent);
        self.anchor = anchor;
        self.changed_at = Some(now);
    }
}

#[derive(Default)]
struct Slot {
    curve: CurveState,
    /// `Some(time)` while this service holds the header in manual mode.
    manual_since: Option<Instant>,
    /// Set by a safety or failure handback. Sticky until the user applies the header again.
    handed_back: bool,
    override_reason: Option<String>,
}

/// Curve interpolation: linear between points, flat outside. Points must be sorted.
pub fn curve_percent(points: &[CurvePoint], temp: f32) -> u8 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        // A curve with no points is not a valid mode; full speed is the safe answer.
        return 100;
    };
    if temp <= first.temp_c {
        return first.percent;
    }
    if temp >= last.temp_c {
        return last.percent;
    }
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if temp <= b.temp_c {
            let span = b.temp_c - a.temp_c;
            let t = if span > 0.0 { (temp - a.temp_c) / span } else { 0.0 };
            let value = f32::from(a.percent) + (f32::from(b.percent) - f32::from(a.percent)) * t;
            return value.round().clamp(0.0, 100.0) as u8;
        }
    }
    last.percent
}

/// The reason every header reports while the channel map is unconfirmed.
pub fn unconfirmed_reason(map: &str) -> String {
    format!("channel map {map} not yet confirmed against LibreHardwareMonitor; fans stay on BIOS")
}

pub struct FanController {
    home: PathBuf,
    chip: Option<Box<dyn FanChip>>,
    chip_error: Option<String>,
    temps: BTreeMap<TempSource, Box<dyn TempReader>>,
    config: FansFile,
    slots: Vec<Slot>,
    failed: BTreeSet<TempSource>,
    last_good: BTreeMap<TempSource, Instant>,
    temp_cache: BTreeMap<TempSource, f32>,
    bios_state: BiosState,
    /// True only when `chip.toml` confirms the map that is detected now. Until then no
    /// register is written by this controller (reads still work).
    writes_allowed: bool,
    /// Set by the service once the state file passes the owner check.
    state_trusted: bool,
}

impl FanController {
    /// Builds the controller. `confirmed_map` is the map named in `chip.toml`, if any.
    /// Call [`FanController::startup`] before the first tick.
    pub fn new(
        home: PathBuf,
        chip: Option<Box<dyn FanChip>>,
        chip_error: Option<String>,
        temps: BTreeMap<TempSource, Box<dyn TempReader>>,
        config: FansFile,
        confirmed_map: Option<String>,
    ) -> Self {
        let headers = chip.as_ref().map_or(0, |c| c.channels().len());
        let writes_allowed = chip
            .as_ref()
            .is_some_and(|c| confirmed_map.as_deref() == Some(c.map_name()));
        if let Some(c) = chip.as_ref().filter(|_| !writes_allowed) {
            tracing::warn!(map = c.map_name(), "channel map not confirmed; no register writes will be made");
        }
        Self {
            home,
            chip,
            chip_error,
            temps,
            config,
            slots: (0..headers).map(|_| Slot::default()).collect(),
            failed: BTreeSet::new(),
            last_good: BTreeMap::new(),
            temp_cache: BTreeMap::new(),
            bios_state: BiosState::default(),
            writes_allowed,
            state_trusted: false,
        }
    }

    /// Whether this controller may write fan registers (the channel map is confirmed).
    pub fn writes_allowed(&self) -> bool {
        self.writes_allowed
    }

    /// Whether `bios-state.json` passed the owner check. Startup reads it only then.
    pub fn set_state_trusted(&mut self, trusted: bool) {
        self.state_trusted = trusted;
    }

    /// Why the fan chip is unavailable, if it is (set at startup, logged then).
    pub fn chip_error(&self) -> Option<&str> {
        self.chip_error.as_deref()
    }

    pub fn boot_color(&self) -> Option<Rgb8> {
        self.config.boot_color
    }

    pub fn active_profile(&self) -> &str {
        &self.config.active_profile
    }

    /// Reads `bios-state.json` only when it passed the owner check (see `set_state_trusted`).
    fn read_state(&self, path: &std::path::Path) -> Result<BiosState, StoreError> {
        if self.state_trusted {
            store::load_bios_state(path)
        } else {
            Ok(BiosState::default())
        }
    }

    /// Restores BIOS on every header, then applies the active profile. A `bios-state.json`
    /// left by an earlier run is logged as an unclean exit.
    pub fn startup(&mut self, now: Instant) {
        let path = paths::bios_state_path(&self.home);
        match self.read_state(&path) {
            Ok(previous) if !previous.headers.is_empty() => {
                let names: Vec<&String> = previous.headers.keys().collect();
                tracing::warn!(headers = ?names, "bios-state.json present: an earlier run did not restore BIOS; restoring now");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "cannot read bios-state.json; restoring every header anyway"),
        }
        // Restore-to-auto is allowed even while unconfirmed: a header must never stay manual
        // because the map is unconfirmed. Applying the profile is not.
        if !self.restore_all("startup: BIOS before any profile") {
            tracing::error!("BIOS restore incomplete: the profile is not applied");
            return;
        }
        if !self.writes_allowed {
            tracing::warn!("channel map unconfirmed: BIOS restored, profile not applied");
            return;
        }
        self.apply_profile(now);
    }

    /// Returns every controllable header to BIOS control, then clears the saved state.
    /// Failures are logged per header; the rest still get restored. Writes only the fan-mode
    /// register, and only to clear a header's manual bit (auto), so it runs while unconfirmed.
    /// Returns true only when every header was handed back. With no chip, or after any failure,
    /// `bios-state.json` stays on disk and the result is false.
    pub fn restore_all(&mut self, reason: &str) -> bool {
        let mut any_failed = self.chip.is_none();
        if let Some(chip) = self.chip.as_mut() {
            for (ch, spec) in chip.channels().iter().enumerate() {
                if !spec.controllable() {
                    continue;
                }
                match chip.set_manual(ch, false) {
                    Ok(()) => tracing::info!(header = spec.id, reason, "header returned to BIOS control"),
                    Err(e) => {
                        any_failed = true;
                        tracing::error!(header = spec.id, error = %e, reason, "could not return header to BIOS");
                    }
                }
            }
        }
        for slot in &mut self.slots {
            slot.manual_since = None;
        }
        if any_failed {
            // A header may still be manual: keep bios-state.json so the next start restores it.
            tracing::error!(reason, "keeping bios-state.json: nothing or not every header was handed back");
            return false;
        }
        self.bios_state = BiosState::default();
        if let Err(e) = store::remove_bios_state(&paths::bios_state_path(&self.home)) {
            tracing::error!(error = %e, "could not remove bios-state.json");
        }
        true
    }

    /// Restores BIOS for the headers the saved state names. Used by `--restore-bios`
    /// and by the panic hook, which run without a controller. Returns how many were restored.
    pub fn restore_from_file(home: &std::path::Path, chip: &mut dyn FanChip) -> io::Result<usize> {
        let count = Self::restore_chip_only(chip)?;
        store::remove_bios_state(&paths::bios_state_path(home)).map_err(|e| io::Error::other(e.to_string()))?;
        Ok(count)
    }

    /// Hands every controllable header to auto and touches no file. Used when the machine home
    /// could not be secured (it may be a junction), so no file operation happens there.
    pub fn restore_chip_only(chip: &mut dyn FanChip) -> io::Result<usize> {
        // The saved state is not read here: every controllable header is set to auto regardless.
        let mut count = 0;
        for (ch, spec) in chip.channels().iter().enumerate() {
            if !spec.controllable() {
                continue;
            }
            chip.set_manual(ch, false)?;
            count += 1;
            tracing::info!(header = spec.id, "restored BIOS control");
        }
        Ok(count)
    }

    fn desired(&self, ch: usize) -> FanMode {
        let Some(id) = self.chip.as_ref().and_then(|c| c.channels().get(ch)).map(|c| c.id) else {
            return FanMode::Bios;
        };
        self.config
            .profiles
            .iter()
            .find(|p| p.name == self.config.active_profile)
            .and_then(|p| p.headers.get(id).cloned())
            .unwrap_or(FanMode::Bios)
    }

    /// Applies the active profile to every header.
    pub fn apply_profile(&mut self, now: Instant) {
        for ch in 0..self.slots.len() {
            self.apply_header(ch, now);
        }
    }

    fn apply_header(&mut self, ch: usize, now: Instant) {
        if ch >= self.slots.len() || !self.writes_allowed {
            return;
        }
        self.slots[ch].handed_back = false;
        self.slots[ch].curve = CurveState::default();
        self.slots[ch].override_reason = None;
        match self.desired(ch) {
            FanMode::Bios => self.release(ch, "mode set to BIOS"),
            _ => {
                if self.slots[ch].manual_since.is_none() {
                    if let Err(reason) = self.engage(ch, now) {
                        self.hand_back(ch, reason);
                        return;
                    }
                }
                self.drive(ch, now);
            }
        }
    }

    /// Saves the header's BIOS state, then takes it to manual. Refuses when the save fails.
    fn engage(&mut self, ch: usize, now: Instant) -> Result<(), String> {
        let chip = self.chip.as_mut().ok_or("no fan chip")?;
        let spec = *chip.channels().get(ch).ok_or("no such header")?;
        let manual = chip.read_manual(ch).map_err(|e| format!("read manual bit: {e}"))?;
        let duty = chip.read_duty_percent(ch).ok().flatten();
        self.bios_state.headers.insert(spec.id.to_string(), SavedHeader { manual, duty_percent: duty });
        store::save_bios_state(&paths::bios_state_path(&self.home), &self.bios_state)
            .map_err(|e| format!("cannot save bios-state.json: {e}"))?;
        chip.set_manual(ch, true).map_err(|e| format!("set manual: {e}"))?;
        tracing::info!(header = spec.id, "header taken to manual control");
        self.slots[ch].manual_since = Some(now);
        Ok(())
    }

    /// Takes a manual header back to BIOS when the user asks for BIOS. Failures are logged.
    fn release(&mut self, ch: usize, reason: &str) {
        let Some(slot) = self.slots.get_mut(ch) else { return };
        if slot.manual_since.is_none() {
            return;
        }
        slot.manual_since = None;
        let id = self.chip.as_ref().and_then(|c| c.channels().get(ch)).map_or("", |s| s.id);
        if let Some(chip) = self.chip.as_mut() {
            if let Err(e) = chip.set_manual(ch, false) {
                tracing::error!(header = id, error = %e, "could not return header to BIOS");
            }
        }
        self.bios_state.headers.remove(id);
        if let Err(e) = store::save_bios_state(&paths::bios_state_path(&self.home), &self.bios_state) {
            tracing::error!(error = %e, "could not update bios-state.json");
        }
        tracing::info!(header = id, reason, "header returned to BIOS control");
    }

    /// Hands a header back to BIOS for safety and makes the handback sticky.
    fn hand_back(&mut self, ch: usize, reason: String) {
        let Some(chip) = self.chip.as_mut() else { return };
        let Some(id) = chip.channels().get(ch).map(|s| s.id) else { return };
        tracing::warn!(header = id, reason = %reason, "safety handback: header returned to BIOS");
        if let Err(e) = chip.set_manual(ch, false) {
            tracing::error!(header = id, error = %e, "handback write failed; chip state unknown");
        }
        self.bios_state.headers.remove(id);
        if let Err(e) = store::save_bios_state(&paths::bios_state_path(&self.home), &self.bios_state) {
            tracing::error!(error = %e, "could not update bios-state.json");
        }
        if let Some(slot) = self.slots.get_mut(ch) {
            slot.manual_since = None;
            slot.handed_back = true;
            slot.override_reason = Some(reason);
        }
    }

    fn read_temp(&mut self, source: &TempSource) -> Option<f32> {
        if self.failed.contains(source) {
            return None;
        }
        let value = match source {
            TempSource::Board(name) => self.chip.as_mut()?.read_board_temp(name).ok().flatten(),
            other => self.temps.get_mut(other)?.read_celsius().ok(),
        }?;
        value.is_finite().then_some(value)
    }

    fn refresh_temps(&mut self, now: Instant) {
        let mut sources: Vec<TempSource> = self.temps.keys().cloned().collect();
        if let Some(chip) = self.chip.as_ref() {
            sources.extend(chip.board_sensor_names().iter().map(|n| TempSource::Board((*n).to_string())));
        }
        self.temp_cache.clear();
        for source in sources {
            if let Some(value) = self.read_temp(&source) {
                self.last_good.insert(source.clone(), now);
                self.temp_cache.insert(source, value);
            }
        }
    }

    /// One 1 Hz pass: refresh temperatures, then drive every header in a manual mode.
    /// Does not depend on any pipe client being connected.
    pub fn tick(&mut self, now: Instant) {
        self.refresh_temps(now);
        if !self.writes_allowed {
            return;
        }
        for ch in 0..self.slots.len() {
            let slot = &self.slots[ch];
            if slot.manual_since.is_some() && !slot.handed_back {
                self.drive(ch, now);
            }
        }
    }

    /// Computes this header's target, clamps it, writes it and reads it back.
    fn drive(&mut self, ch: usize, now: Instant) {
        let mode = self.desired(ch);
        let Some(spec) = self.chip.as_ref().and_then(|c| c.channels().get(ch).copied()) else { return };
        let (target, reference) = match &mode {
            FanMode::Bios => return,
            FanMode::Fixed { percent } => {
                let reference = self.temp_cache.get(&TempSource::Cpu).copied();
                (*percent, reference)
            }
            FanMode::Curve { source, points } => {
                if let Some(temp) = self.temp_cache.get(source).copied() {
                    (self.slots[ch].curve.step(points, temp, now), Some(temp))
                } else {
                    let since = self.slots[ch].manual_since.unwrap_or(now);
                    let last = self.last_good.get(source).copied().unwrap_or(since);
                    if now.saturating_duration_since(last) >= Duration::from_millis(SENSOR_LOSS_TIMEOUT_MS) {
                        self.hand_back(
                            ch,
                            format!("curve source {} unavailable for over {} ms", source_label(source), SENSOR_LOSS_TIMEOUT_MS),
                        );
                    }
                    return;
                }
            }
        };
        let base = floor_for(spec.id, self.config.floor_percent);
        let floor = runtime_floor(spec.id, self.config.floor_percent, reference);
        let mut percent = target.max(floor);
        let mut reason = None;
        if floor > base && percent == floor {
            if let Some(t) = reference {
                reason = Some(format!("hot floor {floor}% while CPU is {t:.0} °C"));
            }
        }
        if percent > 100 {
            percent = 100;
        }
        match self.write_and_verify(ch, percent) {
            Ok(()) => {
                if let Some(slot) = self.slots.get_mut(ch) {
                    slot.override_reason = reason;
                }
            }
            Err(e) => self.hand_back(ch, format!("write failed: {e}")),
        }
    }

    fn write_and_verify(&mut self, ch: usize, percent: u8) -> Result<(), String> {
        let chip = self.chip.as_mut().ok_or("no fan chip")?;
        chip.write_duty_percent(ch, percent).map_err(|e| format!("write {percent}%: {e}"))?;
        // The EC applies a new duty a moment after it accepts it (nct6687d: writes "don't take
        // effect immediately" on some MSI boards), so the readback gets READBACK_SETTLE.
        let started = Instant::now();
        let mut seen = Vec::new();
        loop {
            match chip.read_duty_percent(ch).map_err(|e| format!("readback: {e}"))? {
                Some(read) if read.abs_diff(percent) <= READBACK_TOLERANCE => {
                    if !seen.is_empty() {
                        tracing::info!(channel = ch, percent, settle_ms = started.elapsed().as_millis() as u64, ?seen, "the duty settled");
                    }
                    break;
                }
                Some(read) => seen.push(read),
                None => return Err("no duty readback".into()),
            }
            if started.elapsed() >= READBACK_SETTLE {
                let last = seen.last().copied().unwrap_or_default();
                return Err(format!("readback {last}% for {percent}% after {} ms", READBACK_SETTLE.as_millis()));
            }
            std::thread::sleep(READBACK_POLL);
        }
        if !chip.read_manual(ch).map_err(|e| format!("manual readback: {e}"))? {
            return Err("chip reports BIOS control after a manual write".into());
        }
        Ok(())
    }

    fn persist(&self) -> Result<(), FanError> {
        store::save_fans(&paths::fans_path(&self.home), &self.config)?;
        Ok(())
    }

    fn check_headers(&self, headers: &BTreeMap<String, FanMode>) -> Result<(), FanError> {
        let Some(chip) = self.chip.as_ref() else { return Ok(()) };
        for (id, mode) in headers {
            let spec = chip
                .channels()
                .iter()
                .find(|c| c.id == id.as_str())
                .ok_or_else(|| FanError::Validation(format!("unknown header {id}")))?;
            if !spec.controllable() && *mode != FanMode::Bios {
                return Err(FanError::Validation(format!("{id} is tach-only and cannot take a mode")));
            }
        }
        Ok(())
    }

    /// Switches the active profile and applies it.
    pub fn set_profile(&mut self, name: &str, now: Instant) -> Result<(), FanError> {
        if !self.config.profiles.iter().any(|p| p.name == name) {
            return Err(FanError::NotFound(format!("no profile named {name}")));
        }
        self.config.active_profile = name.to_string();
        self.persist()?;
        self.apply_profile(now);
        Ok(())
    }

    /// Creates or replaces a profile after validating every mode in it.
    pub fn put_profile(&mut self, profile: FanProfile, now: Instant) -> Result<(), FanError> {
        validate_profile(&profile, self.config.floor_percent)?;
        self.check_headers(&profile.headers)?;
        let active = profile.name == self.config.active_profile;
        match self.config.profiles.iter_mut().find(|p| p.name == profile.name) {
            Some(existing) => *existing = profile,
            None => self.config.profiles.push(profile),
        }
        self.persist()?;
        if active {
            self.apply_profile(now);
        }
        Ok(())
    }

    /// Removes a saved profile. The active profile and the last remaining one can't be removed.
    pub fn delete_profile(&mut self, name: &str) -> Result<(), FanError> {
        if !self.config.profiles.iter().any(|p| p.name == name) {
            return Err(FanError::NotFound(format!("no fan profile named {name:?}")));
        }
        if name == self.config.active_profile {
            return Err(FanError::Conflict(format!("{name:?} is the active profile; switch to another one first")));
        }
        if self.config.profiles.len() <= 1 {
            return Err(FanError::Conflict("the last fan profile can't be removed".into()));
        }
        self.config.profiles.retain(|p| p.name != name);
        self.persist()?;
        tracing::info!(profile = name, "fan profile deleted");
        Ok(())
    }

    /// Sets one header's mode in the active profile, validates it, and applies it.
    /// Refused when no fan chip is attached: nothing could be driven.
    pub fn set_header_mode(&mut self, header: &str, mode: FanMode, now: Instant) -> Result<(), FanError> {
        if self.chip.is_none() {
            return Err(FanError::Unavailable(
                self.chip_error.clone().unwrap_or_else(|| "no fan chip is attached".into()),
            ));
        }
        validate_mode(header, &mode, self.config.floor_percent)?;
        let single = BTreeMap::from([(header.to_string(), mode.clone())]);
        self.check_headers(&single)?;
        let active = self.config.active_profile.clone();
        let profile = self
            .config
            .profiles
            .iter_mut()
            .find(|p| p.name == active)
            .ok_or_else(|| FanError::NotFound(format!("no profile named {active}")))?;
        profile.headers.insert(header.to_string(), mode);
        self.persist()?;
        if let Some(ch) = self.chip.as_ref().and_then(|c| c.channels().iter().position(|s| s.id == header)) {
            self.apply_header(ch, now);
        }
        Ok(())
    }

    /// Replaces the whole `fans.toml` (the daemon's `PutFanConfig`).
    pub fn put_config(&mut self, file: FansFile, now: Instant) -> Result<(), FanError> {
        if file.schema_version != FANS_SCHEMA {
            return Err(FanError::Validation(format!("schema_version {} is not {FANS_SCHEMA}", file.schema_version)));
        }
        for profile in &file.profiles {
            validate_profile(profile, file.floor_percent)?;
            self.check_headers(&profile.headers)?;
        }
        if !file.profiles.iter().any(|p| p.name == file.active_profile) {
            return Err(FanError::NotFound(format!("no profile named {}", file.active_profile)));
        }
        self.config = file;
        self.persist()?;
        self.apply_profile(now);
        Ok(())
    }

    pub fn set_boot_color(&mut self, color: Rgb8) -> Result<(), FanError> {
        self.config.boot_color = Some(color);
        self.persist()
    }

    /// Diagnostics: treats a temperature source as failed until cleared.
    pub fn set_sensor_failed(&mut self, source: TempSource, failed: bool) {
        tracing::warn!(source = %source_label(&source), failed, "diagnostic sensor failure toggled");
        if failed {
            self.failed.insert(source);
        } else {
            self.failed.remove(&source);
        }
    }

    /// Current status, reading each header live from the chip.
    pub fn status(&mut self) -> FansStatus {
        let headers = match self.chip.as_ref().map(|c| c.channels()) {
            Some(specs) => specs.to_vec(),
            None => Vec::new(),
        };
        let mut out = Vec::with_capacity(headers.len());
        let unconfirmed = self.chip.as_ref().map(|c| unconfirmed_reason(c.map_name()));
        for (ch, spec) in headers.iter().enumerate() {
            let mode = self.desired(ch);
            let slot = &self.slots[ch];
            let (rpm, pwm) = match self.chip.as_mut() {
                Some(chip) => (
                    chip.read_rpm(ch).ok().flatten(),
                    chip.read_duty_percent(ch).ok().flatten(),
                ),
                None => (None, None),
            };
            let on_bios = !self.writes_allowed || slot.manual_since.is_none() || matches!(mode, FanMode::Bios);
            let override_reason = if self.writes_allowed {
                slot.override_reason.clone()
            } else {
                unconfirmed.clone()
            };
            out.push(FanHeaderStatus {
                id: spec.id.to_string(),
                label: spec.label.to_string(),
                rpm,
                pwm_percent: pwm,
                mode,
                on_bios,
                override_reason,
            });
        }
        let mut temps: Vec<TempReading> = Vec::new();
        let mut sources: Vec<TempSource> = self.temps.keys().cloned().collect();
        if let Some(chip) = self.chip.as_ref() {
            sources.extend(chip.board_sensor_names().iter().map(|n| TempSource::Board((*n).to_string())));
        }
        for source in sources {
            temps.push(TempReading {
                label: source_label(&source),
                celsius: self.temp_cache.get(&source).copied(),
                source,
            });
        }
        FansStatus {
            chip: self.chip.as_ref().map(|c| c.chip_name()),
            hardware_watchdog: HARDWARE_WATCHDOG,
            headers: out,
            temps,
            active_profile: self.config.active_profile.clone(),
            profiles: self.config.profiles.iter().map(|p| p.name.clone()).collect(),
            floor_percent: self.config.floor_percent,
        }
    }
}

fn source_label(source: &TempSource) -> String {
    match source {
        TempSource::Cpu => "CPU (Tctl)".into(),
        TempSource::Gpu => "GPU (NVML)".into(),
        TempSource::Board(name) => name.clone(),
    }
}
