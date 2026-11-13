//! Prints the Rust arrangement planner's output for a fixed set of inputs as JSON.
//!
//! The editor draws a group's arrangement with its own TypeScript planner
//! (`app/web/src/lib/arrangement.ts`), and that planner is checked against this file:
//! `app/web/src/lib/__tests__/arrangement.vectors.json` holds the output below, and the
//! editor's test compares the two plans to 1e-4.
//!
//! Print it and write the file with:
//! `powershell -NoProfile -File tools/arrangement-vectors.ps1`
//!
//! The JSON is written by hand (no serde here): the file is a small, fixed set of inputs
//! and their plans, and the printer keeps the crate's dependencies as they are.

use canvas_core::arrangement::{GroupPlan, LightSamples, build_group_plan};
use canvas_core::geometry::LedSample;
use canvas_proto::device::{DeviceId, DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::{CANVAS_HEIGHT, CANVAS_WIDTH, LedSlice};
use canvas_proto::scene::{Arrangement, GroupMode, Light};

/// One fixed input and the plan the planner makes of it.
struct Case {
    /// What the case shows.
    name: &'static str,
    /// The group's members, in order.
    members: Vec<String>,
    /// How the group shares its canvas.
    mode: GroupMode,
    /// The lights the members name.
    lights: Vec<Light>,
    /// The devices the lights live on.
    devices: Vec<DeviceInfo>,
}

fn zone(index: u32, kind: ZoneKind, led_count: u32) -> ZoneInfo {
    ZoneInfo {
        index,
        name: format!("zone{index}"),
        kind,
        led_count,
        min_leds: led_count,
        max_leds: led_count,
        resizable: false,
    }
}

fn device(id: &str, zones: Vec<ZoneInfo>) -> DeviceInfo {
    DeviceInfo {
        id: DeviceId(id.to_string()),
        name: id.to_string(),
        vendor: "test".into(),
        kind: DeviceKind::Other,
        adapter: "fake".into(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones,
        connected: true,
    }
}

fn light(id: &str, device_id: &str, zone_index: u32, arrangement: Arrangement) -> Light {
    Light {
        id: id.to_string(),
        device_id: DeviceId(device_id.to_string()),
        zone_index,
        leds: None,
        name: None,
        arrangement,
        reversed: false,
    }
}

/// The fixed inputs the editor's planner is checked against.
fn cases() -> Vec<Case> {
    // Two strips of 12 and 36 LEDs: the group's slots share the canvas by LED share.
    let share_devices =
        vec![device("d0", vec![zone(0, ZoneKind::Linear, 12)]), device("d1", vec![zone(0, ZoneKind::Linear, 36)])];
    let share_lights = vec![light("a", "d0", 0, Arrangement::Strip), light("b", "d1", 0, Arrangement::Strip)];

    // A ring of 8 and a ring of 24: the circle's cells are one LED's share of it.
    let ring8_devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 8)])];
    let ring8_lights = vec![light("a", "d0", 0, Arrangement::Ring)];
    let ring24_devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 24)])];
    let ring24_lights = vec![light("a", "d0", 0, Arrangement::Ring)];

    // A slice on the second zone of a device, running the other way.
    let slice_devices =
        vec![device("d0", vec![zone(0, ZoneKind::Linear, 8), zone(1, ZoneKind::Linear, 40)])];
    let mut sliced = light("a", "d0", 1, Arrangement::Strip);
    sliced.leds = Some(LedSlice { start: 12, count: 12 });
    sliced.reversed = true;
    let slice_lights = vec![sliced];

    // A grid on a matrix with a gap: the gap LED has no cell and stays black.
    let grid_devices = vec![device(
        "d0",
        vec![zone(0, ZoneKind::Matrix { width: 2, height: 2, map: vec![Some(0), Some(1), None, Some(3)] }, 4)],
    )];
    let grid_lights = vec![light("a", "d0", 0, Arrangement::Grid)];

    // A member named twice: one member, one slot.
    let twice_devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 12)])];
    let twice_lights = vec![light("a", "d0", 0, Arrangement::Strip)];

    // A single light: every LED reads the slot's centre, and the cell is one logical unit
    // there, not the whole slot.
    let single_devices = vec![device("d0", vec![zone(0, ZoneKind::Linear, 12)])];
    let single_lights = vec![light("a", "d0", 0, Arrangement::Single)];

    vec![
        Case {
            name: "flow across 12 and 36",
            members: vec!["a".into(), "b".into()],
            mode: GroupMode::FlowAcross,
            lights: share_lights.clone(),
            devices: share_devices.clone(),
        },
        Case {
            name: "each its own 12 and 36",
            members: vec!["a".into(), "b".into()],
            mode: GroupMode::EachItsOwn,
            lights: share_lights,
            devices: share_devices,
        },
        Case {
            name: "a ring of 8",
            members: vec!["a".into()],
            mode: GroupMode::EachItsOwn,
            lights: ring8_lights,
            devices: ring8_devices,
        },
        Case {
            name: "a ring of 24",
            members: vec!["a".into()],
            mode: GroupMode::EachItsOwn,
            lights: ring24_lights,
            devices: ring24_devices,
        },
        Case {
            name: "a reversed slice on the second zone",
            members: vec!["a".into()],
            mode: GroupMode::FlowAcross,
            lights: slice_lights,
            devices: slice_devices,
        },
        Case {
            name: "a grid with a gap",
            members: vec!["a".into()],
            mode: GroupMode::EachItsOwn,
            lights: grid_lights,
            devices: grid_devices,
        },
        Case {
            name: "a member named twice",
            members: vec!["a".into(), "a".into()],
            mode: GroupMode::FlowAcross,
            lights: twice_lights,
            devices: twice_devices,
        },
        Case {
            name: "a single light of 12",
            members: vec!["a".into()],
            mode: GroupMode::EachItsOwn,
            lights: single_lights,
            devices: single_devices,
        },
    ]
}

/// A JSON string. The names here are plain, but the quotes are escaped anyway.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A JSON number from a canvas unit.
fn json_number(value: f32) -> String {
    format!("{value}")
}

fn json_device_kind(kind: &DeviceKind) -> String {
    json_string(match kind {
        DeviceKind::Motherboard => "motherboard",
        DeviceKind::Dram => "dram",
        DeviceKind::Keyboard => "keyboard",
        DeviceKind::Mouse => "mouse",
        DeviceKind::Microphone => "microphone",
        DeviceKind::Headset => "headset",
        DeviceKind::LedStrip => "led_strip",
        DeviceKind::Gpu => "gpu",
        DeviceKind::Other => "other",
    })
}

fn json_zone_kind(kind: &ZoneKind) -> String {
    match kind {
        ZoneKind::Single => "{\"type\":\"single\"}".to_string(),
        ZoneKind::Linear => "{\"type\":\"linear\"}".to_string(),
        ZoneKind::Matrix { width, height, map } => {
            let cells: Vec<String> = map
                .iter()
                .map(|led| match led {
                    Some(led) => led.to_string(),
                    None => "null".to_string(),
                })
                .collect();
            format!(
                "{{\"type\":\"matrix\",\"width\":{width},\"height\":{height},\"map\":[{}]}}",
                cells.join(",")
            )
        }
    }
}

fn json_zone(zone: &ZoneInfo) -> String {
    format!(
        "{{\"index\":{},\"name\":{},\"kind\":{},\"led_count\":{},\"min_leds\":{},\"max_leds\":{},\"resizable\":{}}}",
        zone.index,
        json_string(&zone.name),
        json_zone_kind(&zone.kind),
        zone.led_count,
        zone.min_leds,
        zone.max_leds,
        zone.resizable
    )
}

fn json_device(device: &DeviceInfo) -> String {
    let zones: Vec<String> = device.zones.iter().map(json_zone).collect();
    format!(
        "{{\"id\":{},\"name\":{},\"vendor\":{},\"kind\":{},\"adapter\":{},\"adapter_index\":{},\"serial\":{},\"location\":{},\"zones\":[{}],\"connected\":{}}}",
        json_string(&device.id.0),
        json_string(&device.name),
        json_string(&device.vendor),
        json_device_kind(&device.kind),
        json_string(&device.adapter),
        device.adapter_index,
        json_string(&device.serial),
        json_string(&device.location),
        zones.join(","),
        device.connected
    )
}

fn json_arrangement(arrangement: &Arrangement) -> String {
    json_string(match arrangement {
        Arrangement::Single => "single",
        Arrangement::Strip => "strip",
        Arrangement::Ring => "ring",
        Arrangement::Grid => "grid",
    })
}

fn json_light(light: &Light) -> String {
    let mut out = format!(
        "{{\"id\":{},\"device_id\":{},\"zone_index\":{},",
        json_string(&light.id),
        json_string(&light.device_id.0),
        light.zone_index
    );
    if let Some(slice) = &light.leds {
        out.push_str(&format!(
            "\"leds\":{{\"start\":{},\"count\":{}}},",
            slice.start, slice.count
        ));
    }
    if let Some(name) = &light.name {
        out.push_str(&format!("\"name\":{},", json_string(name)));
    }
    out.push_str(&format!(
        "\"arrangement\":{},\"reversed\":{}}}",
        json_arrangement(&light.arrangement),
        light.reversed
    ));
    out
}

fn json_led(led: &LedSample) -> String {
    format!(
        "{{\"center_x\":{},\"center_y\":{},\"bounds\":[{},{},{},{}],\"sampled\":{}}}",
        json_number(led.center_x),
        json_number(led.center_y),
        json_number(led.bounds[0]),
        json_number(led.bounds[1]),
        json_number(led.bounds[2]),
        json_number(led.bounds[3]),
        led.sampled
    )
}

fn json_light_samples(samples: &LightSamples) -> String {
    let leds: Vec<String> = samples.leds.iter().map(json_led).collect();
    format!(
        "{{\"light\":{},\"device_id\":{},\"frame_start\":{},\"leds\":[{}]}}",
        json_string(&samples.light),
        json_string(&samples.device_id.0),
        samples.frame_start,
        leds.join(",")
    )
}

fn json_plan(plan: &GroupPlan) -> String {
    let lights: Vec<String> = plan.lights.iter().map(json_light_samples).collect();
    format!("{{\"lights\":[{}]}}", lights.join(","))
}

/// The whole file: the inputs and the plan each of them makes.
fn print_json() -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(
        "  \"note\": \"The Rust arrangement planner's output for the fixed inputs. Printed by crates/canvas-core/examples/arrangement_vectors.rs; tools/arrangement-vectors.ps1 writes this file. The editor's TypeScript planner (app/web/src/lib/arrangement.ts) is checked against it.\",\n",
    );
    out.push_str(&format!(
        "  \"canvas\": {{ \"width\": {CANVAS_WIDTH}, \"height\": {CANVAS_HEIGHT} }},\n"
    ));
    out.push_str("  \"cases\": [\n");
    let cases = cases();
    for (index, case) in cases.iter().enumerate() {
        let plan = build_group_plan(&case.members, case.mode, &case.lights, &case.devices);
        let members: Vec<String> = case.members.iter().map(|m| json_string(m)).collect();
        let lights: Vec<String> = case.lights.iter().map(json_light).collect();
        let devices: Vec<String> = case.devices.iter().map(json_device).collect();
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": {},\n", json_string(case.name)));
        out.push_str(&format!("      \"members\": [{}],\n", members.join(",")));
        out.push_str(&format!(
            "      \"mode\": {},\n",
            json_string(match case.mode {
                GroupMode::FlowAcross => "flow_across",
                GroupMode::EachItsOwn => "each_its_own",
            })
        ));
        out.push_str(&format!("      \"lights\": [{}],\n", lights.join(",")));
        out.push_str(&format!("      \"devices\": [{}],\n", devices.join(",")));
        out.push_str(&format!("      \"plan\": {}\n", json_plan(&plan)));
        out.push_str(if index + 1 == cases.len() { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

fn main() {
    let json = print_json();
    // With a path, the file is written there (tools/arrangement-vectors.ps1 does that);
    // without one, the JSON goes to the standard output.
    let mut args = std::env::args().skip(1);
    match args.next() {
        Some(path) => {
            if let Err(err) = std::fs::write(&path, &json) {
                eprintln!("arrangement_vectors: cannot write {path}: {err}");
                std::process::exit(1);
            }
            eprintln!("arrangement_vectors: wrote {path} ({} bytes)", json.len());
        }
        None => print!("{json}"),
    }
}
