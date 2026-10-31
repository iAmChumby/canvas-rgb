//! Test doubles. Nothing here touches hardware or starts a process.
//!
//! [`FakeChip`] is a register-level simulation of an NCT6687D-R: the Super I/O config
//! registers, the EC page/index/data ports, the fan RPM with a lag, the duty and mode bits,
//! the command handshake, and the board temperature registers. The real driver runs over it.

use crate::chip::{
    Channel, PortIo, RegisterMap, BOARD_SENSORS, CMD_DONE, CMD_REQUEST, COMMAND_REG, ENGINE_STS_REG,
    STS_CHECK_DONE, STS_LOCK, STS_PHASE, TEMP_BASE_REG,
};
use crate::gpu_fans::GpuFanApi;
use crate::process::{ChildProcess, ProcessSpawner};
use crate::temps::TempReader;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

const CHIP_ID_HI: u8 = 0xD5;
const CHIP_ID_LO: u8 = 0x92;
const HWM_BASE: u16 = 0xA20;
/// Duty at power-on, as the BIOS leaves it (raw 0..255).
const BIOS_RAW_DUTY: u8 = 0x66;
/// RPM the simulated fan settles at under BIOS control.
const BIOS_RPM: f32 = 900.0;
/// Time constant of the RPM response, so RPM "follows duty with a lag".
const RPM_TAU_SECS: f32 = 2.0;
const CHANNELS: usize = 9;

struct Sim {
    map: RegisterMap,
    present: bool,
    base: u16,
    /// The Super I/O config port the chip answers on (0x2E or 0x4E).
    sio_port: u16,
    sio_config: bool,
    sio_index: u8,
    sio_ld: u8,
    /// Set by a 0xFF write to the page register; the next write there is the page.
    page_armed: bool,
    page: u8,
    index: u8,
    regs: HashMap<u16, u8>,
    duty_raw: [u8; CHANNELS],
    rpm: [f32; CHANNELS],
    manual: [bool; CHANNELS],
    drop_duty_writes: bool,
    drop_commands: bool,
    /// Firmware owns the fan controller (the resume transition): mode and duty writes are
    /// ignored and the duty reads back as the BIOS value, until [`FakeChip::release_to_service`].
    firmware_owned: bool,
    /// The chip's own clock, moved by tests ([`FakeChip::set_clock`]); firmware ownership set with
    /// [`FakeChip::simulate_resume_for`] ends when this reaches `owned_until`.
    clock: Duration,
    owned_until: Option<Duration>,
    /// The EC's fan configuration window (engine status PHASE set, LOCK clear).
    cfg_open: bool,
    /// The EC checked the last configuration (engine status CHECK_DONE).
    cfg_checked: bool,
    writes: u32,
    write_log: Vec<(u16, u8)>,
}

/// Shared handle to one simulated chip. Clones see the same registers.
#[derive(Clone)]
pub struct FakeChip {
    sim: Arc<Mutex<Sim>>,
}

fn lock(sim: &Mutex<Sim>) -> MutexGuard<'_, Sim> {
    sim.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl FakeChip {
    /// A present NCT6687D-R at the expected base, with every header on BIOS.
    pub fn new(map: RegisterMap) -> Self {
        Self::build(map, true, HWM_BASE)
    }

    /// No chip at this Super I/O port.
    pub fn absent(map: RegisterMap) -> Self {
        Self::build(map, false, HWM_BASE)
    }

    /// A chip whose hardware monitor reports `base` instead of 0xA20.
    pub fn with_base(map: RegisterMap, base: u16) -> Self {
        Self::build(map, true, base)
    }

    /// A present chip that answers on Super I/O port `sio` instead of 0x2E.
    pub fn at_port(map: RegisterMap, sio: u16) -> Self {
        let chip = Self::build(map, true, HWM_BASE);
        lock(&chip.sim).sio_port = sio;
        chip
    }

    fn build(map: RegisterMap, present: bool, base: u16) -> Self {
        let sim = Sim {
            map,
            present,
            base,
            sio_port: 0x2E,
            sio_config: false,
            sio_index: 0,
            sio_ld: 0,
            page_armed: false,
            page: 0,
            index: 0,
            regs: HashMap::new(),
            duty_raw: [BIOS_RAW_DUTY; CHANNELS],
            rpm: [BIOS_RPM; CHANNELS],
            manual: [false; CHANNELS],
            drop_duty_writes: false,
            drop_commands: false,
            firmware_owned: false,
            clock: Duration::ZERO,
            owned_until: None,
            cfg_open: false,
            cfg_checked: false,
            writes: 0,
            write_log: Vec::new(),
        };
        Self { sim: Arc::new(Mutex::new(sim)) }
    }

    /// Simulates a chip that ignores duty writes (readback will not match).
    pub fn set_drop_duty_writes(&self, drop: bool) {
        lock(&self.sim).drop_duty_writes = drop;
    }

    /// Simulates a chip that never acknowledges the command handshake.
    pub fn set_drop_commands(&self, drop: bool) {
        lock(&self.sim).drop_commands = drop;
    }

    /// Simulates a resume (issue #29): every header drops out of manual mode, every duty is back at
    /// the BIOS value, and firmware owns the fan controller (writes ignored, readbacks stuck at the
    /// BIOS duty) until [`FakeChip::release_to_service`]. Tests call that release at the point the
    /// real firmware would be done, because the controller's grace runs on test-supplied `Instant`s
    /// and the chip cannot share that clock.
    pub fn simulate_resume(&self) {
        let mut sim = lock(&self.sim);
        sim.manual = [false; CHANNELS];
        sim.duty_raw = [BIOS_RAW_DUTY; CHANNELS];
        let mode_regs: Vec<u16> = sim.map.channels.iter().map(|c| c.mode_reg).collect();
        for reg in mode_regs {
            sim.regs.insert(reg, 0);
        }
        sim.firmware_owned = true;
    }

    /// Like [`FakeChip::simulate_resume`], but firmware lets go by itself `owned_for` after the chip's
    /// clock (see [`FakeChip::set_clock`]), so a test can check that the controller's own grace is
    /// what decides when writes land.
    pub fn simulate_resume_for(&self, owned_for: Duration) {
        self.simulate_resume();
        let mut sim = lock(&self.sim);
        sim.owned_until = Some(sim.clock + owned_for);
    }

    /// Moves the chip's clock. Tests keep it in step with the `Instant`s they give the controller.
    pub fn set_clock(&self, clock: Duration) {
        lock(&self.sim).clock = clock;
    }

    /// Firmware lets go of the fan controller: writes land again.
    pub fn release_to_service(&self) {
        let mut sim = lock(&self.sim);
        sim.firmware_owned = false;
        sim.owned_until = None;
    }

    /// Sets a board sensor in degrees C (half-degree bit from the fraction).
    pub fn set_board_temp(&self, index: usize, celsius: f32) {
        let mut sim = lock(&self.sim);
        let reg = TEMP_BASE_REG + 2 * index as u16;
        sim.regs.insert(reg, celsius.floor().clamp(0.0, 127.0) as u8);
        sim.regs.insert(reg + 1, if celsius.fract() >= 0.5 { 0x80 } else { 0 });
    }

    /// Advances the simulated fans by `dt`. RPM moves toward the target for the current mode.
    pub fn advance(&self, dt: Duration) {
        let mut sim = lock(&self.sim);
        let k = (dt.as_secs_f32() / RPM_TAU_SECS).min(1.0);
        for ch in 0..CHANNELS {
            let target = if sim.manual[ch] {
                f32::from(sim.duty_raw[ch]) / 255.0 * 2000.0
            } else {
                BIOS_RPM
            };
            sim.rpm[ch] += (target - sim.rpm[ch]) * k;
        }
    }

    /// Current simulated RPM of channel `ch`.
    pub fn rpm(&self, ch: usize) -> u32 {
        lock(&self.sim).rpm.get(ch).map_or(0, |r| r.round() as u32)
    }

    /// Whether the chip's mode bit for `ch` is set (manual).
    pub fn manual(&self, ch: usize) -> bool {
        lock(&self.sim).manual.get(ch).copied().unwrap_or(false)
    }

    /// Raw duty register value of `ch`.
    pub fn duty_raw(&self, ch: usize) -> u8 {
        lock(&self.sim).duty_raw.get(ch).copied().unwrap_or(0)
    }

    /// Number of register writes the chip has accepted through the EC port.
    pub fn writes(&self) -> u32 {
        lock(&self.sim).writes
    }

    /// Every EC register written so far, in order (reads are not recorded).
    pub fn written_regs(&self) -> Vec<u16> {
        lock(&self.sim).write_log.iter().map(|(reg, _)| *reg).collect()
    }

    /// Every EC write so far as (register, value), in order.
    pub fn writes_log(&self) -> Vec<(u16, u8)> {
        lock(&self.sim).write_log.clone()
    }

    /// Current stored value of an EC register (writes only; no read side effects).
    pub fn register(&self, reg: u16) -> u8 {
        lock(&self.sim).regs.get(&reg).copied().unwrap_or(0)
    }

    pub fn map(&self) -> RegisterMap {
        lock(&self.sim).map
    }
}

impl Sim {
    fn channel_of_pwm_write(&self, reg: u16) -> Option<usize> {
        self.map.channels.iter().position(|c: &Channel| c.pwm_write_reg == Some(reg))
    }

    fn channel_of_pwm_read(&self, reg: u16) -> Option<usize> {
        self.map.channels.iter().position(|c: &Channel| c.pwm_read_reg == Some(reg))
    }

    fn channel_of_rpm(&self, reg: u16) -> Option<(usize, bool)> {
        self.map.channels.iter().enumerate().find_map(|(i, c)| {
            if c.rpm_reg == reg {
                Some((i, true))
            } else if c.rpm_reg + 1 == reg {
                Some((i, false))
            } else {
                None
            }
        })
    }

    /// The duty register as the EC answers it: the BIOS value while firmware owns the controller.
    fn duty_value(&self, ch: usize) -> u8 {
        if self.owned() {
            BIOS_RAW_DUTY
        } else {
            self.duty_raw[ch]
        }
    }

    /// Whether firmware owns the fan controller at the chip's current clock.
    fn owned(&self) -> bool {
        self.firmware_owned && self.owned_until.is_none_or(|until| self.clock < until)
    }

    fn ec_read(&self, reg: u16) -> u8 {
        if reg == ENGINE_STS_REG {
            return if self.cfg_open {
                STS_PHASE
            } else if self.cfg_checked {
                STS_LOCK | STS_CHECK_DONE
            } else {
                STS_LOCK
            };
        }
        if let Some(ch) = self.channel_of_pwm_write(reg) {
            return self.duty_value(ch);
        }
        if let Some(ch) = self.channel_of_pwm_read(reg) {
            return self.duty_value(ch);
        }
        if let Some((ch, high)) = self.channel_of_rpm(reg) {
            let value = self.rpm[ch].round().clamp(0.0, 65535.0) as u16;
            return if high { (value >> 8) as u8 } else { (value & 0xFF) as u8 };
        }
        self.regs.get(&reg).copied().unwrap_or(0)
    }

    fn ec_write(&mut self, reg: u16, value: u8) {
        self.writes += 1;
        self.write_log.push((reg, value));
        // Like the real EC: a request opens the configuration window (seen in 0xCF8, never in
        // 0xA01), done closes it, and duty writes land only while it is open.
        if reg == COMMAND_REG {
            if value == CMD_REQUEST && !self.drop_commands {
                self.cfg_open = true;
                self.cfg_checked = false;
            } else if value == CMD_DONE && self.cfg_open {
                self.cfg_open = false;
                self.cfg_checked = true;
            }
            self.regs.insert(COMMAND_REG, value & !CMD_REQUEST);
            return;
        }
        // A fan-mode register (0xA00, or 0x80F for MSI AM5 system fans): each header follows
        // its own bit in its own register.
        if self.map.channels.iter().any(|c| c.mode_reg == reg) {
            if self.owned() {
                return;
            }
            for (ch, c) in self.map.channels.iter().enumerate() {
                if let (true, Some(bit)) = (c.mode_reg == reg, c.mode_bit) {
                    self.manual[ch] = value & (1 << bit) != 0;
                }
            }
            self.regs.insert(reg, value);
            return;
        }
        if let Some(ch) = self.channel_of_pwm_write(reg) {
            if self.cfg_open && !self.drop_duty_writes && !self.owned() {
                self.duty_raw[ch] = value;
            }
            return;
        }
        self.regs.insert(reg, value);
    }
}

impl PortIo for FakeChip {
    fn read_port(&mut self, port: u16) -> io::Result<u8> {
        let sim = lock(&self.sim);
        if port == sim.sio_port + 1 {
            if !sim.present {
                return Ok(0xFF);
            }
            return Ok(match sim.sio_index {
                0x20 => CHIP_ID_HI,
                0x21 => CHIP_ID_LO,
                0x07 => sim.sio_ld,
                0x60 if sim.sio_ld == 0x0B => (sim.base >> 8) as u8,
                0x61 if sim.sio_ld == 0x0B => (sim.base & 0xFF) as u8,
                _ => 0,
            });
        }
        if port == sim.base + 4 {
            return Ok(sim.page);
        }
        if port == sim.base + 5 {
            return Ok(sim.index);
        }
        if port == sim.base + 6 {
            if !sim.present {
                return Ok(0xFF);
            }
            let reg = (u16::from(sim.page) << 8) | u16::from(sim.index);
            return Ok(sim.ec_read(reg));
        }
        Ok(0xFF)
    }

    fn write_port(&mut self, port: u16, value: u8) -> io::Result<()> {
        let mut sim = lock(&self.sim);
        if port == sim.sio_port {
            match value {
                0x87 => sim.sio_config = true,
                0xAA => sim.sio_config = false,
                other => sim.sio_index = other,
            }
            return Ok(());
        }
        if port == sim.sio_port + 1 {
            if sim.sio_config && sim.sio_index == 0x07 {
                sim.sio_ld = value;
            }
            return Ok(());
        }
        let base = sim.base;
        if port == base + 4 {
            // Like the real chip: the page latches only right after the 0xFF page select.
            if value == 0xFF {
                sim.page_armed = true;
            } else if sim.page_armed {
                sim.page = value;
                sim.page_armed = false;
            }
        } else if port == base + 5 {
            sim.index = value;
        } else if port == base + 6 {
            let reg = (u16::from(sim.page) << 8) | u16::from(sim.index);
            sim.ec_write(reg, value);
        }
        Ok(())
    }
}

/// A temperature source whose value the test controls. `None` makes reads fail.
#[derive(Clone)]
pub struct FakeTemp {
    value: Arc<Mutex<Option<f32>>>,
}

impl FakeTemp {
    pub fn new(celsius: f32) -> Self {
        Self { value: Arc::new(Mutex::new(Some(celsius))) }
    }

    pub fn set(&self, celsius: Option<f32>) {
        *self.value.lock().unwrap_or_else(|p| p.into_inner()) = celsius;
    }
}

impl TempReader for FakeTemp {
    fn read_celsius(&mut self) -> io::Result<f32> {
        self.value
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fake sensor is down"))
    }
}

/// What one simulated GPU fan holds.
struct GpuFan {
    target: u32,
    manual: bool,
}

struct GpuSim {
    fans: Vec<GpuFan>,
    fail_set_speed: bool,
    fail_count: bool,
    /// Every `set_speed` call as (fan, percent), including failed ones.
    speed_calls: Vec<(u32, u32)>,
    /// Every `set_default` call, by fan.
    default_calls: Vec<u32>,
}

/// A simulated set of GPU fans behind [`GpuFanApi`]. Like the real card, a speed write makes the
/// fan manual with the new target at once, and a default call returns it to auto at 30%.
/// Clones see the same fans.
#[derive(Clone)]
pub struct FakeGpuFans {
    sim: Arc<Mutex<GpuSim>>,
}

fn gpu_lock(sim: &Mutex<GpuSim>) -> MutexGuard<'_, GpuSim> {
    sim.lock().unwrap_or_else(|p| p.into_inner())
}

impl FakeGpuFans {
    /// `count` fans, all on auto with the 30% idle target.
    pub fn new(count: usize) -> Self {
        let fans = (0..count).map(|_| GpuFan { target: 30, manual: false }).collect();
        let sim = GpuSim { fans, fail_set_speed: false, fail_count: false, speed_calls: Vec::new(), default_calls: Vec::new() };
        Self { sim: Arc::new(Mutex::new(sim)) }
    }

    /// Makes every `set_speed` fail, as a driver that rejects the write.
    pub fn set_fail_set_speed(&self, fail: bool) {
        gpu_lock(&self.sim).fail_set_speed = fail;
    }

    /// Makes `count` fail.
    pub fn set_fail_count(&self, fail: bool) {
        gpu_lock(&self.sim).fail_count = fail;
    }

    pub fn speed_calls(&self) -> Vec<(u32, u32)> {
        gpu_lock(&self.sim).speed_calls.clone()
    }

    pub fn default_calls(&self) -> Vec<u32> {
        gpu_lock(&self.sim).default_calls.clone()
    }

    pub fn clear_calls(&self) {
        let mut sim = gpu_lock(&self.sim);
        sim.speed_calls.clear();
        sim.default_calls.clear();
    }

    pub fn target_of(&self, fan: u32) -> Option<u32> {
        gpu_lock(&self.sim).fans.get(fan as usize).map(|f| f.target)
    }

    pub fn manual_of(&self, fan: u32) -> bool {
        gpu_lock(&self.sim).fans.get(fan as usize).is_some_and(|f| f.manual)
    }
}

fn no_fan(fan: u32) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("no GPU fan {fan}"))
}

impl GpuFanApi for FakeGpuFans {
    fn count(&mut self) -> io::Result<u32> {
        let sim = gpu_lock(&self.sim);
        if sim.fail_count {
            return Err(io::Error::other("fake NVML count failure"));
        }
        Ok(sim.fans.len() as u32)
    }

    fn speed(&mut self, fan: u32) -> io::Result<u32> {
        let sim = gpu_lock(&self.sim);
        let f = sim.fans.get(fan as usize).ok_or_else(|| no_fan(fan))?;
        Ok(if f.manual { f.target } else { 0 })
    }

    fn target(&mut self, fan: u32) -> io::Result<u32> {
        let sim = gpu_lock(&self.sim);
        sim.fans.get(fan as usize).map(|f| f.target).ok_or_else(|| no_fan(fan))
    }

    fn policy_manual(&mut self, fan: u32) -> io::Result<bool> {
        let sim = gpu_lock(&self.sim);
        sim.fans.get(fan as usize).map(|f| f.manual).ok_or_else(|| no_fan(fan))
    }

    fn set_speed(&mut self, fan: u32, percent: u32) -> io::Result<()> {
        let mut sim = gpu_lock(&self.sim);
        sim.speed_calls.push((fan, percent));
        if sim.fail_set_speed {
            return Err(io::Error::other("fake NVML set_speed failure"));
        }
        let f = sim.fans.get_mut(fan as usize).ok_or_else(|| no_fan(fan))?;
        f.target = percent;
        f.manual = true;
        Ok(())
    }

    fn set_default(&mut self, fan: u32) -> io::Result<()> {
        let mut sim = gpu_lock(&self.sim);
        sim.default_calls.push(fan);
        let f = sim.fans.get_mut(fan as usize).ok_or_else(|| no_fan(fan))?;
        f.target = 30;
        f.manual = false;
        Ok(())
    }
}

/// Records spawns and lets a test make a spawned process exit.
/// One recorded spawn: the exe, its arguments, and the flag that makes it report an exit.
type SpawnRecord = (PathBuf, Vec<String>, Arc<AtomicBool>);

#[derive(Clone, Default)]
pub struct FakeProcesses {
    spawns: Arc<Mutex<Vec<SpawnRecord>>>,
    fail_spawns: Arc<AtomicUsize>,
    /// Daemons "already running" before the service starts: (pid, image, session, exited flag).
    running: Arc<Mutex<Vec<RunningRecord>>>,
    /// The console session; `u32::MAX` means none.
    console: Arc<AtomicU32>,
    /// While set, every spawn fails the way a launch does before anyone has logged on.
    no_session: Arc<AtomicBool>,
    /// Sessions whose user has logged off (the console id does not change).
    dead_sessions: Arc<Mutex<Vec<u32>>>,
    /// A killed process needs this many `try_wait` polls before it is gone.
    slow_kill_polls: Arc<AtomicU32>,
    /// Launches that happened while an earlier daemon had not exited yet.
    overlaps: Arc<AtomicU32>,
}

type RunningRecord = (u32, PathBuf, u32, Arc<AtomicBool>);

impl FakeProcesses {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every spawn so far, as (exe, args).
    pub fn spawns(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.spawns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(exe, args, _)| (exe.clone(), args.clone()))
            .collect()
    }

    /// Makes the most recent spawn exit with code 1.
    pub fn exit_last(&self) {
        if let Some((_, _, flag)) = self.spawns.lock().unwrap_or_else(|p| p.into_inner()).last() {
            flag.store(true, Ordering::SeqCst);
        }
    }

    /// Adds a `canvas-rgbd` that was already running, with its image path and session.
    pub fn add_running(&self, pid: u32, image: &str, session: u32) {
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((pid, PathBuf::from(image), session, Arc::new(AtomicBool::new(false))));
    }

    /// Whether the service ended the pre-existing daemon `pid`.
    pub fn running_ended(&self, pid: u32) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .any(|(p, _, _, flag)| *p == pid && flag.load(Ordering::SeqCst))
    }

    pub fn set_console(&self, session: u32) {
        self.console.store(session, Ordering::SeqCst);
    }

    pub fn clear_console(&self) {
        self.console.store(u32::MAX, Ordering::SeqCst);
    }

    /// Makes the next `n` spawns fail with an error.
    pub fn fail_spawns(&self, n: usize) {
        self.fail_spawns.store(n, Ordering::SeqCst);
    }

    /// Marks the user of `session` as logged off while the console id stays what it was.
    pub fn log_off(&self, session: u32) {
        self.dead_sessions.lock().unwrap_or_else(|p| p.into_inner()).push(session);
    }

    /// The user of `session` is logged on again.
    pub fn log_on(&self, session: u32) {
        self.dead_sessions.lock().unwrap_or_else(|p| p.into_inner()).retain(|s| *s != session);
    }

    /// A killed process takes `polls` `try_wait` calls to be gone.
    pub fn set_slow_kill(&self, polls: u32) {
        self.slow_kill_polls.store(polls, Ordering::SeqCst);
    }

    /// How many launches started while an earlier daemon was still running.
    pub fn overlaps(&self) -> u32 {
        self.overlaps.load(Ordering::SeqCst)
    }

    /// While `true`, spawns fail as "no user session yet" (`ErrorKind::NotConnected`).
    pub fn set_no_session(&self, none: bool) {
        self.no_session.store(none, Ordering::SeqCst);
    }

    fn start(&self, exe: &Path, args: &[String]) -> io::Result<Box<dyn ChildProcess>> {
        let console = self.console.load(Ordering::SeqCst);
        if self.dead_sessions.lock().unwrap_or_else(|p| p.into_inner()).contains(&console) {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "nobody is logged on in the console session (fake)"));
        }
        if self.no_session.load(Ordering::SeqCst) {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "no user session yet (fake)"));
        }
        let pending = self.fail_spawns.load(Ordering::SeqCst);
        if pending > 0 {
            self.fail_spawns.store(pending - 1, Ordering::SeqCst);
            return Err(io::Error::other("fake spawn failure"));
        }
        let exited = Arc::new(AtomicBool::new(false));
        let pid = {
            let mut spawns = self.spawns.lock().unwrap_or_else(|p| p.into_inner());
            if spawns.iter().any(|s| !s.2.load(Ordering::SeqCst)) {
                self.overlaps.fetch_add(1, Ordering::SeqCst);
            }
            let pid = 4000 + spawns.len() as u32;
            spawns.push((exe.to_path_buf(), args.to_vec(), Arc::clone(&exited)));
            pid
        };
        Ok(Box::new(FakeChild {
            exited,
            pid,
            slow_kill: self.slow_kill_polls.load(Ordering::SeqCst),
            dying: None,
        }))
    }
}

struct FakeChild {
    exited: Arc<AtomicBool>,
    pid: u32,
    /// Polls a killed process needs before it has exited (0 = at once).
    slow_kill: u32,
    /// Polls left before a killed process exits.
    dying: Option<u32>,
}

impl ChildProcess for FakeChild {
    fn try_wait(&mut self) -> io::Result<Option<i32>> {
        if let Some(left) = self.dying {
            if left == 0 {
                self.exited.store(true, Ordering::SeqCst);
                self.dying = None;
            } else {
                self.dying = Some(left - 1);
            }
        }
        Ok(self.exited.load(Ordering::SeqCst).then_some(1))
    }

    fn kill(&mut self) -> io::Result<()> {
        if self.slow_kill > 0 {
            self.dying = Some(self.slow_kill);
        } else {
            self.exited.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn id(&self) -> u32 {
        self.pid
    }
}

/// [`ProcessSpawner`] over [`FakeProcesses`].
pub struct FakeSpawner(pub FakeProcesses);

impl ProcessSpawner for FakeSpawner {
    fn spawn(&mut self, exe: &Path, args: &[String]) -> io::Result<Box<dyn ChildProcess>> {
        self.0.start(exe, args)
    }
}

/// Session launcher seam for the daemon (see `daemon.rs`), backed by the same fake.
pub struct FakeLauncher(pub FakeProcesses);

impl crate::daemon::SessionLauncher for FakeLauncher {
    fn launch(&mut self, exe: &Path) -> io::Result<Box<dyn ChildProcess>> {
        self.0.start(exe, &[])
    }

    fn running_daemons(&mut self) -> Vec<crate::daemon::RunningDaemon> {
        self.0
            .running
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(pid, image, session, flag)| crate::daemon::RunningDaemon {
                image: image.clone(),
                session: *session,
                process: Box::new(FakeChild { exited: Arc::clone(flag), pid: *pid, slow_kill: 0, dying: None }),
            })
            .collect()
    }

    fn console_session(&mut self) -> Option<u32> {
        match self.0.console.load(Ordering::SeqCst) {
            u32::MAX => None,
            session => Some(session),
        }
    }

    fn session_alive(&mut self, session: u32) -> bool {
        !self.0.dead_sessions.lock().unwrap_or_else(|p| p.into_inner()).contains(&session)
    }
}

/// Board sensor names, for tests that want to name them.
pub fn board_sensor_names() -> &'static [&'static str] {
    &BOARD_SENSORS
}
