//! The Nuvoton NCT6687D-R fan and board-sensor driver, written over a byte-level port
//! interface so the same code runs against PawnIO (`pawnio.rs`) and against the register
//! simulation (`fakes.rs`).
//!
//! Register sources (re-implemented from these public documents, not copied):
//! - `nct6687d` out-of-tree Linux driver, `nct6687.c` by Fred78290 (GPL): Super I/O
//!   chip ID at config registers 0x20/0x21 (ID 0xD590, mask 0xFFF0), logical device 0x0B
//!   (hardware monitor), its base address at 0x60/0x61 (`IOREGION_OFFSET 4` gives the EC
//!   access ports), the RPM, PWM, fan-mode (0xA00, bit set = manual) and command (0xA01,
//!   0x80 request / 0x40 done) registers, and temperatures at 0x100 + 2n with the half-degree
//!   bit in bit 7 of the next byte.
//! - LibreHardwareMonitor `Nct677X.cs` (MPL-2.0): EC access for NCT6687 uses a page,
//!   an index and a data port, and the NCT6687 fan-mode register is 0xA00.
//! - PawnIO.Modules `LpcIO.p` (GPL-2): the port-access primitives the PawnIO port adapter uses.
//!
//! Where the public sources disagree or do not reach, the item is marked in the code
//! as UNCONFIRMED. The msi_alt1 table is the one this board needs, and it is UNCONFIRMED
//! until it is checked against LibreHardwareMonitor on the board.

use std::io;

/// Byte-level access to the chip: Super I/O config ports and the EC/hardware-monitor
/// ports. [`crate::pawnio::PawnIoPortIo`] implements it on the real hardware through the
/// PawnIO LpcIO module; [`crate::fakes::FakeChip`] implements it as a register simulation.
pub trait PortIo: Send {
    fn read_port(&mut self, port: u16) -> io::Result<u8>;
    fn write_port(&mut self, port: u16, value: u8) -> io::Result<()>;
    /// Called once the Super I/O config sequence is entered. PawnIO uses it to find the
    /// logical device BARs, so the hardware monitor ports become permitted.
    fn config_mode_entered(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// One fan header on the chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    /// Header id used in profiles and the API, e.g. `cpu_fan`.
    pub id: &'static str,
    pub label: &'static str,
    /// 16-bit big-endian RPM register (high byte first). UNCONFIRMED byte order for NCT6687.
    pub rpm_reg: u16,
    pub pwm_read_reg: Option<u16>,
    pub pwm_write_reg: Option<u16>,
    /// Bit in the fan-mode register (0xA00); set = manual, clear = BIOS/auto.
    pub mode_bit: Option<u8>,
}

impl Channel {
    /// A header this service can drive. Tach-only headers report RPM and nothing else.
    pub const fn controllable(&self) -> bool {
        self.pwm_write_reg.is_some() && self.pwm_read_reg.is_some() && self.mode_bit.is_some()
    }
}

/// A named, swappable channel table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterMap {
    pub name: &'static str,
    /// `false` while the table is not checked against LibreHardwareMonitor on the board.
    pub confirmed: bool,
    pub channels: &'static [Channel],
}

/// Fan-mode register: one bit per fan index, set = manual (nct6687d `NCT6687_REG_FAN_CTRL_MODE`).
pub const MODE_REG: u16 = 0xA00;
/// Fan command register (nct6687d `NCT6687_REG_FAN_PWM_COMMAND`).
pub const COMMAND_REG: u16 = 0xA01;
pub const CMD_REQUEST: u8 = 0x80;
pub const CMD_DONE: u8 = 0x40;
/// Board temperature registers: `0x100 + 2n` (integer), `+1` bit 7 = half degree.
pub const TEMP_BASE_REG: u16 = 0x100;
pub const BOARD_SENSORS: [&str; 7] = [
    "board_temp0",
    "board_temp1",
    "board_temp2",
    "board_temp3",
    "board_temp4",
    "board_temp5",
    "board_temp6",
];

/// Default mapping (nct6687d `reg_rpm`/`reg_pwm`/`reg_pwm_write` defaults). Fan 1 is the CPU
/// fan and fan 2 the pump by that driver's convention.
pub const DEFAULT_MAP: RegisterMap = RegisterMap {
    name: "default",
    confirmed: true,
    channels: &[
        Channel { id: "cpu_fan", label: "CPU fan", rpm_reg: 0x140, pwm_read_reg: Some(0x160), pwm_write_reg: Some(0xA28), mode_bit: Some(0) },
        Channel { id: "pump_fan", label: "Pump fan", rpm_reg: 0x142, pwm_read_reg: Some(0x161), pwm_write_reg: Some(0xA29), mode_bit: Some(1) },
        Channel { id: "sys_fan1", label: "System fan 1", rpm_reg: 0x144, pwm_read_reg: Some(0x162), pwm_write_reg: Some(0xA2A), mode_bit: Some(2) },
        Channel { id: "sys_fan2", label: "System fan 2", rpm_reg: 0x146, pwm_read_reg: Some(0x163), pwm_write_reg: Some(0xA2B), mode_bit: Some(3) },
        Channel { id: "sys_fan3", label: "System fan 3", rpm_reg: 0x148, pwm_read_reg: Some(0x164), pwm_write_reg: Some(0xA2C), mode_bit: Some(4) },
        Channel { id: "sys_fan4", label: "System fan 4", rpm_reg: 0x14A, pwm_read_reg: Some(0x165), pwm_write_reg: Some(0xA2D), mode_bit: Some(5) },
        Channel { id: "sys_fan5", label: "System fan 5", rpm_reg: 0x14C, pwm_read_reg: Some(0x166), pwm_write_reg: Some(0xA2E), mode_bit: Some(6) },
        Channel { id: "sys_fan6", label: "System fan 6", rpm_reg: 0x14E, pwm_read_reg: Some(0x167), pwm_write_reg: Some(0xA2F), mode_bit: Some(7) },
    ],
};

/// The `msi_alt1` mapping that nct6687d applies to MSI boards in the B850 family. UNCONFIRMED:
/// the register addresses are taken from that driver's table and must be checked against
/// LibreHardwareMonitor's readings on this board before any write (spec, fans line 9).
/// The ninth tach-only fan (`pump_fan2`) has RPM only.
pub const MSI_ALT1_MAP: RegisterMap = RegisterMap {
    name: "msi_alt1",
    confirmed: false,
    channels: &[
        Channel { id: "cpu_fan", label: "CPU fan", rpm_reg: 0x140, pwm_read_reg: Some(0x160), pwm_write_reg: Some(0xA28), mode_bit: Some(0) },
        Channel { id: "pump_fan", label: "Pump fan", rpm_reg: 0x142, pwm_read_reg: Some(0x161), pwm_write_reg: Some(0xA29), mode_bit: Some(1) },
        Channel { id: "sys_fan1", label: "System fan 1", rpm_reg: 0x15E, pwm_read_reg: Some(0xE05), pwm_write_reg: Some(0xC70), mode_bit: Some(2) },
        Channel { id: "sys_fan2", label: "System fan 2", rpm_reg: 0x15C, pwm_read_reg: Some(0xE04), pwm_write_reg: Some(0xC58), mode_bit: Some(3) },
        Channel { id: "sys_fan3", label: "System fan 3", rpm_reg: 0x15A, pwm_read_reg: Some(0xE03), pwm_write_reg: Some(0xC40), mode_bit: Some(4) },
        Channel { id: "sys_fan4", label: "System fan 4", rpm_reg: 0x158, pwm_read_reg: Some(0xE02), pwm_write_reg: Some(0xC28), mode_bit: Some(5) },
        Channel { id: "sys_fan5", label: "System fan 5", rpm_reg: 0x156, pwm_read_reg: Some(0xE01), pwm_write_reg: Some(0xC10), mode_bit: Some(6) },
        Channel { id: "sys_fan6", label: "System fan 6", rpm_reg: 0x154, pwm_read_reg: Some(0xE00), pwm_write_reg: Some(0xBF8), mode_bit: Some(7) },
        Channel { id: "pump_fan2", label: "Pump fan 2 (tach only)", rpm_reg: 0x144, pwm_read_reg: None, pwm_write_reg: None, mode_bit: None },
    ],
};

/// Super I/O config port, slot 0 of PawnIO's LpcIO module.
pub const SIO_PORT: u16 = 0x2E;
/// The second Super I/O config port, LpcIO slot 1. Luke's B850 has nothing at 0x2E (LpcIO's
/// `find_bars` read chip id 0x00/0xFF there, lighting session 2), so both are probed, in this
/// order, as LibreHardwareMonitor does.
pub const SIO_PORT_ALT: u16 = 0x4E;
/// The Super I/O ports by LpcIO slot index.
pub const SIO_PORTS: [u16; 2] = [SIO_PORT, SIO_PORT_ALT];
/// Hardware-monitor base the chip must report for this board.
pub const EXPECTED_HWM_BASE: u16 = 0xA20;
const SIO_CHIP_ID_HI: u8 = 0x20;
const SIO_CHIP_ID_LO: u8 = 0x21;
const SIO_LD_SELECT: u8 = 0x07;
const SIO_LD_HWM: u8 = 0x0B;
const SIO_ADDR_HI: u8 = 0x60;
const SIO_ADDR_LO: u8 = 0x61;
const SIO_ENTER_KEY: u8 = 0x87;
const SIO_EXIT_KEY: u8 = 0xAA;
/// Written to the EC page register before the page itself.
const EC_PAGE_SELECT: u8 = 0xFF;
pub const NCT6687_CHIP_ID: u16 = 0xD590;
pub const CHIP_ID_MASK: u16 = 0xFFF0;

#[derive(Debug, thiserror::Error)]
pub enum ChipError {
    #[error("port io: {0}")]
    Io(#[from] io::Error),
    #[error("no NCT6687D-R at Super I/O port {port:#x} (chip id {id:#06x})")]
    NotFound { port: u16, id: u16 },
    #[error("hardware monitor at {found:#X}, expected {expected:#X}")]
    WrongBase { found: u16, expected: u16 },
    #[error("command {0:#04x} was never acknowledged")]
    CommandTimeout(u16),
    #[error("register {reg:#06x} read back {read:#04x}, wrote {wrote:#04x}")]
    Readback { reg: u16, wrote: u8, read: u8 },
}

/// What the service needs from a fan chip, by header index.
pub trait FanChip: Send {
    fn chip_name(&self) -> String;
    /// The channel map in use (`RegisterMap::name`). Checked against `chip.toml`.
    fn map_name(&self) -> &'static str;
    fn channels(&self) -> &'static [Channel];
    fn board_sensor_names(&self) -> &'static [&'static str];
    /// Current RPM, `None` when the chip reports nothing usable.
    fn read_rpm(&mut self, channel: usize) -> io::Result<Option<u32>>;
    /// Duty the chip reports, 0..=100, `None` for tach-only headers.
    fn read_duty_percent(&mut self, channel: usize) -> io::Result<Option<u8>>;
    /// The chip's manual-mode bit for this header.
    fn read_manual(&mut self, channel: usize) -> io::Result<bool>;
    /// Sets or clears manual mode, then reads the bit back and checks it.
    fn set_manual(&mut self, channel: usize, manual: bool) -> io::Result<()>;
    /// Writes a duty and waits for the chip to acknowledge it. Fails for tach-only headers.
    fn write_duty_percent(&mut self, channel: usize, percent: u8) -> io::Result<()>;
    /// Board sensor in degrees C, `None` for an unknown name.
    fn read_board_temp(&mut self, name: &str) -> io::Result<Option<f32>>;
}

fn bad_channel(channel: usize) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("no header index {channel}"))
}

fn sio_write<P: PortIo>(io: &mut P, sio: u16, reg: u8, value: u8) -> io::Result<()> {
    io.write_port(sio, reg)?;
    io.write_port(sio + 1, value)
}

fn sio_read<P: PortIo>(io: &mut P, sio: u16, reg: u8) -> io::Result<u8> {
    io.write_port(sio, reg)?;
    io.read_port(sio + 1)
}

/// The NCT6687D-R driver over a [`PortIo`].
pub struct Nct6687<P: PortIo> {
    io: P,
    base: u16,
    map: RegisterMap,
}

impl<P: PortIo> Nct6687<P> {
    /// Identifies the chip through Super I/O, checks its hardware-monitor base is
    /// [`EXPECTED_HWM_BASE`], and leaves the chip in run mode. Never writes a fan register.
    pub fn detect(io: P, map: RegisterMap) -> Result<Self, ChipError> {
        Self::detect_at(io, map, SIO_PORT)
    }

    /// [`detect`](Self::detect) at the Super I/O config port `sio` (0x2E or 0x4E).
    pub fn detect_at(mut io: P, map: RegisterMap, sio: u16) -> Result<Self, ChipError> {
        io.write_port(sio, SIO_ENTER_KEY)?;
        io.write_port(sio, SIO_ENTER_KEY)?;
        let probed = io
            .config_mode_entered()
            .map_err(ChipError::from)
            .and_then(|()| probe(&mut io, sio));
        // Always leave config mode, even when entering it or the probe failed.
        let exit = io.write_port(sio, SIO_EXIT_KEY);
        let base = probed?;
        exit?;
        Ok(Self { io, base, map })
    }

    /// The hardware-monitor base the chip reported.
    pub fn base(&self) -> u16 {
        self.base
    }

    /// Latches the page and index of `reg` in EC port 1 (base + 4). The page register takes
    /// `EC_PAGE_SELECT` first, then the page (nct6687d `nct6687_read`/`nct6687_write`, and
    /// LibreHardwareMonitor). Without the select the chip never latches the page and every
    /// read returns the same stale byte (lighting session 2: 1028 RPM and 4 C everywhere).
    fn ec_select(&mut self, reg: u16) -> io::Result<()> {
        self.io.write_port(self.base + 4, EC_PAGE_SELECT)?;
        self.io.write_port(self.base + 4, (reg >> 8) as u8)?;
        self.io.write_port(self.base + 5, (reg & 0xFF) as u8)
    }

    fn ec_read(&mut self, reg: u16) -> io::Result<u8> {
        self.ec_select(reg)?;
        self.io.read_port(self.base + 6)
    }

    fn ec_write(&mut self, reg: u16, value: u8) -> io::Result<()> {
        self.ec_select(reg)?;
        self.io.write_port(self.base + 6, value)
    }

    fn channel(&self, channel: usize) -> io::Result<Channel> {
        self.map.channels.get(channel).copied().ok_or_else(|| bad_channel(channel))
    }
}

fn probe<P: PortIo>(io: &mut P, sio: u16) -> Result<u16, ChipError> {
    let id = (u16::from(sio_read(io, sio, SIO_CHIP_ID_HI)?) << 8)
        | u16::from(sio_read(io, sio, SIO_CHIP_ID_LO)?);
    if id & CHIP_ID_MASK != NCT6687_CHIP_ID {
        return Err(ChipError::NotFound { port: sio, id });
    }
    sio_write(io, sio, SIO_LD_SELECT, SIO_LD_HWM)?;
    let base = (u16::from(sio_read(io, sio, SIO_ADDR_HI)?) << 8)
        | u16::from(sio_read(io, sio, SIO_ADDR_LO)?);
    if base != EXPECTED_HWM_BASE {
        return Err(ChipError::WrongBase { found: base, expected: EXPECTED_HWM_BASE });
    }
    Ok(base)
}

impl<P: PortIo> FanChip for Nct6687<P> {
    fn chip_name(&self) -> String {
        format!("NCT6687D-R @ {:#X}", self.base)
    }

    fn map_name(&self) -> &'static str {
        self.map.name
    }

    fn channels(&self) -> &'static [Channel] {
        self.map.channels
    }

    fn board_sensor_names(&self) -> &'static [&'static str] {
        &BOARD_SENSORS
    }

    fn read_rpm(&mut self, channel: usize) -> io::Result<Option<u32>> {
        let c = self.channel(channel)?;
        let hi = self.ec_read(c.rpm_reg)?;
        let lo = self.ec_read(c.rpm_reg + 1)?;
        Ok(Some(u32::from(hi) << 8 | u32::from(lo)))
    }

    fn read_duty_percent(&mut self, channel: usize) -> io::Result<Option<u8>> {
        let c = self.channel(channel)?;
        let Some(reg) = c.pwm_read_reg else { return Ok(None) };
        let raw = self.ec_read(reg)?;
        // 0..=255 raw duty to 0..=100 percent, rounded.
        Ok(Some(((u32::from(raw) * 100 + 127) / 255) as u8))
    }

    fn read_manual(&mut self, channel: usize) -> io::Result<bool> {
        let c = self.channel(channel)?;
        let Some(bit) = c.mode_bit else { return Ok(false) };
        Ok(self.ec_read(MODE_REG)? & (1 << bit) != 0)
    }

    fn set_manual(&mut self, channel: usize, manual: bool) -> io::Result<()> {
        let c = self.channel(channel)?;
        let bit = c.mode_bit.ok_or_else(|| bad_channel(channel))?;
        let current = self.ec_read(MODE_REG)?;
        let wanted = if manual { current | (1 << bit) } else { current & !(1 << bit) };
        self.ec_write(MODE_REG, wanted)?;
        let read = self.ec_read(MODE_REG)?;
        if read != wanted {
            return Err(io::Error::other(ChipError::Readback { reg: MODE_REG, wrote: wanted, read }.to_string()));
        }
        Ok(())
    }

    fn write_duty_percent(&mut self, channel: usize, percent: u8) -> io::Result<()> {
        let c = self.channel(channel)?;
        let reg = c.pwm_write_reg.ok_or_else(|| bad_channel(channel))?;
        let raw = ((u32::from(percent.min(100)) * 255 + 50) / 100) as u8;
        self.ec_write(reg, raw)?;
        // Command handshake from nct6687d: request, then wait for done. UNCONFIRMED ordering on hardware.
        self.ec_write(COMMAND_REG, CMD_REQUEST)?;
        for _ in 0..10 {
            if self.ec_read(COMMAND_REG)? & CMD_DONE != 0 {
                return Ok(());
            }
        }
        Err(io::Error::other(ChipError::CommandTimeout(COMMAND_REG).to_string()))
    }

    fn read_board_temp(&mut self, name: &str) -> io::Result<Option<f32>> {
        let Some(index) = BOARD_SENSORS.iter().position(|s| *s == name) else { return Ok(None) };
        let reg = TEMP_BASE_REG + 2 * index as u16;
        let whole = self.ec_read(reg)?;
        let half = self.ec_read(reg + 1)?;
        Ok(Some(f32::from(whole) + if half & 0x80 != 0 { 0.5 } else { 0.0 }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::FakeChip;

    #[test]
    fn detects_the_chip_at_the_expected_base() {
        let chip = Nct6687::detect(FakeChip::new(MSI_ALT1_MAP), MSI_ALT1_MAP).unwrap();
        assert_eq!(chip.chip_name(), "NCT6687D-R @ 0xA20");
    }

    #[test]
    fn a_chip_at_the_second_port_is_found_there_and_not_at_the_first() {
        let err = Nct6687::detect_at(FakeChip::at_port(MSI_ALT1_MAP, SIO_PORT_ALT), MSI_ALT1_MAP, SIO_PORT)
            .err()
            .unwrap();
        assert!(matches!(err, ChipError::NotFound { port: SIO_PORT, .. }), "{err}");
        let chip =
            Nct6687::detect_at(FakeChip::at_port(MSI_ALT1_MAP, SIO_PORT_ALT), MSI_ALT1_MAP, SIO_PORT_ALT).unwrap();
        assert_eq!(chip.chip_name(), "NCT6687D-R @ 0xA20");
    }

    #[test]
    fn a_different_chip_is_not_found() {
        let err = Nct6687::detect(FakeChip::absent(MSI_ALT1_MAP), MSI_ALT1_MAP).err().unwrap();
        assert!(matches!(err, ChipError::NotFound { .. }), "{err}");
    }

    #[test]
    fn a_wrong_hardware_monitor_base_is_refused() {
        let err = Nct6687::detect(FakeChip::with_base(MSI_ALT1_MAP, 0xA30), MSI_ALT1_MAP).err().unwrap();
        assert!(matches!(err, ChipError::WrongBase { found: 0xA30, .. }), "{err}");
    }

    #[test]
    fn controllable_flags_follow_the_registers() {
        assert!(MSI_ALT1_MAP.channels.last().map(|c| !c.controllable()).unwrap_or(false));
        assert!(DEFAULT_MAP.channels.iter().all(Channel::controllable));
    }
}
