//! Read-only probe for the BIOS fan curve stored in the NCT6687D-R's embedded controller.
//!
//! Background: `.build/s2/bios-curve-research.md`. Each fan has a 7-point duty array (stride 2,
//! the low byte holds the duty, the high byte reads 0x00); the temperature points are believed
//! to sit just below it. Nothing here is confirmed on this board. This tool reads the candidate
//! bytes and decodes them so a human can check them against the BIOS screen.
//!
//! Strictly read-only on the chip:
//! - the only chip accesses are the Super I/O detect sequence the service itself runs, and the
//!   EC page/index select plus data read of `Nct6687::read_register` (the driver's own read);
//! - it never writes a fan register, a mode bit, or 0xA01 (the configuration-window command).
//!
//! Refuses to run unless elevated, and unless the `canvas-rgb-svc` service is stopped (the
//! service polls the same EC ports every second; interleaved page selects would corrupt both).
//!
//! `--from-file <dump>` decodes a saved raw dump without touching hardware. `--save <path>`
//! writes the raw dump of a hardware run to a file.

use canvas_proto::consts::SERVICE_NAME;
use canvas_proto::fans::{validate_mode, CurvePoint, FanMode, FanProfile, TempSource, DEFAULT_FLOOR_PERCENT};
use canvas_svc::chip::{Nct6687, MSI_ALT1_MAP, SIO_PORTS};
use canvas_svc::pawnio::{pawnio_home, PawnIoPortIo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::process::ExitCode;

/// Points per curve.
const POINTS: usize = 7;
/// Bytes of the duty array (7 points, stride 2).
const DUTY_SPAN: u16 = 14;
/// Bytes read below the duty array for the temperature candidates.
const TEMP_SPAN: u16 = 14;
/// One captured window: the temperature candidate range, then the duty array, contiguous.
const WINDOW: u16 = TEMP_SPAN + DUTY_SPAN;
/// A board temperature register (`board_temp0`), read between isolated reads so the EC latch
/// is never left on a neighbouring address.
const NEUTRAL_REG: u16 = 0x100;
/// Mode registers, read for the baseline.
const MODE_A00: u16 = 0xA00;
const MODE_80F: u16 = 0x80F;
/// Fan engine status (read only here): shows whether the fan registers are locked.
const ENGINE_STS: u16 = 0xCF8;

/// One fan's candidate curve.
#[derive(Debug, Clone, Copy)]
struct Fan {
    id: &'static str,
    /// First byte of the 7-point duty array.
    duty_base: u16,
    /// Where the base comes from.
    basis: &'static str,
}

const FANS: [Fan; 8] = [
    Fan { id: "cpu_fan", duty_base: 0xB08, basis: "LHM #1513, one user's confirmation" },
    Fan { id: "pump_fan", duty_base: 0xB20, basis: "LHM #1513, explicitly unconfirmed" },
    Fan { id: "sys_fan1", duty_base: 0xC70, basis: "nct6687d msi_alt1 table; community decode" },
    Fan { id: "sys_fan2", duty_base: 0xC58, basis: "nct6687d msi_alt1 table" },
    Fan { id: "sys_fan3", duty_base: 0xC40, basis: "nct6687d msi_alt1 table" },
    Fan { id: "sys_fan4", duty_base: 0xC28, basis: "nct6687d msi_alt1 table" },
    Fan { id: "sys_fan5", duty_base: 0xC10, basis: "nct6687d msi_alt1 table" },
    Fan { id: "sys_fan6", duty_base: 0xBF8, basis: "nct6687d msi_alt1 table" },
];

/// The two fans read first; if both look like garbage the probe stops.
const PRIORITY: [&str; 2] = ["cpu_fan", "sys_fan3"];

// ---------------------------------------------------------------------------------------------
// Capture: raw bytes as read (or as loaded from a dump)
// ---------------------------------------------------------------------------------------------

/// A run of consecutive bytes starting at `start`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pass {
    start: u16,
    bytes: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Capture {
    /// Isolated per-byte reads (descending, each preceded by a neutral read).
    iso: Vec<Pass>,
    /// Ascending sweeps.
    sweep: Vec<Pass>,
    /// Single baseline registers.
    regs: Vec<(u16, u8)>,
}

impl Capture {
    fn reg(&self, reg: u16) -> Option<u8> {
        self.regs.iter().find(|(r, _)| *r == reg).map(|(_, v)| *v)
    }
}

/// The bytes of one fan's window out of a pass: duty always, temperatures when the pass starts
/// early enough to include them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Window {
    duty: [u8; DUTY_SPAN as usize],
    temps: Option<[u8; TEMP_SPAN as usize]>,
}

fn extract(passes: &[Pass], fan: &Fan) -> Option<Window> {
    let base = usize::from(fan.duty_base);
    passes.iter().find_map(|p| {
        let start = usize::from(p.start);
        let duty_at = base.checked_sub(start)?;
        let duty: [u8; DUTY_SPAN as usize] = p.bytes.get(duty_at..duty_at + usize::from(DUTY_SPAN))?.try_into().ok()?;
        let temps = duty_at
            .checked_sub(usize::from(TEMP_SPAN))
            .and_then(|from| p.bytes.get(from..duty_at))
            .and_then(|t| <[u8; TEMP_SPAN as usize]>::try_from(t).ok());
        Some(Window { duty, temps })
    })
}

/// Isolated reads: each byte on its own, in descending order, with a neutral-register read
/// before it, so no read sits next to its neighbour in address order.
fn read_isolated(read: &mut dyn FnMut(u16) -> io::Result<u8>, start: u16, len: u16) -> io::Result<Pass> {
    let mut bytes = vec![0u8; usize::from(len)];
    for i in (0..len).rev() {
        read(NEUTRAL_REG)?;
        bytes[usize::from(i)] = read(start + i)?;
    }
    Ok(Pass { start, bytes })
}

/// Ascending sweep: consecutive addresses, no other register between them.
fn read_sweep(read: &mut dyn FnMut(u16) -> io::Result<u8>, start: u16, len: u16) -> io::Result<Pass> {
    let mut bytes = Vec::with_capacity(usize::from(len));
    for i in 0..len {
        bytes.push(read(start + i)?);
    }
    Ok(Pass { start, bytes })
}

fn capture_fan(read: &mut dyn FnMut(u16) -> io::Result<u8>, fan: &Fan, cap: &mut Capture) -> io::Result<()> {
    let start = fan.duty_base - TEMP_SPAN;
    cap.iso.push(read_isolated(read, start, WINDOW)?);
    cap.sweep.push(read_sweep(read, start, WINDOW)?);
    Ok(())
}

/// A window whose every byte is the same value: the stale-latch symptom, or an EC that hides
/// the region. Not a curve.
fn looks_like_garbage(bytes: &[u8]) -> bool {
    bytes.windows(2).all(|w| w[0] == w[1])
}

// ---------------------------------------------------------------------------------------------
// Dump text: render and parse
// ---------------------------------------------------------------------------------------------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// Raw dump lines: `iso|sweep 0xSTART hh hh ...` and `reg 0xADDR hh`. Lines starting with `#`
/// and anything else are ignored on parse, so a whole saved run can be fed back.
fn dump_lines(cap: &Capture) -> String {
    let mut out = String::new();
    for (name, passes) in [("iso", &cap.iso), ("sweep", &cap.sweep)] {
        for p in passes {
            out.push_str(&format!("{name} {:#06X} {}\n", p.start, hex(&p.bytes)));
        }
    }
    for (reg, v) in &cap.regs {
        out.push_str(&format!("reg {reg:#06X} {v:02X}\n"));
    }
    out
}

fn parse_hex_u16(token: &str) -> Option<u16> {
    let t = token.trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(t, 16).ok()
}

fn parse_dump(text: &str) -> Result<Capture, String> {
    let mut cap = Capture::default();
    for (n, line) in text.lines().enumerate() {
        let mut tokens = line.split_whitespace();
        let Some(kind) = tokens.next() else { continue };
        if !matches!(kind, "iso" | "sweep" | "reg") {
            continue;
        }
        let at = |what: &str| format!("line {}: {what}", n + 1);
        let addr = tokens.next().and_then(parse_hex_u16).ok_or_else(|| at("missing or bad address"))?;
        let bytes: Vec<u8> = tokens
            .map(|t| u8::from_str_radix(t.trim_start_matches("0x"), 16).map_err(|_| at(&format!("bad byte {t:?}"))))
            .collect::<Result<_, _>>()?;
        match kind {
            "reg" => {
                if bytes.len() != 1 {
                    return Err(at("a reg line carries exactly one byte"));
                }
                cap.regs.push((addr, bytes[0]));
            }
            _ => {
                if bytes.is_empty() {
                    return Err(at("no bytes"));
                }
                let pass = Pass { start: addr, bytes };
                if kind == "iso" {
                    cap.iso.push(pass);
                } else {
                    cap.sweep.push(pass);
                }
            }
        }
    }
    Ok(cap)
}

// ---------------------------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------------------------

/// One reading of where the temperature points are.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TempLayout {
    name: &'static str,
    /// Where it comes from, relative to the duty base.
    place: &'static str,
    temps: [u8; POINTS],
    /// Non-decreasing.
    monotonic: bool,
    /// Strictly increasing (what a canvas-rgb curve needs).
    strict: bool,
    /// Every value in 20..=100.
    in_range: bool,
}

impl TempLayout {
    fn new(name: &'static str, place: &'static str, temps: [u8; POINTS]) -> Self {
        Self {
            name,
            place,
            monotonic: temps.windows(2).all(|w| w[0] <= w[1]),
            strict: temps.windows(2).all(|w| w[0] < w[1]),
            in_range: temps.iter().all(|t| (20..=100).contains(t)),
            temps,
        }
    }

    fn plausible(&self) -> bool {
        self.strict && self.in_range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Decoded {
    fan: &'static str,
    base: u16,
    /// Duty low bytes, raw 0..=255.
    duty: [u8; POINTS],
    /// Duty high bytes (odd addresses); the research says these read 0x00.
    duty_high: [u8; POINTS],
    /// Present only when the capture reached below the duty array.
    layouts: Vec<TempLayout>,
    duty_monotonic: bool,
    duty_high_zero: bool,
    /// `None` when only one of the two passes exists.
    iso_eq_sweep: Option<bool>,
    garbage: bool,
}

fn percent_f(raw: u8) -> f32 {
    f32::from(raw) * 100.0 / 255.0
}

/// The same rounding as `FanChip::read_duty_percent`.
fn percent_u8(raw: u8) -> u8 {
    ((u32::from(raw) * 100 + 127) / 255) as u8
}

fn decode(fan: &Fan, window: &Window, iso_eq_sweep: Option<bool>) -> Decoded {
    let mut duty = [0u8; POINTS];
    let mut duty_high = [0u8; POINTS];
    for i in 0..POINTS {
        duty[i] = window.duty[2 * i];
        duty_high[i] = window.duty[2 * i + 1];
    }
    let mut layouts = Vec::new();
    if let Some(t) = &window.temps {
        // t[j] is the byte at duty_base - 14 + j.
        let mut a = [0u8; POINTS]; // consecutive bytes, base-8 .. base-2 (ASRock NCT6686D layout)
        let mut b = [0u8; POINTS]; // stride 2, low bytes, base-14 .. base-2
        for i in 0..POINTS {
            a[i] = t[6 + i];
            b[i] = t[2 * i];
        }
        layouts.push(TempLayout::new("A", "consecutive bytes at base-8 .. base-2", a));
        layouts.push(TempLayout::new("B", "stride 2, low bytes, at base-14 .. base-2", b));
    }
    Decoded {
        fan: fan.id,
        base: fan.duty_base,
        duty_monotonic: duty.windows(2).all(|w| w[0] <= w[1]),
        duty_high_zero: duty_high.iter().all(|&h| h == 0),
        garbage: looks_like_garbage(&window.duty) && window.temps.as_ref().is_none_or(|t| looks_like_garbage(t)),
        duty,
        duty_high,
        layouts,
        iso_eq_sweep,
    }
}

/// Decodes every fan the capture covers. The sweep is preferred; the isolated pass is used when
/// it is the only one.
fn decode_all(cap: &Capture) -> Vec<Decoded> {
    FANS.iter()
        .filter_map(|fan| {
            let sweep = extract(&cap.sweep, fan);
            let iso = extract(&cap.iso, fan);
            let eq = match (&sweep, &iso) {
                (Some(s), Some(i)) => Some(s == i),
                _ => None,
            };
            let window = sweep.or(iso)?;
            Some(decode(fan, &window, eq))
        })
        .collect()
}

/// The curve points of a decode that passes every sanity check, with the layout used.
fn curve_points(d: &Decoded) -> Result<(&TempLayout, Vec<CurvePoint>), String> {
    if d.garbage {
        return Err("the bytes look like garbage (all one value)".into());
    }
    if !d.duty_monotonic {
        return Err("duty is not monotonic".into());
    }
    if d.iso_eq_sweep == Some(false) {
        return Err("the isolated and sweep reads disagree".into());
    }
    if d.layouts.is_empty() {
        return Err("no temperature bytes in the capture".into());
    }
    let layout = d
        .layouts
        .iter()
        .find(|l| l.plausible())
        .ok_or("no temperature layout is strictly increasing and inside 20..100 C")?;
    let points = (0..POINTS)
        .map(|i| CurvePoint { temp_c: f32::from(layout.temps[i]), percent: percent_u8(d.duty[i]) })
        .collect();
    Ok((layout, points))
}

#[derive(Serialize, Deserialize)]
struct Snippet {
    profiles: Vec<FanProfile>,
}

/// The BIOS-mirror profile and notes, for every fan whose decode passes. Each fan that fails
/// gets a line saying why.
fn build_snippet(decoded: &[Decoded]) -> (Option<String>, Vec<String>) {
    let mut headers = BTreeMap::new();
    let mut notes = Vec::new();
    for d in decoded {
        match curve_points(d) {
            Ok((layout, points)) => {
                let mode = FanMode::Curve { source: TempSource::Cpu, points };
                if let Err(e) = validate_mode(d.fan, &mode, DEFAULT_FLOOR_PERCENT) {
                    notes.push(format!("{}: the service would reject this curve as it stands ({e}); lift the low points before pasting", d.fan));
                }
                let source_note = if d.fan == "cpu_fan" {
                    "source = Cpu (the CPU fan's BIOS source is assumed to be the CPU)"
                } else {
                    "source = Cpu is a PLACEHOLDER: the BIOS temperature source for this header is UNKNOWN"
                };
                notes.push(format!("{}: temperature layout {} ({}); {source_note}", d.fan, layout.name, layout.place));
                headers.insert(d.fan.to_string(), mode);
            }
            Err(why) => notes.push(format!("{}: no snippet, {why}", d.fan)),
        }
    }
    if headers.is_empty() {
        return (None, notes);
    }
    let snippet = Snippet { profiles: vec![FanProfile { name: "BIOS mirror".into(), headers }] };
    let text = toml::to_string_pretty(&snippet).unwrap_or_else(|e| format!("# could not encode the snippet: {e}\n"));
    (Some(text), notes)
}

// ---------------------------------------------------------------------------------------------
// Printing
// ---------------------------------------------------------------------------------------------

fn yn(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "NO"
    }
}

fn yn_opt(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "yes",
        Some(false) => "NO",
        None => "n/a (one pass only)",
    }
}

fn print_baseline(cap: &Capture) {
    println!("== Baseline (live, one sample) ==");
    let show = |reg: u16| cap.reg(reg).map_or("--".to_string(), |v| format!("{v:02X}"));
    println!("0xA00 mode  = {} (bit set = manual; cpu_fan bit 0, pump bit 1, driver sys_fan bits 2..7)", show(MODE_A00));
    println!("0x80F mode  = {} (bit set = manual; sys_fan1 bit 7 .. sys_fan6 bit 2)", show(MODE_80F));
    println!("0xCF8 sts   = {} (bit 6 = fan registers locked, bit 3 = configuration phase)", show(ENGINE_STS));
    for c in MSI_ALT1_MAP.channels {
        let rpm = match (cap.reg(c.rpm_reg), cap.reg(c.rpm_reg + 1)) {
            (Some(h), Some(l)) => format!("{} rpm", u32::from(h) << 8 | u32::from(l)),
            _ => "-- rpm".into(),
        };
        let duty = c
            .pwm_read_reg
            .and_then(|r| cap.reg(r))
            .map_or("--".to_string(), |v| format!("{:.1}% (raw {v})", percent_f(v)));
        let manual = c
            .mode_bit
            .and_then(|b| cap.reg(c.mode_reg).map(|m| m & (1 << b) != 0))
            .map_or("--", |m| if m { "manual" } else { "BIOS" });
        println!("{:<10} {rpm:<10} duty {duty:<16} {manual}", c.id);
    }
    println!();
}

fn print_decoded(d: &Decoded) {
    let fan = FANS.iter().find(|f| f.id == d.fan);
    println!("== {} (duty array {:#05X}..{:#05X}; {}) ==", d.fan, d.base, d.base + DUTY_SPAN - 2, fan.map_or("", |f| f.basis));
    let (a, b) = (d.layouts.first(), d.layouts.get(1));
    println!("pt | temp A {:>6} | temp B {:>6} | duty hi:lo | duty raw | duty %", "", "");
    for i in 0..POINTS {
        let cell = |l: Option<&TempLayout>| l.map_or("  --".to_string(), |l| format!("{:>3} C", l.temps[i]));
        println!(
            "{i:>2} | {:>13} | {:>13} | {:02X}:{:02X}      | {:>8} | {:>5.1}",
            cell(a),
            cell(b),
            d.duty_high[i],
            d.duty[i],
            d.duty[i],
            percent_f(d.duty[i]),
        );
    }
    println!("duty monotonic (non-decreasing)?   {}", yn(d.duty_monotonic));
    println!("duty high bytes all 0x00?          {}", yn(d.duty_high_zero));
    for l in &d.layouts {
        println!(
            "temp {} ({}) monotonic? {}   strictly increasing? {}   all in 20..100 C? {}",
            l.name,
            l.place,
            yn(l.monotonic),
            yn(l.strict),
            yn(l.in_range)
        );
    }
    if d.layouts.is_empty() {
        println!("temperatures: not in the capture");
    }
    println!("isolated == sweep?                 {}", yn_opt(d.iso_eq_sweep));
    if d.garbage {
        println!("GARBAGE: every byte of the window is the same value. This is not a curve.");
    }
    println!();
}

fn print_report(cap: &Capture, raw: bool) {
    if raw {
        println!("== Raw dump (feed back with --from-file) ==");
        print!("{}", dump_lines(cap));
        println!();
    }
    print_baseline(cap);
    let decoded = decode_all(cap);
    for d in &decoded {
        print_decoded(d);
    }
    let (snippet, notes) = build_snippet(&decoded);
    println!("== canvas-rgb FanProfile snippet ==");
    for n in &notes {
        println!("# {n}");
    }
    match snippet {
        Some(text) => {
            println!("# Paste under the profiles of %ProgramData%\\canvas-rgb\\fans.toml (or import it as a profile).");
            println!("{text}");
        }
        None => println!("# No fan passed every sanity check, so no snippet is printed."),
    }
}

// ---------------------------------------------------------------------------------------------
// Hardware
// ---------------------------------------------------------------------------------------------

#[cfg(windows)]
fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle from GetCurrentProcess needs no close; `token` is a valid out
    // pointer and is closed below; `elevation` and `len` are valid for the call.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
fn is_elevated() -> bool {
    false
}

#[derive(Debug, PartialEq, Eq)]
enum ServiceState {
    Stopped,
    NotInstalled,
    /// Running, pending, or anything else: not safe.
    Busy(String),
    /// `sc` could not be run or its output was not understood: UNKNOWN, so refuse.
    Unknown(String),
}

/// Classifies `sc query` output and exit code.
fn classify_sc(exit_code: Option<i32>, stdout: &str) -> ServiceState {
    if exit_code == Some(1060) {
        return ServiceState::NotInstalled;
    }
    let state = stdout.lines().find(|l| l.trim_start().starts_with("STATE"));
    match state {
        Some(l) if l.contains("STOPPED") => ServiceState::Stopped,
        Some(l) => ServiceState::Busy(l.split_once(':').map_or(l, |(_, r)| r).trim().to_string()),
        None => ServiceState::Unknown(format!("exit {exit_code:?}, output {stdout:?}")),
    }
}

fn service_state() -> ServiceState {
    match std::process::Command::new("sc").args(["query", SERVICE_NAME]).output() {
        Ok(out) => classify_sc(out.status.code(), &String::from_utf8_lossy(&out.stdout)),
        Err(e) => ServiceState::Unknown(format!("could not run sc: {e}")),
    }
}

fn open_nct() -> Result<Nct6687<PawnIoPortIo>, String> {
    let home = pawnio_home();
    let mut errors = Vec::new();
    for (slot, sio) in SIO_PORTS.iter().enumerate() {
        let io = match PawnIoPortIo::open(&home, slot as u64) {
            Ok(io) => io,
            Err(e) => {
                errors.push(format!("{sio:#x}: PawnIO LpcIO: {e}"));
                continue;
            }
        };
        match Nct6687::detect_at(io, MSI_ALT1_MAP, *sio) {
            Ok(chip) => return Ok(chip),
            Err(e) => errors.push(format!("{sio:#x}: {e}")),
        }
    }
    Err(errors.join("; "))
}

fn read_baseline(read: &mut dyn FnMut(u16) -> io::Result<u8>, cap: &mut Capture) -> io::Result<()> {
    let mut regs = vec![MODE_A00, MODE_80F, ENGINE_STS];
    for c in MSI_ALT1_MAP.channels {
        regs.push(c.rpm_reg);
        regs.push(c.rpm_reg + 1);
        regs.extend(c.pwm_read_reg);
    }
    regs.sort_unstable();
    regs.dedup();
    for reg in regs {
        cap.regs.push((reg, read(reg)?));
    }
    Ok(())
}

fn refuse(why: &str) -> ExitCode {
    eprintln!("bios_curve_probe: refusing to run: {why}");
    ExitCode::from(2)
}

fn hardware(save: Option<&str>) -> Result<ExitCode, String> {
    if !is_elevated() {
        return Ok(refuse("this process is not elevated. Run it from an elevated (Run as administrator) prompt."));
    }
    match service_state() {
        ServiceState::Stopped => {}
        ServiceState::NotInstalled => println!("note: {SERVICE_NAME} is not installed, so nothing else polls the EC ports."),
        ServiceState::Busy(state) => {
            return Ok(refuse(&format!(
                "{SERVICE_NAME} is {state}. It polls the same EC ports every second, so interleaved page selects would corrupt both.\n\
                 Stop it first (the fans go to BIOS control on stop, which is what this read wants):\n    sc stop {SERVICE_NAME}\n\
                 Wait until `sc query {SERVICE_NAME}` shows STOPPED, run this probe, then start it again:\n    sc start {SERVICE_NAME}"
            )));
        }
        ServiceState::Unknown(detail) => {
            return Ok(refuse(&format!(
                "could not tell whether {SERVICE_NAME} is stopped ({detail}). Check `sc query {SERVICE_NAME}`, stop it with `sc stop {SERVICE_NAME}` if it runs, and start it again afterwards with `sc start {SERVICE_NAME}`."
            )));
        }
    }
    println!("Other EC readers (LibreHardwareMonitor, SignalRGB, MSI Center) would also race this probe; close them if they run.");
    let mut chip = open_nct()?;
    println!("chip: {}", canvas_svc::chip::FanChip::chip_name(&chip));
    println!("Restart the service afterwards: sc start {SERVICE_NAME}\n");

    let mut read = |reg: u16| chip.read_register(reg);
    let mut cap = Capture::default();
    read_baseline(&mut read, &mut cap).map_err(|e| format!("baseline read: {e}"))?;

    // The two fans that matter first. Both garbage means plain reads hide the region.
    for id in PRIORITY {
        let fan = FANS.iter().find(|f| f.id == id).ok_or("fan table")?;
        capture_fan(&mut read, fan, &mut cap).map_err(|e| format!("{id}: {e}"))?;
    }
    let garbage = |cap: &Capture| {
        PRIORITY.iter().all(|id| {
            FANS.iter().find(|f| f.id == *id).is_some_and(|f| {
                [&cap.iso, &cap.sweep]
                    .iter()
                    .filter_map(|p| extract(p, f))
                    .all(|w| looks_like_garbage(&w.duty) && w.temps.as_ref().is_none_or(|t| looks_like_garbage(t)))
            })
        })
    };
    if garbage(&cap) {
        print_report(&cap, true);
        save_dump(save, &cap);
        println!(
            "STOPPED: plain reads of cpu_fan and sys_fan3 return one repeated value. The EC may hide the curve outside the\n\
             fan-configuration window, but opening it needs a write to 0xA01, which this probe never does. Nothing more was read."
        );
        return Ok(ExitCode::from(3));
    }
    for fan in FANS.iter().filter(|f| !PRIORITY.contains(&f.id)) {
        capture_fan(&mut read, fan, &mut cap).map_err(|e| format!("{}: {e}", fan.id))?;
    }
    print_report(&cap, true);
    save_dump(save, &cap);
    println!("Done. Start the service again: sc start {SERVICE_NAME}");
    Ok(ExitCode::SUCCESS)
}

fn save_dump(save: Option<&str>, cap: &Capture) {
    if let Some(path) = save {
        let text = format!("# bios_curve_probe raw dump\n{}", dump_lines(cap));
        match std::fs::write(path, text) {
            Ok(()) => println!("raw dump saved to {path}"),
            Err(e) => eprintln!("could not save the raw dump to {path}: {e}"),
        }
    }
}

const USAGE: &str = "bios_curve_probe (read-only)\n\
  bios_curve_probe [--save <dump.txt>]     read the chip (elevated, service stopped)\n\
  bios_curve_probe --from-file <dump.txt>  decode a saved dump, no hardware";

fn run(args: &[String]) -> Result<ExitCode, String> {
    let mut from_file = None;
    let mut save = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from-file" => from_file = Some(it.next().ok_or("--from-file needs a path")?.as_str()),
            "--save" => save = Some(it.next().ok_or("--save needs a path")?.as_str()),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
        }
    }
    if let Some(path) = from_file {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let cap = parse_dump(&text)?;
        if cap.iso.is_empty() && cap.sweep.is_empty() {
            return Err(format!("{path}: no iso or sweep lines found"));
        }
        print_report(&cap, false);
        return Ok(ExitCode::SUCCESS);
    }
    hardware(save)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("bios_curve_probe: {e}");
            ExitCode::from(1)
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn fan(id: &str) -> Fan {
        *FANS.iter().find(|f| f.id == id).unwrap()
    }

    /// 0xC70..0xC7C from the community decode in LHM #1513.
    const COMMUNITY: [u8; 14] = [0x66, 0x00, 0x84, 0x00, 0xBF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00];

    fn window_with(temps: Option<[u8; 14]>, duty: [u8; 14]) -> Window {
        Window { duty, temps }
    }

    #[test]
    fn the_community_example_decodes_to_the_msi_center_curve() {
        // The research text lists 13 bytes (the last high byte is cut); the 14th reads 00.
        let d = decode(&fan("sys_fan1"), &window_with(None, COMMUNITY), None);
        assert_eq!(d.duty, [0x66, 0x84, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF]);
        let pct: Vec<u8> = d.duty.iter().map(|&r| percent_u8(r)).collect();
        assert_eq!(pct, [40, 52, 75, 100, 100, 100, 100]);
        assert!(d.duty_monotonic && d.duty_high_zero && !d.garbage);
        assert_eq!(d.iso_eq_sweep, None);
        assert!(d.layouts.is_empty());
        // No temperatures in the capture, so no snippet can be built.
        assert!(curve_points(&d).unwrap_err().contains("no temperature"));
        assert!((percent_f(0x66) - 40.0).abs() < 0.01);
    }

    /// Temperatures at base-8 .. base-2 (layout A), then the community duty array.
    fn layout_a_window(temps: [u8; 7]) -> Window {
        let mut t = [0u8; 14];
        t[6..13].copy_from_slice(&temps);
        window_with(Some(t), COMMUNITY)
    }

    #[test]
    fn layout_a_temperatures_build_a_valid_snippet() {
        let w = layout_a_window([30, 40, 50, 60, 70, 80, 90]);
        let d = decode(&fan("cpu_fan"), &w, Some(true));
        assert!(d.layouts[0].plausible());
        let (layout, points) = curve_points(&d).unwrap();
        assert_eq!(layout.name, "A");
        assert_eq!(points.len(), 7);
        assert_eq!(points[0], CurvePoint { temp_c: 30.0, percent: 40 });
        assert_eq!(points[6], CurvePoint { temp_c: 90.0, percent: 100 });
        let (snippet, notes) = build_snippet(&[d]);
        let text = snippet.expect("a snippet");
        assert!(notes.iter().any(|n| n.starts_with("cpu_fan: temperature layout A")), "{notes:?}");
        // The snippet parses back into the same profile.
        let back: Snippet = toml::from_str(&text).unwrap();
        assert_eq!(back.profiles.len(), 1);
        assert_eq!(back.profiles[0].name, "BIOS mirror");
        match &back.profiles[0].headers["cpu_fan"] {
            FanMode::Curve { source, points: p } => {
                assert_eq!(*source, TempSource::Cpu);
                assert_eq!(p, &points);
            }
            other => panic!("not a curve: {other:?}"),
        }
        println!("{text}");
    }

    #[test]
    fn layout_b_is_used_when_a_fails() {
        // Stride-2 temperatures at base-14 .. base-2; the bytes between hold 0 (outside 20..100).
        let mut t = [0u8; 14];
        for (i, v) in [35u8, 45, 55, 65, 75, 85, 95].iter().enumerate() {
            t[2 * i] = *v;
        }
        let d = decode(&fan("sys_fan3"), &window_with(Some(t), COMMUNITY), None);
        assert!(!d.layouts[0].plausible());
        let (layout, points) = curve_points(&d).unwrap();
        assert_eq!(layout.name, "B");
        assert_eq!(points[3].temp_c, 65.0);
        let (_, notes) = build_snippet(&[d]);
        assert!(notes.iter().any(|n| n.contains("PLACEHOLDER")), "sys_fan3 must flag its unknown source: {notes:?}");
    }

    #[test]
    fn insane_decodes_are_refused() {
        // Duty not monotonic.
        let mut duty = COMMUNITY;
        duty[4] = 0x10;
        let w = Window { duty, temps: layout_a_window([30, 40, 50, 60, 70, 80, 90]).temps };
        let d = decode(&fan("cpu_fan"), &w, None);
        assert!(!d.duty_monotonic);
        assert!(curve_points(&d).unwrap_err().contains("monotonic"));
        // Temperatures out of range.
        let d = decode(&fan("cpu_fan"), &layout_a_window([5, 40, 50, 60, 70, 80, 90]), None);
        assert!(!d.layouts[0].in_range && d.layouts[0].monotonic);
        assert!(curve_points(&d).is_err());
        // Repeated temperature (100, 100): monotonic but not strictly increasing, so no snippet.
        let d = decode(&fan("cpu_fan"), &layout_a_window([30, 40, 50, 60, 70, 100, 100]), None);
        assert!(d.layouts[0].monotonic && !d.layouts[0].strict);
        assert!(curve_points(&d).is_err());
        // Isolated and sweep disagree.
        let d = decode(&fan("cpu_fan"), &layout_a_window([30, 40, 50, 60, 70, 80, 90]), Some(false));
        assert!(curve_points(&d).unwrap_err().contains("disagree"));
        // A constant window is garbage.
        let d = decode(&fan("cpu_fan"), &window_with(Some([0x80; 14]), [0x80; 14]), Some(true));
        assert!(d.garbage);
        assert!(curve_points(&d).unwrap_err().contains("garbage"));
    }

    #[test]
    fn a_dump_round_trips_and_decodes_by_address() {
        let f = fan("sys_fan3");
        let start = f.duty_base - TEMP_SPAN;
        let mut bytes = vec![0u8; 14];
        bytes[6..13].copy_from_slice(&[30, 40, 50, 60, 70, 80, 90]);
        bytes.extend_from_slice(&COMMUNITY);
        let pass = Pass { start, bytes };
        let cap = Capture { iso: vec![pass.clone()], sweep: vec![pass], regs: vec![(0xA00, 0x01), (0x80F, 0x00)] };
        let text = dump_lines(&cap);
        assert!(text.contains("iso 0x0C32 "), "{text}");
        let again = parse_dump(&format!("# header\n{text}some other line\n")).unwrap();
        assert_eq!(again, cap);
        let decoded = decode_all(&again);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].fan, "sys_fan3");
        assert_eq!(decoded[0].iso_eq_sweep, Some(true));
        assert!(parse_dump("sweep 0xC40 ZZ").is_err());
        assert!(parse_dump("reg 0xA00 01 02").is_err());
    }

    #[test]
    fn a_duty_only_dump_decodes_without_temperatures() {
        let cap = parse_dump("sweep 0xC70 66 00 84 00 BF 00 FF 00 FF 00 FF 00 FF 00\n").unwrap();
        let decoded = decode_all(&cap);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].fan, "sys_fan1");
        assert!(decoded[0].layouts.is_empty());
    }

    #[test]
    fn isolated_reads_are_descending_with_a_neutral_read_before_each() {
        let mut log = Vec::new();
        let mut read = |reg: u16| {
            log.push(reg);
            Ok((reg & 0xFF) as u8)
        };
        let iso = read_isolated(&mut read, 0xC40, 3).unwrap();
        assert_eq!(iso.bytes, [0x40, 0x41, 0x42]);
        assert_eq!(log, [NEUTRAL_REG, 0xC42, NEUTRAL_REG, 0xC41, NEUTRAL_REG, 0xC40]);
        log.clear();
        let mut read = |reg: u16| {
            log.push(reg);
            Ok(0u8)
        };
        read_sweep(&mut read, 0xC40, 3).unwrap();
        assert_eq!(log, [0xC40, 0xC41, 0xC42]);
    }

    #[test]
    fn the_service_state_is_read_from_sc_output() {
        let stopped = "SERVICE_NAME: canvas-rgb-svc\n        TYPE               : 10  WIN32_OWN_PROCESS\n        STATE              : 1  STOPPED\n";
        let running = "        STATE              : 4  RUNNING\n";
        assert_eq!(classify_sc(Some(0), stopped), ServiceState::Stopped);
        assert_eq!(classify_sc(Some(0), running), ServiceState::Busy("4  RUNNING".into()));
        assert_eq!(classify_sc(Some(1060), ""), ServiceState::NotInstalled);
        assert!(matches!(classify_sc(Some(5), "Access is denied."), ServiceState::Unknown(_)));
    }
}
