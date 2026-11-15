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
/// The most groups of one scene that may play an HTML effect (#370). Each is a QuickJS runtime
/// whose own script is the cost (about 5.5 to 6.7 ms a frame for the stock effects, whatever the
/// canvas size; `docs/benchmark.md`, "Scenes: cost per group"). The daemon enforces it
/// (`check_html_limit` in `crates/canvas-daemon/src/scenes.rs`) and counts a stack's HTML layers
/// too; [`validate_scene`] does not, so a scene over the limit is still a valid file.
pub const SCENE_HTML_MAX: usize = 2;
/// The id prefix of an HTML effect.
pub const HTML_EFFECT_PREFIX: &str = "html:";
/// Chain setup (0.15.0): the earlier length of a header held raised by an open setup, beside
/// `lights.json`. Written before the raise, deleted once the length is back. Start-up replays it.
pub const CHAIN_RESTORE_FILE: &str = "chain-restore.json";
/// Chain setup: everything a commit needs to be undone, written before the final resize.
/// Start-up rolls a leftover one back.
pub const CHAIN_INTENT_FILE: &str = "chain-intent.json";
/// The most times one run of LEDs may be shown by a header (`Light::copies`). A typo guard, not
/// a hardware limit.
pub const COPIES_MAX: u32 = 32;
/// The longest a part's name may be, in characters.
pub const PART_NAME_MAX: usize = GROUP_NAME_MAX;
/// A setup session ends this long after its last light request (the app re-sends one every
/// 2 s while it is open).
pub const SETUP_TIMEOUT_MS: u64 = 6_000;

/// A light's id. Stable across reconnects: it is built from the device id, never from an
/// adapter index.
pub type LightId = String;

/// `"{device_id}#z{zone}"`: the id of the light that is a whole zone. Parts (0.15.0) append
/// `p{n}`.
pub fn zone_light_id(device: &DeviceId, zone: u32) -> LightId {
    format!("{}#z{}", device.0, zone)
}

/// `"{zone_light_id}p{n}"`: the id of part `n` of a chain, counted from 1 by position along it.
pub fn part_light_id(device: &DeviceId, zone: u32, n: u32) -> LightId {
    format!("{}p{}", zone_light_id(device, zone), n)
}

/// What the owner said is plugged into a chain, part by part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ChainPartKind {
    Fan,
    Strip,
    /// Anything else. Kept as its own kind so the owner's answer is never rewritten.
    Other,
}

impl ChainPartKind {
    /// How a part of this kind reads the canvas: a fan is a ring, anything else a strip.
    pub fn arrangement(self) -> Arrangement {
        match self {
            ChainPartKind::Fan => Arrangement::Ring,
            ChainPartKind::Strip | ChainPartKind::Other => Arrangement::Strip,
        }
    }
}

/// Whether the owner set a header up. A header with no record is not set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ChainState {
    /// The owner said nothing is plugged in: no light, driven black.
    Empty,
    /// Its parts are lights.
    SetUp,
}

/// One header's setup state, in `lights.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChainRecord {
    pub device_id: DeviceId,
    pub zone_index: u32,
    pub state: ChainState,
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
    /// What the owner said this part is. `None` for a whole-zone light and for a light from
    /// before 0.15.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kind: Option<ChainPartKind>,
    /// How many times the header shows this light's samples (#381, #382): fans that all show the
    /// same LEDs are one light with `copies` set to how many were counted. `None` and `Some(1)`
    /// mean once. The engine writes `leds.len()` once whatever this says; it only counts, so a
    /// scene still sees one light. At most [`COPIES_MAX`]; the daemon validates it on write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub copies: Option<u32>,
}

/// `lights.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LightsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub lights: Vec<Light>,
    /// The headers the owner set up or marked empty (0.15.0). Absent in older files.
    #[serde(default)]
    pub chains: Vec<ChainRecord>,
}

impl Default for LightsFile {
    fn default() -> Self {
        Self { schema_version: LIGHTS_SCHEMA, lights: Vec::new(), chains: Vec::new() }
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
}

/// How many groups of `scene` play an HTML effect (by id; a stack's own HTML layers are the
/// daemon's to count). The daemon compares it with [`SCENE_HTML_MAX`]; the spine does not.
pub fn html_groups(scene: &Scene) -> usize {
    scene
        .groups
        .iter()
        .filter(|g| matches!(&g.plays, Plays::Effect { effect, .. } if effect.starts_with(HTML_EFFECT_PREFIX)))
        .count()
}

/// Why a `copies` value is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CopiesError {
    #[error("copies must be at least 1")]
    Zero,
    #[error("copies is {copies}; the most is {max}")]
    AboveMax { copies: u32, max: u32 },
}

/// Checks a `copies` value an owner wrote (a setup part, #381 #382) and returns what to store:
/// `None` and `Some(1)` both mean once and are stored as `None`; `Some(0)` and anything above
/// [`COPIES_MAX`] are refused. Whether `copies` of 2 or more may sit beside other parts of the
/// same header is the daemon's rule (`plan_commit`), not this one.
pub fn check_copies(copies: Option<u32>) -> Result<Option<u32>, CopiesError> {
    match copies {
        None | Some(1) => Ok(None),
        Some(0) => Err(CopiesError::Zero),
        Some(n) if n > COPIES_MAX => Err(CopiesError::AboveMax { copies: n, max: COPIES_MAX }),
        Some(n) => Ok(Some(n)),
    }
}

/// Checks the rules a scene keeps: names fit, group ids are unique, every group has a member,
/// every member is a known light and no light is in two groups. The HTML-group limit
/// ([`SCENE_HTML_MAX`]) is not checked here: the daemon owns it, so a scene over the limit stays a
/// valid file and the user reads the daemon's sentence.
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
            kind: None,
            copies: None,
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

    fn html_scene(count: usize) -> (Scene, Vec<Light>) {
        let names = ["a", "b", "c", "d", "e"];
        let groups = (0..count)
            .map(|i| {
                let mut g = group(&format!("g{i}"), &[names[i]]);
                g.plays = Plays::Effect { effect: format!("html:fx{i}"), props: Props::new() };
                g
            })
            .collect();
        (scene(groups), names.iter().map(|n| light(n)).collect())
    }

    #[test]
    fn two_groups_may_play_an_html_effect() {
        assert_eq!(SCENE_HTML_MAX, 2, "#370: a scene runs two HTML effects");
    }

    /// The HTML count is the daemon's to enforce (`check_html_limit`), because only the daemon sees
    /// a stack's HTML layers. A scene over the limit is still a valid file: it loads and is shown,
    /// and the daemon refuses to save or set it with its own sentence.
    #[test]
    fn a_scene_over_the_html_limit_still_validates() {
        let (over, ls) = html_scene(SCENE_HTML_MAX + 1);
        assert_eq!(html_groups(&over), SCENE_HTML_MAX + 1);
        assert_eq!(validate_scene(&over, &ls), Ok(()));
        let (at_max, ls) = html_scene(SCENE_HTML_MAX);
        assert_eq!(validate_scene(&at_max, &ls), Ok(()));
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
        let file = LightsFile { schema_version: LIGHTS_SCHEMA, lights: vec![l, light("b")], chains: Vec::new() };
        let json = serde_json::to_string(&file).unwrap();
        assert!(json.contains(r#""arrangement":"ring""#), "{json}");
        assert_eq!(serde_json::from_str::<LightsFile>(&json).unwrap(), file);
    }

    #[test]
    fn part_light_id_appends_p_and_counts_from_one() {
        let d = DeviceId("openrgb:mb".into());
        assert_eq!(part_light_id(&d, 0, 1), "openrgb:mb#z0p1");
        assert_eq!(part_light_id(&d, 2, 7), "openrgb:mb#z2p7");
        assert!(part_light_id(&d, 0, 1).starts_with(&zone_light_id(&d, 0)));
    }

    #[test]
    fn chain_part_kind_maps_to_arrangement() {
        assert_eq!(ChainPartKind::Fan.arrangement(), Arrangement::Ring);
        assert_eq!(ChainPartKind::Strip.arrangement(), Arrangement::Strip);
        assert_eq!(ChainPartKind::Other.arrangement(), Arrangement::Strip);
    }

    #[test]
    fn other_survives_a_round_trip_as_other() {
        let json = serde_json::to_string(&ChainPartKind::Other).unwrap();
        assert_eq!(json, r#""other""#);
        assert_eq!(serde_json::from_str::<ChainPartKind>(&json).unwrap(), ChainPartKind::Other);
        let mut l = light("a");
        l.kind = Some(ChainPartKind::Other);
        let back: Light = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back.kind, Some(ChainPartKind::Other));
    }

    #[test]
    fn a_lights_file_without_chains_or_kinds_still_parses() {
        // An 0.14.0 lights.json.
        let old = r#"{"schema_version":1,"lights":[{"id":"openrgb:mb#z0","device_id":"openrgb:mb","zone_index":0,"arrangement":"strip"}]}"#;
        let file: LightsFile = serde_json::from_str(old).unwrap();
        assert!(file.chains.is_empty());
        assert_eq!(file.lights[0].kind, None);
        // A light with no kind writes no kind, so an 0.14.0 reader sees what it wrote.
        assert!(!serde_json::to_string(&file.lights[0]).unwrap().contains("kind"));
    }

    #[test]
    fn check_copies_stores_none_for_once_and_refuses_zero_and_above_the_cap() {
        assert_eq!(check_copies(None), Ok(None));
        assert_eq!(check_copies(Some(1)), Ok(None), "once is stored as none");
        assert_eq!(check_copies(Some(2)), Ok(Some(2)));
        assert_eq!(check_copies(Some(COPIES_MAX)), Ok(Some(COPIES_MAX)));
        assert_eq!(check_copies(Some(0)), Err(CopiesError::Zero));
        assert_eq!(
            check_copies(Some(COPIES_MAX + 1)),
            Err(CopiesError::AboveMax { copies: COPIES_MAX + 1, max: COPIES_MAX })
        );
        assert_eq!(check_copies(Some(u32::MAX)), Err(CopiesError::AboveMax { copies: u32::MAX, max: COPIES_MAX }));
    }

    #[test]
    fn copies_round_trips_and_a_light_without_it_loads() {
        assert_eq!(COPIES_MAX, 32);
        let mut mirrored = light("a");
        mirrored.leds = Some(LedSlice { start: 0, count: 8 });
        mirrored.kind = Some(ChainPartKind::Other);
        mirrored.copies = Some(8);
        let json = serde_json::to_string(&mirrored).unwrap();
        assert!(json.contains(r#""copies":8"#), "{json}");
        assert_eq!(serde_json::from_str::<Light>(&json).unwrap(), mirrored);
        // A light with no copies writes none, so an older reader sees what it always saw.
        assert!(!serde_json::to_string(&light("b")).unwrap().contains("copies"));
        // An 0.16.0 lights.json has no copies anywhere and loads as once.
        let old = r#"{"schema_version":1,"lights":[{"id":"openrgb:mb#z0","device_id":"openrgb:mb","zone_index":0,"arrangement":"strip","kind":"fan"}]}"#;
        let file: LightsFile = serde_json::from_str(old).unwrap();
        assert_eq!(file.lights[0].copies, None);
        // The new file round trips whole.
        let whole = LightsFile { schema_version: LIGHTS_SCHEMA, lights: vec![mirrored, light("b")], chains: Vec::new() };
        let json = serde_json::to_string(&whole).unwrap();
        assert_eq!(serde_json::from_str::<LightsFile>(&json).unwrap(), whole);
    }

    #[test]
    fn chain_records_round_trip() {
        let file = LightsFile {
            schema_version: LIGHTS_SCHEMA,
            lights: vec![light("a")],
            chains: vec![
                ChainRecord { device_id: DeviceId("openrgb:mb".into()), zone_index: 0, state: ChainState::SetUp },
                ChainRecord { device_id: DeviceId("openrgb:mb".into()), zone_index: 1, state: ChainState::Empty },
            ],
        };
        let json = serde_json::to_string(&file).unwrap();
        assert!(json.contains(r#""state":"set_up""#) && json.contains(r#""state":"empty""#), "{json}");
        assert_eq!(serde_json::from_str::<LightsFile>(&json).unwrap(), file);
    }
}
