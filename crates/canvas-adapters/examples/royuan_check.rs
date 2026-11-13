//! The device check for #311 (`docs/adapters.md`, "Device check for Luke"). TOUCHES REAL
//! HARDWARE: run it only in a session Luke has said yes to, with canvas-rgb's output paused
//! (`canvas-rgb output pause`) so the daemon does not write to the keyboard at the same time.
//!
//! ```text
//! cargo run -p canvas-adapters --example royuan_check -- read      # read only: id, key map, mode
//! cargo run -p canvas-adapters --example royuan_check -- picture   # overwrites User Picture slot 0
//! cargo run -p canvas-adapters --example royuan_check -- restore   # canvas-rgb's own mode, white
//! ```
//!
//! `read` sends three kinds of get-requests and prints each reply in hex. `picture` sends the seven
//! `SET_USERPIC` reports of a frame with slot 9 (A) red and every other slot black, 100 ms apart,
//! waits 2 s, then light mode 13. `restore` sends `direct_mode_report` and a white whole-board
//! colour. Nothing here repeats fast: the sources say a fast upload loop killed a keyboard's
//! control endpoint until a re-plug.

use canvas_adapters::royuan::{color_report, direct_mode_report, per_key_reports, user_picture_mode_report, PER_KEY_SLOTS};
use canvas_proto::color::Rgb8;
use std::time::Duration;

const VENDOR_ID: u16 = 0x3151;
const PRODUCT_ID: u16 = 0x4015;
const INTERFACE: i32 = 2;
const USAGE_PAGE: u16 = 0xFFFF;
const USAGE: u16 = 0x0002;
const REPORT_LEN: usize = 65;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

/// The sum-complement checksum the family uses over payload bytes 0..=6 ("Bit7").
fn bit7(payload: &[u8; 8]) -> u8 {
    0xFF - (payload[..7].iter().map(|b| u32::from(*b)).sum::<u32>() & 0xFF) as u8
}

/// A get-request: command byte and one argument byte at payload offset 2, Bit7 checksum.
fn request(cmd: u8, arg: u8) -> [u8; REPORT_LEN] {
    let mut payload = [0u8; 8];
    payload[0] = cmd;
    payload[2] = arg;
    payload[7] = bit7(&payload);
    let mut r = [0u8; REPORT_LEN];
    r[1..9].copy_from_slice(&payload);
    r
}

fn open() -> hidapi::HidDevice {
    let api = hidapi::HidApi::new().expect("hidapi");
    let info = api
        .device_list()
        .find(|d| {
            d.vendor_id() == VENDOR_ID
                && d.product_id() == PRODUCT_ID
                && d.interface_number() == INTERFACE
                && d.usage_page() == USAGE_PAGE
                && d.usage() == USAGE
        })
        .expect("no ROYUAN vendor interface (3151:4015, interface 2, 0xFFFF/0x0002)");
    info.open_device(&api).expect("open the vendor interface")
}

fn send(dev: &hidapi::HidDevice, report: &[u8; REPORT_LEN], what: &str) {
    println!("send {what}: {}", hex(&report[..12]));
    dev.send_feature_report(report).unwrap_or_else(|e| panic!("send {what}: {e}"));
}

fn ask(dev: &hidapi::HidDevice, report: &[u8; REPORT_LEN], what: &str) {
    send(dev, report, what);
    std::thread::sleep(Duration::from_millis(30));
    let mut buf = [0u8; REPORT_LEN];
    match dev.get_feature_report(&mut buf) {
        Ok(n) => println!("  reply ({n} bytes): {}", hex(&buf[..n.min(REPORT_LEN)])),
        Err(e) => println!("  reply: error {e}"),
    }
    std::thread::sleep(Duration::from_millis(100));
}

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "read" => {
            let dev = open();
            ask(&dev, &request(0x8F, 0), "device id (8F)");
            for page in 0..9u8 {
                ask(&dev, &request(0x89, page), &format!("key map page {page} (89)"));
            }
            ask(&dev, &request(0x87, 0), "current light mode (87)");
        }
        "picture" => {
            let dev = open();
            let mut frame = vec![Rgb8::BLACK; PER_KEY_SLOTS];
            frame[9] = Rgb8::new(255, 0, 0);
            for (page, report) in per_key_reports(&frame).iter().enumerate() {
                send(&dev, report, &format!("user picture page {page}"));
                std::thread::sleep(Duration::from_millis(100));
            }
            std::thread::sleep(Duration::from_secs(2));
            send(&dev, &user_picture_mode_report(), "light mode 13 (user picture)");
            println!("Look at the keyboard: is ONLY the A key lit red, every other key dark?");
        }
        "restore" => {
            // canvas-rgb's own mode, then the whole-board colour streamed at 30 fps as the adapter
            // does (a single colour report right after a mode switch may not be shown): white for
            // 2 s, then black for 1 s, as canvas-rgb's paused output leaves the board.
            let dev = open();
            send(&dev, &direct_mode_report(), "canvas-rgb's own mode");
            std::thread::sleep(Duration::from_millis(300));
            for (c, secs, name) in [(Rgb8::new(255, 255, 255), 2.0f32, "white"), (Rgb8::BLACK, 1.0, "black")] {
                println!("whole board {name}, streamed for {secs} s: {}", hex(&color_report(c)[..12]));
                let until = std::time::Instant::now() + Duration::from_secs_f32(secs);
                while std::time::Instant::now() < until {
                    dev.send_feature_report(&color_report(c)).unwrap_or_else(|e| panic!("send {name}: {e}"));
                    std::thread::sleep(Duration::from_millis(33));
                }
            }
        }
        "bytes" => {
            // Prints every report this probe can send, without opening the device.
            println!("8F: {}", hex(&request(0x8F, 0)[..10]));
            for page in 0..9u8 {
                println!("89 page {page}: {}", hex(&request(0x89, page)[..10]));
            }
            println!("87: {}", hex(&request(0x87, 0)[..10]));
            let mut frame = vec![Rgb8::BLACK; PER_KEY_SLOTS];
            frame[9] = Rgb8::new(255, 0, 0);
            for (page, report) in per_key_reports(&frame).iter().enumerate() {
                println!("picture page {page}: {}", hex(report));
            }
            println!("mode 13: {}", hex(&user_picture_mode_report()[..10]));
            println!("direct mode: {}", hex(&direct_mode_report()[..10]));
            println!("white: {}", hex(&color_report(Rgb8::new(255, 255, 255))[..12]));
        }
        _ => eprintln!("usage: royuan_check bytes | read | picture | restore"),
    }
}
