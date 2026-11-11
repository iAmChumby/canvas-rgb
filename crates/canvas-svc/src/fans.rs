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
//! - A sleep pauses the loop; after a resume the controller waits `RESUME_GRACE` for firmware
//!   to give the fan controller back, then re-applies the profile (see `SleepState`).

use crate::chip::FanChip;
use crate::paths;
use crate::store::{self, BiosState, SavedHeader, StoreError};
use crate::temps::TempReader;
use canvas_proto::config::{FansFile, FANS_SCHEMA};
use canvas_proto::fans::{
    floor_for, hot_limit_status, rated_cpu_max_c, runtime_floor_at, validate_hot_limits, validate_mode,
    validate_profile, CurvePoint, FanHeaderStatus, FanMode, FanProfile, FanValidationError, FansStatus, HotKind,
    HotLimitStatus, HotLimits, TempReading, TempSource, SENSOR_LOSS_TIMEOUT_MS,
};
use canvas_proto::Rgb8;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

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
/// The hold alone steps the duty down only when the smoothed temperature asks for at least this many
/// percentage points less. Without it a 50/54 C input dipped the duty by 1 % every hold period.
pub const CURVE_STEP_DOWN_MIN_PERCENT: u8 = 3;
/// Issue #255: the smallest rise of the hot-limit boost, in percentage points per second. A
/// reference temperature at the line still steps the duty up by at least this much, so "Don't
/// exceed" always answers.
pub const HOT_BOOST_MIN_RISE: f32 = 1.0;
/// Issue #255: how fast the boost grows per degree above the line, in percentage points per
/// second per degree. The BIOS Smart Fan step-up shape: the further over the line, the faster.
pub const HOT_BOOST_GAIN: f32 = 1.0;
/// Issue #255: the deadband below the line, in degrees, where the boost holds instead of
/// decaying. A temperature hovering at the line never flips the duty (issue #17).
pub const HOT_BOOST_DEADBAND_C: f32 = 2.0;
/// Issue #255: how fast the boost decays below the deadband, in percentage points per second.
/// The fall is much slower than the rise, so the fan gives the temperature time to come back.
pub const HOT_BOOST_DECAY: f32 = 0.25;
/// Issue #255 review: the largest tick delta the boost counts, in seconds. A stalled loop, a
/// debugger or a missed resume must not wind the integral term by minutes in one step.
pub const HOT_BOOST_MAX_DT_S: f32 = 2.0;
/// Issue #255 review: the boost's own upper bound, in percentage points (anti-windup). The
/// output is capped at 100 separately, so the term is still there when the curve later drops.
pub const HOT_BOOST_MAX: f32 = 100.0;
/// Issue #297: how often the hot-hold state may be logged per header. The loop runs at 1 Hz and
/// the duty can change on every tick, so a line per tick would bury the log; a change held back
/// by this interval is written by the first tick after it.
pub const HOT_LOG_MIN_INTERVAL: Duration = Duration::from_secs(5);
/// A gap between two curve updates longer than this (a missing sensor, a sleep) restarts the
/// smoothing from the current reading.
const CURVE_RESEED: Duration = Duration::from_secs(10);
/// How long the controller leaves the fan chip alone after a resume before it writes again.
/// Issue #29: a duty write that landed in the resume transition, while firmware still owned the
/// fan controller, read back the BIOS duty for the whole `READBACK_SETTLE` and tripped the safety
/// handback. The logged wake took about 2 s from the clock reset (Kernel-General 1) to the
/// Power-Troubleshooter line, so 5 s covers it with margin. UNKNOWN on real hardware: how long
/// the firmware really holds the controller; the next real wake settles it.
pub const RESUME_GRACE: Duration = Duration::from_secs(5);
/// A wall-clock gap between two ticks longer than this is a sleep whose power event never
/// reached the service (the tick is 1 s; 30 s leaves room for a stalled machine).
pub const TICK_GAP_RESUME: Duration = Duration::from_secs(30);

/// A suspend with no resume holds fan writes at most this many ticks (1 s each). Ticks only run
/// while the machine is awake, so 60 of them mean a resume notification never came; without the cap
/// a missed resume would freeze the fans at their last duty with no temperature read, and the chip
/// has no hardware watchdog (`HARDWARE_WATCHDOG`).
pub const SUSPEND_HOLD_CAP_TICKS: u32 = 60;

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

/// Per-header state that smooths a curve and holds its output steady (issue #17). A rise is
/// taken at once (a fan never lags a climbing temperature). Step down needs the smoothed temperature
/// to fall `CURVE_STEP_DOWN_DROP_C` below the peak since the last duty change and to stay
/// there for the smoothing time constant, or `CURVE_STEP_DOWN_HOLD` to pass.
#[derive(Default)]
struct CurveState {
    /// The mode this state was built for; a different curve starts it afresh.
    mode: Option<FanMode>,
    smoothed: Option<f32>,
    last_at: Option<Instant>,
    held: Option<u8>,
    /// The highest smoothed temperature since the last duty change. Every fall is measured
    /// from this peak, never from a value re-anchored by a step down, so a spike train
    /// cannot walk the duty down a few degrees at a time.
    peak: f32,
    changed_at: Option<Instant>,
    /// When the smoothed temperature first fell `CURVE_STEP_DOWN_DROP_C` below `peak`,
    /// while that fall lasts.
    below_since: Option<Instant>,
    /// Whether the hot floor (`runtime_floor`) is latched for this header. Issue #130: it engages
    /// on the first reading at or above the source's hot limit (not the fixed `HOT_TEMP_C` any
    /// more) and releases only after the temperature stays below the limit continuously for
    /// `CURVE_STEP_DOWN_HOLD`.
    hot_latched: bool,
    /// When the temperature first fell below the hot limit while the hot floor is latched.
    hot_below_since: Option<Instant>,
    /// Whether the fixed safety ceiling is latched: at or above the ceiling the header runs at
    /// 100 %, whatever the curve asks for. It releases with the same hold-time rule as the hot
    /// floor (issue #130).
    ceiling_latched: bool,
    /// When the temperature first fell below the ceiling while it is latched.
    ceiling_below_since: Option<Instant>,
    /// Issue #255: the closed-loop boost above the hot limit, in percentage points. It rises
    /// while the reference temperature is at or above the line and decays slowly below the
    /// deadband, so the header holds the line instead of sitting on a flat floor.
    hot_boost: f32,
    /// When the boost was last updated, for the tick delta. `None` before the first tick.
    boost_last_at: Option<Instant>,
    /// Issue #297: what the last hot-hold INFO line said, and when it was written. The line is
    /// written on a change, never more often than `HOT_LOG_MIN_INTERVAL`.
    hot_log: Option<HotLogLine>,
    hot_log_at: Option<Instant>,
}

/// Issue #297: the hot-hold state one log line described, so the next tick can tell whether
/// anything changed worth another line.
#[derive(Clone, PartialEq)]
struct HotLogLine {
    engaged: bool,
    asked: u8,
    reason: Option<String>,
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
        // The peak rises with the temperature and is only reset by a duty change,
        // so a brief dip inside a spike still measures its fall from the real peak.
        self.peak = self.peak.max(smoothed);
        let raw = curve_percent(points, temp);
        let Some(held) = self.held else {
            self.set(raw, now);
            return raw;
        };
        // A rise is taken at once (issue #37, and Fan Control / BIOS Smart Fan step up
        // immediately): a fan must never lag a temperature that is climbing.
        if raw > held {
            self.set(raw, now);
            return raw;
        }
        let calm = curve_percent(points, smoothed);
        let waited = self.changed_at.is_none_or(|t| now.saturating_duration_since(t) >= CURVE_STEP_DOWN_HOLD)
            && calm.saturating_add(CURVE_STEP_DOWN_MIN_PERCENT) <= held;
        if calm < held && smoothed <= self.peak - CURVE_STEP_DOWN_DROP_C {
            if self.below_since.is_none() {
                self.below_since = Some(now);
            }
        } else {
            self.below_since = None;
        }
        // The fall must persist for the smoothing time constant: the average turns a
        // lone spike into about a degree, so a real cool-down looks like a steady
        // fall, not one tick below the line.
        let dwelled = self
            .below_since
            .is_some_and(|since| now.saturating_duration_since(since) >= Duration::from_millis((CURVE_SMOOTHING_TAU_S * 1000.0) as u64));
        if calm < held && (dwelled || waited) {
            self.set(calm, now);
            return calm;
        }
        held
    }

    fn set(&mut self, percent: u8, now: Instant) {
        self.held = Some(percent);
        // A duty change restarts the peak at the current smoothed temperature:
        // every fall after this is measured from here.
        self.peak = self.smoothed.unwrap_or_default();
        self.changed_at = Some(now);
        self.below_since = None;
    }

    /// Issue #297: whether this tick's hot-hold state is worth an INFO line, and records it.
    /// A line goes out on a change (engaged or released, the duty asked, or the reason), never
    /// more often than `HOT_LOG_MIN_INTERVAL`; a change held back by the interval is written by
    /// the first tick after it, because the state still differs from the last one written.
    fn wants_hot_log(&mut self, now: Instant, engaged: bool, asked: u8, reason: &Option<String>) -> bool {
        let state = HotLogLine { engaged, asked, reason: reason.clone() };
        if self.hot_log.as_ref() == Some(&state) {
            return false;
        }
        if self.hot_log_at.is_some_and(|at| now.saturating_duration_since(at) < HOT_LOG_MIN_INTERVAL) {
            return false;
        }
        self.hot_log = Some(state);
        self.hot_log_at = Some(now);
        true
    }

    /// Latches the hot floor: engages on every reading at or above `limit_c`, and releases only
    /// after the temperature stays below the limit for the curve's hold time. It is a floor
    /// under the curve, never a cap. A missing reading changes nothing.
    fn note_temp(&mut self, temp_c: Option<f32>, limit_c: f32, now: Instant) {
        note_latch(&mut self.hot_latched, &mut self.hot_below_since, temp_c, limit_c, now);
    }

    /// Latches the safety ceiling: engages on every reading at or above `ceiling_c`, and releases
    /// after the temperature stays below the ceiling for the same hold time as the hot floor
    /// (issue #130). A missing reading changes nothing.
    fn note_ceiling(&mut self, temp_c: Option<f32>, ceiling_c: f32, now: Instant) {
        note_latch(&mut self.ceiling_latched, &mut self.ceiling_below_since, temp_c, ceiling_c, now);
    }

    /// Issue #255: one tick of the hot-limit boost. `temp_c` is the reference temperature and
    /// `limit_c` the hot line. At or above the line the boost rises (fast), inside the deadband
    /// it holds, and below the deadband it decays (slowly). While the safety ceiling is latched
    /// the boost is frozen: the ceiling already runs the header at 100 %. A missing reading
    /// keeps the boost through the short gap: the Curve arm holds its write and the Fixed arm
    /// keeps its duty until `SENSOR_LOSS_TIMEOUT_MS`. The Curve handback clears it; the Fixed
    /// arm drops it at the timeout and writes its plain duty again.
    fn note_boost(&mut self, temp_c: Option<f32>, limit_c: f32, now: Instant, ceiling_latched: bool) {
        if ceiling_latched {
            self.boost_last_at = Some(now);
            return;
        }
        let Some(temp) = temp_c else {
            // Issue #255 review: a gap shorter than the sensor-loss timeout keeps the boost.
            // The handback resets it when the loss is real.
            self.boost_last_at = Some(now);
            return;
        };
        let dt = self
            .boost_last_at
            .map_or(1.0, |previous| now.saturating_duration_since(previous).as_secs_f32())
            .min(HOT_BOOST_MAX_DT_S);
        self.boost_last_at = Some(now);
        if temp >= limit_c {
            // Anti-windup: bound the term itself, never the output's shortfall, so a curve that
            // later steps down while the temperature is still over the line keeps the fan up.
            self.hot_boost = (self.hot_boost + HOT_BOOST_MIN_RISE.max(HOT_BOOST_GAIN * (temp - limit_c)) * dt)
                .min(HOT_BOOST_MAX);
        } else if temp >= limit_c - HOT_BOOST_DEADBAND_C {
            // Inside the deadband the boost holds: a temperature hovering at the line must not
            // flip the duty (issue #17).
        } else {
            self.hot_boost = (self.hot_boost - HOT_BOOST_DECAY * dt).max(0.0);
        }
    }
}

/// One latched-over-a-line bookkeeping step, shared by the hot floor and the safety ceiling:
/// engage at once at or above `line_c`; release only after `CURVE_STEP_DOWN_HOLD` continuously
/// below `line_c`. The two add nothing the other lacks, so the rule lives in one place.
fn note_latch(latched: &mut bool, below_since: &mut Option<Instant>, temp_c: Option<f32>, line_c: f32, now: Instant) {
    let Some(temp) = temp_c else { return };
    if temp >= line_c {
        *latched = true;
        *below_since = None;
    } else if *latched {
        let since = *below_since.get_or_insert(now);
        if now.saturating_duration_since(since) >= CURVE_STEP_DOWN_HOLD {
            *latched = false;
            *below_since = None;
        }
    }
}

/// The hot-limit kind a mode reads: Fixed reads the CPU reference (as `drive` treats it), a
/// curve reads its source's kind, BIOS none.
fn mode_hot_kind(mode: &FanMode) -> Option<HotKind> {
    match mode {
        FanMode::Bios => None,
        FanMode::Fixed { .. } => Some(HotKind::Cpu),
        FanMode::Curve { source, .. } => Some(source.hot_kind()),
    }
}

/// The hot-limit kind a mode reads, for latch preservation across an edit.
fn same_hot_kind(a: &FanMode, b: &FanMode) -> bool {
    mode_hot_kind(a) == mode_hot_kind(b)
}

#[derive(Default)]
struct Slot {
    curve: CurveState,
    /// The mode this header was last driven in (issue #255). A change resets the boost: the
    /// closed loop starts over on a new mode.
    mode: Option<FanMode>,
    /// `Some(time)` while this service holds the header in manual mode.
    manual_since: Option<Instant>,
    /// Set by a safety or failure handback. Sticky until the user applies the header again.
    handed_back: bool,
    override_reason: Option<String>,
    /// The last duty this service wrote to the header, so a latched header with a
    /// missing reference temperature can hold it instead of writing a lower duty.
    last_written: Option<u8>,
}

/// Sleep and resume bookkeeping. Rust's `Instant` on Windows reads QueryPerformanceCounter, and the
/// std docs say it is "not specified whether system suspends count as elapsed time or not", so
/// `Instant` cannot tell a sleep from a short tick. `SystemTime` follows the wall clock, which the
/// OS sets from the RTC at resume, so a large `SystemTime` gap between ticks marks a sleep.
#[derive(Default)]
struct SleepState {
    /// Between a suspend event and the matching resume: no writes.
    suspended: bool,
    /// Ticks seen since the suspend event; bounds the hold (`SUSPEND_HOLD_CAP_TICKS`).
    held_ticks: u32,
    /// After a resume: no tick writes until this instant.
    grace_until: Option<Instant>,
    /// Headers to re-apply when the grace ends: those under manual control before the sleep, plus
    /// every header the user applied inside the window (held, not written, until then).
    reapply: BTreeSet<usize>,
    /// Wall-clock time of the previous tick, for the gap fallback.
    last_wall: Option<SystemTime>,
    /// Set by a resume; the service core takes it to refresh the daemon heartbeat clock.
    resumed: bool,
}

impl SleepState {
    fn in_window(&self, now: Instant) -> bool {
        self.suspended || self.grace_until.is_some_and(|until| now < until)
    }
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
    sleep: SleepState,
    /// Baseboard product from the registry probe (issue #128); `None` until probed.
    board_product: Option<String>,
    /// Headers the board does not route for `board_product` (issue #128), from the table in
    /// `board.rs`. Empty when the board is unknown.
    absent_headers: BTreeSet<String>,
    /// Per-header `available` overrides in `chip.toml` (issue #128): `true` says "show
    /// anyway", `false` says "not on my board". No entry means the table decides.
    header_overrides: BTreeMap<String, bool>,
    /// Issue #130: the rated maximum temperatures the hot lines are measured from. The CPU one
    /// is read from the CPU brand string; the GPU one is NVML's slowdown threshold. `None`
    /// means unknown: the spine's fallback takes over.
    cpu_rated_max_c: Option<f32>,
    gpu_rated_max_c: Option<f32>,
    /// The CPU brand string the rated maximum came from, for the startup hot-limits log.
    cpu_brand: Option<String>,
    /// Whether the one-time hot-limit line has been logged yet.
    hot_logged: bool,
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
            sleep: SleepState::default(),
            board_product: None,
            absent_headers: BTreeSet::new(),
            header_overrides: BTreeMap::new(),
            cpu_rated_max_c: None,
            gpu_rated_max_c: None,
            cpu_brand: None,
            hot_logged: false,
        }
    }

    /// Sets the baseboard product the service probed at startup (issue #128) and recomputes
    /// which headers the per-board table says this board does not route.
    pub fn set_board_product(&mut self, product: Option<String>) {
        if let Some(p) = &product {
            tracing::info!(board = %p, "baseboard product read at startup");
        }
        self.absent_headers = crate::board::absent_headers_for(product.as_deref());
        self.board_product = product;
    }

    /// Loads the per-header `available` overrides saved in `chip.toml` (issue #128).
    pub fn set_header_overrides(&mut self, overrides: BTreeMap<String, bool>) {
        self.header_overrides = overrides;
    }

    /// Sets the rated maximum temperatures (issue #130). `cpu_brand` is the CPU brand string the
    /// service read from the hardware; `gpu_max_c` is NVML's slowdown threshold. Tests set
    /// either through the same setters with fake values.
    pub fn set_rated(&mut self, cpu_brand: Option<&str>, gpu_max_c: Option<f32>) {
        self.cpu_rated_max_c = cpu_brand.and_then(rated_cpu_max_c);
        self.cpu_brand = cpu_brand.map(str::to_string);
        self.gpu_rated_max_c = gpu_max_c;
        self.hot_logged = false;
    }

    /// Sets the GPU rated maximum (NVML slowdown). Tests go through the fake that carries the
    /// value: `FakeGpuFans::slowdown_threshold_c`.
    pub fn set_gpu_rated_max_c(&mut self, gpu_max_c: Option<f32>) {
        self.gpu_rated_max_c = gpu_max_c;
        self.hot_logged = false;
    }

    /// Sets the CPU rated maximum from a brand string (tests can set the brand directly).
    pub fn set_cpu_brand(&mut self, cpu_brand: Option<&str>) {
        self.cpu_rated_max_c = cpu_brand.and_then(rated_cpu_max_c);
        self.cpu_brand = cpu_brand.map(str::to_string);
        self.hot_logged = false;
    }

    /// The rated maximum for a hot kind, when known. Board sensors have no part maximum.
    fn rated_for(&self, kind: HotKind) -> Option<f32> {
        match kind {
            HotKind::Cpu => self.cpu_rated_max_c,
            HotKind::Gpu => self.gpu_rated_max_c,
            HotKind::Board => None,
        }
    }

    /// The hot line in force per kind, for [`FanController::status`]. Logs the lines once, at
    /// the first call after the rated maxima are known (issue #130).
    pub fn hot_statuses(&mut self) -> Vec<HotLimitStatus> {
        if !self.hot_logged {
            let cpu = hot_limit_status(HotKind::Cpu, &self.config.hot_limits, self.rated_for(HotKind::Cpu));
            let gpu = hot_limit_status(HotKind::Gpu, &self.config.hot_limits, self.rated_for(HotKind::Gpu));
            let board = hot_limit_status(HotKind::Board, &self.config.hot_limits, None);
            let rated = |s: &HotLimitStatus| {
                let origin = if s.user_set { "set by you" } else { "automatic" };
                match s.rated_max_c {
                    Some(r) => format!("{origin}, rated {r:.0} °C"),
                    None => origin.to_string(),
                }
            };
            tracing::info!(
                "the hot limits are CPU {} °C ({}) [{}], GPU {} °C ({}), board {} °C",
                cpu.limit_c, rated(&cpu), self.cpu_brand.as_deref().unwrap_or("unknown CPU"),
                gpu.limit_c, rated(&gpu), board.limit_c,
            );
            self.hot_logged = true;
        }
        [HotKind::Cpu, HotKind::Gpu, HotKind::Board]
            .iter()
            .map(|kind| hot_limit_status(*kind, &self.config.hot_limits, self.rated_for(*kind)))
            .collect()
    }

    /// Whether `header` is routed on this board: a `chip.toml` override wins both ways,
    /// otherwise the per-board table decides (issue #128).
    pub fn header_available(&self, header: &str) -> bool {
        if let Some(available) = self.header_overrides.get(header) {
            return *available;
        }
        !self.absent_headers.contains(header)
    }

    /// Marks a header available ("show anyway") or absent ("not on my board") and persists
    /// it in `chip.toml` (issue #128, decision 3). A header marked absent is returned to
    /// BIOS here, and never written afterwards.
    ///
    /// "Show anyway" only says the header is on this board: it is not written here, and the
    /// header is written when the owner gives it a mode. The pipe request that reaches this
    /// is `PipeRequest::SetHeaderAvailability` (issue #146).
    pub fn set_header_override(&mut self, header: &str, available: bool) -> Result<(), FanError> {
        self.check_known_header(header)?;
        // Build the new map and save it first: a failed save changes nothing in memory
        // and writes nothing to the chip (issue #146 review S1).
        let mut next = self.header_overrides.clone();
        next.insert(header.to_string(), available);
        self.save_header_overrides_with(&next)?;
        self.header_overrides = next;
        tracing::info!(header, available, "header availability override saved");
        if !available {
            if let Some(ch) = self.channel_of(header) {
                self.release(ch, "header marked not on this board");
            }
        }
        Ok(())
    }

    /// Takes the owner's word off a header (issue #146): the key leaves `header_overrides`,
    /// `chip.toml` is saved and the per-board table decides again, as if no word had ever
    /// been given. If the board table hides the header it is released to BIOS; if the
    /// board routes it the active profile's mode for it resumes (or BIOS if the profile
    /// says BIOS), via `apply_header`.
    pub fn clear_header_override(&mut self, header: &str, now: Instant) -> Result<(), FanError> {
        self.check_known_header(header)?;
        // Save first, mutate after: a failed save changes nothing in memory.
        let mut next = self.header_overrides.clone();
        next.remove(header);
        self.save_header_overrides_with(&next)?;
        self.header_overrides = next;
        tracing::info!(header, "the header availability override was cleared");
        if self.header_available(header) {
            if let Some(ch) = self.channel_of(header) {
                self.apply_header(ch, now);
            }
        } else if let Some(ch) = self.channel_of(header) {
            self.release(ch, "header cleared to a board-hidden header");
        }
        Ok(())
    }

    /// The chip channel index for a header id, when the chip has it.
    fn channel_of(&self, header: &str) -> Option<usize> {
        self.chip.as_ref().and_then(|c| c.channels().iter().position(|s| s.id == header))
    }

    /// Refuses a header the chip does not have. A header the chip reads is known, whatever
    /// the board routes: the table decides that, the owner does not. With no chip every
    /// header is refused: there is no channel list to know it from.
    fn check_known_header(&self, header: &str) -> Result<(), FanError> {
        match self.chip.as_ref() {
            Some(chip) if chip.channels().iter().any(|c| c.id == header) => Ok(()),
            _ => Err(FanError::Validation(format!("unknown header {header}"))),
        }
    }

    /// Writes the per-header overrides back to `chip.toml`.
    fn save_header_overrides_with(&self, overrides: &BTreeMap<String, bool>) -> Result<(), FanError> {
        let path = paths::chip_path(&self.home);
        let mut state = store::load_chip_state(&path)?;
        state.header_overrides = overrides.clone();
        store::save_chip_state(&path, &state)?;
        Ok(())
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
        let header = self.chip.as_ref().and_then(|c| c.channels().get(ch)).map(|s| s.id.to_string());
        if let Some(id) = header.as_deref() {
            if !self.header_available(id) {
                tracing::info!(header = id, "header not on this board: skipped");
                return;
            }
        }
        if self.sleep.in_window(now) {
            // Issue #29: a write during the resume transition is exactly what failed. The config is
            // already saved; the header is applied when the window ends.
            tracing::info!(channel = ch, "apply held until the resume grace ends");
            self.sleep.reapply.insert(ch);
            return;
        }
        self.slots[ch].handed_back = false;
        if self.slots[ch].manual_since.is_none() {
            // A header taken over afresh starts its hysteresis afresh. One that stays under control
            // keeps its held duty when the profile is applied again, unless its curve changed
            // (`drive` notices that).
            self.slots[ch].curve = CurveState::default();
        }
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
        // Issue #255: the closed loop ends with the header. The boost is dropped with it.
        slot.curve.hot_boost = 0.0;
        slot.curve.boost_last_at = None;
        slot.mode = None;
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

    /// Headers this service drives in manual mode right now.
    fn controlled(&self) -> BTreeSet<usize> {
        (0..self.slots.len())
            .filter(|&ch| self.slots[ch].manual_since.is_some() && !self.slots[ch].handed_back)
            .collect()
    }

    /// The machine is going to sleep (`PBT_APMSUSPEND`): stop writing. The headers under control
    /// now are re-applied after the resume. Nothing is handed back on purpose; the chip keeps the
    /// last duty, and firmware takes over whatever it takes over.
    pub fn on_suspend(&mut self) {
        self.sleep.suspended = true;
        self.sleep.held_ticks = 0;
        self.sleep.grace_until = None;
        let held = self.controlled();
        tracing::info!(headers = ?held, "suspend: fan writes paused");
        self.sleep.reapply.extend(held);
    }

    /// The machine woke (`PBT_APMRESUME*`, or a tick gap). Writes stay off for `RESUME_GRACE`,
    /// then every header that was under control is re-applied, which clears a handback.
    pub fn on_resume(&mut self, now: Instant, source: &str) {
        self.sleep.suspended = false;
        self.sleep.grace_until = Some(now + RESUME_GRACE);
        self.sleep.resumed = true;
        // The resume is handled. The next tick's wall-clock gap spans the sleep and must not count
        // it a second time (that would restart the grace).
        self.sleep.last_wall = None;
        // Issue #255: a resume restarts every closed loop. The heat history before the sleep
        // says nothing about the temperature after it.
        for slot in &mut self.slots {
            slot.curve.hot_boost = 0.0;
            slot.curve.boost_last_at = None;
        }
        let held = self.controlled();
        tracing::info!(source, headers = ?held, grace_ms = RESUME_GRACE.as_millis() as u64, "resume: fan writes held, profile re-applies after the grace");
        self.sleep.reapply.extend(held);
    }

    /// True once after each resume. The service core uses it to refresh the daemon's heartbeat
    /// clock, because a heartbeat that looks hours old after a sleep is not a crash.
    pub fn take_resumed(&mut self) -> bool {
        std::mem::take(&mut self.sleep.resumed)
    }

    /// Re-applies the headers queued for the end of a grace window. One attempt each; a failure
    /// here is a real failure and the handback sticks.
    fn reapply_after_grace(&mut self, now: Instant) {
        let queued = std::mem::take(&mut self.sleep.reapply);
        if queued.is_empty() {
            return;
        }
        tracing::info!(headers = ?queued, "resume grace over: re-applying the profile");
        for ch in queued {
            self.apply_header(ch, now);
        }
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
            // Issue #255: a handback ends the closed loop; the boost starts over when the
            // header is applied again.
            slot.curve.hot_boost = 0.0;
            slot.curve.boost_last_at = None;
            slot.mode = None;
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
        self.tick_at(now, SystemTime::now());
    }

    /// [`FanController::tick`] with the wall clock passed in, so tests can move it.
    pub fn tick_at(&mut self, now: Instant, wall: SystemTime) {
        let gap = self.sleep.last_wall.and_then(|previous| wall.duration_since(previous).ok());
        let gap_resume = gap.is_some_and(|g| g > TICK_GAP_RESUME);
        if self.sleep.suspended {
            self.sleep.held_ticks += 1;
            if gap_resume {
                tracing::warn!(gap_s = gap.map_or(0, |g| g.as_secs()), "suspended, but the wall clock jumped and no resume event came: treating it as a resume");
                self.on_resume(now, "tick gap while suspended");
            } else if self.sleep.held_ticks > SUSPEND_HOLD_CAP_TICKS {
                tracing::warn!(ticks = self.sleep.held_ticks, "suspended with no resume event while the machine is awake: ending the hold");
                self.on_resume(now, "suspend hold cap");
            }
        } else if gap_resume {
            tracing::warn!(gap_s = gap.map_or(0, |g| g.as_secs()), "no power event, but the wall clock jumped between ticks: treating it as a resume");
            self.on_resume(now, "tick gap");
        }
        self.sleep.last_wall = Some(wall);
        if self.sleep.suspended {
            return;
        }
        self.refresh_temps(now);
        if !self.writes_allowed {
            return;
        }
        if let Some(until) = self.sleep.grace_until {
            if now < until {
                return;
            }
            self.sleep.grace_until = None;
            // `Instant` may or may not have counted the sleep, so a sensor's last good reading can
            // look hours old, and a sensor that is slow to return after a wake (NVML) would trip the
            // sensor-loss handback on the first write. Each sensor gets its full loss timeout from
            // the end of the grace.
            for last in self.last_good.values_mut() {
                *last = now;
            }
            self.reapply_after_grace(now);
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
        // Issue #128: never drive a header the board does not route.
        if !self.header_available(spec.id) {
            return;
        }
        // Issue #255: a mode change restarts the closed loop. The boost is an integral term
        // over the current mode; carrying it across a change would apply one mode's heat
        // history to another.
        if self.slots[ch].mode.as_ref() != Some(&mode) {
            self.slots[ch].curve.hot_boost = 0.0;
            self.slots[ch].curve.boost_last_at = None;
            self.slots[ch].mode = Some(mode.clone());
        }
        let (target, reference, kind) = match &mode {
            FanMode::Bios => return,
            FanMode::Fixed { percent } => {
                let reference = self.temp_cache.get(&TempSource::Cpu).copied();
                // A Fixed header has no curve source: its reference temperature is the CPU
                // reading (see the report on #130), so it takes the CPU hot limit and the CPU
                // ceiling.
                (*percent, reference, HotKind::Cpu)
            }
            FanMode::Curve { source, points } => {
                if let Some(temp) = self.temp_cache.get(source).copied() {
                    let curve = &mut self.slots[ch].curve;
                    if curve.mode.as_ref() != Some(&mode) {
                        // A mode or points change restarts the curve's smoothing, but the
                        // safety latches (hot floor and ceiling) survive it for the same
                        // source kind: clearing them on an edit would drop a latched 100 %
                        // to the edited curve's lower duty. Only a kind change resets them.
                        let kept = match curve.mode.as_ref() {
                            Some(previous) if same_hot_kind(previous, &mode) => Some((
                                curve.hot_latched,
                                curve.hot_below_since,
                                curve.ceiling_latched,
                                curve.ceiling_below_since,
                            )),
                            _ => None,
                        };
                        *curve = CurveState { mode: Some(mode.clone()), ..CurveState::default() };
                        if let Some((hot, hot_since, ceiling, ceiling_since)) = kept {
                            curve.hot_latched = hot;
                            curve.hot_below_since = hot_since;
                            curve.ceiling_latched = ceiling;
                            curve.ceiling_below_since = ceiling_since;
                        }
                    }
                    // The curve keeps stepping while the hot floor is latched: the latch is a
                    // floor under it (`target.max(floor)` below), never a cap, so a CPU that
                    // keeps heating still gets the curve's higher duty at once.
                    let target = curve.step(points, temp, now);
                    (target, Some(temp), source.hot_kind())
                } else {
                    // Issue #255 review: a reading missing for less than the sensor-loss
                    // timeout keeps the boost (nothing is written this tick). The hand_back
                    // below clears it when the loss is real.
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
        // The hot floor is latched per header, in the same state as the curve: it engages on
        // the first reading at or above the source's hot limit (issue #130: the user's line,
        // not the fixed 50 °C) and stays a floor until the temperature has stayed below the
        // limit for the curve's hold time, so a temperature hovering at the line cannot flip
        // the duty every tick (issue #17). It is only ever a floor: a higher curve or Fixed
        // duty always wins (`target.max(floor)`).
        let limit_status = hot_limit_status(kind, &self.config.hot_limits, self.rated_for(kind));
        let limit_c = limit_status.limit_c;
        let ceiling_c = limit_status.ceiling_c;
        let hot = runtime_floor_at(spec.id, self.config.floor_percent, Some(limit_c), limit_c);
        self.slots[ch].curve.note_temp(reference, limit_c, now);
        self.slots[ch].curve.note_ceiling(reference, ceiling_c, now);
        let latched = self.slots[ch].curve.hot_latched;
        let ceiling_latched = self.slots[ch].curve.ceiling_latched;
        // Issue #255 review: a Fixed header needs no sensor, so a lost reading is not a
        // handback by itself. Its kept boost lives through the short gap, and after the
        // sensor-loss timeout it is dropped so the plain Fixed duty returns.
        if matches!(mode, FanMode::Fixed { .. }) && reference.is_none() {
            let since = self.slots[ch].manual_since.unwrap_or(now);
            let last = self.last_good.get(&TempSource::Cpu).copied().unwrap_or(since);
            if now.saturating_duration_since(last) >= Duration::from_millis(SENSOR_LOSS_TIMEOUT_MS) {
                self.slots[ch].curve.hot_boost = 0.0;
            }
        }
        // Issue #255: the closed-loop boost. It runs after the latches so the ceiling freeze
        // sees the latch this tick engaged. A missing reading keeps it for the short gap (see
        // the Fixed case above and the Curve hold and handback below).
        self.slots[ch].curve.note_boost(reference, limit_c, now, ceiling_latched);
        // A Fixed header with no reference reading while a safety latch is set must not
        // write its lower Fixed duty over the latched 100 %. Hold the last written duty
        // (write nothing) and hand back after the sensor-loss timeout, exactly as the
        // Curve path above does for a missing source.
        if reference.is_none() && (latched || ceiling_latched) {
            let since = self.slots[ch].manual_since.unwrap_or(now);
            let last = self.last_good.get(&TempSource::Cpu).copied().unwrap_or(since);
            if now.saturating_duration_since(last) >= Duration::from_millis(SENSOR_LOSS_TIMEOUT_MS) {
                self.hand_back(
                    ch,
                    format!("CPU unavailable for over {} ms", SENSOR_LOSS_TIMEOUT_MS),
                );
            }
            return;
        }
        let floor = if latched && reference.is_some() {
            hot
        } else {
            runtime_floor_at(spec.id, self.config.floor_percent, reference, limit_c)
        };
        let base_duty = target.max(floor);
        // Issue #255: the boost is a closed loop on top of the curve or the Fixed duty. Its
        // own state is bounded at 100 (anti-windup) and only the OUTPUT is capped here, so a
        // curve that steps down while the source is still over the line keeps the fan up.
        let boost = self.slots[ch].curve.hot_boost;
        let mut percent = (f32::from(base_duty) + boost).round().clamp(0.0, 100.0) as u8;
        // The limit as the editor rounds it: no decimals, ties away from zero. `{:.0}` on an
        // f32 rounds ties to even, which would disagree with `Math.round` on a .5 limit.
        let limit_text = limit_c.round() as i32;
        let ceiling_text = ceiling_c.round() as i32;
        let mut reason = None;
        if ceiling_latched && reference.is_some() {
            // The fixed safety ceiling (issue #130): whatever the curve or the setting asks
            // for, the header runs at full speed. Name the fixed ceiling, not the live reading,
            // so the announced sentence stays stable.
            percent = 100;
            let name = match &mode {
                FanMode::Curve { source, .. } => source_label(source),
                _ => "CPU".to_string(),
            };
            reason = Some(format!("at the safety ceiling: {} reached {ceiling_text} °C", ceiling_name(&name)));
        } else if boost >= 1.0 {
            // Issue #255: the closed loop is holding the line. The reason names the limit,
            // never the live temperature, so the editor's stable sentence rule holds.
            let name = match &mode {
                FanMode::Curve { source, .. } => source_label(source),
                _ => "CPU".to_string(),
            };
            reason = Some(format!("holding {} under {limit_text} °C", hot_name(&name)));
        } else if floor > base && base_duty == floor {
            // Issue #255 review: the latch can still hold after the boost has decayed under a
            // point. Say what the latch is waiting for, not a temperature claim that is false.
            let name = match &mode {
                FanMode::Curve { source, .. } => source_label(source),
                _ => "CPU".to_string(),
            };
            reason = Some(format!(
                "hot floor {floor}% until {} has stayed under {limit_text} °C",
                hot_name(&name)
            ));
        }
        // Issue #297: the hot hold's state at INFO, rate-limited, so a real run can confirm why
        // the fans moved: the reading, the line, the error, engaged or released, the boost and
        // the duty asked. The issue's log had no temperature on any of these lines.
        let engaged = latched || boost >= 1.0;
        if self.slots[ch].curve.wants_hot_log(now, engaged, percent, &reason) {
            tracing::info!(
                header = spec.id,
                celsius = ?reference,
                limit_c,
                error_c = reference.map(|t| t - limit_c),
                hold = if engaged { "engaged" } else { "released" },
                latched,
                boost,
                asked = percent,
                reason = ?reason,
                "hot hold state"
            );
        }
        match self.write_and_verify(ch, percent) {
            Ok(()) => {
                if let Some(slot) = self.slots.get_mut(ch) {
                    slot.override_reason = reason;
                    slot.last_written = Some(percent);
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
                // Issue #29: the error keeps only the last reading. Log every reading and the mode
                // bit, so a reset chip (manual = false) tells apart from one firmware still owns.
                let manual = chip.read_manual(ch);
                tracing::warn!(channel = ch, percent, ?seen, manual = ?manual, "the duty never settled");
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
        // Issue #128: a header this board does not route is never written. BIOS is not a
        // write, so it stays allowed; every driving mode is refused before anything is
        // persisted or applied.
        if !self.header_available(header) && mode != FanMode::Bios {
            return Err(FanError::Unavailable(format!(
                "{header} is not on this board: the chip has the channel, the board has no connector"
            )));
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
        // Issue #130: the service validates the hot limits again on receipt; a value
        // outside 40..=max_settable_c is refused here even if the daemon let it through.
        if let Err(e) = validate_hot_limits(&file.hot_limits, self.cpu_rated_max_c, self.gpu_rated_max_c) {
            tracing::warn!(error = %e, "refused a fans config with an invalid hot limit");
            return Err(FanError::Validation(e.to_string()));
        }
        if !file.profiles.iter().any(|p| p.name == file.active_profile) {
            return Err(FanError::NotFound(format!("no profile named {}", file.active_profile)));
        }
        // Issue #255 review: a header whose hot line moved starts its closed loop again; the
        // heat built against the old line says nothing about the new one.
        let previous_limits = self.config.hot_limits;
        self.config = file;
        let new_limits = self.config.hot_limits;
        self.reset_boost_for_changed_limits(&previous_limits, &new_limits);
        self.persist()?;
        self.apply_profile(now);
        Ok(())
    }

    /// Zeroes the boost for every header reading a kind whose hot line changed. A header whose
    /// line is unchanged keeps its integral term, and a mode change is handled by `drive`.
    fn reset_boost_for_changed_limits(&mut self, before: &HotLimits, after: &HotLimits) {
        for ch in 0..self.slots.len() {
            let mode = self.desired(ch);
            let Some(kind) = mode_hot_kind(&mode) else { continue };
            let rated = self.rated_for(kind);
            let old = hot_limit_status(kind, before, rated).limit_c;
            let new = hot_limit_status(kind, after, rated).limit_c;
            if (old - new).abs() > f32::EPSILON {
                self.slots[ch].curve.hot_boost = 0.0;
                self.slots[ch].curve.boost_last_at = None;
            }
        }
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
            let available = self.header_available(spec.id);
            let on_bios = !available || !self.writes_allowed || slot.manual_since.is_none() || matches!(mode, FanMode::Bios);
            let override_reason = if !available {
                None
            } else if self.writes_allowed {
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
                available,
                override_available: self.header_overrides.get(spec.id).copied(),
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
            hot: self.hot_statuses(),
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

/// The short name a reason sentence uses for a source: "CPU (Tctl)" reads as "CPU" in a
/// reason; a board sensor keeps its own name.
fn hot_name(label: &str) -> &str {
    match label {
        "CPU (Tctl)" => "CPU",
        "GPU (NVML)" => "GPU",
        other => other,
    }
}

/// Same short name for the ceiling reason; the ceiling's is the safety one.
fn ceiling_name(label: &str) -> &str {
    hot_name(label)
}

/// Assembles the CPU brand string from the three extended CPUID leaves
/// 0x80000002..4, each given as (EAX, EBX, ECX, EDX) in that order — the register
/// order Intel and AMD document for the brand string (EAX, EBX, ECX, EDX per leaf).
/// Pure, so tests can drive it with the registers of a known brand string.
/// `None` when the leaves carry no string.
pub fn brand_string_from_leaves(leaves: [(u32, u32, u32, u32); 3]) -> Option<String> {
    let mut bytes = [0u8; 48];
    for (i, (eax, ebx, ecx, edx)) in leaves.iter().enumerate() {
        for (j, reg) in [eax, ebx, ecx, edx].iter().enumerate() {
            bytes[i * 16 + j * 4..i * 16 + j * 4 + 4].copy_from_slice(&reg.to_le_bytes());
        }
    }
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(48);
    let brand = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
    if brand.is_empty() {
        None
    } else {
        Some(brand)
    }
}

/// The CPU brand string from CPUID leaves 0x80000002..4 (x86_64), for the rated-maximum
/// table in the spine. `None` on any other architecture or when the leaf reports nothing.
#[cfg(target_arch = "x86_64")]
pub fn cpu_brand_string() -> Option<String> {
    // __cpuid is a safe function; the extended leaves are only read after the max-leaf
    // check below, and the string is NUL-padded by the CPU.
    let leaf = |id: u32| {
        let r = std::arch::x86_64::__cpuid(id);
        (r.eax, r.ebx, r.ecx, r.edx)
    };
    let max = std::arch::x86_64::__cpuid(0x8000_0000).eax;
    if max < 0x8000_0004 {
        return None;
    }
    brand_string_from_leaves([leaf(0x8000_0002), leaf(0x8000_0003), leaf(0x8000_0004)])
}

#[cfg(not(target_arch = "x86_64"))]
pub fn cpu_brand_string() -> Option<String> {
    None
}
