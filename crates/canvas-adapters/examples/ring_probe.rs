//! The 8BitDo Ultimate 2 ring probe (#376, stage S2): read the pad's stored profile, decode the
//! ring block. **It writes nothing to the pad.**
//!
//! Run it as:
//! `cargo run --release -p canvas-adapters --example ring_probe`
//!
//! What it sends: the profile read's own requests and nothing else, 36 read requests on the pad's
//! vendor channel (request word 2, one per 45-byte chunk of the 1592-byte image; a chunk that is
//! not answered is asked for again, up to three times). A guard in front of the channel refuses
//! any report that is not a byte-exact read request before it goes out.
//!
//! What it prints: the raw reply of every chunk as hex, then what it can decode: the stored mode,
//! the active profile, which ring theme records each profile marks in use, the 24 custom colour
//! words, and a guess at the colour byte order with the evidence.
//!
//! What it saves: `--out PATH` (default `ring_probe_<unix seconds>.txt` in the current folder)
//! holds the 36 raw replies and the image as hex. `--replay PATH` reads such a file back and
//! decodes it with no pad at all.
//!
//! `--check` only lists which personality the pad is showing and opens nothing.
//!
//! The daemon does not have to be stopped. If the pad's vendor channel will not open because the
//! daemon or the 8BitDo app holds it, the probe stops, says so, and sends nothing.

use std::time::{SystemTime, UNIX_EPOCH};

use canvas_adapters::eightbitdo_ultimate2::probe::{self, ProfileCapture};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: ring_probe [--check] [--out PATH] [--replay PATH]");
        println!("  (no flag)      read the pad's profile (36 read requests, nothing written), print, save");
        println!("  --check        list the pad's personality and open nothing");
        println!("  --out PATH     where to save the raw replies (default ring_probe_<unix>.txt)");
        println!("  --replay PATH  decode a saved file; no pad is touched");
        return;
    }
    let value = |flag: &str| args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone());

    if args.iter().any(|a| a == "--check") {
        let found = probe::enumerate_personalities();
        if found.is_empty() {
            println!("no 8BitDo pad or receiver found");
        }
        for p in found {
            println!("found: {}", p.label());
        }
        return;
    }

    if let Some(path) = value("--replay") {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return fail(&format!("cannot read {path}: {e}")),
        };
        match ProfileCapture::from_text(&text) {
            Ok(capture) => return show(&capture, None),
            Err(e) => return fail(&format!("{path} is not a ring probe file: {e}")),
        }
    }

    println!("reading the pad's profile: read requests only, nothing is written");
    let capture = match probe::capture_from_hardware() {
        Ok(c) => c,
        Err(e) => return fail(&e.to_string()),
    };
    let out = value("--out").unwrap_or_else(|| {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        format!("ring_probe_{secs}.txt")
    });
    show(&capture, Some(&out));
}

/// Prints the raw replies and the decode; saves the replies when `out` names a file.
fn show(capture: &ProfileCapture, out: Option<&str>) {
    println!(
        "pad: {}; {} read requests sent; {} replies; image {} bytes",
        capture.personality.as_deref().unwrap_or("unknown"),
        capture.requests_sent,
        capture.replies.len(),
        capture.image.len()
    );
    print!("{}", capture.raw_listing());
    println!();
    match capture.decode_report() {
        Ok(report) => print!("{report}"),
        Err(e) => println!("cannot decode: {e}"),
    }
    if let Some(path) = out {
        match std::fs::write(path, capture.to_text()) {
            Ok(()) => println!("\nsaved the raw replies and the image to {path}"),
            Err(e) => eprintln!("\ncould not save {path}: {e}"),
        }
    }
}

fn fail(message: &str) {
    eprintln!("ring_probe: {message}");
    std::process::exit(1);
}
