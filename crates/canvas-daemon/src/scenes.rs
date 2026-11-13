//! Scene store operations: the lights the devices bring, the repair of a damaged
//! `scenes.json`, and the default scene.
//!
//! Ids are plain and deterministic, like the layout ids before them (`default`, then the owner's
//! own names): a slug of a name, with `-2`, `-3` … when it is taken. Nothing here draws a random
//! number, so a test can name the id it expects.

use std::collections::HashSet;

use canvas_proto::device::{DeviceInfo, ZoneInfo};
use canvas_proto::props::Props;
use canvas_proto::scene::{
    zone_light_id, Arrangement, Group, GroupMode, Light, LightId, LightsFile, Plays, Scene,
    ScenesFile, DEFAULT_GROUP_NAME, DEFAULT_SCENE_NAME, GROUP_BRIGHTNESS_MAX, GROUP_NAME_MAX,
    LIGHTS_SCHEMA, SCENES_SCHEMA, SCENE_HTML_MAX, SCENE_NAME_MAX, HTML_EFFECT_PREFIX,
};

/// What the default scene plays: the effect the first run turns on, so the lights are lit.
pub const DEFAULT_EFFECT: &str = crate::first_run::FIRST_RUN_EFFECT;

/// A lowercase, dash-separated id from a name: `"Living room"` becomes `living-room`. A name
/// with nothing to keep becomes `scene`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "scene".into()
    } else {
        out
    }
}

/// `base`, or `base-2`, `base-3` … the first one `taken` does not hold. Deterministic.
pub fn fresh_id<'a>(base: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: HashSet<&str> = taken.into_iter().collect();
    if !taken.contains(base) {
        return base.to_string();
    }
    (2u32..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken.contains(id.as_str()))
        .expect("a free id exists")
}

/// The one group the default scene holds: every light, in list order, flowing across. `None`
/// when there are no lights (a group needs a member).
pub fn everything_group(lights: &[Light]) -> Option<Group> {
    (!lights.is_empty()).then(|| Group {
        id: slug(DEFAULT_GROUP_NAME),
        name: DEFAULT_GROUP_NAME.into(),
        members: lights.iter().map(|l| l.id.clone()).collect(),
        plays: Plays::Effect { effect: DEFAULT_EFFECT.into(), props: Props::new() },
        mode: GroupMode::FlowAcross,
        brightness: GROUP_BRIGHTNESS_MAX,
    })
}

/// The scene made on a first run and when no scene is left: `Everything`, holding every light.
pub fn default_scene(lights: &[Light]) -> Scene {
    Scene {
        id: slug(DEFAULT_SCENE_NAME),
        name: DEFAULT_SCENE_NAME.into(),
        rev: 1,
        groups: everything_group(lights).into_iter().collect(),
    }
}

/// A scenes file of one default scene, active.
pub fn default_scenes(lights: &LightsFile) -> ScenesFile {
    let scene = default_scene(&lights.lights);
    ScenesFile { schema_version: SCENES_SCHEMA, active: scene.id.clone(), scenes: vec![scene] }
}

/// An empty lights file.
pub fn empty_lights() -> LightsFile {
    LightsFile { schema_version: LIGHTS_SCHEMA, lights: Vec::new() }
}

/// The light that is a whole zone, with the arrangement its kind starts as.
fn whole_zone_light(device: &DeviceInfo, zone: &ZoneInfo) -> Light {
    Light {
        id: zone_light_id(&device.id, zone.index),
        device_id: device.id.clone(),
        zone_index: zone.index,
        leds: None,
        name: None,
        arrangement: Arrangement::default_for(&zone.kind),
        reversed: false,
    }
}

/// Adds a light for every zone with `led_count > 0` that has none. Never removes a light.
/// Returns the ids added. New lights join no group.
///
/// A zone has a light when any light names its device and zone index, whole or a slice of it, so
/// a zone the owner split into parts is never given a second whole-zone light.
///
/// Only whole-zone lights are made here. A split-zone light (`…p{n}`) is not recreated: if
/// `lights.json` is lost, a split zone comes back as one whole-zone light, and the slices'
/// labels, arrangements and `reversed` flags are gone (the files hold the only copy).
pub fn sync_lights(lights: &mut LightsFile, devices: &[DeviceInfo]) -> Vec<LightId> {
    let mut added = Vec::new();
    for device in devices {
        for zone in device.zones.iter().filter(|z| z.led_count > 0) {
            let has = lights
                .lights
                .iter()
                .any(|l| l.device_id == device.id && l.zone_index == zone.index);
            if !has {
                let light = whole_zone_light(device, zone);
                added.push(light.id.clone());
                lights.lights.push(light);
            }
        }
    }
    added
}

/// `text` cut to `max` characters (not bytes), `true` when it was cut.
fn cut_chars(text: &mut String, max: usize) -> bool {
    match text.char_indices().nth(max) {
        Some((at, _)) => {
            text.truncate(at);
            true
        }
        None => false,
    }
}

/// Makes a loaded file safe: after it, every scene passes
/// [`canvas_proto::scene::validate_scene`] against `lights`. It cuts scene and group names to
/// their maximum (by characters), clamps brightness to 100, gives a scene id used twice a numeric
/// suffix (the first keeps it), gives a group id used twice one too, drops a light's second
/// membership (the FIRST group that names it keeps it), drops members that name no light, then
/// empty groups. It points `active` at the first scene if it names none, and makes the default
/// scene if there are no scenes. Returns what it changed, for the log.
///
/// One exception: while `lights` is empty (the lights file was lost, or never written) members
/// that name no light are kept, because [`sync_lights`] brings the whole-zone ids back from the
/// devices. Those scenes validate again once the lights are back. (A lost split-zone light
/// does not come back; see [`sync_lights`].)
pub fn repair(scenes: &mut ScenesFile, lights: &LightsFile) -> Vec<String> {
    let mut changes = Vec::new();
    let known: HashSet<&str> = lights.lights.iter().map(|l| l.id.as_str()).collect();
    let lights_lost = lights.lights.is_empty();
    let mut scene_ids: Vec<String> = Vec::new();
    for scene in &mut scenes.scenes {
        if scene_ids.contains(&scene.id) {
            let new = fresh_id(&scene.id, scene_ids.iter().map(String::as_str));
            changes.push(format!("scene id {} was used twice; renamed {new}", scene.id));
            scene.id = new;
        }
        scene_ids.push(scene.id.clone());
        if cut_chars(&mut scene.name, SCENE_NAME_MAX) {
            changes.push(format!("scene {}: its name was cut to {SCENE_NAME_MAX} characters", scene.id));
        }
        let mut held: HashSet<LightId> = HashSet::new();
        for group in &mut scene.groups {
            if cut_chars(&mut group.name, GROUP_NAME_MAX) {
                changes.push(format!(
                    "scene {}: group {} name was cut to {GROUP_NAME_MAX} characters",
                    scene.id, group.id
                ));
            }
            if group.brightness > GROUP_BRIGHTNESS_MAX {
                group.brightness = GROUP_BRIGHTNESS_MAX;
                changes.push(format!("scene {}: group {} brightness was clamped to {GROUP_BRIGHTNESS_MAX}", scene.id, group.id));
            }
            let (mut unknown, mut elsewhere, mut twice) = (0, 0, 0);
            let mut here: HashSet<LightId> = HashSet::new();
            let mut kept = Vec::with_capacity(group.members.len());
            for member in std::mem::take(&mut group.members) {
                if !lights_lost && !known.contains(member.as_str()) {
                    unknown += 1;
                } else if held.contains(&member) {
                    elsewhere += 1;
                } else if !here.insert(member.clone()) {
                    twice += 1;
                } else {
                    kept.push(member);
                }
            }
            held.extend(here);
            group.members = kept;
            for (n, what) in [
                (unknown, "that name no light"),
                (elsewhere, "already in another group"),
                (twice, "listed twice in the group"),
            ] {
                if n > 0 {
                    changes.push(format!("scene {}: group {} lost {n} light(s) {what}", scene.id, group.id));
                }
            }
        }
        let before = scene.groups.len();
        scene.groups.retain(|g| !g.members.is_empty());
        if scene.groups.len() != before {
            changes.push(format!("scene {}: dropped {} empty group(s)", scene.id, before - scene.groups.len()));
        }
        let mut seen: Vec<String> = Vec::new();
        for group in &mut scene.groups {
            if seen.contains(&group.id) {
                let new = fresh_id(&group.id, seen.iter().map(String::as_str));
                changes.push(format!("scene {}: group id {} was used twice; renamed {new}", scene.id, group.id));
                group.id = new;
            }
            seen.push(group.id.clone());
        }
        // At most SCENE_HTML_MAX groups play an HTML effect: later ones, in scene order, go Off.
        let mut html = 0;
        for group in &mut scene.groups {
            let is_html = matches!(&group.plays, Plays::Effect { effect, .. } if effect.starts_with(HTML_EFFECT_PREFIX));
            if is_html {
                html += 1;
                if html > SCENE_HTML_MAX {
                    group.plays = Plays::Off;
                    changes.push(format!(
                        "scene {}: group {} was set to Off (more than {SCENE_HTML_MAX} HTML effect groups)",
                        scene.id, group.id
                    ));
                }
            }
        }
    }
    if scenes.scenes.is_empty() {
        *scenes = default_scenes(lights);
        changes.push("there were no scenes; made the default scene".into());
    } else if !scenes.scenes.iter().any(|s| s.id == scenes.active) {
        scenes.active = scenes.scenes[0].id.clone();
        changes.push(format!("the active scene named nothing; it is now {}", scenes.active));
    }
    changes
}

/// The active scene. `repair` guarantees there is one; on a file that skipped it, the first.
///
/// # Panics
/// When the file holds no scene at all, which `repair` and the loaders never leave.
pub fn active_scene(scenes: &ScenesFile) -> &Scene {
    scenes
        .scenes
        .iter()
        .find(|s| s.id == scenes.active)
        .or_else(|| scenes.scenes.first())
        .expect("a scenes file always holds a scene")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use canvas_proto::device::{DeviceId, DeviceKind, ZoneKind};
    use canvas_proto::layout::LedSlice;

    pub(crate) fn zone(index: u32, kind: ZoneKind, leds: u32) -> ZoneInfo {
        ZoneInfo {
            index,
            name: format!("Zone {index}"),
            kind,
            led_count: leds,
            min_leds: 0,
            max_leds: 64,
            resizable: false,
        }
    }

    pub(crate) fn device(id: &str, adapter_index: u32, zones: Vec<ZoneInfo>) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId(id.into()),
            name: id.into(),
            vendor: "v".into(),
            kind: DeviceKind::Other,
            adapter: "openrgb".into(),
            adapter_index,
            serial: String::new(),
            location: String::new(),
            zones,
            connected: true,
        }
    }

    #[test]
    fn a_new_device_adds_lights_that_join_no_group() {
        let mut lights = empty_lights();
        let board = device("openrgb:board", 0, vec![zone(0, ZoneKind::Linear, 30), zone(1, ZoneKind::Single, 1)]);
        let mut scenes = default_scenes(&lights);
        assert!(scenes.scenes[0].groups.is_empty());
        let added = sync_lights(&mut lights, std::slice::from_ref(&board));
        assert_eq!(added, vec!["openrgb:board#z0".to_string(), "openrgb:board#z1".to_string()]);
        assert_eq!(lights.lights[0].arrangement, Arrangement::Strip);
        assert_eq!(lights.lights[1].arrangement, Arrangement::Single);
        // A second sync finds nothing new, and the new lights are in no group.
        assert!(sync_lights(&mut lights, std::slice::from_ref(&board)).is_empty());
        assert!(repair(&mut scenes, &lights).is_empty());
        assert_eq!(canvas_proto::scene::unassigned(active_scene(&scenes), &lights.lights).len(), 2);
    }

    #[test]
    fn a_zone_with_zero_leds_is_not_a_light() {
        let mut lights = empty_lights();
        let board = device("openrgb:board", 0, vec![zone(0, ZoneKind::Linear, 0), zone(1, ZoneKind::Linear, 5)]);
        assert_eq!(sync_lights(&mut lights, &[board]), vec!["openrgb:board#z1".to_string()]);
    }

    #[test]
    fn a_split_zone_gets_no_second_whole_zone_light() {
        let mut lights = empty_lights();
        let d = device("d", 0, vec![zone(0, ZoneKind::Linear, 30)]);
        let mut part = whole_zone_light(&d, &d.zones[0]);
        part.id = "d#z0p1".into();
        part.leds = Some(LedSlice { start: 0, count: 10 });
        lights.lights.push(part);
        assert!(sync_lights(&mut lights, &[d]).is_empty());
    }

    /// Review Focus 1: the device drops and returns with a new adapter index. Its lights keep
    /// their ids and their groups, and no light is added or lost.
    #[test]
    fn reconnect_keeps_group_membership() {
        let mut lights = empty_lights();
        let before = device("openrgb:board", 0, vec![zone(0, ZoneKind::Linear, 30)]);
        sync_lights(&mut lights, std::slice::from_ref(&before));
        let mut scenes = default_scenes(&lights);
        scenes.scenes[0].groups = everything_group(&lights.lights).into_iter().collect();
        let held = scenes.clone();

        // Gone: the device is not connected and lists no zone. Nothing is removed.
        let mut gone = before.clone();
        gone.connected = false;
        gone.zones.clear();
        assert!(sync_lights(&mut lights, &[gone]).is_empty());
        assert_eq!(lights.lights.len(), 1);

        // Back, on another adapter index: the same light, the same group.
        let mut back = before.clone();
        back.adapter_index = 7;
        assert!(sync_lights(&mut lights, &[back]).is_empty(), "no second light for the same zone");
        assert_eq!(lights.lights.len(), 1);
        assert_eq!(lights.lights[0].id, "openrgb:board#z0");
        assert!(repair(&mut scenes, &lights).is_empty(), "nothing to repair");
        assert_eq!(scenes, held, "the group still holds the light");
    }

    fn group(id: &str, members: &[&str]) -> Group {
        Group {
            id: id.into(),
            name: id.into(),
            members: members.iter().map(|m| m.to_string()).collect(),
            plays: Plays::Off,
            mode: GroupMode::FlowAcross,
            brightness: 100,
        }
    }

    /// Review Focus 5: a light in two groups, an empty group and an `active` naming nothing are
    /// repaired; a file with no scene gets the default one.
    #[test]
    fn damaged_scenes_file_still_lights_everything() {
        let mut lights = empty_lights();
        let board = device("b", 0, vec![zone(0, ZoneKind::Linear, 30), zone(1, ZoneKind::Linear, 20)]);
        sync_lights(&mut lights, &[board]);
        let mut scenes = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "nothing".into(),
            scenes: vec![Scene {
                id: "s".into(),
                name: "S".into(),
                rev: 3,
                groups: vec![group("a", &["b#z0"]), group("b", &["b#z0", "b#z1"]), group("empty", &[])],
            }],
        };
        let changes = repair(&mut scenes, &lights);
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert_eq!(scenes.active, "s");
        let s = active_scene(&scenes);
        canvas_proto::scene::validate_scene(s, &lights.lights).expect("the repaired scene is valid");
        // The FIRST group keeps the light both named; a repair that kept it in the wrong group
        // would leave `a` empty (and dropped) or `b` holding both.
        assert_eq!(s.groups.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(s.groups[0].members, vec!["b#z0".to_string()]);
        assert_eq!(s.groups[1].members, vec!["b#z1".to_string()]);
        assert_eq!(s.rev, 3, "repair keeps the revision");

        // No scene at all: the default scene, with every light in one group, playing something.
        let mut none = ScenesFile { schema_version: SCENES_SCHEMA, active: "x".into(), scenes: vec![] };
        repair(&mut none, &lights);
        let d = active_scene(&none);
        assert_eq!(d.name, DEFAULT_SCENE_NAME);
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].members.len(), 2);
        assert!(matches!(d.groups[0].plays, Plays::Effect { .. }));
    }

    /// A scene whose only fault is the one the test sets.
    fn scene_with(groups: Vec<Group>) -> ScenesFile {
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![Scene { id: "s".into(), name: "S".into(), rev: 1, groups }],
        }
    }

    fn two_lights() -> LightsFile {
        let mut lights = empty_lights();
        sync_lights(&mut lights, &[device("b", 0, vec![zone(0, ZoneKind::Linear, 30), zone(1, ZoneKind::Linear, 20)])]);
        lights
    }

    /// The FIRST group that names a light keeps it; the later group loses it.
    #[test]
    fn the_first_group_keeps_a_light_two_groups_hold() {
        let lights = two_lights();
        let mut scenes = scene_with(vec![group("a", &["b#z0"]), group("b", &["b#z0", "b#z1"])]);
        repair(&mut scenes, &lights);
        let groups = &scenes.scenes[0].groups;
        assert_eq!(groups[0].members, vec!["b#z0".to_string()]);
        assert_eq!(groups[1].members, vec!["b#z1".to_string()]);
    }

    /// A light listed twice inside one group is not "in another group".
    #[test]
    fn a_light_listed_twice_in_one_group_says_so() {
        let lights = two_lights();
        let mut scenes = scene_with(vec![group("a", &["b#z0", "b#z0", "b#z1"])]);
        let changes = repair(&mut scenes, &lights);
        assert_eq!(scenes.scenes[0].groups[0].members, vec!["b#z0".to_string(), "b#z1".to_string()]);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(changes[0].contains("twice"), "{changes:?}");
        assert!(!changes[0].contains("another group"), "{changes:?}");
    }

    /// With the lights known, a member that names no light is dropped, and its group with it when
    /// nothing is left (S2: the scene must pass `validate_scene`).
    #[test]
    fn members_naming_no_light_are_dropped_then_empty_groups() {
        let lights = two_lights();
        let mut scenes = scene_with(vec![group("a", &["gone", "b#z0"]), group("ghost", &["gone-too"])]);
        let changes = repair(&mut scenes, &lights);
        let groups = &scenes.scenes[0].groups;
        assert_eq!(groups.len(), 1, "{changes:?}");
        assert_eq!(groups[0].members, vec!["b#z0".to_string()]);
        canvas_proto::scene::validate_scene(&scenes.scenes[0], &lights.lights).expect("valid");
    }

    /// The one exception: with the lights file empty (lost), the members stay, because
    /// `sync_lights` brings the whole-zone ids back from the devices.
    #[test]
    fn members_stay_while_the_lights_file_is_empty() {
        let mut scenes = scene_with(vec![group("a", &["b#z0", "b#z1"])]);
        let changes = repair(&mut scenes, &empty_lights());
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(scenes.scenes[0].groups[0].members.len(), 2);
    }

    #[test]
    fn long_names_are_cut_and_brightness_is_clamped() {
        use canvas_proto::scene::{GROUP_NAME_MAX, SCENE_NAME_MAX};
        let lights = two_lights();
        let mut g = group("a", &["b#z0"]);
        g.name = "é".repeat(GROUP_NAME_MAX + 9);
        g.brightness = 255;
        let mut scenes = scene_with(vec![g]);
        scenes.scenes[0].name = "ü".repeat(SCENE_NAME_MAX + 5);
        repair(&mut scenes, &lights);
        assert_eq!(scenes.scenes[0].name.chars().count(), SCENE_NAME_MAX);
        assert_eq!(scenes.scenes[0].groups[0].name.chars().count(), GROUP_NAME_MAX);
        assert_eq!(scenes.scenes[0].groups[0].brightness, 100);
        canvas_proto::scene::validate_scene(&scenes.scenes[0], &lights.lights).expect("valid");
    }

    #[test]
    fn a_scene_id_used_twice_gets_a_suffix() {
        let lights = two_lights();
        let mut scenes = scene_with(vec![group("a", &["b#z0"])]);
        let twin = Scene { name: "Twin".into(), groups: vec![group("a", &["b#z1"])], ..scenes.scenes[0].clone() };
        scenes.scenes.push(twin);
        let changes = repair(&mut scenes, &lights);
        let ids: Vec<_> = scenes.scenes.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["s", "s-2"], "{changes:?}");
        assert_eq!(scenes.active, "s", "the first keeps the id the file's active names");
    }

    /// Property-style: damaged scenes built from a seeded generator all validate after repair,
    /// whatever mix of faults they hold.
    #[test]
    fn html_groups_over_the_limit_are_set_off_in_scene_order() {
        let ls = LightsFile {
            schema_version: LIGHTS_SCHEMA,
            lights: ["a", "b", "c"].iter().map(|id| Light {
                id: (*id).into(),
                device_id: canvas_proto::DeviceId("fake:1".into()),
                zone_index: 0,
                leds: None,
                name: None,
                arrangement: Arrangement::Strip,
                reversed: false,
            }).collect(),
        };
        let g = |id: &str, member: &str, effect: &str| Group {
            id: id.into(),
            name: id.into(),
            members: vec![member.into()],
            plays: Plays::Effect { effect: effect.into(), props: Default::default() },
            mode: GroupMode::FlowAcross,
            brightness: 100,
        };
        let mut file = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![Scene {
                id: "s".into(),
                name: "s".into(),
                rev: 1,
                groups: vec![g("g1", "a", "html:ink-drop"), g("g2", "b", "aurora"), g("g3", "c", "html:wet-smear")],
            }],
        };
        let changes = repair(&mut file, &ls);
        let plays: Vec<bool> = file.scenes[0].groups.iter().map(|g| g.plays == Plays::Off).collect();
        // With a limit of one, the first HTML group keeps playing and the second goes Off.
        let mut expect = vec![false, false, false];
        if SCENE_HTML_MAX < 2 {
            expect[2] = true;
        }
        assert_eq!(plays, expect);
        assert_eq!(changes.iter().filter(|c| c.contains("set to Off")).count(), expect.iter().filter(|x| **x).count());
        assert_eq!(canvas_proto::scene::validate_scene(&file.scenes[0], &ls.lights), Ok(()));
    }

    #[test]
    fn whatever_the_damage_the_repaired_scenes_validate() {
        let lights = two_lights();
        let pool = ["b#z0", "b#z1", "gone", "also-gone"];
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for round in 0..200 {
            let scene_count = 1 + next(3);
            let mut scenes = ScenesFile { schema_version: SCENES_SCHEMA, active: "nope".into(), scenes: Vec::new() };
            for _ in 0..scene_count {
                let mut groups = Vec::new();
                for _ in 0..next(4) {
                    let members: Vec<&str> = (0..next(4)).map(|_| pool[next(pool.len())]).collect();
                    let mut g = group(["a", "b"][next(2)], &members);
                    g.name = "n".repeat(next(90));
                    g.brightness = [0u8, 50, 100, 101, 255][next(5)];
                    groups.push(g);
                }
                scenes.scenes.push(Scene {
                    id: ["s", "t"][next(2)].into(),
                    name: "x".repeat(next(90)),
                    rev: 1,
                    groups,
                });
            }
            repair(&mut scenes, &lights);
            assert!(!scenes.scenes.is_empty());
            let mut ids = HashSet::new();
            for scene in &scenes.scenes {
                assert!(ids.insert(scene.id.clone()), "round {round}: scene id used twice");
                canvas_proto::scene::validate_scene(scene, &lights.lights)
                    .unwrap_or_else(|e| panic!("round {round}: {e} in {scene:?}"));
            }
            assert!(scenes.scenes.iter().any(|s| s.id == scenes.active), "round {round}");
        }
    }

    #[test]
    fn two_groups_with_one_id_are_told_apart() {
        let lights = empty_lights();
        let mut scenes = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![Scene { id: "s".into(), name: "S".into(), rev: 1, groups: vec![group("a", &["x"]), group("a", &["y"])] }],
        };
        assert_eq!(repair(&mut scenes, &lights).len(), 1);
        let ids: Vec<_> = scenes.scenes[0].groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "a-2"]);
    }

    #[test]
    fn ids_are_slugs_with_a_numeric_suffix_when_taken() {
        assert_eq!(slug("Living room!"), "living-room");
        assert_eq!(slug("  "), "scene");
        assert_eq!(fresh_id("a", ["b"]), "a");
        assert_eq!(fresh_id("a", ["a", "a-2"]), "a-3");
    }
}
