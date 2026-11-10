//! The peripherals: the mouse and the game controller canvas-rgb can read and configure.
//!
//! A peripheral is a device with settings and no lights of its own. The daemon never opens one
//! it was not told to: the vendor app may own it, and both would write the same device. What
//! the daemon may do with a device is saved per device id in `peripherals.json` (spine:
//! `routes::PERIPHERAL_ACCESS`):
//!
//! - `reads`: open the device and read its settings. A read may send the device a request that
//!   changes no setting.
//! - `writes`: send config writes. It needs `reads`: a write patches what the device last
//!   reported.
//!
//! Both are off until the owner turns them on. A device that is not in `peripherals.json` has
//! both off, so it is enumerated and never opened.
//!
//! The enumeration (`GET /peripherals`) never opens anything: the adapters read the HID
//! metadata of the machine and do no I/O on the device itself. The hub asks an adapter who owns
//! a device with `describe`, which is metadata only too, so the device that is opened is always
//! the one the owner asked for.
//!
//! # Opening the view reads nothing (#127)
//!
//! A settings read holds a receiver's firmware in blocking HID calls for seconds, and the
//! pointer stalls while it does (issue #7: 200-250 ms gaps). So the daemon answers every plain
//! `GET /peripherals/{id}` from values it keeps, never from the device: the kept values are
//! loaded from `peripheral-values.json` on start, so also the first open after a restart reads
//! nothing. A device is read only:
//!
//! - through `?fresh=true`, the person's own **Read again** (the one stall they ask for);
//! - in the background, and only after Windows reports no keyboard or mouse input for
//!   [`PERIPHERAL_IDLE_READ_S`], when the kept values are missing or older than [`KEPT_STALE`].
//!
//! The idle question goes through [`InputIdle`], so a test drives it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use std::sync::{Arc, Mutex as StdMutex};

use canvas_adapters::attack_shark_x6::AttackSharkX6;
use canvas_adapters::ReadSink;
use canvas_proto::device::DeviceId;
use canvas_proto::peripheral::{
    PeripheralAccess, PeripheralInfo, PeripheralSchema, ReadProgress, ReadStage,
};
use canvas_proto::props::Props;
use canvas_proto::traits::{AdapterError, PeripheralAdapter};

use crate::state::App;

/// The file the access settings live in, next to `displays.json` and `layouts.json`.
pub const PERIPHERALS_FILE: &str = "peripherals.json";
/// The schema version of `peripherals.json`.
pub const PERIPHERALS_SCHEMA: u32 = 1;

/// The enumeration the routes report is cached this long, so a page that polls does not
/// enumerate on every request.
pub const DISCOVERY_CACHE: Duration = Duration::from_secs(5);

/// The file the kept values live in, next to `peripherals.json`. They survive a restart, so the
/// first open after one reads nothing (#127).
pub const KEPT_VALUES_FILE: &str = "peripheral-values.json";
/// The schema version of `peripheral-values.json`.
pub const KEPT_VALUES_SCHEMA: u32 = 1;
/// Kept values older than this are refreshed in the background, once the person is idle.
pub const KEPT_STALE: Duration = Duration::from_secs(30 * 60);
/// How long Windows must report no keyboard or mouse input before a background read (#127).
pub const PERIPHERAL_IDLE_READ_S: u64 = 10;
/// The version of the adapters whose read produced the kept values. Values saved by another
/// build are ignored: its decode may not mean the same thing.
pub const ADAPTER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The message the detail carries when the values are missing because reads are off.
pub const READS_OFF: &str = "reads are off for this device";
/// The message the detail carries when reads are on but the device has not been read yet: no
/// read happens on a view open (#127).
pub const NOT_READ_YET: &str = "the device's settings have not been read yet";

/// What Windows says about the person's keyboard and mouse.
///
/// The daemon reads a device only while this says the person is away, so a read's stall lands
/// on an idle mouse. It is a trait so the tests drive it instead of the real clock.
pub trait InputIdle: Send + Sync {
    /// How long it has been since the last keyboard or mouse input.
    fn idle_for(&self) -> Duration;
}

/// A closure is an [`InputIdle`] too, which the tests use to step the clock.
impl<F> InputIdle for F
where
    F: Fn() -> Duration + Send + Sync,
{
    fn idle_for(&self) -> Duration {
        self()
    }
}

/// The real input clock: `GetLastInputInfo` from user32.
#[derive(Debug, Default)]
pub struct WindowsInputIdle;

impl InputIdle for WindowsInputIdle {
    fn idle_for(&self) -> Duration {
        #[cfg(windows)]
        {
            use windows::Win32::System::SystemInformation::GetTickCount;
            use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
            let mut info = LASTINPUTINFO {
                cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            // SAFETY: `info` is a valid, writable LASTINPUTINFO whose cbSize is its own size.
            let ok = unsafe { GetLastInputInfo(&mut info) };
            if ok.as_bool() {
                // Both calls count milliseconds since boot in 32 bits: the wrapping difference
                // is the idle time and stays right across the 49-day tick wrap.
                let idle_ms = unsafe { GetTickCount() }.wrapping_sub(info.dwTime);
                return Duration::from_millis(u64::from(idle_ms));
            }
            // A failed call must never look like an idle person: zero forbids every read.
            Duration::ZERO
        }
        #[cfg(not(windows))]
        {
            Duration::ZERO
        }
    }
}

/// `peripheral-values.json`: the values the daemon keeps for each device, so a restart answers
/// the first open without touching the device (#127).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeptValuesFile {
    pub schema_version: u32,
    /// The kept values per device id.
    #[serde(default)]
    pub devices: BTreeMap<String, KeptValues>,
}

/// One device's kept values, and the adapter version that read them.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeptValues {
    /// The adapter that owns the device, e.g. `attack-shark-x6`.
    pub adapter: String,
    /// The adapter's version: values from another build are ignored.
    pub adapter_version: String,
    /// Seconds since the Unix epoch when the device was last read.
    pub read_at: u64,
    /// The values, keyed by property key.
    pub values: Props,
}

/// The kept values file's path in `home`.
fn kept_values_path(home: &Path) -> PathBuf {
    home.join(KEPT_VALUES_FILE)
}

/// Seconds since the Unix epoch, or 0 when the wall clock is before it.
fn now_unix_s() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `peripherals.json`: what the daemon may do with each device.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PeripheralsFile {
    pub schema_version: u32,
    /// The access per device id. A device that is not in here has both off.
    #[serde(default)]
    pub peripherals: BTreeMap<String, PeripheralAccess>,
}

impl Default for PeripheralsFile {
    fn default() -> Self {
        Self { schema_version: PERIPHERALS_SCHEMA, peripherals: BTreeMap::new() }
    }
}

impl PeripheralsFile {
    /// What the daemon may do with this device. An id that was never saved has both off.
    pub fn access(&self, id: &DeviceId) -> PeripheralAccess {
        self.peripherals.get(&id.0).copied().unwrap_or_default()
    }

    /// Saves the access for one device.
    pub fn set(&mut self, id: &DeviceId, access: PeripheralAccess) {
        self.peripherals.insert(id.0.clone(), access);
    }
}

/// Writes `peripherals.json` in `home`, atomically.
pub fn save(home: &Path, file: &PeripheralsFile) -> std::io::Result<()> {
    crate::storage::write_json(&home.join(PERIPHERALS_FILE), file)
}

/// A peripheral adapter whose config writes can be turned on and off.
///
/// The spine's `PeripheralAdapter` has no gate, but both adapters have one (an inherent
/// `set_writes_enabled`). This trait is the daemon's bridge to it: the daemon turns the gate
/// with the saved access, so nothing ever reaches a device the owner did not allow.
///
/// It also needs `ResumeBehavior`: a peripheral the daemon is given has to say what it does when
/// the machine resumes from sleep, or it does not compile (`cause:sleep-state`).
#[async_trait::async_trait]
pub trait Peripheral: PeripheralAdapter + canvas_adapters::resume::ResumeBehavior {
    /// Turns config writes on or off. Nothing is written to the hardware while they are off.
    fn set_config_writes(&mut self, on: bool);

    /// Where this adapter reports the stage of a read, for the adapters whose read takes long
    /// enough to be worth reporting. An adapter that reports nothing keeps this default, and its
    /// read shows as one stage.
    fn set_read_sink(&mut self, _sink: Option<ReadSink>) {}

    /// Writes the settings and returns the values the device holds for them afterwards. An
    /// adapter whose device changes keys it was not asked to (a mode that forces a brightness,
    /// a count that turns stages off) overrides this and returns those too, so the hub can keep
    /// the values without reading the device again. The default holds that the device took
    /// exactly what was written.
    async fn write_effective(
        &mut self,
        device: &DeviceId,
        props: &Props,
    ) -> Result<Props, AdapterError> {
        self.write_settings(device, props).await?;
        Ok(props.clone())
    }
}

#[async_trait::async_trait]
impl Peripheral for canvas_adapters::attack_shark_x6::AttackSharkX6 {
    async fn write_effective(
        &mut self,
        device: &DeviceId,
        props: &Props,
    ) -> Result<Props, AdapterError> {
        // What the mouse holds after the write, with the keys that were written as the floor.
        let mut effective = props.clone();
        effective.extend(AttackSharkX6::write_effective(self, device, props).await?);
        Ok(effective)
    }

    fn set_config_writes(&mut self, on: bool) {
        self.set_writes_enabled(on);
    }

    fn set_read_sink(&mut self, sink: Option<ReadSink>) {
        AttackSharkX6::set_read_sink(self, sink);
    }
}

impl Peripheral for canvas_adapters::eightbitdo_ultimate2::EightBitDoUltimate2 {
    fn set_config_writes(&mut self, on: bool) {
        self.set_writes_enabled(on);
    }
}

/// The adapters the daemon has, and the last enumeration.
pub struct PeripheralHub {
    /// The adapters. Behind a lock: every call needs `&mut` on the adapter, and a read of a
    /// device can take a while.
    adapters: tokio::sync::Mutex<Vec<Box<dyn Peripheral>>>,
    /// The last enumeration, and when it was taken.
    discovery: tokio::sync::Mutex<Option<Discovery>>,
    /// The values the daemon keeps for each device: its last good read, with every write since
    /// merged in. They stay until something makes them stale (a failed write or read, an
    /// access change, the device leaving the enumeration) or a caller asks for a fresh read,
    /// because a read stalls the pointer on the receiver's radio link (issue #7). The read path
    /// answers from them while the `adapters` lock is held, so concurrent reads of one device
    /// queue behind the first and find its answer here (single-flight).
    reads: std::sync::Mutex<BTreeMap<DeviceId, Kept>>,
    /// How many fresh reads of each device have finished. A fresh read that waited for the
    /// adapters' lock while another one finished answers with that one's result.
    fresh_done: std::sync::Mutex<BTreeMap<DeviceId, u64>>,
    /// The `reads` of the access each device was last given: only a change of it makes the kept
    /// values stale (the device is opened or closed); a repeat, or a change of `writes`, keeps them.
    reads_applied: std::sync::Mutex<BTreeMap<DeviceId, bool>>,
    /// The stage of the read that is running, per device id. A plain lock, never held across an
    /// await and never the adapters' lock: asking for it must answer while a read is still open.
    progress: Arc<StdMutex<BTreeMap<String, ReadProgress>>>,
    /// Where the kept values survive a restart. `None` in the unit tests that do not need it.
    kept_path: Option<PathBuf>,
    /// What Windows (or a test's fake) says about the person's keyboard and mouse.
    idle: Arc<dyn InputIdle>,
}

/// The values kept for one device, and the adapter that read them.
struct Kept {
    /// The adapter that owns the device, so a failed enumeration of another adapter leaves
    /// these alone.
    adapter: String,
    props: Props,
    /// Seconds since the Unix epoch when the read finished.
    read_at: u64,
}

/// Clears the stage of a read when the read ends or its request is dropped, and stops a sink that
/// is still running on the blocking thread from writing it back.
struct ReadingGuard {
    id: String,
    live: Arc<std::sync::atomic::AtomicBool>,
    progress: Arc<StdMutex<BTreeMap<String, ReadProgress>>>,
}

impl Drop for ReadingGuard {
    fn drop(&mut self) {
        // One lock covers the flag and the clear, and the sink checks the flag under the same
        // lock, so a sink that passed its check cannot write after this clear.
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        self.live.store(false, std::sync::atomic::Ordering::SeqCst);
        progress.remove(&self.id);
    }
}

/// One enumeration and the moment it was taken.
struct Discovery {
    at: Instant,
    found: Vec<PeripheralInfo>,
}

/// The hook the unit tests use to hold the sink between the check of the flag and the write of
/// the stage. It is only compiled into a test build, and it is the only way the race between
/// the two can be made to happen on purpose.
#[cfg(test)]
mod sink_hook {
    use std::sync::{Condvar, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct Gate {
        /// A sink is at the gate.
        reached: bool,
        /// The gate is open: the sink carries on.
        open: bool,
        /// The sink has written its stage.
        written: bool,
    }

    static GATE: OnceLock<(Mutex<Gate>, Condvar)> = OnceLock::new();

    fn gate() -> &'static (Mutex<Gate>, Condvar) {
        GATE.get_or_init(|| (Mutex::new(Gate::default()), Condvar::new()))
    }

    /// Opens the gate when it goes out of scope while it is armed.
    ///
    /// A wait that gives up (a panic) must not leave a worker blocked in `pause()`: the
    /// runtime cannot end while a blocking task is still waiting, so the binary would hang
    /// after the failure instead of ending with it. The guard opens the gate on the way out,
    /// so the sink is released and the binary can end.
    struct GateGuard {
        armed: bool,
    }

    impl GateGuard {
        fn armed() -> Self {
            Self { armed: true }
        }

        /// The wait ended the way it should: the gate stays as the test left it.
        fn disarm(&mut self) {
            self.armed = false;
        }
    }

    impl Drop for GateGuard {
        fn drop(&mut self) {
            if self.armed {
                open();
            }
        }
    }

    /// Waits at the gate, with the progress lock held. The test opens the gate.
    pub(crate) fn pause() {
        let (lock, cv) = gate();
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        if g.open {
            return;
        }
        g.reached = true;
        cv.notify_all();
        while !g.open {
            g = cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// The sink has written its stage.
    pub(crate) fn written() {
        let (lock, cv) = gate();
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        g.written = true;
        cv.notify_all();
    }

    /// Opens the gate: the sink writes its stage and carries on.
    pub(crate) fn open() {
        let (lock, cv) = gate();
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        g.open = true;
        cv.notify_all();
    }

    /// True when the sink has written its stage. The test uses it to see that the sink was
    /// still held at the gate: a gate that let the sink straight through would make the race
    /// impossible, and the test would pass without it.
    pub(crate) fn written_yet() -> bool {
        let (lock, _cv) = gate();
        let g = lock.lock().unwrap_or_else(|e| e.into_inner());
        g.written
    }

    /// True when the proof run asks for a gate that never opens.
    pub(crate) fn never_opens() -> bool {
        std::env::var("CANVAS_RGB_GATE_NEVER_OPENS").is_ok()
    }

    /// Waits until a sink is at the gate.
    pub(crate) fn wait_until_reached() {
        wait_for("the sink to reach the gate", |g| g.reached);
    }

    /// Waits until the sink has written its stage.
    pub(crate) fn wait_until_written() {
        wait_for("the sink to write its stage", |g| g.written);
    }

    /// How long a test waits for the sink before it gives up.
    const GATE_TIMEOUT: Duration = Duration::from_secs(10);

    /// Waits for one step of the test, with a deadline. A regression fails the test with the
    /// step it stopped at, instead of hanging the binary.
    fn wait_for(step: &str, done: fn(&Gate) -> bool) {
        // The gate is opened if this wait gives up: the worker that is blocked at it is
        // released, so the binary ends with the failure instead of hanging on it.
        let mut guard = GateGuard::armed();
        let (lock, cv) = gate();
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        let deadline = Instant::now() + GATE_TIMEOUT;
        while !done(&g) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                panic!("the test waited {GATE_TIMEOUT:?} for {step} and gave up");
            }
            let (next, waited) = cv.wait_timeout(g, left).unwrap_or_else(|e| e.into_inner());
            g = next;
            if waited.timed_out() && !done(&g) {
                panic!("the test waited {GATE_TIMEOUT:?} for {step} and gave up");
            }
        }
        guard.disarm();
    }

    /// Puts the gate back, for the next test.
    pub(crate) fn reset() {
        let (lock, _cv) = gate();
        let mut g = lock.lock().unwrap_or_else(|e| e.into_inner());
        *g = Gate::default();
    }
}

impl PeripheralHub {
    /// The hub with these adapters, keeping its values in memory only. The unit tests use it;
    /// the daemon uses [`PeripheralHub::with_home`].
    pub fn new(adapters: Vec<Box<dyn Peripheral>>) -> Self {
        Self::build(adapters, None, Arc::new(WindowsInputIdle))
    }

    /// The hub the daemon runs: its kept values are loaded from and saved to `home`, so the
    /// first open after a restart reads nothing (#127).
    pub fn with_home(adapters: Vec<Box<dyn Peripheral>>, home: impl Into<PathBuf>) -> Self {
        Self::build(adapters, Some(kept_values_path(&home.into())), Arc::new(WindowsInputIdle))
    }

    /// A hub whose input clock the test drives.
    pub fn with_idle(adapters: Vec<Box<dyn Peripheral>>, idle: Arc<dyn InputIdle>) -> Self {
        Self::build(adapters, None, idle)
    }

    /// A hub whose kept values live in `home` and whose input clock the test drives.
    pub fn with_home_and_idle(
        adapters: Vec<Box<dyn Peripheral>>,
        home: impl Into<PathBuf>,
        idle: Arc<dyn InputIdle>,
    ) -> Self {
        Self::build(adapters, Some(kept_values_path(&home.into())), idle)
    }

    fn build(
        adapters: Vec<Box<dyn Peripheral>>,
        kept_path: Option<PathBuf>,
        idle: Arc<dyn InputIdle>,
    ) -> Self {
        // Values saved by another adapter version are dropped here, with a log line each; a
        // file that does not parse is ignored the same way. Never fatal (#127).
        let reads = kept_path
            .as_deref()
            .map(load_kept_values)
            .unwrap_or_default();
        Self {
            adapters: tokio::sync::Mutex::new(adapters),
            discovery: tokio::sync::Mutex::new(None),
            progress: Arc::new(StdMutex::new(BTreeMap::new())),
            reads: std::sync::Mutex::new(reads),
            fresh_done: std::sync::Mutex::new(BTreeMap::new()),
            reads_applied: std::sync::Mutex::new(BTreeMap::new()),
            kept_path,
            idle,
        }
    }

    /// The device's settings as the daemon keeps them, with every write since merged in.
    ///
    /// This never touches the device: opening the view is answered from here, or with
    /// [`NOT_READ_YET`](crate::peripherals::NOT_READ_YET) when nothing has been read (#127).
    pub fn kept(&self, device: &DeviceId) -> Option<Props> {
        self.kept_read(device)
    }

    /// The stage of the read of this device that is running now, [`ReadProgress::IDLE`] when none
    /// is. Never waits on a device: it answers while a read is open.
    pub fn read_progress(&self, device: &DeviceId) -> ReadProgress {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&device.0)
            .copied()
            .unwrap_or(ReadProgress::IDLE)
    }

    /// Records the stage of a read of this device.
    fn set_progress(&self, device: &DeviceId, stage: ReadProgress) {
        self.progress.lock().unwrap_or_else(|e| e.into_inner()).insert(device.0.clone(), stage);
    }

    /// Every peripheral the adapters see, cached for [`DISCOVERY_CACHE`].
    ///
    /// The enumeration never opens a device (see the module doc), so a listed peripheral is
    /// not a peripheral the daemon may touch: only the ones the owner turned reads on for are
    /// ever opened.
    pub async fn discover(&self) -> Vec<PeripheralInfo> {
        let mut cache = self.discovery.lock().await;
        if let Some(cached) = cache.as_ref() {
            if cached.at.elapsed() < DISCOVERY_CACHE {
                return cached.found.clone();
            }
        }
        let found = self.enumerate().await;
        *cache = Some(Discovery { at: Instant::now(), found: found.clone() });
        found
    }

    /// A fresh enumeration, ignoring the cache. Used when a device was not in the last one.
    pub async fn discover_now(&self) -> Vec<PeripheralInfo> {
        let found = self.enumerate().await;
        let mut cache = self.discovery.lock().await;
        *cache = Some(Discovery { at: Instant::now(), found: found.clone() });
        found
    }

    /// Runs every adapter's enumeration. One that fails does not stop the others.
    async fn enumerate(&self) -> Vec<PeripheralInfo> {
        let mut adapters = self.adapters.lock().await;
        let mut found = Vec::new();
        let mut failed: Vec<&'static str> = Vec::new();
        for adapter in adapters.iter_mut() {
            let name = adapter.name();
            match adapter.discover().await {
                Ok(list) => found.extend(list),
                Err(error) => {
                    failed.push(name);
                    tracing::warn!(adapter = name, error = %error, "could not enumerate the peripherals");
                }
            }
        }
        // A device that an adapter no longer lists is gone: what was kept for it is stale. An
        // adapter whose enumeration failed said nothing, so its devices keep their values.
        let dropped = {
            let mut reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
            let before = reads.len();
            reads.retain(|id, kept| {
                failed.contains(&kept.adapter.as_str()) || found.iter().any(|info| &info.id == id)
            });
            reads.len() != before
        };
        if dropped {
            self.save_kept();
        }
        found
    }

    /// What the device has: its sections, its properties and their ranges.
    ///
    /// `describe` is metadata only, so no device is opened here.
    pub async fn describe(&self, device: &DeviceId) -> Result<PeripheralSchema, AdapterError> {
        let mut adapters = self.adapters.lock().await;
        let (_, schema) = owner_of(&mut adapters, device).await?;
        Ok(schema)
    }

    /// The device's settings, read from the device now. This is the only path that touches a
    /// device: the person's **Read again** (`?fresh=true`) and the idle background refresh.
    ///
    /// The device is opened first if it is not open. A failed read keeps nothing. A read that
    /// fails because the device is gone closes it, so the next read opens it again.
    pub async fn read_fresh(&self, device: &DeviceId) -> Result<Props, AdapterError> {
        // How many fresh reads of this device had finished when this ask arrived.
        let done_on_arrival = self.fresh_finished(device);
        // One lock for all adapters. A blocking open inside an adapter holds it, so a slow open
        // of one device queues the reads of the others behind it (the adapters' own HID caches
        // are per adapter and do not share this).
        let mut adapters = self.adapters.lock().await;
        let (index, _) = owner_of(&mut adapters, device).await?;
        let adapter = adapters[index].as_mut();
        // The stage the editor shows while this read is open. The guard clears it however the
        // read ends, also when the request is dropped.
        let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let _guard = ReadingGuard {
            id: device.0.clone(),
            live: live.clone(),
            progress: self.progress.clone(),
        };
        // A read that waited while another read of this device ran takes that one's result (a
        // double click, or a Read again that lands during the background refresh, is one read).
        if adapter.is_connected() && self.fresh_finished(device) > done_on_arrival {
            if let Some(props) = self.kept_read(device) {
                return Ok(props);
            }
        }
        if !adapter.is_connected() {
            self.set_progress(device, ReadProgress::at(ReadStage::Opening));
            adapter.connect().await?;
        }
        // An adapter that reports nothing is in one stage: reading, with no count.
        self.set_progress(device, ReadProgress::at(ReadStage::Reading));
        let sink: ReadSink = {
            let progress = self.progress.clone();
            let id = device.0.clone();
            Arc::new(move |p: ReadProgress| {
                let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
                if live.load(std::sync::atomic::Ordering::SeqCst) {
                    // The hook pauses here, between the check of the flag and the write of the
                    // stage, with the lock held. The old order checked the flag outside the lock,
                    // so the clear could slip in between and the stage came back; the test that
                    // uses this hook fails on that order every run.
                    #[cfg(test)]
                    sink_hook::pause();
                    progress.insert(id.clone(), p);
                    #[cfg(test)]
                    sink_hook::written();
                }
            })
        };
        adapter.set_read_sink(Some(sink));
        let result = adapter.read_settings(device).await;
        adapter.set_read_sink(None);
        match &result {
            Ok(props) => {
                self.reads.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    device.clone(),
                    Kept {
                        adapter: adapter.name().to_string(),
                        props: props.clone(),
                        read_at: now_unix_s(),
                    },
                );
                *self
                    .fresh_done
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(device.clone())
                    .or_insert(0) += 1;
                // The values survive a restart now (#127).
                self.save_kept();
            }
            Err(error) => {
                // A failed read leaves nothing kept: the old values may no longer be true.
                self.forget_read(device);
                if device_failure(error) {
                    let _ = adapter.disconnect().await;
                }
            }
        }
        result
    }

    /// The values kept for this device, if any.
    fn kept_read(&self, device: &DeviceId) -> Option<Props> {
        self.reads.lock().unwrap_or_else(|e| e.into_inner()).get(device).map(|k| k.props.clone())
    }

    /// How many fresh reads of this device have finished.
    fn fresh_finished(&self, device: &DeviceId) -> u64 {
        self.fresh_done.lock().unwrap_or_else(|e| e.into_inner()).get(device).copied().unwrap_or(0)
    }

    /// Forgets the kept values of this device: they are stale.
    fn forget_read(&self, device: &DeviceId) {
        let removed = self.reads.lock().unwrap_or_else(|e| e.into_inner()).remove(device).is_some();
        if removed {
            self.save_kept();
        }
    }

    /// True when this device's kept values are missing or older than [`KEPT_STALE`].
    pub fn needs_refresh(&self, device: &DeviceId) -> bool {
        let reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
        match reads.get(device) {
            None => true,
            Some(kept) => now_unix_s().saturating_sub(kept.read_at) > KEPT_STALE.as_secs(),
        }
    }

    /// One background pass (#127): reads each device whose values are missing or stale, and only
    /// once the person has left the keyboard and mouse alone for [`PERIPHERAL_IDLE_READ_S`].
    ///
    /// It returns how many devices were read. A read is abandoned before it starts when input
    /// arrived while an earlier device was being read. Nothing on the view-open path calls this.
    pub async fn refresh_idle(&self, devices: &[DeviceId]) -> usize {
        let needed: Vec<DeviceId> =
            devices.iter().filter(|device| self.needs_refresh(device)).cloned().collect();
        if needed.is_empty() {
            return 0;
        }
        let enough = Duration::from_secs(PERIPHERAL_IDLE_READ_S);
        if self.idle.idle_for() < enough {
            return 0;
        }
        let mut read = 0;
        for device in needed {
            // The person came back: the read that has not started is abandoned.
            if self.idle.idle_for() < enough {
                break;
            }
            if !self.needs_refresh(&device) {
                continue;
            }
            match self.read_fresh(&device).await {
                Ok(_) => read += 1,
                // A device that is not here is not a failure: it is read when it comes back.
                Err(AdapterError::UnknownDevice(_) | AdapterError::NotConnected) => {}
                Err(error) => {
                    tracing::warn!(device = %device, error = %error, "the peripheral's idle read failed")
                }
            }
        }
        read
    }

    /// Writes the kept values to their file, when the hub has one. A failed save is a warning:
    /// the daemon keeps answering from memory.
    fn save_kept(&self) {
        let Some(path) = &self.kept_path else {
            return;
        };
        let devices: BTreeMap<String, KeptValues> = {
            let reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
            reads
                .iter()
                .map(|(id, kept)| {
                    (
                        id.0.clone(),
                        KeptValues {
                            adapter: kept.adapter.clone(),
                            adapter_version: ADAPTER_VERSION.to_string(),
                            read_at: kept.read_at,
                            values: kept.props.clone(),
                        },
                    )
                })
                .collect()
        };
        let file = KeptValuesFile { schema_version: KEPT_VALUES_SCHEMA, devices };
        if let Err(error) = crate::storage::write_json(path, &file) {
            tracing::warn!(
                path = %path.display(),
                error = %error,
                "the kept peripheral values could not be saved"
            );
        }
    }

    /// Writes the given settings to the device. Only the keys present are written.
    ///
    /// Nothing reaches the hardware while the adapter's write gate is off: the gate follows
    /// the saved access.
    pub async fn write(&self, device: &DeviceId, props: &Props) -> Result<(), AdapterError> {
        let mut adapters = self.adapters.lock().await;
        let (index, _) = owner_of(&mut adapters, device).await?;
        let adapter = adapters[index].as_mut();
        if !adapter.is_connected() {
            self.forget_read(device);
            adapter.connect().await?;
        }
        let result = adapter.write_effective(device, props).await;
        match &result {
            Ok(effective) => {
                // The device holds what it reports back now, including keys it changed itself:
                // merge them into the kept values, so the next open needs no trip to the
                // device. Nothing kept means the values wait for the idle background read.
                let merged = {
                    let mut reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
                    match reads.get_mut(device) {
                        Some(kept) => {
                            for (key, value) in effective {
                                kept.props.insert(key.clone(), value.clone());
                            }
                            true
                        }
                        None => false,
                    }
                };
                if merged {
                    self.save_kept();
                }
            }
            Err(error) => {
                // A refusal before anything was sent (a key the device lacks, a value out of
                // range, writes off) changed nothing: the kept values stand. A device failure
                // may have sent part of the write, so how far it got is unknown: they are stale.
                if device_failure(error) {
                    self.forget_read(device);
                    let _ = adapter.disconnect().await;
                }
            }
        }
        result.map(|_| ())
    }

    /// Applies the access to a device: opens it when reads are on, closes it when they are
    /// off, and follows the access with the adapter's write gate.
    ///
    /// Turning reads off closes the device, so the vendor app can take it back. Turning writes
    /// off turns the gate off, so nothing reaches the hardware afterwards.
    pub async fn apply(&self, device: &DeviceId, access: PeripheralAccess) -> Result<(), AdapterError> {
        let mut adapters = self.adapters.lock().await;
        let (index, _) = owner_of(&mut adapters, device).await?;
        let adapter = adapters[index].as_mut();
        // Only a change of `reads` makes the kept values stale: the device is opened or closed.
        // A repeat of the same access, or a change of `writes` alone, keeps them. The first
        // apply after a start is not a change: it is what opens the device the values in the
        // file came from (#127).
        let previous = self
            .reads_applied
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(device.clone(), access.reads);
        if previous.is_some_and(|reads| reads != access.reads) {
            self.forget_read(device);
        }
        // The gate follows the access, whatever the access is. Writes need reads, so an
        // access that says otherwise keeps the writes off.
        adapter.set_config_writes(access.writes && access.reads);
        if access.reads {
            if !adapter.is_connected() {
                adapter.connect().await?;
            }
        } else if adapter.is_connected() {
            adapter.disconnect().await?;
        }
        Ok(())
    }
}

/// The adapter that owns this device, and its schema.
///
/// The owner is found with `describe`, which is metadata only: no device is opened while the
/// adapters are asked who owns what. An adapter that does not know the id is skipped.
async fn owner_of(
    adapters: &mut [Box<dyn Peripheral>],
    device: &DeviceId,
) -> Result<(usize, PeripheralSchema), AdapterError> {
    for (index, adapter) in adapters.iter_mut().enumerate() {
        match adapter.describe(device).await {
            Ok(schema) => return Ok((index, schema)),
            Err(AdapterError::UnknownDevice(_)) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(AdapterError::UnknownDevice(device.clone()))
}

/// True when the error means the device is gone or the link failed, not that the request was
/// wrong.
fn device_failure(error: &AdapterError) -> bool {
    matches!(error, AdapterError::Io(_) | AdapterError::NotConnected)
}

/// Reads the kept values file. A missing file has nothing kept; one that does not parse, or an
/// entry from another adapter version, is ignored with a log line and never fatal (#127).
fn load_kept_values(path: &Path) -> BTreeMap<DeviceId, Kept> {
    let Some(bytes) = crate::paths::read_optional(path) else {
        return BTreeMap::new();
    };
    let file: KeptValuesFile = match serde_json::from_slice(&bytes) {
        Ok(file) => file,
        Err(error) => {
            tracing::error!(
                path = %path.display(),
                error = %error,
                "peripheral-values.json did not parse; the kept peripheral values are ignored"
            );
            return BTreeMap::new();
        }
    };
    if file.schema_version != KEPT_VALUES_SCHEMA {
        tracing::warn!(
            path = %path.display(),
            found = file.schema_version,
            expected = KEPT_VALUES_SCHEMA,
            "peripheral-values.json has another schema version; its values are ignored"
        );
        return BTreeMap::new();
    }
    let mut kept = BTreeMap::new();
    for (id, values) in file.devices {
        if values.adapter_version != ADAPTER_VERSION {
            tracing::warn!(
                device = %id,
                found = %values.adapter_version,
                expected = ADAPTER_VERSION,
                "kept peripheral values from another adapter version are ignored"
            );
            continue;
        }
        kept.insert(
            DeviceId(id),
            Kept { adapter: values.adapter, props: values.values, read_at: values.read_at },
        );
    }
    kept
}

/// Starts the background refresh (#127): every second, it reads a device whose kept values are
/// missing or older than [`KEPT_STALE`], and only once Windows reports no keyboard or mouse
/// input for [`PERIPHERAL_IDLE_READ_S`]. Opening the view never triggers a read.
pub fn start_idle_refresh(app: Arc<App>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if app.shutting_down.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                return;
            }
            let saved: Vec<DeviceId> = {
                let store = app.store.read().await;
                store
                    .peripherals
                    .peripherals
                    .iter()
                    .filter(|(_, access)| access.reads)
                    .map(|(id, _)| DeviceId(id.clone()))
                    .collect()
            };
            let wanted: Vec<DeviceId> =
                saved.into_iter().filter(|id| app.peripherals.needs_refresh(id)).collect();
            if wanted.is_empty() {
                continue;
            }
            // Only the devices the adapters can see: an unplugged device is not opened and does
            // not log a failure every pass. The enumeration is metadata only and cached.
            let found = app.peripherals.discover().await;
            let present: Vec<DeviceId> = wanted
                .into_iter()
                .filter(|id| found.iter().any(|info| &info.id == id))
                .collect();
            if present.is_empty() {
                continue;
            }
            app.peripherals.refresh_idle(&present).await;
        }
    });
}

/// Applies the saved access to every device that has one.
///
/// The devices the owner turned reads on are opened; the others are left closed. A device that
/// is not here is a warning, not an error: it comes back with the access it was given.
pub async fn apply_saved(app: &App) {
    let saved: Vec<(DeviceId, PeripheralAccess)> = {
        let store = app.store.read().await;
        store
            .peripherals
            .peripherals
            .iter()
            .map(|(id, access)| (DeviceId(id.clone()), *access))
            .collect()
    };
    for (device, access) in saved {
        match app.peripherals.apply(&device, access).await {
            Ok(()) => tracing::info!(device = %device, reads = access.reads, writes = access.writes, "the saved peripheral access was applied"),
            Err(error) => tracing::warn!(device = %device, error = %error, "the saved peripheral access could not be applied"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::peripheral::{PeripheralInfo, PeripheralKind, PeripheralSchema};

    /// A peripheral that is never opened for real: it takes the sink the daemon gives it, calls
    /// it on a thread of its own and then keeps the read open, so the test can cancel the read
    /// while the sink is still at the hook.
    struct HeldPeripheral {
        id: DeviceId,
        connected: StdMutex<bool>,
        sink: StdMutex<Option<ReadSink>>,
    }

    impl HeldPeripheral {
        fn new(id: DeviceId) -> Self {
            Self { id, connected: StdMutex::new(false), sink: StdMutex::new(None) }
        }
    }

    #[async_trait::async_trait]
    impl canvas_proto::traits::PeripheralAdapter for HeldPeripheral {
        fn name(&self) -> &'static str {
            "held"
        }

        async fn connect(&mut self) -> Result<(), AdapterError> {
            *self.connected.lock().unwrap() = true;
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), AdapterError> {
            *self.connected.lock().unwrap() = false;
            Ok(())
        }

        fn is_connected(&self) -> bool {
            *self.connected.lock().unwrap()
        }

        async fn discover(&mut self) -> Result<Vec<PeripheralInfo>, AdapterError> {
            Ok(vec![PeripheralInfo {
                id: self.id.clone(),
                name: "Held Mouse".into(),
                vendor: "fake".into(),
                kind: PeripheralKind::Mouse,
                adapter: "held".into(),
                serial: String::new(),
                location: String::new(),
                connected: true,
            }])
        }

        async fn describe(&mut self, device: &DeviceId) -> Result<PeripheralSchema, AdapterError> {
            if &self.id != device {
                return Err(AdapterError::UnknownDevice(device.clone()));
            }
            Ok(PeripheralSchema {
                device: device.clone(),
                kind: PeripheralKind::Mouse,
                sections: Vec::new(),
                read_only: Vec::new(),
                unreported: Vec::new(),
            })
        }

        async fn read_settings(&mut self, _device: &DeviceId) -> Result<Props, AdapterError> {
            // The sink runs on a thread of its own: the hook holds it at the gate, and a read that
            // is held must not block the runtime.
            if let Some(sink) = self.sink.lock().unwrap().clone() {
                tokio::task::spawn_blocking(move || sink(ReadProgress::reading(1, 2)));
            }
            // The read stays open until the request is dropped.
            std::future::pending::<()>().await;
            Ok(Props::new())
        }

        async fn write_settings(
            &mut self,
            _device: &DeviceId,
            _props: &Props,
        ) -> Result<(), AdapterError> {
            Ok(())
        }

        fn writes_enabled(&self) -> bool {
            true
        }
    }

    impl canvas_adapters::resume::ResumeBehavior for HeldPeripheral {
        fn on_resume(&self) -> canvas_adapters::resume::OnResume {
            // A test fake: it holds no mode.
            canvas_adapters::resume::OnResume::NothingToRestore
        }
    }

    impl Peripheral for HeldPeripheral {
        fn set_config_writes(&mut self, _on: bool) {}

        fn set_read_sink(&mut self, sink: Option<ReadSink>) {
            *self.sink.lock().unwrap() = sink;
        }
    }

    /// The check of the flag and the write of the stage are one step: a report that arrives after
    /// the read ended must not bring the stage back.
    ///
    /// The hook holds the sink between the check and the write, so the clear of the ended read
    /// lands exactly there. With the check and the write under one lock the clear waits for the
    /// sink and takes the stage away again. With the old order (the check outside the lock) the
    /// clear slips in between, the sink writes the stage afterwards and the stage comes back:
    /// this test fails on that order every run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_report_that_arrives_after_the_read_ended_does_not_bring_the_stage_back() {
        sink_hook::reset();
        let id = DeviceId("held:one".into());
        let hub = Arc::new(PeripheralHub::new(
            vec![Box::new(HeldPeripheral::new(id.clone()))],
        ));

        let reader = {
            let hub = hub.clone();
            let id = id.clone();
            tokio::spawn(async move { hub.read_fresh(&id).await })
        };

        // The sink is at the gate: it has checked the flag and has not written the stage yet.
        sink_hook::wait_until_reached();
        // And it is still there: the gate holds it. A gate that let the sink straight through
        // would make this test pass without the race ever happening, so the sink is given a
        // moment to write its stage, and it must not have written it.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !sink_hook::written_yet(),
            "the sink was not held at the gate: the race was not tested"
        );

        // The reader goes away while the sink waits: the stage is cleared.
        reader.abort();
        // The clear takes the lock, and the sink holds it: on the fixed order the clear waits for
        // the gate, and on the old one it finishes right away. The test waits a moment to see
        // which happened, then lets the sink write.
        let mut reader = reader;
        let cleared = tokio::time::timeout(Duration::from_secs(1), &mut reader).await.is_ok();
        sink_hook::open();
        // The sink has written its stage.
        sink_hook::wait_until_written();
        if !cleared {
            // The clear was waiting for the lock: it runs now that the sink is done.
            let _ = tokio::time::timeout(Duration::from_secs(10), &mut reader).await;
        }

        assert_eq!(
            hub.read_progress(&id),
            ReadProgress::IDLE,
            "a report that arrived after the read ended brought the stage back"
        );
        sink_hook::reset();
    }

    /// The proof that a gate that never opens fails the test and does not hang the binary.
    ///
    /// It runs only when CANVAS_RGB_GATE_NEVER_OPENS is set: the other test opens the gate
    /// itself, so the "never opens" case has to be asked for. The wait gives up after
    /// GATE_TIMEOUT, the test fails, and the guard opens the gate on the way out: the worker
    /// that is blocked at the gate is released, so the binary ends with the failure instead of
    /// hanging on it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_gate_that_never_opens_fails_the_test_and_does_not_hang_the_binary() {
        if !sink_hook::never_opens() {
            eprintln!(
                "skipped: set CANVAS_RGB_GATE_NEVER_OPENS=1 to run the never-opening gate proof"
            );
            return;
        }
        sink_hook::reset();
        let id = DeviceId("held:never".into());
        let hub = Arc::new(PeripheralHub::new(
            vec![Box::new(HeldPeripheral::new(id.clone()))],
        ));
        let _reader = {
            let hub = hub.clone();
            let id = id.clone();
            tokio::spawn(async move { hub.read_fresh(&id).await })
        };
        // The sink is at the gate and nothing opens it: the wait gives up and the test fails.
        sink_hook::wait_until_reached();
        sink_hook::wait_until_written();
        // The wait above gives up and panics: this line is only reached if it does not.
        panic!("the wait for the sink to write its stage should have given up");
    }

    /// The never-opening gate, run in the default suite.
    ///
    /// The proof above needs CANVAS_RGB_GATE_NEVER_OPENS, so a plain `cargo test` skips it and a
    /// regression of the guard (a binary that hangs after the failure) would only show as a CI
    /// job that never ends. This test runs the proof as a child process of this same test binary
    /// under a hard deadline: the child must fail, with the wait's own message, before the
    /// deadline. A child that is still running at the deadline is killed and the test fails.
    #[test]
    fn a_gate_that_never_opens_ends_the_binary_with_a_failure_within_the_deadline() {
        use std::io::Read;
        use std::process::{Command, Stdio};

        /// Far over the 10 s the wait gives up after, far under what a hung job costs.
        const HARD_DEADLINE: Duration = Duration::from_secs(45);

        let exe = std::env::current_exe().expect("the path of this test binary");
        // `module_path!()` starts with the crate's name; the test harness names tests without it.
        let module = module_path!().split_once("::").map_or("", |(_, rest)| rest);
        let proof = format!("{module}::a_gate_that_never_opens_fails_the_test_and_does_not_hang_the_binary");

        // token: the test runner's own, non-elevated; the child is this same test binary.
        let mut child = Command::new(exe)
            .args([proof.as_str(), "--exact", "--test-threads=1", "--nocapture"])
            .env("CANVAS_RGB_GATE_NEVER_OPENS", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the proof starts");
        // Drained on threads, so a child that writes a lot cannot block on a full pipe.
        let drain = |mut pipe: Box<dyn Read + Send>| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = pipe.read_to_string(&mut text);
                text
            })
        };
        let out = drain(Box::new(child.stdout.take().expect("piped")));
        let err = drain(Box::new(child.stderr.take().expect("piped")));

        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("the child can be polled") {
                break Some(status);
            }
            if started.elapsed() > HARD_DEADLINE {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let text = format!("{}{}", out.join().unwrap_or_default(), err.join().unwrap_or_default());

        let status = status.unwrap_or_else(|| {
            panic!("a gate that never opens hung the binary for {HARD_DEADLINE:?}; the guard no longer releases it
{text}")
        });
        assert!(!status.success(), "a gate that never opens must fail the test, not pass it
{text}");
        assert!(
            text.contains("gave up"),
            "the child failed, but not with the wait's own message
{text}"
        );
    }

    #[test]
    fn an_unknown_device_has_both_off() {
        let file = PeripheralsFile::default();
        let access = file.access(&DeviceId("fake:one".into()));
        assert!(!access.reads);
        assert!(!access.writes);
    }

    #[test]
    fn the_access_is_saved_per_device_id() {
        let mut file = PeripheralsFile::default();
        let device = DeviceId("fake:one".into());
        file.set(&device, PeripheralAccess { reads: true, writes: true });
        assert_eq!(file.access(&device), PeripheralAccess { reads: true, writes: true });
        // Another device is untouched.
        assert_eq!(file.access(&DeviceId("fake:two".into())), PeripheralAccess::default());
    }

    #[test]
    fn the_file_round_trips_through_json() {
        let mut file = PeripheralsFile::default();
        file.set(
            &DeviceId("fake:one".into()),
            PeripheralAccess { reads: true, writes: false },
        );
        let text = serde_json::to_string(&file).expect("serializes");
        let back: PeripheralsFile = serde_json::from_str(&text).expect("parses");
        assert_eq!(back, file);
        let empty: PeripheralsFile = serde_json::from_str(r#"{"schema_version":1}"#)
            .expect("a missing list is empty");
        assert!(empty.peripherals.is_empty());
    }

    #[test]
    fn a_device_failure_is_not_a_refusal() {
        assert!(device_failure(&AdapterError::NotConnected));
        assert!(device_failure(&AdapterError::Io(std::io::Error::other("gone"))));
        assert!(!device_failure(&AdapterError::Protocol("no such setting".into())));
        assert!(!device_failure(&AdapterError::Unsupported("writes are off")));
    }
}
