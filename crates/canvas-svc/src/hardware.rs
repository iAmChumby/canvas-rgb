//! Opens the real fan chip and temperature sources. Every failure becomes "unavailable" and
//! is logged; the service still runs, with the fan headers absent from status.

use crate::chip::{ChipError, FanChip, Nct6687, PortIo, MSI_ALT1_MAP};
use crate::gpu_fans::{CompositeChip, GpuFanApi, NvmlFans};
use crate::pawnio::{pawnio_home, PawnIoError, PawnIoPortIo, TctlSource};
use crate::temps::{NvmlSource, TempReader};
use canvas_proto::fans::{ChipProblem, TempSource};
use std::collections::BTreeMap;

pub struct Hardware {
    pub chip: Option<Box<dyn FanChip>>,
    pub chip_error: Option<String>,
    /// Issue #580 (R1): why the NCT half could not be opened, from the open itself. The daemon
    /// must never parse `chip_error` to learn this; a GPU can be present when the NCT is not.
    pub chip_problem: Option<ChipProblem>,
    /// Issue #580 (R13): the chip open was tried, so `chip_problem: None` means "fine". An older
    /// service has no such field and reads false, which is never a healthy observation.
    pub chip_checked: bool,
    pub temps: BTreeMap<TempSource, Box<dyn TempReader>>,
    /// The GPU's slowdown threshold in °C (issue #130), read through the same NVML
    /// loader as the fans. `None` when there is no GPU or the driver reports none.
    pub gpu_rated_max_c: Option<f32>,
}

/// Why the NCT half could not be opened (#580 R1, R17). The human text stays for the log; the
/// `ChipProblem` is the typed answer that crosses the pipe.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ChipOpenError {
    #[error("PawnIO unavailable: {0}")]
    PawnIoUnavailable(String),
    #[error("chip not found: {0}")]
    ChipNotFound(String),
}

impl From<PawnIoError> for ChipOpenError {
    fn from(error: PawnIoError) -> Self {
        Self::PawnIoUnavailable(error.to_string())
    }
}

/// The typed answer for one failed Super I/O port (R17). A transport failure means PawnIO could
/// not be used; a chip that answered with the wrong id or base means no supported chip.
pub fn chip_problem_from(error: &ChipOpenError) -> ChipProblem {
    match error {
        ChipOpenError::PawnIoUnavailable(_) => ChipProblem::PawnIoUnavailable,
        ChipOpenError::ChipNotFound(_) => ChipProblem::ChipNotFound,
    }
}

/// Two ports are tried. A failed PawnIO on either port wins (R17): one port with no driver says
/// more about the machine than one port with no chip.
fn merge_problems(a: Option<ChipProblem>, b: Option<ChipProblem>) -> Option<ChipProblem> {
    match (a, b) {
        (Some(ChipProblem::PawnIoUnavailable), _) | (_, Some(ChipProblem::PawnIoUnavailable)) => {
            Some(ChipProblem::PawnIoUnavailable)
        }
        (Some(ChipProblem::ChipNotFound), _) | (_, Some(ChipProblem::ChipNotFound)) => {
            Some(ChipProblem::ChipNotFound)
        }
        _ => None,
    }
}

/// The problem from the ports that failed, in port order. `None` for a port means it was not
/// tried because an earlier port found a chip; the open then has no problem at all.
fn chip_problem_from_ports(outcomes: &[ChipOpenError]) -> Option<ChipProblem> {
    // A fold over what was actually tried: no fixed port count and nothing to panic on (C7).
    outcomes
        .iter()
        .fold(None, |problem, e| merge_problems(problem, Some(chip_problem_from(e))))
}

/// Opens one Super I/O port. The real opener uses PawnIO; tests inject a fake (C8).
pub trait SioOpener: Send {
    fn open(&self, slot: u64, sio: u16) -> Result<Box<dyn PortIo>, ChipOpenError>;
}

/// The PawnIO implementation of [`SioOpener`], with the install directory from the environment.
struct PawnIoOpener {
    home: std::path::PathBuf,
}

impl SioOpener for PawnIoOpener {
    fn open(&self, slot: u64, _sio: u16) -> Result<Box<dyn PortIo>, ChipOpenError> {
        PawnIoPortIo::open(&self.home, slot).map(|io| Box::new(io) as Box<dyn PortIo>).map_err(ChipOpenError::from)
    }
}

/// The chip open and the GPU threshold as one answer (#580 R17): the NCT problem is carried in
/// both the composite case and the case where no chip at all was opened.
struct ChipOpen {
    chip: Result<Box<dyn FanChip>, String>,
    threshold: Option<f32>,
    problem: Option<ChipProblem>,
}

/// Opens the fan headers: the NCT6687D-R's, then the GPU's through NVML, as one chip. Either
/// half may be missing; both missing is the NCT error. Used by service startup and by
/// `--restore-bios`, so the GPU handback runs in both.
pub fn open_chip() -> Result<Box<dyn FanChip>, String> {
    open_chip_and_threshold().chip
}

/// Like [`open_chip`], with the GPU's slowdown threshold (issue #130) read from the
/// NVML instance before it is boxed into the composite chip, and the NCT's problem.
fn open_chip_and_threshold() -> ChipOpen {
    let nct = open_nct();
    let problem = nct.as_ref().err().map(chip_problem_from);
    let (gpu, threshold): (Option<Box<dyn GpuFanApi>>, Option<f32>) = match NvmlFans::open() {
        Ok(mut fans) => {
            let threshold = match fans.slowdown_threshold_c() {
                Ok(value) => value,
                Err(e) => {
                    tracing::warn!(error = %e, "the GPU slowdown threshold is unknown; the hot limit falls back");
                    None
                }
            };
            (Some(Box::new(fans)), threshold)
        }
        Err(e) => {
            tracing::warn!(error = %e, "GPU fans unavailable; only the motherboard headers will be driven");
            (None, None)
        }
    };
    let chip: Result<Box<dyn FanChip>, String> = match (nct, gpu) {
        (Err(e), None) => Err(e.to_string()),
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
    };
    ChipOpen { chip, threshold, problem }
}

/// Opens the NCT6687D-R through PawnIO's LpcIO module and checks it with the detect path. Both
/// Super I/O ports are tried, 0x2E first; the errors of both are reported.
fn open_nct() -> Result<Box<dyn FanChip>, ChipOpenError> {
    open_nct_with(&PawnIoOpener { home: pawnio_home() })
}

/// The port loop, driven through an opener. A found chip returns early; otherwise the typed
/// answer is folded from every failure actually seen, with no fixed port count (C7, C8).
fn open_nct_with(opener: &dyn SioOpener) -> Result<Box<dyn FanChip>, ChipOpenError> {
    let mut errors = Vec::new();
    let mut outcomes = Vec::new();
    for (slot, sio) in crate::chip::SIO_PORTS.iter().enumerate() {
        let io = match opener.open(slot as u64, *sio) {
            Ok(io) => io,
            Err(e) => {
                errors.push(format!("{sio:#x}: {e}"));
                outcomes.push(e);
                continue;
            }
        };
        match Nct6687::detect_at(io, MSI_ALT1_MAP, *sio) {
            Ok(chip) => return Ok(Box::new(chip)),
            Err(e) => {
                errors.push(format!("{sio:#x}: {e}"));
                outcomes.push(match chip_error_problem(&e) {
                    ChipProblem::PawnIoUnavailable => ChipOpenError::PawnIoUnavailable(e.to_string()),
                    ChipProblem::ChipNotFound => ChipOpenError::ChipNotFound(e.to_string()),
                });
            }
        }
    }
    // No chip on any port; the typed answer follows R17's rule. The human text keeps each port's
    // detail for the log.
    let problem = chip_problem_from_ports(&outcomes).unwrap_or(ChipProblem::ChipNotFound);
    Err(match problem {
        ChipProblem::PawnIoUnavailable => ChipOpenError::PawnIoUnavailable(errors.join("; ")),
        ChipProblem::ChipNotFound => ChipOpenError::ChipNotFound(errors.join("; ")),
    })
}

/// One chip-detect failure as a typed problem (R17). A port read that failed is a PawnIO
/// transport failure; a chip that answered with no id or a wrong monitor base is no chip.
pub fn chip_error_problem(error: &ChipError) -> ChipProblem {
    match error {
        ChipError::Io(_) => ChipProblem::PawnIoUnavailable,
        ChipError::NotFound { .. }
        | ChipError::WrongBase { .. }
        | ChipError::CommandTimeout(_)
        | ChipError::Readback { .. } => ChipProblem::ChipNotFound,
    }
}

/// Opens every source the service can read. Nothing here panics on a missing driver.
pub fn open() -> Hardware {
    let opened = open_chip_and_threshold();
    let (chip, chip_error) = match opened.chip {
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
    Hardware {
        chip,
        chip_error,
        chip_problem: opened.problem,
        chip_checked: true,
        temps,
        gpu_rated_max_c: opened.threshold,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_errors_map_to_the_two_problems_r17() {
        let io = ChipError::Io(std::io::Error::other("port read failed"));
        assert_eq!(chip_error_problem(&io), ChipProblem::PawnIoUnavailable);
        assert_eq!(
            chip_error_problem(&ChipError::NotFound { port: 0x2E, id: 0x1234 }),
            ChipProblem::ChipNotFound
        );
        assert_eq!(
            chip_error_problem(&ChipError::WrongBase { found: 0xA30, expected: 0xA20 }),
            ChipProblem::ChipNotFound
        );
    }

    #[test]
    fn every_nct_outcome_gives_the_documented_problem_r17() {
        let available = ChipOpenError::PawnIoUnavailable("no driver".into());
        let missing = ChipOpenError::ChipNotFound("no chip".into());
        // Both ports with no PawnIO: unavailable.
        assert_eq!(chip_problem_from_ports(&[available.clone(), available.clone()]), Some(ChipProblem::PawnIoUnavailable));
        // Either port with no PawnIO wins, even when the other found no chip.
        assert_eq!(chip_problem_from_ports(&[missing.clone(), available.clone()]), Some(ChipProblem::PawnIoUnavailable));
        assert_eq!(chip_problem_from_ports(&[available, missing.clone()]), Some(ChipProblem::PawnIoUnavailable));
        // No chip on both ports: chip not found.
        assert_eq!(chip_problem_from_ports(&[missing.clone(), missing.clone()]), Some(ChipProblem::ChipNotFound));
        // The loop folds whatever outcomes it saw, so no fixed array length is assumed (C7).
        assert_eq!(chip_problem_from_ports(&[]), None);
        assert_eq!(
            chip_problem_from_ports(std::slice::from_ref(&missing)),
            Some(ChipProblem::ChipNotFound)
        );
        assert_eq!(
            chip_problem_from_ports(&[missing.clone(), missing.clone(), missing]),
            Some(ChipProblem::ChipNotFound)
        );
    }

    use crate::pawnio::PawnIoError;
    use std::io;

    #[test]
    fn a_pawnio_failure_has_one_prefix_in_logs_and_chip_error() {
        let error = ChipOpenError::from(PawnIoError::Unavailable("LpcIO open failed".into()));
        let text = error.to_string();
        assert!(text.starts_with("PawnIO unavailable:"), "{text}");
        assert_eq!(text.matches("PawnIO").count(), 1, "{text}");
        assert!(!text.contains("unavailable: PawnIO"), "{text}");
    }

    /// A Super I/O port that answers zero to every probe, so the detect path reports NotFound.
    #[derive(Default)]
    struct ZeroPort;

    impl crate::chip::PortIo for ZeroPort {
        fn read_port(&mut self, _port: u16) -> io::Result<u8> {
            Ok(0)
        }
        fn write_port(&mut self, _port: u16, _value: u8) -> io::Result<()> {
            Ok(())
        }
    }

    /// The fake opener drives the same loop as PawnIO without opening a real port (C8).
    struct FakeOpener {
        first_port_unavailable: bool,
    }

    impl SioOpener for FakeOpener {
        fn open(&self, slot: u64, _sio: u16) -> Result<Box<dyn crate::chip::PortIo>, ChipOpenError> {
            if slot == 0 && self.first_port_unavailable {
                return Err(ChipOpenError::PawnIoUnavailable("fake PawnIO failure".into()));
            }
            Ok(Box::new(ZeroPort))
        }
    }

    #[test]
    fn the_port_loop_itself_reports_the_problem_c8() {
        let unavailable = FakeOpener { first_port_unavailable: true };
        let error = open_nct_with(&unavailable).err().expect("no chip on a fake port");
        assert_eq!(chip_problem_from(&error), ChipProblem::PawnIoUnavailable);
        let not_found = FakeOpener { first_port_unavailable: false };
        let error = open_nct_with(&not_found).err().expect("every fake port reports no chip");
        assert_eq!(chip_problem_from(&error), ChipProblem::ChipNotFound);
    }
}
