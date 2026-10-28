//! The 8BitDo Ultimate 2 first-hardware-test probe.
//!
//! Run it as:
//! `cargo run --release -p canvas-adapters --example pad_probe`
//!
//! It does four things and nothing else:
//! - enumerates the pad (or its 2.4G receiver) and says which personality it is showing;
//! - reads the pad's settings **once**, with the vendor's chunked profile read, and prints them;
//! - runs the research's **read-only** paddle experiment: it polls the XInput state and every HID
//!   collection of the pad and says what moved;
//! - with `--set-paddles`, writes the four paddles of the active profile. That write needs
//!   `--i-mean-it` as well, or it is refused.
//!
//! `--check` enumerates only and never opens the device.
//! `--paddles` runs the paddle experiment on its own.
//! `--seconds N` sets how long the paddle experiment watches (60 by default).
//! `--dump PATH` writes the profile image (1592 bytes) the read brought back to PATH.
//! `--set-paddles P1=a,P2=b` says what the paddles of the active profile should produce.
//! `--i-mean-it` is the second half of `--set-paddles`.
//!
//! The profile read sends the vendor's own chunk requests: 36 output reports on the vendor
//! collection. They ask the pad for what it already holds and change no setting, but they are
//! still output reports, so this needs an explicit go-ahead. The paddle experiment sends **nothing** at all:
//! it only reads.
//!
//! Config writes are off by default: the adapter's write path (`writes_enabled`) is false until a
//! caller turns it on. Only `--set-paddles ... --i-mean-it` turns it on, and only for that one
//! write. Before it writes, the probe prints the 4 original bytes of every slot it will change
//! and the exact command that restores them, and it prints them again if anything fails.
//! Only the plain buttons are accepted as targets (a/b/x/y, bumpers, triggers, stick clicks,
//! d-pad, start/select/menu/home, none): the paddle, swap, turbo and screenshot targets are
//! refused for now.

use std::collections::BTreeMap;
use std::time::Duration;

use canvas_adapters::eightbitdo_ultimate2::{
    codec, EightBitDoUltimate2, PaddleProbe, PeripheralAdapter, Personality, PRODUCT_ID_DINPUT,
    PRODUCT_ID_IDLE, PRODUCT_ID_XINPUT, VENDOR_ID,
};
use canvas_proto::props::{PropKind, PropValue};

/// The value as the probe prints it.
fn show(value: &PropValue) -> String {
    match value {
        PropValue::Color(c) => c.to_hex(),
        PropValue::Number(n) => format!("{n}"),
        PropValue::Bool(b) => format!("{b}"),
        PropValue::Choice(c) => c.clone(),
        PropValue::Hue(h) => format!("{h}"),
    }
}

/// The range or the options of a property, as the probe prints it under the label.
fn shape(kind: &PropKind) -> String {
    match kind {
        PropKind::Number { min, max, step, .. } => format!("{min}..{max} step {step}"),
        PropKind::Choice { options, .. } => options.join(", "),
        PropKind::Bool { .. } => "on / off".to_string(),
        PropKind::Color { .. } => "colour".to_string(),
        PropKind::Hue { .. } => "hue".to_string(),
    }
}

/// What a personality means for this probe.
fn personality_note(personality: Personality) -> &'static str {
    match personality {
        Personality::XInput => {
            "the vendor channel is there (usage page 0xff7a) and the pad reads through XInput; \
             the XInput personality has no field for the paddles"
        }
        Personality::Idle => {
            "the receiver is idle: no controller is linked, so it cannot answer the profile read"
        }
        Personality::DInput => {
            "the pad reads through its HID gamepad collection (the paddles and the battery are in \
             it) but the receiver's DInput descriptor has no 0x81 output report, so there is no \
             vendor channel to read the profile through"
        }
    }
}

/// True when a collection is the vendor channel (the one the config packets travel over).
fn is_vendor_channel(label: &str) -> bool {
    label.contains("up=ff7a") || label.contains("up=ffa0")
}

/// The name of a button, as a person would say it.
fn friendly(name: &str) -> String {
    match name {
        "a" | "b" | "x" | "y" => name.to_uppercase(),
        "dpad-up" => "d-pad up".into(),
        "dpad-down" => "d-pad down".into(),
        "dpad-left" => "d-pad left".into(),
        "dpad-right" => "d-pad right".into(),
        other => other.replace('-', " "),
    }
}

/// What the paddle experiment saw, in plain words: one line per source.
///
/// The XInput line says what the pad reported, the HID line what its own collections showed,
/// the vendor line what the config channel showed, and the paddle line whether anything at all
/// looked like a paddle. The pad's heartbeat is named as such: it moves the XInput packet
/// counter whether or not anything is pressed (the 2026-10-27 runs), so it is never a paddle.
fn paddle_summary(probe: &PaddleProbe) -> String {
    let mut out = String::new();

    // XInput: the buttons and the triggers the pad reported.
    let mut seen: Vec<String> = probe.xinput.buttons.iter().map(|b| friendly(b)).collect();
    if probe.xinput.left_trigger > 0 {
        seen.push("left trigger".into());
    }
    if probe.xinput.right_trigger > 0 {
        seen.push("right trigger".into());
    }
    if seen.is_empty() {
        out.push_str("XInput: nothing.\n");
    } else {
        out.push_str(&format!("XInput: {}.\n", seen.join(", ")));
    }

    // HID: what each collection the probe could open showed.
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for (label, _) in &probe.hid_changes {
        *counts.entry(label.as_str()).or_default() += 1;
    }
    if counts.is_empty() {
        out.push_str("HID: nothing changed on any collection.\n");
    } else {
        let total: u32 = counts.values().sum();
        let names: Vec<String> = counts
            .iter()
            .map(|(label, n)| format!("{label} ({n})"))
            .collect();
        out.push_str(&format!(
            "HID: {total} change(s) on {} collection(s): {}.\n",
            counts.len(),
            names.join(", ")
        ));
    }

    // The vendor channel on its own.
    let vendor: u32 = probe
        .hid_changes
        .iter()
        .filter(|(label, _)| is_vendor_channel(label))
        .count() as u32;
    if vendor == 0 {
        out.push_str("Vendor channel: nothing.\n");
    } else {
        out.push_str(&format!("Vendor channel: {vendor} change(s).\n"));
    }

    // The heartbeat, named as what it is.
    let cadence = probe.xinput.cadence;
    if cadence.samples > 0 {
        out.push_str(&format!(
            "the packet counter moved {} time(s) on its own, in steps of about {}: that is the \
             pad's heartbeat, not a paddle.\n",
            probe.xinput.packet_only_moves, cadence.median
        ));
    }

    // And the paddles themselves.
    if probe.xinput.candidates.is_empty() {
        out.push_str("Paddles: no input seen anywhere.\n");
    } else {
        out.push_str(&format!(
            "Paddles: {} step(s) broke the heartbeat's rhythm ({}): those are the only things \
             that could be a paddle.\n",
            probe.xinput.candidates.len(),
            probe
                .xinput
                .candidates
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out
}

/// The paddles the `--set-paddles` argument asks for: the paddle name and the target.
///
/// `P1=a,P2=b` gives two; `P1` alone means the target the pad already holds.
/// The targets a paddle may be set to for now: the plain buttons and `none`. The paddle,
/// swap, turbo and screenshot targets are refused until a first plain write has been seen working.
const WRITABLE_TARGETS: [&str; 19] = [
    "none", "a", "b", "x", "y", "left-bumper", "right-bumper", "left-trigger", "right-trigger",
    "left-stick-click", "right-stick-click", "dpad-up", "dpad-down", "dpad-left", "dpad-right",
    "start", "select", "menu", "home",
];

/// Refuses every target outside [`WRITABLE_TARGETS`].
fn check_target(target: &str) -> Result<(), String> {
    if WRITABLE_TARGETS.contains(&target) {
        return Ok(());
    }
    let why = match target {
        "paddle-1" | "paddle-2" | "paddle-3" | "paddle-4" => "the paddle targets are refused for now",
        "swap" => "swap is refused for now",
        "turbo-single" | "turbo-auto" => "the turbo targets are refused for now",
        "screenshot" => "screenshot is refused for now",
        _ => "it is not one of the plain buttons",
    };
    Err(format!(
        "{target:?} is refused: {why}. Allowed: a, b, x, y, left-bumper, right-bumper, \
         left-trigger, right-trigger, left-stick-click, right-stick-click, dpad-up, dpad-down, \
         dpad-left, dpad-right, start, select, menu, home, none"
    ))
}

/// What one slot held before the write, for the restore.
struct Original {
    /// `P1`..`P4`.
    paddle: String,
    /// The property key.
    key: String,
    /// The 4 stored bytes, hex.
    bytes: String,
    /// The target name they decode to.
    name: String,
}

/// The command that puts every original back.
fn restore_command(originals: &[Original]) -> String {
    let list: Vec<String> =
        originals.iter().map(|o| format!("{}={}", o.paddle, o.name)).collect();
    format!("pad_probe --set-paddles {} --i-mean-it", list.join(","))
}

/// Prints the originals and the restore command.
fn print_restore(originals: &[Original]) {
    println!("  the original bytes of each slot being changed:");
    for o in originals {
        println!("    {} ({}): {} = {}", o.paddle, o.key, o.bytes, o.name);
    }
    println!("  to put them back, run:");
    println!("    {}", restore_command(originals));
}

fn parse_paddles(arg: &str) -> Result<Vec<(usize, Option<String>)>, String> {
    let mut out = Vec::new();
    for part in arg.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, target) = match part.split_once('=') {
            Some((name, target)) => (name.trim(), Some(target.trim().to_string())),
            None => (part, None),
        };
        let slot = match name.to_ascii_uppercase().as_str() {
            "P1" => 0,
            "P2" => 1,
            "P3" => 2,
            "P4" => 3,
            other => return Err(format!("{other:?} is not a paddle: the pad has P1, P2, P3 and P4")),
        };
        if let Some(t) = &target {
            check_target(t)?;
        }
        out.push((slot, target));
    }
    if out.is_empty() {
        return Err("--set-paddles needs at least one paddle: P1=a,P2=b".into());
    }
    Ok(out)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: pad_probe [--check] [--paddles] [--seconds N] [--dump PATH] [--set-paddles P1=a,...] [--i-mean-it]");
        println!("  (no flag)        enumerate the pad and read its settings once");
        println!("  --check          enumerate only; the device is never opened");
        println!("  --paddles        the research's read-only paddle experiment");
        println!("  --seconds N      how long the paddle experiment watches (60 by default);");
        println!("                   the first 3 s of it measure the pad's idle heartbeat");
        println!("  --dump PATH      write the profile image (1592 bytes) to PATH");
        println!("  --set-paddles    write the paddles of the active profile (needs --i-mean-it)");
        println!("  --i-mean-it      the second half of --set-paddles");
        return std::process::ExitCode::SUCCESS;
    }
    let check_only = args.iter().any(|a| a == "--check");
    let paddles = args.iter().any(|a| a == "--paddles");
    let i_mean_it = args.iter().any(|a| a == "--i-mean-it");
    let seconds: f64 = args
        .windows(2)
        .find(|w| w[0] == "--seconds")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(60.0);
    let dump = args.windows(2).find(|w| w[0] == "--dump").map(|w| w[1].clone());
    let set_paddles = args
        .windows(2)
        .find(|w| w[0] == "--set-paddles")
        .map(|w| w[1].clone());

    println!("== pad_probe ==");
    println!("vendor:              {VENDOR_ID:04x} (8BitDo)");
    println!(
        "personalities:       {PRODUCT_ID_XINPUT:04x} (XInput), {PRODUCT_ID_DINPUT:04x} (DInput), \
         {PRODUCT_ID_IDLE:04x} (2.4G receiver, idle)"
    );
    println!("vendor collection:   usage page 0xff7a (XInput) or 0xffa0 (idle), usage 0x0001");
    println!("reports:             out 0x81, in 0x02, 64 bytes each");

    let mut pad = EightBitDoUltimate2::new();
    let found = match pad.discover().await {
        Ok(found) => found,
        Err(e) => {
            eprintln!("the enumeration failed: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let Some(info) = found.first() else {
        println!();
        println!("the pad: NOT FOUND");
        println!("nothing was opened and nothing was sent.");
        return std::process::ExitCode::SUCCESS;
    };
    let personality = pad.personality().unwrap_or(Personality::XInput);
    println!();
    println!("the pad:              PRESENT ({} {}, {})", info.name, info.id, info.location);
    println!("personality:         {}", personality.label());
    println!("{}", personality_note(personality));

    if check_only {
        println!();
        println!("--check: nothing was opened and nothing was sent.");
        return std::process::ExitCode::SUCCESS;
    }

    if paddles {
        println!();
        println!("the paddle experiment (read-only: it sends nothing at all).");
        println!("press these, one at a time:");
        println!("  PL  the left back paddle");
        println!("  PR  the right back paddle");
        println!("  L4  the left fast bumper");
        println!("  R4  the right fast bumper");
        println!("hands off the pad at the start: the first three seconds of the run measure the");
        println!("pad's own heartbeat, so that a real press can be told from it.");
        for n in [3, 2, 1] {
            println!("{n}...");
            std::thread::sleep(Duration::from_secs(1));
        }
        let probe = match pad.paddle_probe(seconds).await {
            Ok(probe) => probe,
            Err(e) => {
                eprintln!("the paddle experiment failed: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        println!();
        println!("collections opened:");
        for label in &probe.opened {
            println!("  {label}");
        }
        if !probe.hid_changes.is_empty() {
            println!();
            println!("HID collection changes:");
            for (label, bytes) in &probe.hid_changes {
                println!("  {label}: {bytes}");
            }
        }
        println!();
        println!("what this means:");
        print!("{}", paddle_summary(&probe));
        return std::process::ExitCode::SUCCESS;
    }

    println!();
    println!("reading the settings once (the vendor's chunked profile read: 36 output reports).");
    println!("this asks the pad for what it already holds and changes no setting.");
    if set_paddles.is_some() && i_mean_it {
        println!("config writes: --set-paddles with --i-mean-it was given, so ONE paddle write follows the read.");
    } else {
        println!("config writes are off: without --set-paddles and --i-mean-it this probe never turns them on.");
    }

    if let Err(e) = pad.connect().await {
        eprintln!("the pad could not be opened: {e}");
        return std::process::ExitCode::FAILURE;
    }
    let id = pad.device_id();
    let schema = match pad.describe(&id).await {
        Ok(schema) => schema,
        Err(e) => {
            eprintln!("the pad could not be described: {e}");
            let _ = pad.disconnect().await;
            return std::process::ExitCode::FAILURE;
        }
    };
    let props = match pad.read_settings(&id).await {
        Ok(props) => props,
        Err(e) => {
            eprintln!("the profile read failed: {e}");
            let _ = pad.disconnect().await;
            return std::process::ExitCode::FAILURE;
        }
    };

    // The image, for the record: the whole 1592 bytes the read brought back.
    if let Some(path) = &dump {
        match pad.image() {
            Some(image) => match std::fs::write(path, image) {
                Ok(()) => println!("the profile image ({} bytes) is written to {path}", image.len()),
                Err(e) => eprintln!("the profile image could not be written to {path}: {e}"),
            },
            None => eprintln!("there is no profile image to write"),
        }
    }

    println!();
    for section in &schema.sections {
        println!("{}:", section.title);
        for def in &section.properties {
            let value = props.get(&def.key);
            let shown = match value {
                Some(v) => show(v),
                None => "not reported".to_string(),
            };
            let write = if schema.is_writable(&def.key) { "writable" } else { "read-only" };
            println!("  {:<24} {:<28} [{}]  ({})", def.label, shown, write, shape(&def.kind));
        }
        println!();
    }
    if !schema.unreported.is_empty() {
        println!("not reported (the pad holds them, this build cannot name them):");
        for key in &schema.unreported {
            println!("  {key}");
        }
        println!();
    }

    // The image's own CRC, as data: which candidate matches the value the pad stores, if any.
    if let Some(image) = pad.image() {
        match codec::image_crc(image) {
            Ok(crc) => {
                println!("the image's own CRC:");
                println!("  the pad stores          {:#010x}", crc.stored);
                println!("  over all 1592 bytes    {:#06x}", crc.whole);
                println!("  with the field zeroed   {:#06x}", crc.zeroed);
                println!("  without the field       {:#06x}", crc.without_field);
                println!("  after the field         {:#06x}", crc.after_field);
                match crc.matching() {
                    Some(which) => println!("  the stored value matches: {which}"),
                    None => println!("  no candidate matches the stored value"),
                }
                println!();
            }
            Err(e) => println!("the image's CRC could not be computed: {e}\n"),
        }
    }

    // The paddle write, when it is asked for and meant.
    if let Some(arg) = &set_paddles {
        if !i_mean_it {
            eprintln!();
            eprintln!("--set-paddles refused: it needs --i-mean-it as well.");
            eprintln!("nothing was written. The pad still holds what it held.");
            let _ = pad.disconnect().await;
            return std::process::ExitCode::FAILURE;
        }
        let wanted = match parse_paddles(arg) {
            Ok(wanted) => wanted,
            Err(e) => {
                eprintln!();
                eprintln!("--set-paddles: {e}");
                let _ = pad.disconnect().await;
                return std::process::ExitCode::FAILURE;
            }
        };
        let active = match props.get("active-profile") {
            Some(PropValue::Choice(name)) => name.clone(),
            _ => "1".to_string(),
        };
        let profile: usize = active.parse().unwrap_or(1);
        println!();
        println!("the paddle write (profile {active} is the active profile):");
        let mut changes = Vec::new();
        let mut originals = Vec::new();
        let image = pad.image().map(<[u8]>::to_vec);
        for (slot, target) in &wanted {
            let key = format!("profile-{profile}-paddle-{}", slot + 1);
            let before = props.get(&key).map(show).unwrap_or_else(|| "not reported".into());
            let at = codec::map_entry_offset(profile - 1, 18 + slot);
            let bytes = match (&image, at) {
                (Some(img), Some(at)) if at + 4 <= img.len() => codec::hex(&img[at..at + 4]),
                _ => {
                    eprintln!("  {key}: its stored bytes are not in the image the read brought back.");
                    eprintln!("  nothing was written.");
                    let _ = pad.disconnect().await;
                    return std::process::ExitCode::FAILURE;
                }
            };
            if before == "other" || before == "not reported" {
                eprintln!(
                    "  {key} holds {before} ({bytes}), which has no name this probe can write \
                     back, so there would be no restore command. Nothing was written."
                );
                let _ = pad.disconnect().await;
                return std::process::ExitCode::FAILURE;
            }
            let target = target.clone().unwrap_or_else(|| before.clone());
            println!("  {key}: {before} -> {target}");
            originals.push(Original {
                paddle: format!("P{}", slot + 1),
                key: key.clone(),
                bytes,
                name: before,
            });
            changes.push((key, PropValue::Choice(target)));
        }
        println!();
        print_restore(&originals);
        println!();
        let write_props: canvas_proto::props::Props = changes.into_iter().collect();
        let packets = match pad.plan_packets(&write_props) {
            Ok(packets) => packets,
            Err(e) => {
                eprintln!("  the write could not be planned: {e}");
                let _ = pad.disconnect().await;
                return std::process::ExitCode::FAILURE;
            }
        };
        println!("  these are the exact packets that go out, in order (Reportstate(0), the write, the commit, Reportstate(1)):");
        for packet in &packets {
            println!("    {}", codec::hex(packet));
        }
        // The write itself. The gate is off until this line.
        pad.set_writes_enabled(true);
        if let Err(e) = pad.write_settings(&id, &write_props).await {
            eprintln!("  the write failed: {e}");
            eprintln!("  the pad may hold either the old or the new value. To be sure it holds the old one:");
            print_restore(&originals);
            let _ = pad.disconnect().await;
            return std::process::ExitCode::FAILURE;
        }
        println!("  the write went out. Reading the pad again to see what it holds:");
        let after = match pad.read_settings(&id).await {
            Ok(after) => after,
            Err(e) => {
                eprintln!("  the read-back failed: {e}");
                print_restore(&originals);
                let _ = pad.disconnect().await;
                return std::process::ExitCode::FAILURE;
            }
        };
        let mut mismatch = false;
        for (slot, _) in &wanted {
            let key = format!("profile-{profile}-paddle-{}", slot + 1);
            let now = after.get(&key).map(show).unwrap_or_else(|| "not reported".into());
            println!("  {key}: {now}");
            if write_props.get(&key).map(show).as_deref() != Some(now.as_str()) {
                mismatch = true;
            }
        }
        if mismatch {
            eprintln!("  the pad does not hold what was written. Restore with:");
            print_restore(&originals);
        } else {
            println!("  to undo this write:");
            println!("    {}", restore_command(&originals));
        }
    }

    let _ = pad.disconnect().await;
    println!();
    println!("the device is closed again. Nothing else was written.");
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_adapters::eightbitdo_ultimate2::codec;

    #[test]
    fn a_fresh_adapter_has_writes_off() {
        // Config writes are off until a caller says otherwise; only --set-paddles --i-mean-it does.
        let pad = EightBitDoUltimate2::new();
        assert!(!pad.writes_enabled());
    }

    #[test]
    fn the_values_print_the_way_the_hub_shows_them() {
        assert_eq!(show(&PropValue::Number(100.0)), "100");
        assert_eq!(show(&PropValue::Choice("xinput".into())), "xinput");
        assert_eq!(show(&PropValue::Bool(true)), "true");
    }

    #[test]
    fn the_shape_line_shows_the_range_or_the_options() {
        assert_eq!(
            shape(&PropKind::Number { min: 0.0, max: 100.0, step: 20.0, default: 0.0 }),
            "0..100 step 20"
        );
        assert_eq!(
            shape(&PropKind::Choice { options: vec!["a".into(), "b".into()], default: "a".into() }),
            "a, b"
        );
    }

    #[test]
    fn the_summary_says_in_plain_words_what_was_seen_where() {
        // The first hardware run, 2026-10-27: the pad reported A, both bumpers and both
        // triggers over XInput, its gamepad collection showed something, the vendor channel
        // showed nothing, and the 67 "packet moved and nothing else" lines were the pad's own
        // heartbeat: no paddle was seen anywhere.
        let mut probe = PaddleProbe {
            opened: vec!["pid=310b if=0 up=0001 u=0005".into()],
            ..PaddleProbe::default()
        };
        probe.xinput.buttons = vec!["a", "left-bumper", "right-bumper"];
        probe.xinput.left_trigger = 255;
        probe.xinput.right_trigger = 255;
        probe.xinput.packet_only_moves = 67;
        probe.xinput.cadence = codec::IdleCadence { samples: 3, min: 23, max: 32, median: 31 };
        probe.hid_changes.push(("pid=310b if=0 up=0001 u=0005".into(), "00 80 ff 7f".into()));
        let summary = paddle_summary(&probe);
        assert!(summary.contains("XInput: A, left bumper, right bumper, left trigger, right trigger."), "{summary}");
        assert!(summary.contains("HID: 1 change(s) on 1 collection(s)"), "{summary}");
        assert!(summary.contains("Vendor channel: nothing."), "{summary}");
        assert!(summary.contains("the pad's heartbeat, not a paddle"), "{summary}");
        assert!(summary.contains("Paddles: no input seen anywhere."), "{summary}");
    }

    #[test]
    fn the_summary_names_a_candidate_when_one_breaks_the_rhythm() {
        let mut probe = PaddleProbe::default();
        probe.xinput.cadence = codec::IdleCadence { samples: 3, min: 23, max: 32, median: 31 };
        probe.xinput.packet_only_moves = 67;
        probe.xinput.candidates = vec![100];
        let summary = paddle_summary(&probe);
        assert!(summary.contains("Paddles: 1 step(s) broke the heartbeat's rhythm (100)"), "{summary}");
        assert!(!summary.contains("no input seen anywhere"), "{summary}");
    }

    #[test]
    fn the_vendor_channel_is_told_apart_from_the_other_collections() {
        assert!(is_vendor_channel("pid=310b if=2 up=ff7a u=0001"));
        assert!(is_vendor_channel("pid=6013 if=1 up=ffa0 u=0001"));
        assert!(!is_vendor_channel("pid=310b if=0 up=0001 u=0005"));
        assert!(!is_vendor_channel("pid=310b if=1 up=000c u=0001"));
    }

    #[test]
    fn the_paddle_argument_names_the_paddles_and_their_targets() {
        let wanted = parse_paddles("P1=a,P2=b").unwrap();
        assert_eq!(wanted, vec![(0, Some("a".into())), (1, Some("b".into()))]);
        let wanted = parse_paddles(" p4 = left-trigger ").unwrap();
        assert_eq!(wanted, vec![(3, Some("left-trigger".into()))]);
        let wanted = parse_paddles("P3").unwrap();
        assert_eq!(wanted, vec![(2, None)]);
        assert!(parse_paddles("P5=a").is_err());
        assert!(parse_paddles("").is_err());
    }

    #[test]
    fn only_the_plain_buttons_are_accepted_as_paddle_targets() {
        for ok in ["a", "b", "x", "y", "none", "left-bumper", "right-trigger", "left-stick-click",
                   "dpad-up", "start", "select", "menu", "home"] {
            assert!(parse_paddles(&format!("P1={ok}")).is_ok(), "{ok}");
        }
        for bad in ["paddle-1", "paddle-2", "paddle-3", "paddle-4", "swap", "turbo-single",
                    "turbo-auto", "screenshot", "other", "nonsense"] {
            let err = parse_paddles(&format!("P1={bad}")).unwrap_err();
            assert!(err.contains("refused"), "{bad}: {err}");
        }
        // Every allowed name is a real target of the codec.
        for t in WRITABLE_TARGETS {
            assert!(codec::target_mask(t).is_some(), "{t}");
        }
    }

    #[test]
    fn the_restore_command_puts_every_original_back() {
        let originals = vec![
            Original { paddle: "P1".into(), key: "profile-1-paddle-1".into(), bytes: "00 00 00 00".into(), name: "none".into() },
            Original { paddle: "P3".into(), key: "profile-1-paddle-3".into(), bytes: "00 20 00 00".into(), name: "a".into() },
        ];
        assert_eq!(restore_command(&originals), "pad_probe --set-paddles P1=none,P3=a --i-mean-it");
    }

    #[test]
    fn the_personalities_name_what_each_one_carries() {
        assert!(personality_note(Personality::XInput).contains("0xff7a"));
        assert!(personality_note(Personality::DInput).contains("paddles"));
        assert!(personality_note(Personality::Idle).contains("idle"));
        // The personality the probe reports is the one the adapter found.
        assert_eq!(Personality::XInput.product_id(), PRODUCT_ID_XINPUT);
        assert_eq!(Personality::DInput.product_id(), PRODUCT_ID_DINPUT);
        assert_eq!(Personality::Idle.product_id(), PRODUCT_ID_IDLE);
        // The codec's constants are the ones the research names.
        assert_eq!(codec::IMAGE_LEN, 0x638);
        assert_eq!(codec::CHUNK, 0x2D);
        assert_eq!(codec::REPORT_LEN, 64);
    }
}
