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
//!
//! `query`, `show` and `hold` refuse to run while ZEUS CAST is running: it owns the display, and
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
use std::time::{Duration, Instant};

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
        other => {
            eprintln!("unknown step '{other}'");
            eprintln!("usage: chione_probe [check|query|show|hold] [--force]");
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
