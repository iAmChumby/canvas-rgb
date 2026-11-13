//! `layouts.json` to lights and scenes, once (spec "Storage and migration").
//!
//! [`migrate`] is pure. [`run`] is what a start does when `scenes.json` is absent: it takes a
//! snapshot, then writes `lights.json` and `scenes.json`. Nothing here writes, renames or
//! deletes `layouts.json`: 0.14.0 never touches it, and a reinstalled 0.13 finds it as it left it.

use std::collections::HashMap;
use std::path::Path;

use canvas_proto::config::LayoutsFile;
use canvas_proto::device::{DeviceInfo, ZoneKind};
use canvas_proto::layout::{Layout, PlacedZone, ZoneShape};
use canvas_proto::scene::{
    zone_light_id, Arrangement, Group, GroupMode, Light, LightId, LightsFile, Plays, Scene,
    ScenesFile, DEFAULT_GROUP_NAME, GROUP_BRIGHTNESS_MAX, LIGHTS_SCHEMA, SCENES_SCHEMA,
    SCENE_NAME_MAX,
};

use crate::scenes::{default_scenes, empty_lights, fresh_id, slug, sync_lights};

/// A light's identity while the lights are being collected: device, zone and the run of it.
type Key = (String, u32, Option<(u32, u32)>);

fn key_of(placed: &PlacedZone) -> Key {
    (
        placed.device_id.0.clone(),
        placed.zone_index,
        placed.leds.map(|s| (s.start, s.count)),
    )
}

/// The zone a placement names, when the device list has it.
fn zone_of<'a>(placed: &PlacedZone, devices: &'a [DeviceInfo]) -> Option<&'a canvas_proto::device::ZoneInfo> {
    devices
        .iter()
        .find(|d| d.id == placed.device_id)
        .and_then(|d| d.zones.iter().find(|z| z.index == placed.zone_index))
}

/// The arrangement a placement becomes: a ring stays a ring; a matrix zone is a grid; a single
/// zone is a single; everything else is a strip. A device list that does not hold the zone
/// (nothing is connected at the moment of the migration) leaves the placement's own shape to
/// decide: a grid shape is a grid, a point is a single.
fn arrangement_of(placed: &PlacedZone, devices: &[DeviceInfo]) -> Arrangement {
    if placed.shape == ZoneShape::Ring {
        return Arrangement::Ring;
    }
    match zone_of(placed, devices).map(|z| &z.kind) {
        Some(ZoneKind::Matrix { .. }) => return Arrangement::Grid,
        Some(ZoneKind::Single) => return Arrangement::Single,
        _ => {}
    }
    match placed.shape {
        ZoneShape::Grid { .. } => Arrangement::Grid,
        ZoneShape::Point => Arrangement::Single,
        _ => Arrangement::Strip,
    }
}

/// A placement with no LEDs to light is not a light: an empty slice, or a zone the device list
/// says has none.
fn has_leds(placed: &PlacedZone, devices: &[DeviceInfo]) -> bool {
    if placed.leds.is_some_and(|s| s.count == 0) {
        return false;
    }
    zone_of(placed, devices).is_none_or(|z| z.led_count > 0)
}

/// Builds the hardware map and the scenes from the saved layouts. Pure. `None` layouts means
/// first run: one scene, `Everything`, from the detected devices.
pub fn migrate(layouts: Option<&LayoutsFile>, devices: &[DeviceInfo]) -> (LightsFile, ScenesFile) {
    let Some(layouts) = layouts else {
        let mut lights = empty_lights();
        sync_lights(&mut lights, devices);
        let scenes = default_scenes(&lights);
        return (lights, scenes);
    };

    // The active layout's placements first, then the zones only other layouts placed.
    let mut ordered: Vec<&Layout> = layouts.layouts.iter().filter(|l| l.id == layouts.active).collect();
    ordered.extend(layouts.layouts.iter().filter(|l| l.id != layouts.active));

    let mut first_seen: Vec<(Key, &PlacedZone)> = Vec::new();
    for layout in &ordered {
        for placed in layout.zones.iter().filter(|p| has_leds(p, devices)) {
            let key = key_of(placed);
            if !first_seen.iter().any(|(k, _)| *k == key) {
                first_seen.push((key, placed));
            }
        }
    }

    // A zone's slices are numbered by where they start, from 1, whatever order they were placed in.
    let mut slice_starts: HashMap<(String, u32), Vec<(u32, u32)>> = HashMap::new();
    for ((device, zone, slice), _) in &first_seen {
        if let Some(slice) = slice {
            slice_starts.entry((device.clone(), *zone)).or_default().push(*slice);
        }
    }
    for slices in slice_starts.values_mut() {
        slices.sort_unstable();
    }

    let mut ids: HashMap<Key, LightId> = HashMap::new();
    let mut lights = LightsFile { schema_version: LIGHTS_SCHEMA, lights: Vec::new() };
    for (key, placed) in &first_seen {
        let base = zone_light_id(&placed.device_id, placed.zone_index);
        let id = match key.2 {
            None => base,
            Some(slice) => {
                let n = slice_starts[&(key.0.clone(), key.1)]
                    .iter()
                    .position(|s| *s == slice)
                    .map_or(1, |i| i + 1);
                format!("{base}p{n}")
            }
        };
        ids.insert(key.clone(), id.clone());
        lights.lights.push(Light {
            id,
            device_id: placed.device_id.clone(),
            zone_index: placed.zone_index,
            leds: placed.leds,
            name: placed.label.clone(),
            arrangement: arrangement_of(placed, devices),
            reversed: placed.reversed,
        });
    }

    let mut scenes: Vec<Scene> = Vec::new();
    for layout in &layouts.layouts {
        let mut members: Vec<LightId> = Vec::new();
        for placed in layout.zones.iter().filter(|p| p.visible) {
            if let Some(id) = ids.get(&key_of(placed)) {
                if !members.contains(id) {
                    members.push(id.clone());
                }
            }
        }
        let plays = match &layout.effect {
            Some(effect) => Plays::Effect { effect: effect.id.clone(), props: effect.props.clone() },
            None => Plays::Off,
        };
        // A group needs a member, so a layout with nothing visible is a scene with no group.
        let groups = if members.is_empty() {
            Vec::new()
        } else {
            vec![Group {
                id: slug(DEFAULT_GROUP_NAME),
                name: DEFAULT_GROUP_NAME.into(),
                members,
                plays,
                mode: GroupMode::FlowAcross,
                brightness: GROUP_BRIGHTNESS_MAX,
            }]
        };
        scenes.push(Scene {
            id: fresh_id(&layout.id, scenes.iter().map(|s| s.id.as_str())),
            name: layout.name.chars().take(SCENE_NAME_MAX).collect(),
            rev: 1,
            groups,
        });
    }

    if scenes.is_empty() {
        return (lights.clone(), default_scenes(&lights));
    }
    let active = if scenes.iter().any(|s| s.id == layouts.active) {
        layouts.active.clone()
    } else {
        scenes[0].id.clone()
    };
    (lights, ScenesFile { schema_version: SCENES_SCHEMA, active, scenes })
}

/// What a start does when `scenes.json` is absent and `layouts.json` was read: snapshot the data
/// folder as `before-scenes`, migrate, and write `lights.json` and `scenes.json`. The returned
/// files are what is on disk. Never writes `layouts.json`.
pub fn run(
    home: &Path,
    layouts: &LayoutsFile,
    devices: &[DeviceInfo],
) -> (LightsFile, ScenesFile) {
    crate::logging::warn_if_err(
        "the snapshot before the scenes migration could not be taken; it goes on without one",
        crate::storage::snapshot(home, "before-scenes"),
    );
    let (lights, scenes) = migrate(Some(layouts), devices);
    crate::logging::warn_if_err(
        "lights.json could not be written",
        crate::storage::write_json(&home.join(canvas_proto::scene::LIGHTS_FILE), &lights),
    );
    crate::logging::warn_if_err(
        "scenes.json could not be written",
        crate::storage::write_json(&home.join(canvas_proto::scene::SCENES_FILE), &scenes),
    );
    tracing::info!(
        layouts = layouts.layouts.len(),
        lights = lights.lights.len(),
        scenes = scenes.scenes.len(),
        "the saved layouts were migrated to lights and scenes"
    );
    (lights, scenes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::tests::{device, zone};
    use crate::storage::Store;
    use canvas_proto::device::{DeviceId, ZoneKind};
    use canvas_proto::layout::{ActiveEffect, CaseForm, LedSlice};
    use canvas_proto::props::Props;

    fn placed(device: &str, zone: u32) -> PlacedZone {
        PlacedZone {
            device_id: DeviceId(device.into()),
            zone_index: zone,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rotation_deg: 0.0,
            reversed: false,
            visible: true,
            shape: ZoneShape::Auto,
            label: None,
            leds: None,
            part: None,
            locked: false,
        }
    }

    fn layout(id: &str, name: &str, zones: Vec<PlacedZone>, effect: Option<&str>) -> Layout {
        Layout {
            id: id.into(),
            name: name.into(),
            render_scale: Default::default(),
            sampling: Default::default(),
            zones,
            effect: effect.map(|e| ActiveEffect { id: e.into(), props: Props::new() }),
            case_form: CaseForm::None,
            landmarks: Vec::new(),
        }
    }

    fn file(active: &str, layouts: Vec<Layout>) -> LayoutsFile {
        LayoutsFile { schema_version: canvas_proto::config::LAYOUTS_SCHEMA, active: active.into(), layouts }
    }

    fn board() -> DeviceInfo {
        device(
            "openrgb:board",
            0,
            vec![zone(0, ZoneKind::Linear, 30), zone(1, ZoneKind::Linear, 20), zone(2, ZoneKind::Single, 1)],
        )
    }

    #[test]
    fn first_run_makes_one_scene_with_every_light() {
        let (lights, scenes) = migrate(None, &[board()]);
        assert_eq!(lights.lights.len(), 3);
        assert_eq!(scenes.scenes.len(), 1);
        let scene = &scenes.scenes[0];
        assert_eq!(scene.name, "Everything");
        assert_eq!(scenes.active, scene.id);
        assert_eq!(scene.groups.len(), 1);
        assert_eq!(scene.groups[0].members.len(), 3);
        assert_eq!(scene.groups[0].mode, GroupMode::FlowAcross);
        canvas_proto::scene::validate_scene(scene, &lights.lights).unwrap();
    }

    #[test]
    fn each_saved_layout_becomes_a_scene_with_its_name_and_effect() {
        let layouts = file(
            "a",
            vec![
                layout("a", "Gaming", vec![placed("openrgb:board", 0), placed("openrgb:board", 1)], Some("aurora")),
                layout("b", "Quiet", vec![placed("openrgb:board", 1)], None),
            ],
        );
        let (lights, scenes) = migrate(Some(&layouts), &[board()]);
        assert_eq!(scenes.scenes.len(), 2);
        let (a, b) = (&scenes.scenes[0], &scenes.scenes[1]);
        assert_eq!((a.id.as_str(), a.name.as_str(), a.rev), ("a", "Gaming", 1));
        assert_eq!(a.groups.len(), 1);
        assert_eq!(a.groups[0].name, "Everything");
        assert_eq!(a.groups[0].members, vec!["openrgb:board#z0".to_string(), "openrgb:board#z1".to_string()]);
        assert!(matches!(&a.groups[0].plays, Plays::Effect { effect, .. } if effect == "aurora"));
        assert_eq!(b.name, "Quiet");
        assert_eq!(b.groups[0].plays, Plays::Off);
        assert_eq!(b.groups[0].members, vec!["openrgb:board#z1".to_string()]);
        for scene in &scenes.scenes {
            canvas_proto::scene::validate_scene(scene, &lights.lights).unwrap();
        }
    }

    #[test]
    fn the_active_layout_is_the_active_scene() {
        let layouts = file(
            "b",
            vec![
                layout("a", "A", vec![placed("openrgb:board", 0)], None),
                layout("b", "B", vec![placed("openrgb:board", 1)], None),
            ],
        );
        let (lights, scenes) = migrate(Some(&layouts), &[board()]);
        assert_eq!(scenes.active, "b");
        // The active layout's lights come first, then the zones only another layout placed.
        let ids: Vec<_> = lights.lights.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, vec!["openrgb:board#z1", "openrgb:board#z0"]);
    }

    #[test]
    fn labels_slices_ring_shape_and_reversed_carry_over() {
        let mut ring = placed("openrgb:board", 0);
        ring.shape = ZoneShape::Ring;
        ring.label = Some("Front fan".into());
        ring.reversed = true;
        let mut slice = placed("openrgb:board", 1);
        slice.leds = Some(LedSlice { start: 4, count: 8 });
        let matrix_dev = device(
            "openrgb:kb",
            1,
            vec![zone(0, ZoneKind::Matrix { width: 2, height: 1, map: vec![Some(0), Some(1)] }, 2)],
        );
        let layouts = file("a", vec![layout("a", "A", vec![ring, slice, placed("openrgb:kb", 0), placed("openrgb:board", 2)], None)]);
        let (lights, _) = migrate(Some(&layouts), &[board(), matrix_dev]);
        let by = |id: &str| lights.lights.iter().find(|l| l.id == id).unwrap_or_else(|| panic!("no light {id}"));
        let fan = by("openrgb:board#z0");
        assert_eq!(fan.name.as_deref(), Some("Front fan"));
        assert_eq!(fan.arrangement, Arrangement::Ring);
        assert!(fan.reversed);
        let part = by("openrgb:board#z1p1");
        assert_eq!(part.leds, Some(LedSlice { start: 4, count: 8 }));
        assert_eq!(part.arrangement, Arrangement::Strip);
        assert_eq!(by("openrgb:kb#z0").arrangement, Arrangement::Grid);
        assert_eq!(by("openrgb:board#z2").arrangement, Arrangement::Single);
    }

    #[test]
    fn a_hidden_zone_is_a_light_in_no_group() {
        let mut hidden = placed("openrgb:board", 1);
        hidden.visible = false;
        let layouts = file("a", vec![layout("a", "A", vec![placed("openrgb:board", 0), hidden], None)]);
        let (lights, scenes) = migrate(Some(&layouts), &[board()]);
        assert_eq!(lights.lights.len(), 2, "the hidden zone is still a light");
        assert_eq!(scenes.scenes[0].groups[0].members, vec!["openrgb:board#z0".to_string()]);
        let unassigned = canvas_proto::scene::unassigned(&scenes.scenes[0], &lights.lights);
        assert_eq!(unassigned.len(), 1);
        assert_eq!(unassigned[0].id, "openrgb:board#z1");
    }

    #[test]
    fn three_slices_of_one_zone_become_three_lights_in_start_order() {
        let slice = |start, count| {
            let mut p = placed("openrgb:board", 0);
            p.leds = Some(LedSlice { start, count });
            p
        };
        let layouts = file("a", vec![layout("a", "A", vec![slice(0, 10), slice(10, 10), slice(20, 10)], None)]);
        let (lights, scenes) = migrate(Some(&layouts), &[board()]);
        let ids: Vec<_> = lights.lights.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, vec!["openrgb:board#z0p1", "openrgb:board#z0p2", "openrgb:board#z0p3"]);
        assert_eq!(lights.lights[2].leds, Some(LedSlice { start: 20, count: 10 }));
        assert_eq!(scenes.scenes[0].groups[0].members.len(), 3);

        // Placed out of order, the numbers still follow the start.
        let shuffled = file("a", vec![layout("a", "A", vec![slice(20, 10), slice(0, 10), slice(10, 10)], None)]);
        let (lights, _) = migrate(Some(&shuffled), &[board()]);
        let start_of = |id: &str| lights.lights.iter().find(|l| l.id == id).unwrap().leds.unwrap().start;
        assert_eq!((start_of("openrgb:board#z0p1"), start_of("openrgb:board#z0p2"), start_of("openrgb:board#z0p3")), (0, 10, 20));
    }

    #[test]
    fn a_zone_with_zero_leds_is_not_a_light() {
        let dead = device("openrgb:empty", 1, vec![zone(0, ZoneKind::Linear, 0)]);
        let mut empty_slice = placed("openrgb:board", 1);
        empty_slice.leds = Some(LedSlice { start: 0, count: 0 });
        let layouts = file(
            "a",
            vec![layout("a", "A", vec![placed("openrgb:board", 0), placed("openrgb:empty", 0), empty_slice], None)],
        );
        let (lights, scenes) = migrate(Some(&layouts), &[board(), dead]);
        assert_eq!(lights.lights.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), vec!["openrgb:board#z0"]);
        assert_eq!(scenes.scenes[0].groups[0].members.len(), 1);
    }

    /// A fresh temp home with a `layouts.json` in it.
    fn home_with_layouts(tag: &str, layouts: &LayoutsFile) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-scene-migration-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::storage::write_json(&dir.join(canvas_proto::config::LAYOUTS_FILE), layouts).unwrap();
        dir
    }

    fn sample_layouts() -> LayoutsFile {
        file(
            "a",
            vec![
                layout("a", "Gaming", vec![placed("openrgb:board", 0), placed("openrgb:board", 1)], Some("aurora")),
                layout("b", "Quiet", vec![placed("openrgb:board", 1)], None),
            ],
        )
    }

    #[test]
    fn migration_never_writes_layouts_json() {
        let dir = home_with_layouts("never-writes", &sample_layouts());
        let path = dir.join(canvas_proto::config::LAYOUTS_FILE);
        let bytes = std::fs::read(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));

        let store = Store::load_from(&dir);

        assert_eq!(store.scenes.scenes.len(), 2, "the migration ran");
        assert!(dir.join(canvas_proto::scene::SCENES_FILE).exists());
        assert!(dir.join(canvas_proto::scene::LIGHTS_FILE).exists());
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "layouts.json bytes are unchanged");
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified, "and so is its mtime");
        let corrupt = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("layouts.json."))
            .count();
        assert_eq!(corrupt, 0, "layouts.json was not renamed either");
        let snapshots = std::fs::read_dir(crate::paths::backups_dir_in(&dir)).unwrap().filter_map(|e| e.ok()).count();
        assert_eq!(snapshots, 1, "one snapshot, taken before the files were written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_runs_once() {
        let dir = home_with_layouts("runs-once", &sample_layouts());
        let first = Store::load_from(&dir);
        let scenes_path = dir.join(canvas_proto::scene::SCENES_FILE);
        let lights_path = dir.join(canvas_proto::scene::LIGHTS_FILE);
        let scenes_bytes = std::fs::read(&scenes_path).unwrap();
        let lights_bytes = std::fs::read(&lights_path).unwrap();
        let scenes_mtime = std::fs::metadata(&scenes_path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));

        // A different layouts file on the second start changes nothing: the scenes are the truth now.
        crate::storage::write_json(
            &dir.join(canvas_proto::config::LAYOUTS_FILE),
            &file("z", vec![layout("z", "Other", vec![placed("openrgb:board", 2)], None)]),
        )
        .unwrap();
        let second = Store::load_from(&dir);

        assert_eq!(second.scenes, first.scenes);
        assert_eq!(second.lights, first.lights);
        assert_eq!(std::fs::read(&scenes_path).unwrap(), scenes_bytes);
        assert_eq!(std::fs::read(&lights_path).unwrap(), lights_bytes);
        assert_eq!(std::fs::metadata(&scenes_path).unwrap().modified().unwrap(), scenes_mtime, "not rewritten");
        let snapshots = std::fs::read_dir(crate::paths::backups_dir_in(&dir)).unwrap().filter_map(|e| e.ok()).count();
        assert_eq!(snapshots, 1, "no second snapshot");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review Focus 5, the file half: scenes.json that will not parse is moved aside and the
    /// default scene holds every light; the old layouts file is left alone and not migrated again.
    #[test]
    fn an_unparseable_scenes_file_is_quarantined_and_the_default_scene_made() {
        let dir = home_with_layouts("damaged", &sample_layouts());
        let _ = Store::load_from(&dir);
        let scenes_path = dir.join(canvas_proto::scene::SCENES_FILE);
        std::fs::write(&scenes_path, b"{ not json").unwrap();

        let store = Store::load_from(&dir);

        assert_eq!(store.scenes.scenes.len(), 1);
        assert_eq!(store.scenes.scenes[0].name, "Everything");
        assert_eq!(store.scenes.scenes[0].groups.len(), 1);
        assert_eq!(store.scenes.scenes[0].groups[0].members.len(), store.lights.lights.len());
        assert!(!store.lights.lights.is_empty());
        let quarantined = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("scenes.json.") && n.ends_with(".corrupt")
            })
            .count();
        assert_eq!(quarantined, 1);
        let on_disk: ScenesFile = serde_json::from_slice(&std::fs::read(&scenes_path).unwrap()).unwrap();
        assert_eq!(on_disk, store.scenes, "the repaired file is saved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review Focus 5, the content half, through the loader: a light in two groups, an empty
    /// group and an `active` naming nothing are repaired on load, and the repair is saved.
    #[test]
    fn a_hand_edited_scenes_file_is_repaired_on_load() {
        let dir = home_with_layouts("hand-edited", &sample_layouts());
        let first = Store::load_from(&dir);
        let mut edited = first.scenes.clone();
        edited.active = "gone".into();
        let dup = edited.scenes[0].groups[0].clone();
        edited.scenes[0].groups.push(Group { id: "second".into(), ..dup });
        let empty = Group { id: "empty".into(), members: vec![], ..edited.scenes[0].groups[0].clone() };
        edited.scenes[0].groups.push(empty);
        crate::storage::write_json(&dir.join(canvas_proto::scene::SCENES_FILE), &edited).unwrap();

        let store = Store::load_from(&dir);

        assert_eq!(store.scenes.active, edited.scenes[0].id);
        assert_eq!(store.scenes.scenes[0].groups.len(), 1);
        canvas_proto::scene::validate_scene(&store.scenes.scenes[0], &store.lights.lights).unwrap();
        let on_disk: ScenesFile =
            serde_json::from_slice(&std::fs::read(dir.join(canvas_proto::scene::SCENES_FILE)).unwrap()).unwrap();
        assert_eq!(on_disk, store.scenes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With no layouts file at all there is nothing to migrate: the store holds the default scene
    /// in memory and writes no scene file until a device or an edit gives it something to say.
    #[test]
    fn a_first_start_with_no_layouts_file_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-scene-migration-first-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::load_from(&dir);
        assert_eq!(store.scenes.scenes.len(), 1);
        assert_eq!(store.scenes.scenes[0].name, "Everything");
        assert!(!dir.join(canvas_proto::scene::SCENES_FILE).exists());
        assert!(!dir.join(canvas_proto::scene::LIGHTS_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reads the file named by `CANVAS_RGB_REAL_LAYOUTS` and prints what the migration makes of
    /// it. The main agent runs this on a copy of the owner's file; it writes nothing.
    #[test]
    #[ignore]
    fn migrates_a_real_layouts_file() {
        let path = std::env::var("CANVAS_RGB_REAL_LAYOUTS").expect("set CANVAS_RGB_REAL_LAYOUTS to a layouts.json");
        let layouts: LayoutsFile = serde_json::from_slice(&std::fs::read(path).unwrap()).expect("layouts.json parses");
        let zones: usize = layouts.layouts.iter().map(|l| l.zones.len()).sum();
        let (lights, scenes) = migrate(Some(&layouts), &[]);
        println!(
            "layouts {} zones {} lights {} scenes {} active {}",
            layouts.layouts.len(),
            zones,
            lights.lights.len(),
            scenes.scenes.len(),
            scenes.active
        );
        for scene in &scenes.scenes {
            println!(
                "  scene {} ({}): {} group(s), {} light(s) in groups",
                scene.id,
                scene.name,
                scene.groups.len(),
                scene.groups.iter().map(|g| g.members.len()).sum::<usize>()
            );
            canvas_proto::scene::validate_scene(scene, &lights.lights).expect("each migrated scene is valid");
        }
    }
}
