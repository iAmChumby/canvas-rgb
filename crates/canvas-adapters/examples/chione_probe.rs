//! The CHIONE first-hardware-test probe.
//!
//! Run it as:
//! `cargo run --release -p canvas-adapters --example chione_probe -- <step>`
//!
//! Steps:
//! - `check` (the default): is 1B80:B53C there, is ZEUS CAST running, and what would be sent.
//!   No device I/O: the device is enumerated, never opened.
//! - `query`: open, send the 0xFF query, print the raw reply and the fields the adapter reads,
//!   close. Nothing is written to the display.
//! - `show`: connect, send 42.5 °C every 250 ms for 10 s, print every packet and any ack, then
//!   stop sending and close.
//! - `hold`: like `show`, then stop writing for 30 s and count down, so the display can be seen
//!   holding, blanking or reverting.
//! - `dot`: does a decimal-point byte of 0 turn the dot off? Three phases of 8 s, a packet every
//!   250 ms: (A) "425" in temperature mode with byte 8 = 0, (B) "425" in fan-RPM mode with byte 8
//!   = 0, (C) the normal 42.5 °C packet. Then 42.5 for 5 s so the display is left on a normal value.
//!
//! `query`, `show`, `hold` and `dot` refuse to run while ZEUS CAST is running: it owns the display, and
//! both writing at once would confuse it. The tool never starts or stops ZEUS CAST itself; every
//! step says how to restore it.
//!
//! `--force` (on `show` and `hold`): open the device without the adapter's check of the 0xFF
//! reply. The reply layout on the real unit is still a guess (recon section 2), and `connect`
//! refuses a reply it cannot read. With `--force` the raw reply is printed and the writing goes
//! on anyway.
//!
//! TOUCHES REAL HARDWARE from `query` on: run those steps only with Luke watching the pump head.

use canvas_adapters::chione::{
    encode, parse_query_reply, query_report, ChioneDisplay, PRODUCT_ID, VENDOR_ID,
};
use canvas_proto::device::DeviceId;
use canvas_proto::display::{DisplayMode, DisplayUnit, DisplayValue};
use canvas_proto::traits::NumericDisplayAdapter;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The vendor app that owns the display while it runs.
const ZEUS_CAST_EXE: &str = "ZEUS CAST.exe";
/// How to restore the vendor app after a step.
const ZEUS_CAST_PATH: &str = r"C:\Program Files\GAMDIAS\ZEUS CAST\ZEUS CAST.exe";
/// The number the `show` and `hold` steps put on the glass.
const SHOW_VALUE: DisplayValue = DisplayValue {
    value: Some(42.5),
    decimals: 1,
    unit: DisplayUnit::Celsius,
    mode: DisplayMode::Temperature,
    blink: false,
};
/// How long `show` and `hold` write.
const SHOW_SECONDS: u64 = 10;
/// How long `hold` stops writing.
const HOLD_SECONDS: u64 = 30;
/// How long each `dot` phase writes.
const DOT_PHASE_SECONDS: u64 = 8;
/// How long `dot` keeps writing 42.5 after phase C.
const DOT_TAIL_SECONDS: u64 = 5;

/// One phase of the dot test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DotPhase {
    /// "425" in temperature mode, dot byte 0.
    A,
    /// "425" in fan-RPM mode, dot byte 0.
    B,
    /// The normal 42.5 °C packet.
    C,
}

impl DotPhase {
    fn name(self) -> &'static str {
        match self {
            DotPhase::A => "A: 425, temperature mode, dot off",
            DotPhase::B => "B: 425, fan-RPM mode, dot off",
            DotPhase::C => "C: 42.5 C, normal",
        }
    }
}

/// The known-good 42.5 °C packet (recon section 2; the `show` hardware test), checksum included.
const KNOWN_GOOD_42_5: [u8; 14] = [0x00, 0x3A, 0xB5, 0x01, 0x20, 0x04, 0x02, 0x05, 0x01, 0x01, 0x01, 0x00, 0x00, 0x1E];

/// Byte 8 is the dot, byte 11 the mode (0 temperature, 1 fan RPM). Only those bytes and the
/// checksum (byte 13, the sum of bytes 0..=12 masked to 0xFF) differ from the known-good packet.
fn dot_packet(phase: DotPhase) -> [u8; 65] {
    let mut r = [0u8; 65];
    r[..14].copy_from_slice(&KNOWN_GOOD_42_5);
    match phase {
        DotPhase::A => r[8] = 0,
        DotPhase::B => {
            r[8] = 0;
            r[11] = 1;
        }
        DotPhase::C => {}
    }
    r[13] = r[..13].iter().fold(0u8, |acc, b| acc.wrapping_add(*b));
    r
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let step = args.first().map(String::as_str).unwrap_or("check");
    let force = args.iter().any(|a| a == "--force");
    let result = match step {
        "check" => check().await,
        "query" => query().await,
        "show" => show(force).await,
        "hold" => hold(force).await,
        "dot" => dot(force).await,
        other => {
            eprintln!("unknown step '{other}'");
            eprintln!("usage: chione_probe [check|query|show|hold|dot] [--force]");
            return std::process::ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!();
            eprintln!("FAILED: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Is the device there, is the vendor app running, and what would the other steps send?
async fn check() -> Result<(), String> {
    println!("== chione_probe check ==");
    let mut display = ChioneDisplay::new();
    // The enumeration reads the HID metadata: it opens no device and does no I/O.
    let found = display.discover().await.map_err(|e| format!("the enumeration failed: {e}"))?;
    match found.first() {
        Some(info) => println!("device {VENDOR_ID:04x}:{PRODUCT_ID:04x} ({}): PRESENT", info.name),
        None => println!(
            "device {VENDOR_ID:04x}:{PRODUCT_ID:04x} (GAMDIAS CHIONE Digital Display): not found"
        ),
    }
    println!(
        "{ZEUS_CAST_EXE}: {}",
        if zeus_cast_running() { "RUNNING (it owns the display)" } else { "not running" }
    );
    println!();
    println!("the packets the other steps send:");
    println!("  query (0xFF):   {}", hex(&query_report()));
    println!(
        "  show  (42.5 C): {}",
        hex(&encode(&SHOW_VALUE).map_err(|e| e.to_string())?)
    );
    println!("(every packet is {} bytes; the checksum is byte 13)", query_report().len());
    println!();
    println!("nothing was opened and nothing was written.");
    restore();
    Ok(())
}

/// Open, send the 0xFF query, print the raw reply and its fields, close.
async fn query() -> Result<(), String> {
    refuse_if_vendor_running()?;
    println!("== chione_probe query ==");
    println!("opening the device and sending the 0xFF query. Nothing is written to the display.");
    let mut display = ChioneDisplay::new();
    let reply = display.query_raw().await.map_err(|e| format!("the query failed: {e}"))?;
    println!("the reply: {} bytes", reply.len());
    println!("  {}", hex(&reply));
    match parse_query_reply(&reply) {
        Ok(fields) => {
            println!("the adapter reads it as:");
            println!(
                "  hfw={} lfw={} tempFW={} cpuFW={} fanFW={}",
                fields.hfw, fields.lfw, fields.temp_fw, fields.cpu_fw, fields.fan_fw
            );
        }
        Err(error) => {
            println!("the adapter refuses this reply: {error}");
            println!("(the layout is still a guess - recon section 2. The bytes above are the answer.)");
        }
    }
    let _ = display.disconnect().await;
    println!();
    println!("the device is closed again.");
    restore();
    Ok(())
}

/// Connect and write 42.5 °C for ten seconds, then stop and close.
async fn show(force: bool) -> Result<(), String> {
    refuse_if_vendor_running()?;
    println!("== chione_probe show ==");
    println!("LOOK AT THE PUMP HEAD NOW");
    let mut display = ChioneDisplay::new();
    open(&mut display, force).await?;
    let id = DeviceId(format!("chione:{VENDOR_ID:04x}:{PRODUCT_ID:04x}"));
    let packet = encode(&SHOW_VALUE).map_err(|e| e.to_string())?;
    let interval = Duration::from_millis(u64::from(display.write_interval_ms()));
    println!("the packet, every {} ms:", interval.as_millis());
    println!("  {}", hex(&packet));
    let start = Instant::now();
    let until = start + Duration::from_secs(SHOW_SECONDS);
    let mut writes = 0u32;
    while Instant::now() < until {
        display.write_value(&id, &SHOW_VALUE).await.map_err(|e| format!("the write failed: {e}"))?;
        writes += 1;
        let ack = match display.last_ack() {
            Some(bytes) => hex(bytes),
            None => "none".to_string(),
        };
        println!(
            "  write {writes:>3} at {:>5.1}s: {}  ack: {ack}",
            start.elapsed().as_secs_f32(),
            hex(&packet)
        );
        tokio::time::sleep(interval).await;
    }
    let _ = display.disconnect().await;
    println!();
    println!("stopped sending after {writes} writes and closed the device.");
    restore();
    Ok(())
}

/// Like `show`, then stop writing for thirty seconds and count down.
async fn hold(force: bool) -> Result<(), String> {
    refuse_if_vendor_running()?;
    println!("== chione_probe hold ==");
    println!("LOOK AT THE PUMP HEAD NOW");
    let mut display = ChioneDisplay::new();
    open(&mut display, force).await?;
    let id = DeviceId(format!("chione:{VENDOR_ID:04x}:{PRODUCT_ID:04x}"));
    let packet = encode(&SHOW_VALUE).map_err(|e| e.to_string())?;
    let interval = Duration::from_millis(u64::from(display.write_interval_ms()));
    println!("the packet, every {} ms:", interval.as_millis());
    println!("  {}", hex(&packet));
    let start = Instant::now();
    let until = start + Duration::from_secs(SHOW_SECONDS);
    let mut writes = 0u32;
    while Instant::now() < until {
        display.write_value(&id, &SHOW_VALUE).await.map_err(|e| format!("the write failed: {e}"))?;
        writes += 1;
        let ack = match display.last_ack() {
            Some(bytes) => hex(bytes),
            None => "none".to_string(),
        };
        println!(
            "  write {writes:>3} at {:>5.1}s: {}  ack: {ack}",
            start.elapsed().as_secs_f32(),
            hex(&packet)
        );
        tokio::time::sleep(interval).await;
    }
    println!();
    println!("stopped sending after {writes} writes. The device stays open and nothing is sent for {HOLD_SECONDS} s.");
    println!("LOOK AT THE PUMP HEAD: does it hold 42.5, go blank, or show what it showed before?");
    for left in (1..=HOLD_SECONDS).rev() {
        println!("  {left:>2} s left");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let _ = display.disconnect().await;
    println!();
    println!("closed the device.");
    restore();
    Ok(())
}

/// The dot test: phases A, B and C at 250 ms per packet, then 42.5 for a short tail. Stops at the
/// first write error, since the device is dropped by then.
async fn dot(force: bool) -> Result<(), String> {
    refuse_if_vendor_running()?;
    println!("== chione_probe dot ==");
    println!("LOOK AT THE PUMP HEAD NOW");
    let mut display = ChioneDisplay::new();
    open(&mut display, force).await?;
    let interval = Duration::from_millis(u64::from(display.write_interval_ms()));
    println!("writing every {} ms; {DOT_PHASE_SECONDS} s per phase.", interval.as_millis());

    let mut failure = None;
    for phase in [DotPhase::A, DotPhase::B, DotPhase::C] {
        let packet = dot_packet(phase);
        println!();
        println!("phase {}", phase.name());
        println!("  start: {}", clock());
        println!("  packet: {}", hex(&packet));
        let tally = run_phase(&mut display, &packet, interval, DOT_PHASE_SECONDS).await;
        println!("  writes {}, acks {}", tally.writes, tally.acks);
        if let Some(error) = tally.error {
            println!("  WRITE ERROR: {error}");
            failure = Some(format!("phase {} stopped on a write error: {error}", phase.name()));
            break;
        }
    }

    if failure.is_none() {
        let normal = dot_packet(DotPhase::C);
        println!();
        println!("tail: 42.5 C for {DOT_TAIL_SECONDS} s so the display is left on a normal value");
        println!("  start: {}", clock());
        let tally = run_phase(&mut display, &normal, interval, DOT_TAIL_SECONDS).await;
        println!("  writes {}, acks {}", tally.writes, tally.acks);
        if let Some(error) = tally.error {
            println!("  WRITE ERROR: {error}");
            failure = Some(format!("the tail stopped on a write error: {error}"));
        }
    }

    let _ = display.disconnect().await;
    println!();
    println!("closed the device.");
    restore();
    match failure {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

/// What one phase did: writes sent, acks read back, and the first write error (which ends it).
struct PhaseTally {
    writes: u32,
    acks: u32,
    error: Option<String>,
}

/// Writes `packet` every `interval` for `seconds`, counting the acks.
async fn run_phase(display: &mut ChioneDisplay, packet: &[u8; 65], interval: Duration, seconds: u64) -> PhaseTally {
    let mut tally = PhaseTally { writes: 0, acks: 0, error: None };
    let until = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < until {
        if let Err(e) = display.send_report(packet).await {
            tally.error = Some(e.to_string());
            break;
        }
        tally.writes += 1;
        if display.last_ack().is_some() {
            tally.acks += 1;
        }
        tokio::time::sleep(interval).await;
    }
    tally
}

/// The current UTC time of day, to the millisecond.
fn clock() -> String {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs() % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:03} UTC",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
        since.subsec_millis()
    )
}

/// Opens the display for a step that writes to it.
///
/// Without `force` this is the daemon's own path: `connect` runs the 0xFF handshake and refuses a
/// reply it cannot read. With `force` the device is opened and the query sent without that check,
/// and the raw reply is printed, so a wrong guess about the layout does not stop the test.
async fn open(display: &mut ChioneDisplay, force: bool) -> Result<(), String> {
    if !force {
        return display.connect().await.map_err(|e| format!("the handshake failed: {e}"));
    }
    let reply = display.query_raw().await.map_err(|e| format!("the query failed: {e}"))?;
    println!("--force: the 0xFF reply was not checked.");
    println!("  {}", hex(&reply));
    match parse_query_reply(&reply) {
        Ok(fields) => println!(
            "  hfw={} lfw={} tempFW={} cpuFW={} fanFW={}",
            fields.hfw, fields.lfw, fields.temp_fw, fields.cpu_fw, fields.fan_fw
        ),
        Err(error) => println!("  the adapter would refuse it: {error}"),
    }
    Ok(())
}

/// Whether the vendor app is running. It holds the display, so the steps that write refuse while
/// it does.
///
/// The process list is read with `tasklist` and the image name is matched exactly, as the daemon
/// matches it.
fn zeus_cast_running() -> bool {
    let Ok(output) = std::process::Command::new("tasklist").args(["/FO", "CSV", "/NH"]).output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let name = line.split(',').next().unwrap_or("").trim().trim_matches('"');
        name.eq_ignore_ascii_case(ZEUS_CAST_EXE)
    })
}

/// The steps that touch the device refuse while the vendor app runs.
fn refuse_if_vendor_running() -> Result<(), String> {
    if zeus_cast_running() {
        return Err(format!(
            "{ZEUS_CAST_EXE} is running: it owns the display. Close it (and the tray it sits in) first."
        ));
    }
    Ok(())
}

/// How to put the display back.
fn restore() {
    println!();
    println!("restore: start {ZEUS_CAST_PATH}");
    println!("(it takes the display back and re-handshakes on its own)");
}

/// The bytes as hex, spaced, like the adapter's own log.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 13 bytes of a report before its checksum, as hex: the test's own expectation.
    fn head(r: &[u8; 65]) -> String {
        hex(&r[..14])
    }

    #[test]
    fn phase_a_is_425_temperature_with_the_dot_off() {
        // dot 0, mode 0: sum 285 -> 0x1D
        let r = dot_packet(DotPhase::A);
        assert_eq!(head(&r), "00 3a b5 01 20 04 02 05 00 01 01 00 00 1d");
        assert!(r[14..].iter().all(|b| *b == 0));
    }

    #[test]
    fn phase_b_is_425_fan_rpm_with_the_dot_off() {
        // dot 0, mode 1, bytes 9 and 10 kept from the known-good packet: sum 286 -> 0x1E
        let r = dot_packet(DotPhase::B);
        assert_eq!(head(&r), "00 3a b5 01 20 04 02 05 00 01 01 01 00 1e");
    }

    #[test]
    fn phase_c_is_the_known_good_42_5() {
        // sum 286 -> 0x1E; the same bytes the adapter's encode builds for 42.5 C
        let r = dot_packet(DotPhase::C);
        assert_eq!(head(&r), "00 3a b5 01 20 04 02 05 01 01 01 00 00 1e");
        let encoded = crate::encode(&DisplayValue {
            value: Some(42.5),
            decimals: 1,
            unit: DisplayUnit::Celsius,
            mode: DisplayMode::Temperature,
            blink: false,
        })
        .unwrap();
        assert_eq!(r, encoded);
    }

    #[test]
    fn every_checksum_is_the_sum_of_bytes_0_to_12() {
        for phase in [DotPhase::A, DotPhase::B, DotPhase::C] {
            let r = dot_packet(phase);
            let sum: u32 = r[..13].iter().map(|b| u32::from(*b)).sum();
            assert_eq!(u32::from(r[13]), sum % 256, "{phase:?}");
            assert_eq!(r.len(), 65);
            assert_eq!(r[0], 0x00, "report ID");
        }
    }

    #[test]
    fn only_the_dot_mode_and_checksum_differ_from_the_known_good_packet() {
        let good = dot_packet(DotPhase::C);
        let a = dot_packet(DotPhase::A);
        let b = dot_packet(DotPhase::B);
        let differing = |x: &[u8; 65]| (0..14).filter(|i| x[*i] != good[*i]).collect::<Vec<_>>();
        assert_eq!(differing(&a), vec![8, 13]);
        // B's sum is 286 like C's, so its checksum byte is the same 0x1E
        assert_eq!(differing(&b), vec![8, 11]);
    }
}
