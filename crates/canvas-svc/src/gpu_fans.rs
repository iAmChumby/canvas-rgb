//! GPU fan control through NVML, presented as more fan headers (`gpu_fan1`, `gpu_fan2`) after
//! the NCT6687D-R's.
//!
//! [`NvmlFans`] is the real NVML binding; [`GpuFanApi`] is the small seam over it so tests run
//! against `fakes::FakeGpuFans`. [`CompositeChip`] is the [`FanChip`] the service drives: it
//! routes the first headers to the NCT chip (when there is one) and the rest to the GPU.
//!
//! Behaviour proven on Luke's RTX 5070 Ti (driver 610.88): `SetFanSpeed_v2` switches the fan to
//! the manual policy at once and the target at once, while the actual speed takes about 6 s to
//! arrive; `SetDefaultFanSpeed_v2` returns the fan to the auto policy. NVML's minimum is 30%.

use crate::chip::{Channel, FanChip};
use std::ffi::{c_int, c_void};
use std::io;

/// NVML's lowest manual fan speed in percent (`nvmlDeviceGetMinMaxFanSpeed` read 30/100).
/// Mirrors `canvas_proto::fans::GPU_FAN_MIN_PERCENT`, which the API already enforces.
pub const NVML_MIN_FAN_PERCENT: u8 = canvas_proto::fans::GPU_FAN_MIN_PERCENT;
/// The most GPU fans the composite exposes (ids come from a static table).
pub const MAX_GPU_FANS: usize = 4;
/// `NVML_FAN_POLICY_MANUAL` in nvml.h (`NVML_FAN_POLICY_TEMPERATURE_CONTINOUS_SW` is 0).
const NVML_FAN_POLICY_MANUAL: u32 = 1;
const NVML_SUCCESS: c_int = 0;

/// What the service needs from the GPU's fans, by NVML fan index (0-based).
pub trait GpuFanApi: Send {
    /// Number of fans on the GPU.
    fn count(&mut self) -> io::Result<u32>;
    /// The speed the fan actually spins at, percent. Lags the target by about 6 s.
    fn speed(&mut self, fan: u32) -> io::Result<u32>;
    /// The speed NVML is aiming for, percent. Changes the moment a speed is set.
    fn target(&mut self, fan: u32) -> io::Result<u32>;
    /// Whether the fan control policy is manual.
    fn policy_manual(&mut self, fan: u32) -> io::Result<bool>;
    /// Sets a manual speed. NVML switches the policy to manual by itself.
    fn set_speed(&mut self, fan: u32, percent: u32) -> io::Result<()>;
    /// Returns the fan to the driver's automatic control.
    fn set_default(&mut self, fan: u32) -> io::Result<()>;
}

type NvmlInit = unsafe extern "C" fn() -> c_int;
type NvmlShutdown = unsafe extern "C" fn() -> c_int;
type NvmlHandleByIndex = unsafe extern "C" fn(u32, *mut *mut c_void) -> c_int;
/// `(device, *out)` calls: `GetNumFans`.
type NvmlDeviceOut = unsafe extern "C" fn(*mut c_void, *mut u32) -> c_int;
/// `(device, fan, *out)` calls: speed, target, policy.
type NvmlFanOut = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> c_int;
/// `(device, fan, value)` calls: `SetFanSpeed_v2`.
type NvmlFanSet = unsafe extern "C" fn(*mut c_void, u32, u32) -> c_int;
/// `(device, fan)` calls: `SetDefaultFanSpeed_v2`.
type NvmlFanDefault = unsafe extern "C" fn(*mut c_void, u32) -> c_int;

/// The NVML fan entry points of GPU 0. `nvml.dll` is loaded at runtime.
pub struct NvmlFans {
    _library: libloading::Library,
    device: *mut c_void,
    num_fans: NvmlDeviceOut,
    get_speed: NvmlFanOut,
    get_target: NvmlFanOut,
    get_policy: NvmlFanOut,
    set_speed: NvmlFanSet,
    set_default: NvmlFanDefault,
    shutdown: NvmlShutdown,
}

// SAFETY: the device handle is a process-wide NVML handle with no thread affinity, and the
// function pointers come from `nvml.dll`, which stays loaded for the life of `_library`.
unsafe impl Send for NvmlFans {}

fn unavailable(what: &str, detail: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("NVML {what}: {detail}"))
}

fn call_failed(name: &str, status: c_int) -> io::Error {
    io::Error::other(format!("NVML {name} failed (status {status})"))
}

impl NvmlFans {
    /// Loads NVML, initialises it and opens GPU 0. A missing driver, a missing entry point or a
    /// GPU with no fans is an `Err`, never a panic.
    pub fn open() -> io::Result<Self> {
        // SAFETY: loading a DLL runs its initialiser; `nvml.dll` is the NVIDIA driver library.
        // Only from System32 (where the NVIDIA driver installs it), never the DLL search path:
        // this runs as LocalSystem, and a PATH directory may be user-writable.
        let library: libloading::Library = unsafe {
            libloading::os::windows::Library::load_with_flags(
                "nvml.dll",
                libloading::os::windows::LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .map_err(|e| unavailable("load", e))?
        .into();
        // SAFETY: each symbol is looked up under its documented name and the signature from
        // nvml.h; the copied function pointers are used only while `library` is alive.
        let (init, handle, num_fans, get_speed, get_target, get_policy, set_speed, set_default, shutdown) = unsafe {
            let init: libloading::Symbol<NvmlInit> =
                library.get(b"nvmlInit_v2\0").map_err(|e| unavailable("symbol nvmlInit_v2", e))?;
            let handle: libloading::Symbol<NvmlHandleByIndex> = library
                .get(b"nvmlDeviceGetHandleByIndex_v2\0")
                .map_err(|e| unavailable("symbol nvmlDeviceGetHandleByIndex_v2", e))?;
            let num_fans: libloading::Symbol<NvmlDeviceOut> = library
                .get(b"nvmlDeviceGetNumFans\0")
                .map_err(|e| unavailable("symbol nvmlDeviceGetNumFans", e))?;
            let get_speed: libloading::Symbol<NvmlFanOut> = library
                .get(b"nvmlDeviceGetFanSpeed_v2\0")
                .map_err(|e| unavailable("symbol nvmlDeviceGetFanSpeed_v2", e))?;
            let get_target: libloading::Symbol<NvmlFanOut> = library
                .get(b"nvmlDeviceGetTargetFanSpeed\0")
                .map_err(|e| unavailable("symbol nvmlDeviceGetTargetFanSpeed", e))?;
            let get_policy: libloading::Symbol<NvmlFanOut> = library
                .get(b"nvmlDeviceGetFanControlPolicy_v2\0")
                .map_err(|e| unavailable("symbol nvmlDeviceGetFanControlPolicy_v2", e))?;
            let set_speed: libloading::Symbol<NvmlFanSet> = library
                .get(b"nvmlDeviceSetFanSpeed_v2\0")
                .map_err(|e| unavailable("symbol nvmlDeviceSetFanSpeed_v2", e))?;
            let set_default: libloading::Symbol<NvmlFanDefault> = library
                .get(b"nvmlDeviceSetDefaultFanSpeed_v2\0")
                .map_err(|e| unavailable("symbol nvmlDeviceSetDefaultFanSpeed_v2", e))?;
            let shutdown: libloading::Symbol<NvmlShutdown> =
                library.get(b"nvmlShutdown\0").map_err(|e| unavailable("symbol nvmlShutdown", e))?;
            (*init, *handle, *num_fans, *get_speed, *get_target, *get_policy, *set_speed, *set_default, *shutdown)
        };
        // SAFETY: `init` is the NVML entry point above; no arguments.
        if unsafe { init() } != NVML_SUCCESS {
            return Err(unavailable("init", "nvmlInit_v2 failed (no NVIDIA driver?)"));
        }
        let mut device: *mut c_void = std::ptr::null_mut();
        // SAFETY: `device` is a valid out pointer; index 0 is the first GPU.
        let status = unsafe { handle(0, &mut device) };
        if status != NVML_SUCCESS || device.is_null() {
            // SAFETY: NVML was initialised above; shut it down before failing.
            unsafe { shutdown() };
            return Err(unavailable("device", format!("GPU 0 not available (status {status})")));
        }
        let mut fans = 0u32;
        // SAFETY: `device` is the handle above and `fans` is a valid out pointer.
        let status = unsafe { num_fans(device, &mut fans) };
        if status != NVML_SUCCESS || fans == 0 {
            // SAFETY: NVML was initialised above; shut it down before failing.
            unsafe { shutdown() };
            return Err(unavailable("fans", format!("GPU 0 reports no controllable fans (status {status}, count {fans})")));
        }
        Ok(Self { _library: library, device, num_fans, get_speed, get_target, get_policy, set_speed, set_default, shutdown })
    }

    fn read(&self, name: &str, call: NvmlFanOut, fan: u32) -> io::Result<u32> {
        let mut value = 0u32;
        // SAFETY: `device` came from nvmlDeviceGetHandleByIndex_v2 and `value` is a valid out pointer.
        let status = unsafe { call(self.device, fan, &mut value) };
        if status != NVML_SUCCESS {
            return Err(call_failed(name, status));
        }
        Ok(value)
    }
}

impl GpuFanApi for NvmlFans {
    fn count(&mut self) -> io::Result<u32> {
        let mut value = 0u32;
        // SAFETY: `device` came from nvmlDeviceGetHandleByIndex_v2 and `value` is a valid out pointer.
        let status = unsafe { (self.num_fans)(self.device, &mut value) };
        if status != NVML_SUCCESS {
            return Err(call_failed("nvmlDeviceGetNumFans", status));
        }
        Ok(value)
    }

    fn speed(&mut self, fan: u32) -> io::Result<u32> {
        self.read("nvmlDeviceGetFanSpeed_v2", self.get_speed, fan)
    }

    fn target(&mut self, fan: u32) -> io::Result<u32> {
        self.read("nvmlDeviceGetTargetFanSpeed", self.get_target, fan)
    }

    fn policy_manual(&mut self, fan: u32) -> io::Result<bool> {
        Ok(self.read("nvmlDeviceGetFanControlPolicy_v2", self.get_policy, fan)? == NVML_FAN_POLICY_MANUAL)
    }

    fn set_speed(&mut self, fan: u32, percent: u32) -> io::Result<()> {
        // SAFETY: `device` came from nvmlDeviceGetHandleByIndex_v2; the arguments are plain integers.
        let status = unsafe { (self.set_speed)(self.device, fan, percent) };
        if status != NVML_SUCCESS {
            return Err(call_failed("nvmlDeviceSetFanSpeed_v2", status));
        }
        Ok(())
    }

    fn set_default(&mut self, fan: u32) -> io::Result<()> {
        // SAFETY: `device` came from nvmlDeviceGetHandleByIndex_v2; the argument is a plain integer.
        let status = unsafe { (self.set_default)(self.device, fan) };
        if status != NVML_SUCCESS {
            return Err(call_failed("nvmlDeviceSetDefaultFanSpeed_v2", status));
        }
        Ok(())
    }
}

impl Drop for NvmlFans {
    fn drop(&mut self) {
        // SAFETY: balances the successful nvmlInit_v2 in `open`.
        unsafe { (self.shutdown)() };
    }
}

/// A GPU header. Only `id`, `label` and `controllable()` matter: the composite never uses the
/// register fields for GPU entries, they are set so `controllable()` is true.
const fn gpu_channel(id: &'static str, label: &'static str) -> Channel {
    Channel { id, label, rpm_reg: 0, pwm_read_reg: Some(0), pwm_write_reg: Some(0), mode_reg: 0, mode_bit: Some(0) }
}

const GPU_CHANNELS: [Channel; MAX_GPU_FANS] = [
    gpu_channel("gpu_fan1", "GPU fan 1"),
    gpu_channel("gpu_fan2", "GPU fan 2"),
    gpu_channel("gpu_fan3", "GPU fan 3"),
    gpu_channel("gpu_fan4", "GPU fan 4"),
];

/// Map name when there is no NCT chip: never named in `chip.toml`, so nothing is written.
pub const NVML_ONLY_MAP: &str = "nvml-only";

fn bad_channel(channel: usize) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("no header index {channel}"))
}

/// Where a header index lands.
enum Route {
    Nct,
    Gpu(u32),
}

/// The NCT6687D-R's headers first, then `gpu_fan1..N`.
pub struct CompositeChip {
    nct: Option<Box<dyn FanChip>>,
    gpu: Option<Box<dyn GpuFanApi>>,
    nct_len: usize,
    gpu_len: usize,
    channels: &'static [Channel],
}

impl CompositeChip {
    /// Builds the combined channel table. It is leaked: it must be `'static` for
    /// [`FanChip::channels`], and one is built per service run (a few hundred bytes).
    pub fn new(nct: Option<Box<dyn FanChip>>, mut gpu: Option<Box<dyn GpuFanApi>>) -> Self {
        let gpu_len = match gpu.as_mut().map(|g| g.count()) {
            Some(Ok(n)) => (n as usize).min(MAX_GPU_FANS),
            Some(Err(e)) => {
                tracing::warn!(error = %e, "cannot count the GPU fans; none will be driven");
                gpu = None;
                0
            }
            None => 0,
        };
        let mut table: Vec<Channel> = nct.as_ref().map_or_else(Vec::new, |c| c.channels().to_vec());
        let nct_len = table.len();
        table.extend_from_slice(&GPU_CHANNELS[..gpu_len]);
        let channels: &'static [Channel] = Box::leak(table.into_boxed_slice());
        Self { nct, gpu, nct_len, gpu_len, channels }
    }

    fn route(&self, channel: usize) -> io::Result<Route> {
        if channel < self.nct_len {
            Ok(Route::Nct)
        } else if channel - self.nct_len < self.gpu_len {
            Ok(Route::Gpu((channel - self.nct_len) as u32))
        } else {
            Err(bad_channel(channel))
        }
    }

    fn nct(&mut self) -> io::Result<&mut Box<dyn FanChip>> {
        self.nct.as_mut().ok_or_else(|| io::Error::other("no NCT6687D-R"))
    }

    fn gpu(&mut self) -> io::Result<&mut Box<dyn GpuFanApi>> {
        self.gpu.as_mut().ok_or_else(|| io::Error::other("no GPU fans"))
    }
}

impl FanChip for CompositeChip {
    fn chip_name(&self) -> String {
        match (&self.nct, self.gpu_len) {
            (Some(nct), 0) => nct.chip_name(),
            (Some(nct), n) => format!("{} + NVML {n} fans", nct.chip_name()),
            (None, n) => format!("NVML {n} fans"),
        }
    }

    /// The NCT chip's map: its confirmation in `chip.toml` gates writes for every header,
    /// the GPU ones included. Without the NCT chip no map can be confirmed.
    fn map_name(&self) -> &'static str {
        self.nct.as_ref().map_or(NVML_ONLY_MAP, |c| c.map_name())
    }

    fn channels(&self) -> &'static [Channel] {
        self.channels
    }

    fn board_sensor_names(&self) -> &'static [&'static str] {
        self.nct.as_ref().map_or(&[], |c| c.board_sensor_names())
    }

    /// GPU fans report no RPM: NVML gives percent here, and RPM is not invented.
    fn read_rpm(&mut self, channel: usize) -> io::Result<Option<u32>> {
        match self.route(channel)? {
            Route::Nct => self.nct()?.read_rpm(channel),
            Route::Gpu(_) => Ok(None),
        }
    }

    /// A GPU fan reports its TARGET speed, not its actual speed. The real fan takes about 6 s to
    /// reach the target, and `fans.rs` `READBACK_SETTLE` allows 3 s, so the actual speed would
    /// fail the readback of every write. The target is what NVML accepted, and it is immediate.
    fn read_duty_percent(&mut self, channel: usize) -> io::Result<Option<u8>> {
        match self.route(channel)? {
            Route::Nct => self.nct()?.read_duty_percent(channel),
            Route::Gpu(fan) => Ok(Some(self.gpu()?.target(fan)?.min(100) as u8)),
        }
    }

    fn read_manual(&mut self, channel: usize) -> io::Result<bool> {
        match self.route(channel)? {
            Route::Nct => self.nct()?.read_manual(channel),
            Route::Gpu(fan) => self.gpu()?.policy_manual(fan),
        }
    }

    /// For a GPU fan, manual is entered by the first speed write, so `true` makes no call.
    /// `false` is the BIOS handback: the driver's automatic control.
    fn set_manual(&mut self, channel: usize, manual: bool) -> io::Result<()> {
        match self.route(channel)? {
            Route::Nct => self.nct()?.set_manual(channel, manual),
            Route::Gpu(_) if manual => Ok(()),
            Route::Gpu(fan) => self.gpu()?.set_default(fan),
        }
    }

    /// A GPU fan never goes below NVML's 30% minimum, whatever the caller asks for.
    fn write_duty_percent(&mut self, channel: usize, percent: u8) -> io::Result<()> {
        match self.route(channel)? {
            Route::Nct => self.nct()?.write_duty_percent(channel, percent),
            Route::Gpu(fan) => {
                self.gpu()?.set_speed(fan, u32::from(percent.clamp(NVML_MIN_FAN_PERCENT, 100)))
            }
        }
    }

    fn read_board_temp(&mut self, name: &str) -> io::Result<Option<f32>> {
        match self.nct.as_mut() {
            Some(nct) => nct.read_board_temp(name),
            None => Ok(None),
        }
    }
}
