//! Prints the pattern fit rules' verdicts for fixed cases as JSON.
//!
//! The editor copies two of the daemon's fit rules:
//!
//! - `fit_kind` (`crates/canvas-proto/src/pattern.rs`), one light's fit on its own, copied in
//!   `app/web/src/lib/patternFit.ts` `fitKind`;
//! - `member_fits` (`crates/canvas-daemon/src/scenes.rs`), a group's members against a pattern,
//!   with and without a `sequence`, copied in `patternFit.ts` `lightFits`/`groupFit`.
//!
//! The two copies are tied together by `app/web/src/lib/__tests__/patternFit.vectors.json`:
//! this printer writes the file, the editor's test runs both TypeScript rules over every row,
//! and `crates/canvas-daemon/tests/pattern_fit_vectors.rs` fails when the checked-in file is no
//! longer what the Rust rules make.
//!
//! The file also carries the 4096-LED cap (`canvas_core::arrangement::MAX_ZONE_LEDS`), because
//! a change to it on one side alone would move no case's verdict and pass unseen.
//!
//! Each run case's members also carry the count `GET /api/v1/lights` answers
//! (`crates/canvas-daemon/src/api/lights.rs` `view`), which is the count the editor's fit marks
//! read. `view` asks `canvas_core::arrangement::light_led_count`, the same function `member_fits`
//! uses, so the file records one rule and not two; the rows with a stored slice (`Light::leds`)
//! are the branch a one-sided change would otherwise pass unseen (#504, r-516).
//!
//! Print it and write the file with:
//! `powershell -NoProfile -File tools/pattern-fit-vectors.ps1`
//!
//! The JSON is written by hand (no serde here), as in the drive vectors printer: it is a small,
//! fixed set of cases and their verdicts, and this keeps the printer's dependencies as they are.

use canvas_core::arrangement::MAX_ZONE_LEDS;
use canvas_daemon::scenes::member_fits;
use canvas_proto::api::LightView;
use canvas_proto::device::{DeviceInfo, DeviceKind, ZoneInfo, ZoneKind};
use canvas_proto::layout::LedSlice;
use canvas_proto::pattern::{fit_kind, BuiltOn, FitKind, GridSize};
use canvas_proto::scene::{Arrangement, Light, LightId, SequenceShape};
use canvas_proto::DeviceId;

/// A pattern of `led_count` LEDs laid out as `arrangement`, no cell map.
fn on(led_count: u32, arrangement: Arrangement) -> BuiltOn {
    BuiltOn { led_count, arrangement, grid: None }
}

/// A grid pattern of `width` by `height` cells.
fn on_grid(width: u32, height: u32) -> BuiltOn {
    BuiltOn {
        led_count: width * height,
        arrangement: Arrangement::Grid,
        grid: Some(GridSize { width, height }),
    }
}

/// One single-light fit case: a pattern, and the light it lands on.
struct FitCase {
    /// What the case shows.
    name: &'static str,
    /// Where the case comes from, on the Rust side.
    from: &'static str,
    /// The pattern's own shape.
    built_on: BuiltOn,
    /// The light's LEDs.
    target_leds: u32,
    /// The light's arrangement.
    target: Arrangement,
}

/// One group member: the light, its arrangement, the zone behind it, and the run it owns.
struct Member {
    /// The light's id, and the case's name for it.
    light: &'static str,
    /// The light's arrangement, which the per-light fit reads.
    arrangement: Arrangement,
    /// The zone's LED count. `None` for a device that is gone: the light is listed with no LEDs.
    zone_leds: Option<u32>,
    /// The run of the zone this light owns (`Light::leds`): its start and count. `None` owns the
    /// whole zone; a slice that runs past the end of a zone that shrank keeps only what is left.
    leds: Option<(u32, u32)>,
    /// Whether the device is listed as connected.
    connected: bool,
}

/// One run case: a pattern, a group's members and the group's shape.
struct RunCase {
    /// What the case shows.
    name: &'static str,
    /// Where the case comes from, on the Rust side.
    from: &'static str,
    /// The pattern's own shape.
    built_on: BuiltOn,
    /// The group's shape: `None` is Each light its own.
    sequence: Option<SequenceShape>,
    /// The group's members, in the group's order.
    members: Vec<Member>,
}

/// The single-light cases: the same set `fit_kind_follows_the_rule_order` and the
/// `pattern_fit.rs` fit tests assert, written down once here.
fn fit_cases() -> Vec<FitCase> {
    vec![
        FitCase {
            name: "a ring of 12 on a ring of 12",
            from: "pattern.rs fit_kind_follows_the_rule_order",
            built_on: on(12, Arrangement::Ring),
            target_leds: 12,
            target: Arrangement::Ring,
        },
        FitCase {
            name: "a ring of 12 on a ring of 24",
            from: "pattern.rs fit_kind_follows_the_rule_order, pattern_fit.rs a_ring_stretched_to_double_keeps_the_top_led_at_the_top",
            built_on: on(12, Arrangement::Ring),
            target_leds: 24,
            target: Arrangement::Ring,
        },
        FitCase {
            name: "a ring of 12 on a strip of 12",
            from: "pattern.rs fit_kind_follows_the_rule_order",
            built_on: on(12, Arrangement::Ring),
            target_leds: 12,
            target: Arrangement::Strip,
        },
        FitCase {
            name: "a ring of 12 on a single LED",
            from: "pattern.rs fit_kind_follows_the_rule_order, pattern_fit.rs a_single_led_takes_the_first_colour_that_is_on",
            built_on: on(12, Arrangement::Ring),
            target_leds: 1,
            target: Arrangement::Single,
        },
        FitCase {
            name: "a single LED on a single LED",
            from: "pattern.rs fit_kind_follows_the_rule_order",
            built_on: on(1, Arrangement::Single),
            target_leds: 1,
            target: Arrangement::Single,
        },
        FitCase {
            name: "a ring of 12 on a ring of one LED",
            from: "pattern.rs fit_kind_follows_the_rule_order, pattern_fit.rs a_single_led_takes_the_first_colour_that_is_on",
            built_on: on(12, Arrangement::Ring),
            target_leds: 1,
            target: Arrangement::Ring,
        },
        FitCase {
            name: "a ring of 24 on a ring of 12",
            from: "pattern_fit.rs a_ring_stretched_to_half_reads_every_second_led",
            built_on: on(24, Arrangement::Ring),
            target_leds: 12,
            target: Arrangement::Ring,
        },
        FitCase {
            name: "a strip of 10 on a strip of 25",
            from: "pattern_fit.rs a_strip_stretch_maps_first_to_first_and_last_to_last",
            built_on: on(10, Arrangement::Strip),
            target_leds: 25,
            target: Arrangement::Strip,
        },
        FitCase {
            name: "a strip of 25 on a strip of 10",
            from: "pattern_fit.rs a_strip_stretch_maps_first_to_first_and_last_to_last",
            built_on: on(25, Arrangement::Strip),
            target_leds: 10,
            target: Arrangement::Strip,
        },
        FitCase {
            name: "a strip of 60 on a ring of 60",
            from: "pattern_fit.rs a_different_arrangement_stretches_by_led_order",
            built_on: on(60, Arrangement::Strip),
            target_leds: 60,
            target: Arrangement::Ring,
        },
        FitCase {
            name: "a grid of 5 by 4 on a grid of 20",
            from: "pattern.rs fit_kind (a grid is its count and arrangement; the cell map is not read)",
            built_on: on_grid(5, 4),
            target_leds: 20,
            target: Arrangement::Grid,
        },
        FitCase {
            name: "a grid of 5 by 4 on a grid of 24",
            from: "pattern.rs fit_kind (a grid stretched by LED order)",
            built_on: on_grid(5, 4),
            target_leds: 24,
            target: Arrangement::Grid,
        },
        FitCase {
            name: "a grid of 5 by 4 on a strip of 20",
            from: "pattern.rs fit_kind",
            built_on: on_grid(5, 4),
            target_leds: 20,
            target: Arrangement::Strip,
        },
        FitCase {
            name: "a grid of 5 by 4 on a grid of 2 by 10 with 20 LEDs",
            from: "pattern.rs fit_kind reads the count and the arrangement, never the cell map",
            built_on: on_grid(5, 4),
            target_leds: 20,
            target: Arrangement::Grid,
        },
        FitCase {
            name: "a strip of 30 on a single LED",
            from: "pattern.rs fit_kind",
            built_on: on(30, Arrangement::Strip),
            target_leds: 1,
            target: Arrangement::Single,
        },
        FitCase {
            name: "a single LED on a strip of 30",
            from: "pattern_fit.rs a_one_led_source_fills_every_led (the count differs, so the arrangement decides)",
            built_on: on(1, Arrangement::Single),
            target_leds: 30,
            target: Arrangement::Strip,
        },
        FitCase {
            name: "a ring of 300 on a strip of 300",
            from: "pattern_fit.rs fit_colors_agrees_with_fit_kind_on_when_it_copies",
            built_on: on(300, Arrangement::Ring),
            target_leds: 300,
            target: Arrangement::Strip,
        },
        FitCase {
            name: "a ring of 2 on a ring of 3",
            from: "pattern_fit.rs the_output_length_is_always_target_leds",
            built_on: on(2, Arrangement::Ring),
            target_leds: 3,
            target: Arrangement::Ring,
        },
        FitCase {
            name: "a ring of 4096 on a ring of 4096",
            from: "pattern_fit.rs a_4096_led_ring_stretches_without_overflow",
            built_on: on(4096, Arrangement::Ring),
            target_leds: 4096,
            target: Arrangement::Ring,
        },
        FitCase {
            name: "a ring of 4096 on a ring of 4095",
            from: "pattern_fit.rs a_4096_led_ring_stretches_without_overflow",
            built_on: on(4096, Arrangement::Ring),
            target_leds: 4095,
            target: Arrangement::Ring,
        },
        FitCase {
            name: "a ring of 12 on a grid of 20",
            from: "pattern_fit.rs an_all_off_pattern_is_all_off (a ring onto a grid is another shape)",
            built_on: on(12, Arrangement::Ring),
            target_leds: 20,
            target: Arrangement::Grid,
        },
    ]
}

/// One member that owns its whole zone, connected.
fn whole(light: &'static str, arrangement: Arrangement, zone_leds: Option<u32>) -> Member {
    Member { light, arrangement, zone_leds, leds: None, connected: true }
}

/// One member of the three-light group the run cases share: 8, 8 and 10 LEDs.
fn three() -> Vec<Member> {
    vec![
        whole("fan one", Arrangement::Ring, Some(8)),
        whole("fan two", Arrangement::Strip, Some(8)),
        whole("strip", Arrangement::Strip, Some(10)),
    ]
}

/// The run cases: what `group_fits` says for a group, which is what the editor's note and the
/// picker's mark show.
fn run_cases() -> Vec<RunCase> {
    vec![
        RunCase {
            name: "no sequence: each light plays the pattern on its own",
            from: "scenes.rs member_fits (the no-sequence arm), pattern.rs fit_kind",
            built_on: on(26, Arrangement::Ring),
            sequence: None,
            members: three(),
        },
        RunCase {
            name: "one ring: 8 + 8 + 10 LEDs are exactly the pattern's 26",
            from: "scenes.rs member_fits (a sequence), patterns.rs a_sequence_group_reports_the_fit_of_the_whole_run",
            built_on: on(26, Arrangement::Ring),
            sequence: Some(SequenceShape::Ring),
            members: three(),
        },
        RunCase {
            name: "one line: the same 26 LEDs in the shape the pattern was built on",
            from: "scenes.rs member_fits (a sequence runs a line as a strip)",
            built_on: on(26, Arrangement::Strip),
            sequence: Some(SequenceShape::Line),
            members: three(),
        },
        RunCase {
            name: "one ring under the pattern: 8 + 8 is 16 of a ring of 26",
            from: "patterns.rs a_sequence_group_reports_the_fit_of_the_whole_run (two fans)",
            built_on: on(26, Arrangement::Ring),
            sequence: Some(SequenceShape::Ring),
            members: three().into_iter().take(2).collect(),
        },
        RunCase {
            name: "one ring over the pattern: 8 + 8 + 10 + 10 is 36 of a ring of 26",
            from: "scenes.rs member_fits (the run sums every member's LEDs)",
            built_on: on(26, Arrangement::Ring),
            sequence: Some(SequenceShape::Ring),
            members: {
                let mut members = three();
                members.push(whole("fan three", Arrangement::Ring, Some(10)));
                members
            },
        },
        RunCase {
            name: "a member with no LEDs is not in the run, and not in the fits",
            from: "scenes.rs member_fits (light_led_count is 0 for an empty zone)",
            built_on: on(16, Arrangement::Ring),
            sequence: Some(SequenceShape::Ring),
            members: vec![
                whole("fan one", Arrangement::Ring, Some(8)),
                whole("empty zone", Arrangement::Ring, Some(0)),
                whole("fan two", Arrangement::Ring, Some(8)),
            ],
        },
        RunCase {
            name: "a member whose device is not listed is not in the run either",
            from: "scenes.rs member_fits (a member the daemon does not hold has no LEDs)",
            built_on: on(8, Arrangement::Ring),
            sequence: Some(SequenceShape::Ring),
            members: vec![
                whole("fan one", Arrangement::Ring, Some(8)),
                Member {
                    light: "gone",
                    arrangement: Arrangement::Ring,
                    zone_leds: None,
                    leds: None,
                    connected: false,
                },
            ],
        },
        RunCase {
            name: "a disconnected member keeps its LEDs in the run",
            from: "scenes.rs member_fits, canvas-core arrangement.rs light_led_count (asks only for the zone)",
            built_on: on(16, Arrangement::Ring),
            sequence: Some(SequenceShape::Ring),
            members: vec![
                whole("fan one", Arrangement::Ring, Some(8)),
                Member {
                    light: "fan two",
                    arrangement: Arrangement::Ring,
                    zone_leds: Some(8),
                    leds: None,
                    connected: false,
                },
            ],
        },
        RunCase {
            name: "a zone past the 4096-LED cap is planned as the cap, in a run",
            from: "scenes.rs member_fits, arrangement.rs light_led_count (count.min(MAX_ZONE_LEDS))",
            built_on: on(MAX_ZONE_LEDS, Arrangement::Ring),
            sequence: Some(SequenceShape::Ring),
            members: vec![whole("damaged zone", Arrangement::Ring, Some(MAX_ZONE_LEDS + 904))],
        },
        RunCase {
            name: "a zone past the 4096-LED cap is planned as the cap, each light its own",
            from: "scenes.rs member_fits (the per-light arm), arrangement.rs light_led_count",
            built_on: on(MAX_ZONE_LEDS, Arrangement::Ring),
            sequence: None,
            members: vec![whole("damaged zone", Arrangement::Ring, Some(MAX_ZONE_LEDS + 904))],
        },
        RunCase {
            name: "a sliced member counts only the LEDs it keeps, in a run",
            from: "arrangement.rs light_led_count (slice.count.min(zone - start)), api/lights.rs view",
            built_on: on(12, Arrangement::Ring),
            sequence: Some(SequenceShape::Ring),
            members: vec![
                Member {
                    light: "sliced fan",
                    arrangement: Arrangement::Ring,
                    zone_leds: Some(12),
                    leds: Some((8, 8)),
                    connected: true,
                },
                whole("fan two", Arrangement::Ring, Some(8)),
            ],
        },
        RunCase {
            name: "a sliced member's own fit is its kept LEDs, each light its own",
            from: "arrangement.rs light_led_count, scenes.rs member_fits (the per-light arm)",
            built_on: on(4, Arrangement::Ring),
            sequence: None,
            members: vec![
                Member {
                    light: "sliced strip",
                    arrangement: Arrangement::Ring,
                    zone_leds: Some(12),
                    leds: Some((8, 8)),
                    connected: true,
                },
                whole("whole zone", Arrangement::Ring, Some(8)),
            ],
        },
        RunCase {
            name: "a slice that starts past the end of a shrunken zone has no LEDs, and leaves the run",
            from: "arrangement.rs light_led_count (zone.saturating_sub(start)), api/lights.rs view",
            built_on: on(8, Arrangement::Ring),
            sequence: Some(SequenceShape::Ring),
            members: vec![
                whole("fan one", Arrangement::Ring, Some(8)),
                Member {
                    light: "stale slice",
                    arrangement: Arrangement::Ring,
                    zone_leds: Some(10),
                    leds: Some((20, 4)),
                    connected: true,
                },
            ],
        },
    ]
}

/// A JSON string.
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

/// The snake_case name the file and the editor use for an arrangement.
fn arrangement_name(arrangement: Arrangement) -> &'static str {
    match arrangement {
        Arrangement::Single => "single",
        Arrangement::Strip => "strip",
        Arrangement::Ring => "ring",
        Arrangement::Grid => "grid",
    }
}

/// The snake_case name the file and the editor use for a fit.
fn fit_name(fit: FitKind) -> &'static str {
    match fit {
        FitKind::Exact => "exact",
        FitKind::Stretched => "stretched",
        FitKind::OtherShape => "other_shape",
        FitKind::OneLed => "one_led",
    }
}

/// The `built_on` object, on one line, as the file writes it.
fn built_on_json(built_on: &BuiltOn) -> String {
    let grid = match &built_on.grid {
        Some(grid) => format!(", \"grid\": {{ \"width\": {}, \"height\": {} }}", grid.width, grid.height),
        None => String::new(),
    };
    format!(
        "{{ \"led_count\": {}, \"arrangement\": \"{}\"{grid} }}",
        built_on.led_count,
        arrangement_name(built_on.arrangement)
    )
}

/// The light a member names, as `lights.json` holds it.
fn light_of(member: &Member) -> Light {
    Light {
        id: member.light.to_string(),
        device_id: DeviceId(format!("openrgb:{}", member.light)),
        zone_index: 0,
        leds: member.leds.map(|(start, count)| LedSlice { start, count }),
        name: None,
        arrangement: member.arrangement,
        reversed: false,
        kind: None,
        copies: None,
        first_led: None,
    }
}

/// The device behind a member, or `None` for a light whose device is not listed.
fn device_of(member: &Member) -> Option<DeviceInfo> {
    let led_count = member.zone_leds?;
    Some(DeviceInfo {
        id: DeviceId(format!("openrgb:{}", member.light)),
        name: member.light.to_string(),
        vendor: "vectors".to_string(),
        kind: DeviceKind::Motherboard,
        adapter: "openrgb".to_string(),
        adapter_index: 0,
        serial: String::new(),
        location: String::new(),
        zones: vec![ZoneInfo {
            index: 0,
            name: "Zone".to_string(),
            kind: ZoneKind::Linear,
            led_count,
            min_leds: led_count,
            max_leds: led_count,
            resizable: false,
        }],
        connected: member.connected,
    })
}

/// The whole file: every fixed case and the verdict the daemon's rule gives it.
///
/// Public so the parity test (`crates/canvas-daemon/tests/pattern_fit_vectors.rs`) can ask the
/// printer what it makes today and compare it with the checked-in file.
pub fn print_json() -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(
        "  \"rule\": \"crates/canvas-proto/src/pattern.rs fit_kind: the same LED count and arrangement is exact; a light of one LED is one_led; the same arrangement is stretched; anything else is other_shape. The order matters: a light of one LED is one_led even when its arrangement differs, and a pattern built on one LED of another arrangement is other_shape.\",\n",
    );
    out.push_str("  \"source\": [\n");
    out.push_str("    \"crates/canvas-proto/src/pattern.rs, fit_kind_follows_the_rule_order\",\n");
    out.push_str("    \"crates/canvas-core/src/pattern_fit.rs, the fit_colors tests (each asserts a fit kind by what the colours do)\",\n");
    out.push_str("    \"crates/canvas-daemon/src/scenes.rs, member_fits (a sequence group's run, a light with no LEDs)\",\n");
    out.push_str("    \"crates/canvas-core/src/arrangement.rs, light_led_count (a slice clamped to its zone, capped at zone_led_cap), and crates/canvas-daemon/src/api/lights.rs, view (led_count is that same call)\"\n");
    out.push_str("  ],\n");
    out.push_str(&format!("  \"zone_led_cap\": {MAX_ZONE_LEDS},\n"));
    out.push_str("  \"cases\": [\n");
    let cases = fit_cases();
    for (index, case) in cases.iter().enumerate() {
        let fit = fit_kind(&case.built_on, case.target_leds, case.target);
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": {},\n", json_string(case.name)));
        out.push_str(&format!("      \"from\": {},\n", json_string(case.from)));
        out.push_str(&format!("      \"built_on\": {},\n", built_on_json(&case.built_on)));
        out.push_str(&format!("      \"target_leds\": {},\n", case.target_leds));
        out.push_str(&format!("      \"target\": \"{}\",\n", arrangement_name(case.target)));
        out.push_str(&format!("      \"fit\": \"{}\"\n", fit_name(fit)));
        out.push_str(if index + 1 == cases.len() { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ],\n");
    out.push_str("  \"run_cases\": [\n");
    let runs = run_cases();
    for (index, case) in runs.iter().enumerate() {
        let lights: Vec<Light> = case.members.iter().map(light_of).collect();
        let devices: Vec<DeviceInfo> = case.members.iter().filter_map(device_of).collect();
        let members: Vec<LightId> = case.members.iter().map(|m| m.light.to_string()).collect();
        // The count `GET /api/v1/lights` answers, from the API's own view: the number the editor's
        // fit marks read. `member_fits` below asks `light_led_count` itself, so a change to one
        // side alone shows up in this file.
        let views: Vec<LightView> = lights
            .iter()
            .map(|light| canvas_daemon::api::lights::view(light, &devices))
            .collect();
        let fits = member_fits(&case.built_on, &members, case.sequence, &lights, &devices);
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": {},\n", json_string(case.name)));
        out.push_str(&format!("      \"from\": {},\n", json_string(case.from)));
        out.push_str(&format!("      \"built_on\": {},\n", built_on_json(&case.built_on)));
        let sequence = match case.sequence {
            None => "null".to_string(),
            Some(shape) => format!(
                "\"{}\"",
                match shape {
                    SequenceShape::Ring => "ring",
                    SequenceShape::Line => "line",
                }
            ),
        };
        out.push_str(&format!("      \"sequence\": {sequence},\n"));
        out.push_str("      \"members\": [\n");
        for (n, member) in case.members.iter().enumerate() {
            let zone = match member.zone_leds {
                Some(count) => count.to_string(),
                None => "null".to_string(),
            };
            let run = match member.leds {
                Some((start, count)) => format!("{{ \"start\": {start}, \"count\": {count} }}"),
                None => "null".to_string(),
            };
            out.push_str(&format!(
                "        {{ \"light\": {}, \"arrangement\": \"{}\", \"zone_leds\": {zone}, \"leds\": {run}, \"led_count\": {}, \"connected\": {} }}",
                json_string(member.light),
                arrangement_name(member.arrangement),
                views[n].led_count,
                views[n].connected
            ));
            out.push_str(if n + 1 == case.members.len() { "\n" } else { ",\n" });
        }
        out.push_str("      ],\n");
        out.push_str("      \"fits\": [\n");
        for (n, fit) in fits.iter().enumerate() {
            out.push_str(&format!(
                "        {{ \"light\": {}, \"fit\": \"{}\" }}",
                json_string(&fit.light),
                fit_name(fit.fit)
            ));
            out.push_str(if n + 1 == fits.len() { "\n" } else { ",\n" });
        }
        out.push_str("      ]\n");
        out.push_str(if index + 1 == runs.len() { "    }\n" } else { "    },\n" });
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

fn main() {
    let json = print_json();
    // With a path, the file is written there (tools/pattern-fit-vectors.ps1 does that);
    // without one, the JSON goes to the standard output.
    let mut args = std::env::args().skip(1);
    match args.next() {
        Some(path) => {
            if let Err(err) = std::fs::write(&path, &json) {
                eprintln!("pattern_fit_vectors: cannot write {path}: {err}");
                std::process::exit(1);
            }
            eprintln!("pattern_fit_vectors: wrote {path} ({} bytes)", json.len());
        }
        None => print!("{json}"),
    }
}
