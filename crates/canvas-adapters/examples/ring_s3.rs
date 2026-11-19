//! Stage S3 of #376: write 24 LEDs of pure red to the 8BitDo Ultimate 2's rings, wait, and put
//! the saved profile back. **This writes the pad.**
//!
//! Run it as:
//! `ring_s3 --order bgr --i-mean-it`          (or `--order rgb`)
//!
//! It drives the ring adapter's own write path: the same checks, chunks, commit, read-back and
//! exact restore. Steps:
//! 1. reads the profile image (36 read requests) and saves it to the backup file BEFORE writing;
//!    refuses unless the pad is on profile 1 (`--profile N` to change) and that profile's custom
//!    ring record is marked in use;
//! 2. writes pure red to all 24 LEDs, commits, reads the image back and checks that only the ring
//!    record changed (a mismatch restores at once and stops);
//! 3. prints "rings should be RED now; press Enter to restore" and waits for Enter, or for
//!    `--hold-secs N` seconds;
//! 4. writes the saved bytes back, commits, reads the image back and confirms it is byte-identical
//!    to the backup.
//!
//! An error at any step after the first write restores and says so.
//!
//! `--backup PATH` where the backup goes (default `ring_s3_backup_<unix>.txt` in the current folder).
//! `--restore-from FILE --i-mean-it` is the escape hatch: it puts a backup back and does nothing else.
//! `--hold-secs N` waits N seconds instead of Enter (for a run with no keyboard).
//!
//! Turn the pad's reads off on the Peripherals page first, or the daemon's idle read may overlap
//! the write session. Ctrl-C while the rings are red leaves them red: run `--restore-from` with the
//! backup file.

use std::io::BufRead;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use canvas_adapters::eightbitdo_ultimate2::probe::ProfileCapture;
use canvas_adapters::eightbitdo_ultimate2::ring::ByteOrder;
use canvas_adapters::eightbitdo_ultimate2::s3::{self, S3Io, S3Options};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") || args.is_empty() {
        println!("usage: ring_s3 --order bgr|rgb --i-mean-it [--profile N] [--backup PATH] [--hold-secs N]");
        println!("       ring_s3 --restore-from FILE --i-mean-it");
        println!("  writes pure red to the rings, waits, restores the saved profile; see the file's header");
        return;
    }
    let value = |flag: &str| args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone());
    if !args.iter().any(|a| a == "--i-mean-it") {
        return fail("this writes the pad: add --i-mean-it");
    }
    let mut say = |line: &str| println!("{line}");

    if let Some(path) = value("--restore-from") {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return fail(&format!("cannot read {path}: {e}")),
        };
        let backup = match ProfileCapture::from_text(&text) {
            Ok(c) => c,
            Err(e) => return fail(&format!("{path} is not a ring probe or S3 backup file: {e}")),
        };
        match s3::restore_backup_on_hardware(&backup, &mut say) {
            Ok(how) => println!("restored ({how:?})"),
            Err(e) => fail(&format!("the restore failed: {e}")),
        }
        return;
    }

    let order = match value("--order").as_deref() {
        Some("bgr") => ByteOrder::Bgr,
        Some("rgb") => ByteOrder::Rgb,
        _ => return fail("--order bgr or --order rgb is required"),
    };
    let profile: usize = match value("--profile").map(|p| p.parse()) {
        None => 1,
        Some(Ok(n)) if (1..=3).contains(&n) => n,
        _ => return fail("--profile takes 1, 2 or 3"),
    };
    let hold_secs: Option<u64> = match value("--hold-secs").map(|p| p.parse()) {
        None => None,
        Some(Ok(n)) => Some(n),
        Some(Err(_)) => return fail("--hold-secs takes a number of seconds"),
    };
    let backup_path = value("--backup").unwrap_or_else(|| {
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        format!("ring_s3_backup_{secs}.txt")
    });

    let mut backup = |capture: &ProfileCapture| -> Result<(), String> {
        let text = capture.to_text();
        let mut file = std::fs::File::create(&backup_path).map_err(|e| format!("{backup_path}: {e}"))?;
        std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|e| format!("{backup_path}: {e}"))?;
        file.sync_all().map_err(|e| format!("{backup_path}: {e}"))?;
        drop(file);
        // Read it back: a backup that does not parse to the same image is no backup.
        let back = std::fs::read_to_string(&backup_path).map_err(|e| format!("{backup_path}: {e}"))?;
        let parsed = ProfileCapture::from_text(&back).map_err(|e| format!("{backup_path}: {e}"))?;
        if parsed.image != capture.image {
            return Err(format!("{backup_path} does not read back as the image"));
        }
        println!("backup saved and checked: {backup_path}");
        println!("if anything goes wrong: ring_s3 --restore-from {backup_path} --i-mean-it");
        Ok(())
    };
    let mut hold = || -> Result<(), String> {
        match hold_secs {
            Some(n) => {
                println!("holding for {n} s");
                std::thread::sleep(Duration::from_secs(n));
                Ok(())
            }
            None => {
                let mut line = String::new();
                match std::io::stdin().lock().read_line(&mut line) {
                    Ok(0) => Err("stdin closed before Enter".into()),
                    Ok(_) => Ok(()),
                    Err(e) => Err(e.to_string()),
                }
            }
        }
    };
    let opts = S3Options { order, expect_profile: profile - 1 };
    let mut io = S3Io { say: &mut say, backup: &mut backup, hold: &mut hold };
    match s3::run_on_hardware(&opts, &mut io) {
        Ok(outcome) => println!("done: {outcome:?}"),
        Err(e) => fail(&format!("S3 stopped: {e}")),
    }
}

fn fail(message: &str) {
    eprintln!("ring_s3: {message}");
    std::process::exit(1);
}
