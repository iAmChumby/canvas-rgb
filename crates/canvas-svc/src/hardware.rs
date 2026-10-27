//! Opens the real fan chip and temperature sources. Every failure becomes "unavailable" and
//! is logged; the service still runs, with the fan headers absent from status.

use crate::chip::{FanChip, Nct6687, MSI_ALT1_MAP};
use crate::gpu_fans::{CompositeChip, GpuFanApi, NvmlFans};
use crate::pawnio::{pawnio_home, PawnIoPortIo, TctlSource};
use crate::temps::{NvmlSource, TempReader};
use canvas_proto::fans::TempSource;
use std::collections::BTreeMap;

pub struct Hardware {
    pub chip: Option<Box<dyn FanChip>>,
    pub chip_error: Option<String>,
    pub temps: BTreeMap<TempSource, Box<dyn TempReader>>,
}

/// Opens the fan headers: the NCT6687D-R's, then the GPU's through NVML, as one chip. Either
/// half may be missing; both missing is the NCT error. Used by service startup and by
/// `--restore-bios`, so the GPU handback runs in both.
pub fn open_chip() -> Result<Box<dyn FanChip>, String> {
    let nct = open_nct();
    let gpu: Option<Box<dyn GpuFanApi>> = match NvmlFans::open() {
        Ok(fans) => Some(Box::new(fans)),
        Err(e) => {
            tracing::warn!(error = %e, "GPU fans unavailable; only the motherboard headers will be driven");
            None
        }
    };
    match (nct, gpu) {
        (Err(e), None) => Err(e),
        (nct, gpu) => {
            let nct = match nct {
                Ok(chip) => Some(chip),
                Err(e) => {
                    tracing::error!(error = %e, "fan chip unavailable; only the GPU fans can be handed back");
                    None
                }
            };
            Ok(Box::new(CompositeChip::new(nct, gpu)))
        }
    }
}

/// Opens the NCT6687D-R through PawnIO's LpcIO module and checks it with the detect path.
fn open_nct() -> Result<Box<dyn FanChip>, String> {
    let home = pawnio_home();
    // Both Super I/O ports are tried, 0x2E first; the errors of both are reported.
    let mut errors = Vec::new();
    for (slot, sio) in crate::chip::SIO_PORTS.iter().enumerate() {
        let io = match PawnIoPortIo::open(&home, slot as u64) {
            Ok(io) => io,
            Err(e) => {
                errors.push(format!("{sio:#x}: PawnIO LpcIO: {e}"));
                continue;
            }
        };
        match Nct6687::detect_at(io, MSI_ALT1_MAP, *sio) {
            Ok(chip) => return Ok(Box::new(chip)),
            Err(e) => errors.push(format!("{sio:#x}: {e}")),
        }
    }
    Err(errors.join("; "))
}

/// Opens every source the service can read. Nothing here panics on a missing driver.
pub fn open() -> Hardware {
    let (chip, chip_error) = match open_chip() {
        Ok(chip) => {
            tracing::info!(chip = %chip.chip_name(), map = MSI_ALT1_MAP.name, confirmed = MSI_ALT1_MAP.confirmed, "fan chip detected");
            (Some(chip), None)
        }
        Err(e) => {
            tracing::error!(error = %e, "fan chip unavailable; fan headers will not be driven");
            (None, Some(e))
        }
    };
    let mut temps: BTreeMap<TempSource, Box<dyn TempReader>> = BTreeMap::new();
    match TctlSource::open(&pawnio_home()) {
        Ok(source) => {
            temps.insert(TempSource::Cpu, Box::new(source));
        }
        Err(e) => tracing::warn!(error = %e, "CPU Tctl unavailable"),
    }
    match NvmlSource::open() {
        Ok(source) => {
            temps.insert(TempSource::Gpu, Box::new(source));
        }
        Err(e) => tracing::warn!(error = %e, "GPU temperature unavailable"),
    }
    Hardware { chip, chip_error, temps }
}
