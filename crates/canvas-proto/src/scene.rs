//! Scenes (0.14.0): lights, groups and scenes.
//!
//! A **light** is a run of LEDs the owner treats as one thing: a whole zone of a device today,
//! a part of a zone from 0.15.0. A **group** is one or more lights playing one effect with its
//! own settings. A **scene** is a named set of groups applied in one go; one scene is active.
//! A light in no group of the active scene is dark. Order replaces position: a group's members
//! are a list, and how they share the effect is the group's [`GroupMode`] and each light's
//! [`Arrangement`].
//!
//! Files: `lights.json` ([`LightsFile`]) and `scenes.json` ([`ScenesFile`]), beside the other
//! config files. The design is `docs/superpowers/specs/2026-11-12-scenes-design.md`.

use std::collections::HashSet;

use crate::device::{DeviceId, ZoneKind};
use crate::layout::LedSlice;
use crate::props::Props;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The lights file, beside `scenes.json`.
pub const LIGHTS_FILE: &str = "lights.json";
/// The scenes file.
pub const SCENES_FILE: &str = "scenes.json";
/// The `schema_version` of `lights.json`.
pub const LIGHTS_SCHEMA: u32 = 1;
/// The `schema_version` of `scenes.json`.
pub const SCENES_SCHEMA: u32 = 1;
/// The longest a scene's name may be, in characters.
pub const SCENE_NAME_MAX: usize = 40;
/// The longest a group's name may be, in characters.
pub const GROUP_NAME_MAX: usize = 40;
/// How long setting a scene blends the old frames into the new ones.
pub const SCENE_FADE_MS: u64 = 250;
/// The name of the scene made on a first run and when no scene is left.
pub const DEFAULT_SCENE_NAME: &str = "Everything";
/// The name of the one group that scene holds.
pub const DEFAULT_GROUP_NAME: &str = "Everything";
/// The most a group's brightness may be.
pub const GROUP_BRIGHTNESS_MAX: u8 = 100;
/// The most groups of one scene that may play an HTML effect. Each is a QuickJS runtime whose own
/// script is the cost (about 5.5 to 6.7 ms a frame for the stock effects, whatever the canvas
/// size; `docs/benchmark.md`, "Scenes: cost per group"). One is what is proved to fit a 20-group
/// scene; two is unproven until the bench runs through the real engine with every stock effect
/// (Scenes plan, Task 4 Step 5). Raise it only on that evidence. The daemon also counts a
/// stack's HTML layers.
pub const SCENE_HTML_MAX: usize = 1;
/// The id prefix of an HTML effect.
pub const HTML_EFFECT_PREFIX: &str = "html:";

/// A light's id. Stable across reconnects: it is built from the device id, never from an
/// adapter index.
pub type LightId = String;

/// `"{device_id}#z{zone}"`: the id of the light that is a whole zone. Parts (0.15.0) append
/// `p{n}`.
pub fn zone_light_id(device: &DeviceId, zone: u32) -> LightId {
    format!("{}#z{}", device.0, zone)
}

/// How a light's LEDs read the group's canvas inside the light's slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Arrangement {
    /// One colour: the slot's centre.
    Single,
    /// A line, left to right at mid-height.
    Strip,
    /// A circle, from the top, clockwise.
    Ring,
    /// The zone's own matrix map fills the slot. On a zone that is not a matrix, a strip.
    Grid,
}

impl Arrangement {
    /// What a light of this zone kind is until the owner says otherwise. Never `Ring`: the
    /// hardware cannot tell a ring from a strip.
    pub fn default_for(kind: &ZoneKind) -> Self {
        match kind {
            ZoneKind::Single => Arrangement::Single,
            ZoneKind::Linear => Arrangement::Strip,
            ZoneKind::Matrix { .. } => Arrangement::Grid,
        }
    }
}

/// One light: a zone of a device, or a run of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Light {
    pub id: LightId,
    pub device_id: DeviceId,
    pub zone_index: u32,
    /// The run of the zone this light owns. `None` owns the whole zone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub leds: Option<LedSlice>,
    /// The owner's name for it. `None` shows the device and zone name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    pub arrangement: Arrangement,
    /// The LEDs run the other way.
    #[serde(default)]
    pub reversed: bool,
}

/// `lights.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LightsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub lights: Vec<Light>,
}

impl Default for LightsFile {
    fn default() -> Self {
        Self { schema_version: LIGHTS_SCHEMA, lights: Vec::new() }
    }
}

/// How a group's lights share its effect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum GroupMode {
    /// One picture across every light, in member order, each light's share by its LED count.
    #[default]
    FlowAcross,
    /// Every light shows the whole picture.
    EachItsOwn,
}

/// What a group plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Plays {
    /// Dark.
    Off,
    /// An effect by id, with this group's own values.
    Effect {
        effect: String,
        #[serde(default)]
        props: Props,
    },
}

fn default_brightness() -> u8 {
    GROUP_BRIGHTNESS_MAX
}

/// One group: lights in order and the one effect they play.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// In order: order replaces position.
    pub members: Vec<LightId>,
    pub plays: Plays,
    #[serde(default)]
    pub mode: GroupMode,
    /// 0..=100, multiplied with the global brightness.
    #[serde(default = "default_brightness")]
    pub brightness: u8,
}

/// One scene: named groups, applied in one go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Scene {
    pub id: String,
    pub name: String,
    /// Bumped by every saved change. A write names the `rev` it was made from, so two editor
    /// windows never overwrite each other.
    #[ts(type = "number")]
    pub rev: u64,
    #[serde(default)]
    pub groups: Vec<Group>,
}

/// `scenes.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScenesFile {
    pub schema_version: u32,
    /// The id of the active scene.
    pub active: String,
    #[serde(default)]
    pub scenes: Vec<Scene>,
}

/// Why a scene is refused. The first problem found, in group order.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SceneError {
    #[error("group {group} has no devices")]
    EmptyGroup { group: String },
    #[error("light {light} is in more than one group")]
    LightTwice { light: LightId },
    #[error("there is no light {light}")]
    UnknownLight { light: LightId },
    #[error("the name of {what} is too long")]
    NameTooLong { what: String },
    #[error("two groups have the id {id}")]
    DuplicateId { id: String },
    #[error("the brightness of group {group} is over {GROUP_BRIGHTNESS_MAX}")]
    BrightnessRange { group: String },
    #[error("more than {SCENE_HTML_MAX} groups play an HTML effect")]
    TooManyHtml { count: usize },
}

/// How many groups of `scene` play an HTML effect (by id; a stack's own HTML layers are the
/// daemon's to count).
pub fn html_groups(scene: &Scene) -> usize {
    scene
        .groups
        .iter()
        .filter(|g| matches!(&g.plays, Plays::Effect { effect, .. } if effect.starts_with(HTML_EFFECT_PREFIX)))
        .count()
}

/// Checks the rules a scene keeps: names fit, group ids are unique, every group has a member,
/// every member is a known light, no light is in two groups, and at most [`SCENE_HTML_MAX`]
/// groups play an HTML effect.
pub fn validate_scene(scene: &Scene, lights: &[Light]) -> Result<(), SceneError> {
    if scene.name.chars().count() > SCENE_NAME_MAX {
        return Err(SceneError::NameTooLong { what: format!("scene {}", scene.id) });
    }
    let known: HashSet<&str> = lights.iter().map(|l| l.id.as_str()).collect();
    let mut ids = HashSet::new();
    let mut held = HashSet::new();
    for group in &scene.groups {
        if !ids.insert(group.id.as_str()) {
            return Err(SceneError::DuplicateId { id: group.id.clone() });
        }
        if group.name.chars().count() > GROUP_NAME_MAX {
            return Err(SceneError::NameTooLong { what: format!("group {}", group.id) });
        }
        if group.brightness > GROUP_BRIGHTNESS_MAX {
            return Err(SceneError::BrightnessRange { group: group.id.clone() });
        }
        if group.members.is_empty() {
            return Err(SceneError::EmptyGroup { group: group.id.clone() });
        }
        for member in &group.members {
            if !known.contains(member.as_str()) {
                return Err(SceneError::UnknownLight { light: member.clone() });
            }
            if !held.insert(member.as_str()) {
                return Err(SceneError::LightTwice { light: member.clone() });
            }
        }
    }
    let html = html_groups(scene);
    if html > SCENE_HTML_MAX {
        return Err(SceneError::TooManyHtml { count: html });
    }
    Ok(())
}

/// Lights of `lights` that no group of `scene` holds, in `lights` order.
pub fn unassigned<'a>(scene: &Scene, lights: &'a [Light]) -> Vec<&'a Light> {
    let held: HashSet<&str> =
        scene.groups.iter().flat_map(|g| g.members.iter().map(String::as_str)).collect();
    lights.iter().filter(|l| !held.contains(l.id.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::props::PropValue;

    fn light(id: &str) -> Light {
        Light {
            id: id.into(),
            device_id: DeviceId("fake:1".into()),
            zone_index: 0,
            leds: None,
            name: None,
            arrangement: Arrangement::Strip,
            reversed: false,
        }
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

    fn scene(groups: Vec<Group>) -> Scene {
        Scene { id: "s".into(), name: "Scene".into(), rev: 1, groups }
    }

    fn lights() -> Vec<Light> {
        vec![light("a"), light("b"), light("c")]
    }

    #[test]
    fn a_valid_scene_passes() {
        assert_eq!(validate_scene(&scene(vec![group("g1", &["a"]), group("g2", &["b", "c"])]), &lights()), Ok(()));
    }

    #[test]
    fn a_light_in_two_groups_is_refused() {
        let s = scene(vec![group("g1", &["a", "b"]), group("g2", &["b"])]);
        assert_eq!(validate_scene(&s, &lights()), Err(SceneError::LightTwice { light: "b".into() }));
        let twice_in_one = scene(vec![group("g1", &["a", "a"])]);
        assert_eq!(validate_scene(&twice_in_one, &lights()), Err(SceneError::LightTwice { light: "a".into() }));
    }

    #[test]
    fn an_empty_group_is_refused() {
        let s = scene(vec![group("g1", &["a"]), group("g2", &[])]);
        assert_eq!(validate_scene(&s, &lights()), Err(SceneError::EmptyGroup { group: "g2".into() }));
    }

    #[test]
    fn a_group_naming_no_such_light_is_refused() {
        let s = scene(vec![group("g1", &["a", "zz"])]);
        assert_eq!(validate_scene(&s, &lights()), Err(SceneError::UnknownLight { light: "zz".into() }));
    }

    #[test]
    fn brightness_over_100_is_refused() {
        let mut g = group("g1", &["a"]);
        g.brightness = 101;
        assert_eq!(validate_scene(&scene(vec![g.clone()]), &lights()), Err(SceneError::BrightnessRange { group: "g1".into() }));
        g.brightness = 100;
        assert_eq!(validate_scene(&scene(vec![g]), &lights()), Ok(()));
    }

    #[test]
    fn names_over_the_limit_and_duplicate_ids_are_refused() {
        let mut s = scene(vec![group("g1", &["a"])]);
        s.name = "x".repeat(SCENE_NAME_MAX);
        assert_eq!(validate_scene(&s, &lights()), Ok(()));
        s.name = "x".repeat(SCENE_NAME_MAX + 1);
        assert!(matches!(validate_scene(&s, &lights()), Err(SceneError::NameTooLong { .. })));
        let mut long_group = group("g1", &["a"]);
        long_group.name = "é".repeat(GROUP_NAME_MAX + 1);
        assert!(matches!(validate_scene(&scene(vec![long_group]), &lights()), Err(SceneError::NameTooLong { .. })));
        let dup = scene(vec![group("g1", &["a"]), group("g1", &["b"])]);
        assert_eq!(validate_scene(&dup, &lights()), Err(SceneError::DuplicateId { id: "g1".into() }));
    }

    #[test]
    fn an_html_group_over_the_limit_is_refused() {
        let html = |id: &str, member: &str| {
            let mut g = group(id, &[member]);
            g.plays = Plays::Effect { effect: format!("html:{id}"), props: Props::new() };
            g
        };
        let mut built_in = group("g4", &["d"]);
        built_in.plays = Plays::Effect { effect: "aurora".into(), props: Props::new() };
        let mut ls = lights();
        ls.push(light("d"));
        let at_max: Vec<Group> =
            ["a", "b", "c"].iter().take(SCENE_HTML_MAX).enumerate().map(|(i, m)| html(&format!("g{i}"), m)).collect();
        let mut ok = at_max.clone();
        ok.push(built_in.clone());
        assert_eq!(html_groups(&scene(ok.clone())), SCENE_HTML_MAX);
        assert_eq!(validate_scene(&scene(ok), &ls), Ok(()));
        let mut over = at_max;
        over.push(html("gx", "c"));
        over.push(built_in);
        assert_eq!(validate_scene(&scene(over), &ls), Err(SceneError::TooManyHtml { count: SCENE_HTML_MAX + 1 }));
    }

    #[test]
    fn unassigned_lists_lights_no_group_holds_in_order() {
        let ls = lights();
        let s = scene(vec![group("g1", &["b"])]);
        let ids: Vec<&str> = unassigned(&s, &ls).iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]);
        assert!(unassigned(&scene(vec![group("g", &["c", "a", "b"])]), &ls).is_empty());
    }

    #[test]
    fn zone_light_id_is_stable_for_the_same_zone() {
        let d = DeviceId("openrgb:ABC123".into());
        assert_eq!(zone_light_id(&d, 2), zone_light_id(&DeviceId("openrgb:ABC123".into()), 2));
        assert_eq!(zone_light_id(&d, 2), "openrgb:ABC123#z2");
        assert_ne!(zone_light_id(&d, 2), zone_light_id(&d, 3));
    }

    #[test]
    fn default_arrangement_follows_zone_kind() {
        assert_eq!(Arrangement::default_for(&ZoneKind::Single), Arrangement::Single);
        assert_eq!(Arrangement::default_for(&ZoneKind::Linear), Arrangement::Strip);
        let m = ZoneKind::Matrix { width: 2, height: 1, map: vec![Some(0), None] };
        assert_eq!(Arrangement::default_for(&m), Arrangement::Grid);
    }

    #[test]
    fn scenes_file_round_trips_and_keeps_unknown_plays_out() {
        let mut props = Props::new();
        props.insert("speed".into(), PropValue::Number(3.0));
        let mut on = group("g1", &["a"]);
        on.plays = Plays::Effect { effect: "aurora".into(), props };
        on.mode = GroupMode::EachItsOwn;
        on.brightness = 40;
        let file = ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: "s".into(),
            scenes: vec![scene(vec![on, group("g2", &["b"])])],
        };
        let json = serde_json::to_string(&file).unwrap();
        assert!(json.contains(r#""type":"effect""#) && json.contains(r#""type":"off""#), "{json}");
        assert!(json.contains(r#""mode":"each_its_own""#) && json.contains(r#""mode":"flow_across""#), "{json}");
        assert_eq!(serde_json::from_str::<ScenesFile>(&json).unwrap(), file);

        // A kind of play this version does not know is refused, never read as something else.
        let unknown = r#"{"id":"g","name":"g","members":["a"],"plays":{"type":"pattern","id":"p"}}"#;
        assert!(serde_json::from_str::<Group>(unknown).is_err());
        // Missing mode and brightness read as their defaults.
        let bare: Group = serde_json::from_str(r#"{"id":"g","name":"g","members":["a"],"plays":{"type":"off"}}"#).unwrap();
        assert_eq!((bare.mode, bare.brightness), (GroupMode::FlowAcross, 100));
    }

    #[test]
    fn lights_file_round_trips() {
        let mut l = light("a");
        l.leds = Some(LedSlice { start: 12, count: 12 });
        l.name = Some("Front fan".into());
        l.arrangement = Arrangement::Ring;
        l.reversed = true;
        let file = LightsFile { schema_version: LIGHTS_SCHEMA, lights: vec![l, light("b")] };
        let json = serde_json::to_string(&file).unwrap();
        assert!(json.contains(r#""arrangement":"ring""#), "{json}");
        assert_eq!(serde_json::from_str::<LightsFile>(&json).unwrap(), file);
    }
}
