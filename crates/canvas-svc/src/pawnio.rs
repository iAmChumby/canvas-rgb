//! PawnIO access: loads `PawnIOLib.dll`, opens a signed PawnIO module by file name, and
//! exposes the LpcIO module as a [`PortIo`] and the AMDFamily17 module as the CPU Tctl source.
//!
//! The C entry points (`pawnio_version`, `pawnio_open`, `pawnio_load`, `pawnio_execute`,
//! `pawnio_close`) and their argument shapes are taken from Hypercolor's
//! `hypercolor-windows-pawnio/src/windows.rs` (Apache-2.0). The LpcIO ioctl names
//! (`ioctl_select_slot`, `ioctl_find_bars`, `ioctl_pio_inb`, `ioctl_pio_outb`) come from
//! PawnIO.Modules `LpcIO.p`. The AMD SMN read and its bit layout come from Hypercolor's
//! `CpuTempReader` and AMD's THM_TCON_CUR_TMP register.
//!
//! Nothing here runs in the test suite or on this machine. It compiles, and it is
//! exercised only by Luke's real-hardware session.

use crate::chip::PortIo;
use crate::temps::TempReader;
use libloading::{Library, Symbol};
use std::ffi::{c_char, c_void, CString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Environment override for the PawnIO install directory (`PawnIOLib.dll` and `modules\`).
pub const PAWNIO_HOME_ENV: &str = "CANVAS_RGB_PAWNIO_HOME";
/// Default install directory. Services run as LocalSystem, so this must be admin-owned.
pub const DEFAULT_PAWNIO_HOME: &str = r"C:\Program Files\PawnIO";
pub const LPC_MODULE: &str = "LpcIO.bin";
pub const AMD_MODULE: &str = "AMDFamily17.bin";

type PawnVersion = unsafe extern "C" fn(*mut u32) -> i32;
type PawnOpen = unsafe extern "C" fn(*mut *mut c_void) -> i32;
type PawnLoad = unsafe extern "C" fn(*mut c_void, *const u8, usize) -> i32;
type PawnExecute = unsafe extern "C" fn(*mut c_void, *const c_char, *const u64, usize, *mut u64, usize, *mut usize) -> i32;
type PawnClose = unsafe extern "C" fn(*mut c_void) -> i32;

/// The install directory: env override, else the default.
pub fn pawnio_home() -> PathBuf {
    std::env::var_os(PAWNIO_HOME_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_PAWNIO_HOME))
}

fn io_other(detail: impl std::fmt::Display) -> io::Error {
    io::Error::other(detail.to_string())
}

fn check(operation: &str, status: i32) -> io::Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(io_other(format!("{operation} failed with status {status:#x}")))
    }
}

/// The loaded `PawnIOLib.dll` entry points. Shared by every module handle.
struct Runtime {
    _library: Library,
    open: PawnOpen,
    load: PawnLoad,
    execute: PawnExecute,
    close: PawnClose,
}

impl Runtime {
    fn load(home: &Path) -> io::Result<Arc<Self>> {
        let path = home.join("PawnIOLib.dll");
        // SAFETY: loading the PawnIO runtime DLL from the admin-owned install directory.
        let library = unsafe { Library::new(&path) }.map_err(|e| io_other(format!("{}: {e}", path.display())))?;
        // SAFETY: each symbol is looked up under the documented name; the copied function
        // pointers are used only while `library` is alive (it is stored in the same struct).
        let (version, open, load, execute, close) = unsafe {
            let version: Symbol<PawnVersion> = library.get(b"pawnio_version\0").map_err(io_other)?;
            let open: Symbol<PawnOpen> = library.get(b"pawnio_open\0").map_err(io_other)?;
            let load: Symbol<PawnLoad> = library.get(b"pawnio_load\0").map_err(io_other)?;
            let execute: Symbol<PawnExecute> = library.get(b"pawnio_execute\0").map_err(io_other)?;
            let close: Symbol<PawnClose> = library.get(b"pawnio_close\0").map_err(io_other)?;
            (*version, *open, *load, *execute, *close)
        };
        let mut raw = 0u32;
        // SAFETY: `raw` is a valid out pointer.
        check("pawnio_version", unsafe { version(&mut raw) })?;
        tracing::info!(version = format_args!("{}.{}.{}", raw >> 16, (raw >> 8) & 0xff, raw & 0xff), "PawnIO runtime loaded");
        Ok(Arc::new(Self { _library: library, open, load, execute, close }))
    }
}

/// One opened PawnIO module. Closed on drop.
pub struct Module {
    runtime: Arc<Runtime>,
    handle: *mut c_void,
}

// SAFETY: the handle is an opaque PawnIO handle used from one owner at a time; PawnIO
// serialises its own ioctls. `Module` is never shared (no `Sync`).
unsafe impl Send for Module {}

impl Module {
    /// Opens `modules_dir\file_name` (a `.bin` PawnIO module) on a fresh handle.
    pub fn open(home: &Path, file_name: &str) -> io::Result<Self> {
        let runtime = Runtime::load(home)?;
        let blob_path = home.join("modules").join(file_name);
        let blob = std::fs::read(&blob_path).map_err(|e| io_other(format!("{}: {e}", blob_path.display())))?;
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: `handle` is a valid out pointer.
        check("pawnio_open", unsafe { (runtime.open)(&mut handle) })?;
        // SAFETY: `handle` came from pawnio_open; `blob` is valid for the call.
        let loaded = unsafe { (runtime.load)(handle, blob.as_ptr(), blob.len()) };
        if let Err(e) = check(&format!("pawnio_load {file_name}"), loaded) {
            // SAFETY: `handle` is open and not used again.
            unsafe { (runtime.close)(handle) };
            return Err(e);
        }
        Ok(Self { runtime, handle })
    }

    /// Calls one named ioctl with 64-bit input cells and fills `output`. Returns the number of
    /// output cells PawnIO reports.
    pub fn call(&mut self, name: &str, input: &[u64], output: &mut [u64]) -> io::Result<usize> {
        let c_name = CString::new(name).map_err(io_other)?;
        let mut returned = 0usize;
        // SAFETY: the handle is open; the name, input and output buffers are valid for the call.
        let status = unsafe {
            (self.runtime.execute)(
                self.handle,
                c_name.as_ptr(),
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                output.len(),
                &mut returned,
            )
        };
        check(name, status)?;
        Ok(returned)
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `Module::open` and is closed exactly once here.
        unsafe { (self.runtime.close)(self.handle) };
    }
}

/// Port access on the real board: the PawnIO LpcIO module. Port reads and writes go through
/// `ioctl_pio_inb` / `ioctl_pio_outb`, which allow only the Super I/O ports and the BARs that
/// `ioctl_find_bars` discovers.
pub struct PawnIoPortIo {
    module: Module,
}

impl PawnIoPortIo {
    /// Opens LpcIO and selects Super I/O slot 0 (port 0x2E).
    pub fn open(home: &Path) -> io::Result<Self> {
        let mut module = Module::open(home, LPC_MODULE)?;
        module.call("ioctl_select_slot", &[0], &mut [])?;
        Ok(Self { module })
    }
}

impl PortIo for PawnIoPortIo {
    fn read_port(&mut self, port: u16) -> io::Result<u8> {
        let mut out = [0u64; 1];
        self.module.call("ioctl_pio_inb", &[u64::from(port)], &mut out)?;
        Ok(out[0] as u8)
    }

    fn write_port(&mut self, port: u16, value: u8) -> io::Result<()> {
        self.module.call("ioctl_pio_outb", &[u64::from(port), u64::from(value)], &mut [])?;
        Ok(())
    }

    fn config_mode_entered(&mut self) -> io::Result<()> {
        self.module.call("ioctl_find_bars", &[], &mut [])?;
        Ok(())
    }
}

/// AMD Family 17h CPU temperature (Tctl) through the AMDFamily17 module's SMN read.
pub struct TctlSource {
    module: Module,
}

/// `THM_TCON_CUR_TMP` in the SMN space (Hypercolor's `AMD_F17_THM_TCON_CUR_TMP`).
pub const AMD_THM_TCON_CUR_TMP: u64 = 0x0005_9800;

impl TctlSource {
    pub fn open(home: &Path) -> io::Result<Self> {
        Ok(Self { module: Module::open(home, AMD_MODULE)? })
    }
}

/// Decodes `THM_TCON_CUR_TMP`: CurTmp in bits 31:21 at 0.125 C per step, minus 49 C when
/// CurTempRangeSel (bit 19) is set.
pub fn decode_tctl(raw: u32) -> f32 {
    let cur = (raw >> 21) & 0x7FF;
    let mut celsius = cur as f32 * 0.125;
    if raw & (1 << 19) != 0 {
        celsius -= 49.0;
    }
    celsius
}

impl TempReader for TctlSource {
    fn read_celsius(&mut self) -> io::Result<f32> {
        let mut out = [0u64; 1];
        self.module.call("ioctl_read_smn", &[AMD_THM_TCON_CUR_TMP], &mut out)?;
        Ok(decode_tctl(out[0] as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tctl_decodes_the_documented_layout() {
        // CurTmp = 400 steps = 50.0 C, range select clear.
        assert_eq!(decode_tctl(400 << 21), 50.0);
        // Range select set subtracts 49 C.
        assert_eq!(decode_tctl((400 << 21) | (1 << 19)), 1.0);
    }
}
