//! Temperature sources for the curves. The CPU source (`TctlSource`) lives in `pawnio.rs`
//! because it reads through PawnIO. The GPU source loads NVML dynamically, so a missing
//! driver or GPU is "unavailable", never a crash.

use std::ffi::c_void;
use std::io;

/// One temperature source, read in degrees C.
pub trait TempReader: Send {
    fn read_celsius(&mut self) -> io::Result<f32>;
}

type NvmlInit = unsafe extern "C" fn() -> std::ffi::c_int;
type NvmlShutdown = unsafe extern "C" fn() -> std::ffi::c_int;
type NvmlHandleByIndex = unsafe extern "C" fn(u32, *mut *mut c_void) -> std::ffi::c_int;
type NvmlTemperature = unsafe extern "C" fn(*mut c_void, std::ffi::c_int, *mut u32) -> std::ffi::c_int;

const NVML_SUCCESS: std::ffi::c_int = 0;
/// `NVML_TEMPERATURE_GPU` in nvml.h.
const NVML_TEMPERATURE_GPU: std::ffi::c_int = 0;

/// GPU core temperature through NVML. `nvml.dll` is loaded at runtime.
pub struct NvmlSource {
    _library: libloading::Library,
    device: *mut c_void,
    temperature: NvmlTemperature,
    shutdown: NvmlShutdown,
}

// SAFETY: the device handle is a process-wide NVML handle with no thread affinity, and the
// function pointers come from `nvml.dll`, which stays loaded for the life of `_library`.
unsafe impl Send for NvmlSource {}

fn unavailable(what: &str, detail: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("NVML {what}: {detail}"))
}

impl NvmlSource {
    /// Loads NVML, initialises it and opens GPU 0. Any failure is an `Err`, not a panic.
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
        // SAFETY: each symbol is looked up under the documented name and signature from nvml.h;
        // the copied function pointers are used only while `library` is alive.
        let (init, handle, temperature, shutdown) = unsafe {
            let init: libloading::Symbol<NvmlInit> =
                library.get(b"nvmlInit_v2\0").map_err(|e| unavailable("symbol nvmlInit_v2", e))?;
            let handle: libloading::Symbol<NvmlHandleByIndex> = library
                .get(b"nvmlDeviceGetHandleByIndex_v2\0")
                .map_err(|e| unavailable("symbol nvmlDeviceGetHandleByIndex_v2", e))?;
            let temperature: libloading::Symbol<NvmlTemperature> = library
                .get(b"nvmlDeviceGetTemperature\0")
                .map_err(|e| unavailable("symbol nvmlDeviceGetTemperature", e))?;
            let shutdown: libloading::Symbol<NvmlShutdown> =
                library.get(b"nvmlShutdown\0").map_err(|e| unavailable("symbol nvmlShutdown", e))?;
            (*init, *handle, *temperature, *shutdown)
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
        Ok(Self { _library: library, device, temperature, shutdown })
    }
}

impl TempReader for NvmlSource {
    fn read_celsius(&mut self) -> io::Result<f32> {
        let mut value: u32 = 0;
        // SAFETY: `device` came from nvmlDeviceGetHandleByIndex_v2 and `value` is a valid out pointer.
        let status = unsafe { (self.temperature)(self.device, NVML_TEMPERATURE_GPU, &mut value) };
        if status != NVML_SUCCESS {
            return Err(unavailable("temperature", format!("status {status}")));
        }
        Ok(value as f32)
    }
}

impl Drop for NvmlSource {
    fn drop(&mut self) {
        // SAFETY: balances the successful nvmlInit_v2 in `open`.
        unsafe { (self.shutdown)() };
    }
}
