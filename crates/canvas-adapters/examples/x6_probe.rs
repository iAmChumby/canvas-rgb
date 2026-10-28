//! The Attack Shark X6 first-hardware-test probe.
//!
//! Run it as:
//! `cargo run --release -p canvas-adapters --example x6_probe`
//!
//! It does two things and nothing else:
//! - enumerates the receiver and says whether the mouse is there;
//! - reads the mouse's settings **once**, with the `0xA0` read-back handshake, and prints them.
//!
//! It sends **A0 read requests** and **never a config report**. The read request is a feature
//! report (`SET_FEATURE`), but it changes no setting: it asks the receiver to hand back what it
//! already holds. Config writes are off: the adapter's write path is off (`writes_enabled` is
//! false until a caller turns it on) and this probe does not turn it on, so no setting on the
//! mouse can change.
//!
//! `--check` enumerates only and never opens the device.
//!
//! TOUCHES REAL HARDWARE from the read step on. Run it with the mouse awake and the receiver
//! plugged in.

use canvas_adapters::attack_shark_x6::{
    AttackSharkX6, PeripheralAdapter, ASLEEP, CONFIG_USAGE, CONFIG_USAGE_PAGE, EVENT_USAGE,
    EVENT_USAGE_PAGE, PRODUCT_ID, VENDOR_ID,
};
use canvas_proto::device::DeviceId;
use canvas_proto::props::{PropKind, PropValue};
use canvas_proto::traits::AdapterError;

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
        PropKind::Number { min, max, step, .. } => {
            format!("{min}..{max} step {step}")
        }
        PropKind::Choice { options, .. } => options.join(", "),
        PropKind::Bool { .. } => "on / off".to_string(),
        PropKind::Color { .. } => "colour".to_string(),
        PropKind::Hue { .. } => "hue".to_string(),
    }
}

/// Why the read-back failed, in the words the message itself carries.
///
/// The asleep error is its own thing: the mouse is not answering because it is asleep, and the
/// instruction is the useful part, so it is printed on its own lines.
fn print_read_failure(error: &AdapterError) {
    if let AdapterError::Protocol(message) = error {
        if message == ASLEEP {
            eprintln!("the read-back failed: {message}");
            eprintln!("move the mouse on the desk: any movement wakes it up.");
            eprintln!("then run this again.");
            return;
        }
    }
    eprintln!("the read-back failed: {error}");
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: x6_probe [--check]");
        println!("  (no flag)  enumerate the receiver and read the mouse's settings once");
        println!("  --check    enumerate only; the device is never opened");
        return std::process::ExitCode::SUCCESS;
    }
    let check_only = args.iter().any(|a| a == "--check");

    println!("== x6_probe ==");
    println!("receiver:            {VENDOR_ID:04x}:{PRODUCT_ID:04x} (2.4G, Beken)");
    println!(
        "configuration:       usage page {CONFIG_USAGE_PAGE:#06x}, usage {CONFIG_USAGE:#06x} (COL04)"
    );
    println!(
        "status:              usage page {EVENT_USAGE_PAGE:#06x}, usage {EVENT_USAGE:#06x} (COL03)"
    );

    let mut mouse = AttackSharkX6::new();
    let found = match mouse.discover().await {
        Ok(found) => found,
        Err(e) => {
            eprintln!("the enumeration failed: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let Some(info) = found.first() else {
        println!();
        println!("the mouse: NOT FOUND");
        println!("nothing was opened and nothing was sent.");
        return std::process::ExitCode::SUCCESS;
    };
    println!();
    println!("the mouse:           PRESENT ({} {}, {})", info.name, info.id, info.location);

    if check_only {
        println!();
        println!("--check: nothing was opened and nothing was sent.");
        return std::process::ExitCode::SUCCESS;
    }

    println!();
    println!("reading the settings once (the 0xA0 read-back handshake, about 3.3 s of quiet time).");
    println!("this sends A0 read requests and never a config report.");
    println!("config writes are off: the adapter's write path is off and this probe never turns it on.");

    if let Err(e) = mouse.connect().await {
        eprintln!("the mouse could not be opened: {e}");
        return std::process::ExitCode::FAILURE;
    }
    let id: DeviceId = mouse.device_id();
    let schema = match mouse.describe(&id).await {
        Ok(schema) => schema,
        Err(e) => {
            eprintln!("the mouse could not be described: {e}");
            let _ = mouse.disconnect().await;
            return std::process::ExitCode::FAILURE;
        }
    };
    let props = match mouse.read_settings(&id).await {
        Ok(props) => props,
        Err(e) => {
            print_read_failure(&e);
            let _ = mouse.disconnect().await;
            return std::process::ExitCode::FAILURE;
        }
    };

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
            println!(
                "  {:<24} {:<28} [{}]  ({})",
                def.label,
                shown,
                write,
                shape(&def.kind)
            );
        }
        println!();
    }
    if !schema.unreported.is_empty() {
        println!("not reported (the device holds them, this build cannot name them):");
        for key in &schema.unreported {
            println!("  {key}");
        }
        println!();
    }

    let _ = mouse.disconnect().await;
    println!("the device is closed again. Nothing was written.");
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_never_turns_the_write_path_on() {
        // Config writes are off until a caller says otherwise, and this probe is not that caller.
        let mouse = AttackSharkX6::new();
        assert!(!mouse.writes_enabled());
    }

    #[test]
    fn the_values_print_the_way_the_hub_shows_them() {
        assert_eq!(show(&PropValue::Number(1000.0)), "1000");
        assert_eq!(show(&PropValue::Choice("hz-1000".into())), "hz-1000");
        assert_eq!(
            show(&PropValue::Color(canvas_proto::color::Rgb8::new(0xEF, 0x3F, 0xFF))),
            "#ef3fff"
        );
        assert_eq!(show(&PropValue::Bool(true)), "true");
        assert_eq!(show(&PropValue::Hue(90.0)), "90");
    }

    #[test]
    fn the_shape_line_shows_the_range_or_the_options() {
        assert_eq!(
            shape(&PropKind::Number { min: 50.0, max: 26000.0, step: 50.0, default: 800.0 }),
            "50..26000 step 50"
        );
        assert_eq!(
            shape(&PropKind::Choice { options: vec!["125".into(), "1000".into()], default: "1000".into() }),
            "125, 1000"
        );
    }

    #[test]
    fn the_asleep_error_is_printed_with_its_instruction() {
        // The message the daemon shows as it is: the mouse is asleep, and what to do about it.
        assert_eq!(ASLEEP, "the mouse is asleep: move it and try again");
    }
}
