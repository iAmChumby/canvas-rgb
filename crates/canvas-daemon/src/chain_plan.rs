//! The pure plan a chain commit makes: from the owner's rows to the lights file and the scenes
//! file, with nothing read and nothing written. The daemon snapshots, writes and resizes around
//! it; everything here is a value in, a value out.
//!
//! A part keeps its identity by position: part `n` of the new list reuses the old part `n`'s id,
//! `reversed` flag and groups, and takes the name and kind the owner sent, even when its length
//! changes. A part with no old part at its position joins no group; an old part with no new part
//! at its position leaves every group.

use std::collections::HashSet;

use canvas_proto::api::SetupPart;
use canvas_proto::layout::LedSlice;
use canvas_proto::scene::{
    part_light_id, validate_scene, zone_light_id, ChainRecord, ChainState, Light, LightId,
    LightsFile, ScenesFile, PART_NAME_MAX,
};
use canvas_proto::DeviceId;

/// The even split the UI shows first. The first `len % count` parts get one extra LED. `Err` when
/// `count == 0` or `count > len`.
pub fn even_split(len: u32, count: u32) -> Result<Vec<u32>, ChainPlanError> {
    if count == 0 || count > len {
        return Err(ChainPlanError::CountAboveLength);
    }
    let base = len / count;
    let extra = len % count;
    Ok((0..count).map(|i| base + u32::from(i < extra)).collect())
}

/// The files a commit writes, and the header's final length.
#[derive(Debug, Clone, PartialEq)]
pub struct CommitPlan {
    pub lights: LightsFile,
    pub scenes: ScenesFile,
    pub final_leds: u32,
}

/// Replaces the zone's lights with one light per part, in chain order, and rewrites every group of
/// every scene. Pure: nothing is read, written or timed.
pub fn plan_commit(
    lights: &LightsFile,
    scenes: &ScenesFile,
    device: &DeviceId,
    zone: u32,
    parts: &[SetupPart],
    max_leds: u32,
) -> Result<CommitPlan, ChainPlanError> {
    if parts.is_empty() {
        return Err(ChainPlanError::NoParts);
    }
    for (index, part) in parts.iter().enumerate() {
        if part.leds == 0 {
            return Err(ChainPlanError::PartUnderOneLed { index });
        }
        if part.name.chars().count() > PART_NAME_MAX {
            return Err(ChainPlanError::NameTooLong { index });
        }
    }
    let total: u64 = parts.iter().map(|p| u64::from(p.leds)).sum();
    if total > u64::from(max_leds) {
        return Err(ChainPlanError::SumAboveMax {
            sum: total.min(u64::from(u32::MAX)) as u32,
            max: max_leds,
        });
    }
    let final_leds = total as u32;

    let old_ids: Vec<LightId> = lights
        .lights
        .iter()
        .filter(|l| in_zone(l, device, zone))
        .map(|l| l.id.clone())
        .collect();
    let old: HashSet<&str> = old_ids.iter().map(String::as_str).collect();

    let mut start = 0;
    let mut made = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let id = part_light_id(device, zone, index as u32 + 1);
        let previous = lights.lights.iter().find(|l| l.id == id);
        made.push(Light {
            id,
            device_id: device.clone(),
            zone_index: zone,
            leds: Some(LedSlice { start, count: part.leds }),
            name: Some(part.name.clone()),
            arrangement: part.kind.arrangement(),
            reversed: previous.map(|l| l.reversed).unwrap_or(false),
            kind: Some(part.kind),
        });
        start += part.leds;
    }
    let place: Vec<LightId> = made.iter().map(|l| l.id.clone()).collect();

    let mut next_lights = Vec::with_capacity(lights.lights.len() + made.len());
    let mut placed = false;
    for light in &lights.lights {
        if in_zone(light, device, zone) {
            if !placed {
                next_lights.extend(made.iter().cloned());
                placed = true;
            }
        } else {
            next_lights.push(light.clone());
        }
    }
    if !placed {
        next_lights.extend(made);
    }
    let mut out_lights = lights.clone();
    out_lights.lights = next_lights;
    set_chain(&mut out_lights, device, zone, ChainState::SetUp);

    let out_scenes = rewrite_scenes(scenes, &zone_light_id(device, zone), &old, &place);
    for scene in &out_scenes.scenes {
        validate_scene(scene, &out_lights.lights)
            .map_err(|e| ChainPlanError::Invalid(e.to_string()))?;
    }
    Ok(CommitPlan { lights: out_lights, scenes: out_scenes, final_leds })
}

/// The empty-header plan: removes the zone's lights from `lights` and from every group, and marks
/// the chain `Empty`.
pub fn plan_empty(
    lights: &LightsFile,
    scenes: &ScenesFile,
    device: &DeviceId,
    zone: u32,
) -> CommitPlan {
    let old_ids: Vec<LightId> = lights
        .lights
        .iter()
        .filter(|l| in_zone(l, device, zone))
        .map(|l| l.id.clone())
        .collect();
    let old: HashSet<&str> = old_ids.iter().map(String::as_str).collect();

    let mut out_lights = lights.clone();
    out_lights.lights.retain(|l| !in_zone(l, device, zone));
    set_chain(&mut out_lights, device, zone, ChainState::Empty);

    let out_scenes = rewrite_scenes(scenes, &zone_light_id(device, zone), &old, &[]);
    CommitPlan { lights: out_lights, scenes: out_scenes, final_leds: 0 }
}

/// Why a plan is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainPlanError {
    /// A chain needs at least one part.
    NoParts,
    /// A row has no LEDs.
    PartUnderOneLed { index: usize },
    /// The rows ask for more LEDs than the header can take.
    SumAboveMax { sum: u32, max: u32 },
    /// A row's name is longer than [`PART_NAME_MAX`].
    NameTooLong { index: usize },
    /// The split asks for more parts than the header has LEDs, or for none at all.
    CountAboveLength,
    /// The planned scenes would not validate; the text is the validator's own message.
    Invalid(String),
}

impl std::fmt::Display for ChainPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainPlanError::NoParts => write!(f, "a chain needs at least one part"),
            ChainPlanError::PartUnderOneLed { index } => {
                write!(f, "part {} has no LEDs; give it at least one", index + 1)
            }
            ChainPlanError::SumAboveMax { sum, max } => write!(
                f,
                "the parts add up to {sum} LEDs, and this header takes at most {max}"
            ),
            ChainPlanError::NameTooLong { index } => write!(
                f,
                "the name of part {} is longer than {PART_NAME_MAX} characters",
                index + 1
            ),
            ChainPlanError::CountAboveLength => write!(
                f,
                "a chain cannot have more parts than LEDs, and it needs at least one part"
            ),
            ChainPlanError::Invalid(text) => {
                write!(f, "the scenes would not be valid: {text}")
            }
        }
    }
}

impl std::error::Error for ChainPlanError {}

/// `true` for a light that belongs to this device's zone, whole or split.
fn in_zone(light: &Light, device: &DeviceId, zone: u32) -> bool {
    light.device_id == *device && light.zone_index == zone
}

/// Adds the zone's chain record or replaces its state; other chains keep their place.
fn set_chain(lights: &mut LightsFile, device: &DeviceId, zone: u32, state: ChainState) {
    match lights.chains.iter_mut().find(|c| c.device_id == *device && c.zone_index == zone) {
        Some(record) => record.state = state,
        None => lights.chains.push(ChainRecord {
            device_id: device.clone(),
            zone_index: zone,
            state,
        }),
    }
}

/// Rewrites every group of every scene for a changed zone.
///
/// `zone_light` is the id of the zone's whole-zone light: where a group holds it, its `place`
/// takes its spot in the member list, in chain order. A member in `old` is a light of this zone:
/// it stays while `place` still holds it (a part keeps its id by position) and leaves otherwise.
/// Every other member stays as it is. A group left with no members is dropped, and the first
/// group that names a light keeps it, as `scenes::repair` does.
fn rewrite_scenes(
    scenes: &ScenesFile,
    zone_light: &str,
    old: &HashSet<&str>,
    place: &[LightId],
) -> ScenesFile {
    let mut out = scenes.clone();
    for scene in &mut out.scenes {
        let mut held: HashSet<LightId> = HashSet::new();
        for group in &mut scene.groups {
            let mut kept = Vec::with_capacity(group.members.len());
            for member in std::mem::take(&mut group.members) {
                if member == zone_light {
                    for id in place {
                        if held.insert(id.clone()) {
                            kept.push(id.clone());
                        }
                    }
                } else if old.contains(member.as_str()) {
                    if place.iter().any(|id| id == &member) && held.insert(member.clone()) {
                        kept.push(member);
                    }
                } else if held.insert(member.clone()) {
                    kept.push(member);
                }
            }
            group.members = kept;
        }
        scene.groups.retain(|g| !g.members.is_empty());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::scene::{
        Arrangement, ChainPartKind, Group, GroupMode, Plays, Scene, LIGHTS_SCHEMA, SCENES_SCHEMA,
    };

    const ZONE: u32 = 0;

    fn device() -> DeviceId {
        DeviceId("fake:chain".into())
    }

    fn other_device() -> DeviceId {
        DeviceId("fake:other".into())
    }

    fn whole(device: &DeviceId, zone: u32) -> Light {
        Light {
            id: zone_light_id(device, zone),
            device_id: device.clone(),
            zone_index: zone,
            leds: None,
            name: None,
            arrangement: Arrangement::Strip,
            reversed: false,
            kind: None,
        }
    }

    fn old_part(
        device: &DeviceId,
        zone: u32,
        n: u32,
        run: (u32, u32),
        name: &str,
        kind: ChainPartKind,
        reversed: bool,
    ) -> Light {
        Light {
            id: part_light_id(device, zone, n),
            device_id: device.clone(),
            zone_index: zone,
            leds: Some(LedSlice { start: run.0, count: run.1 }),
            name: Some(name.into()),
            arrangement: kind.arrangement(),
            reversed,
            kind: Some(kind),
        }
    }

    fn part(name: &str, kind: ChainPartKind, leds: u32) -> SetupPart {
        SetupPart { name: name.into(), kind, leds }
    }

    fn group(id: &str, members: &[&str]) -> Group {
        Group {
            id: id.into(),
            name: id.into(),
            members: members.iter().map(|m| (*m).to_string()).collect(),
            plays: Plays::Off,
            mode: GroupMode::FlowAcross,
            brightness: 100,
        }
    }

    fn scene(id: &str, groups: Vec<Group>) -> Scene {
        Scene { id: id.into(), name: id.into(), rev: 0, groups }
    }

    fn scenes(list: Vec<Scene>) -> ScenesFile {
        ScenesFile {
            schema_version: SCENES_SCHEMA,
            active: list[0].id.clone(),
            scenes: list,
        }
    }

    fn lights_file(list: Vec<Light>) -> LightsFile {
        LightsFile { schema_version: LIGHTS_SCHEMA, lights: list, chains: Vec::new() }
    }

    fn part_ids(device: &DeviceId, zone: u32, count: u32) -> Vec<LightId> {
        (1..=count).map(|n| part_light_id(device, zone, n)).collect()
    }

    fn ids(lights: &[Light]) -> Vec<LightId> {
        lights.iter().map(|l| l.id.clone()).collect()
    }

    #[test]
    fn even_split_of_96_into_7_gives_14_14_14_14_14_13_13() {
        let split = even_split(96, 7).expect("96 splits into 7");
        assert_eq!(split, vec![14, 14, 14, 14, 14, 13, 13]);
        assert_eq!(split.iter().sum::<u32>(), 96);
    }

    #[test]
    fn a_count_above_the_length_is_refused() {
        assert_eq!(even_split(4, 5), Err(ChainPlanError::CountAboveLength));
        assert_eq!(even_split(10, 0), Err(ChainPlanError::CountAboveLength));
        assert_eq!(even_split(0, 0), Err(ChainPlanError::CountAboveLength));
        assert_eq!(even_split(5, 5), Ok(vec![1, 1, 1, 1, 1]));
    }

    #[test]
    fn splitting_a_whole_zone_light_puts_its_parts_in_its_groups_in_chain_order() {
        let d = device();
        let zone_light = zone_light_id(&d, ZONE);
        let lights = lights_file(vec![whole(&d, ZONE)]);
        let scenes = scenes(vec![scene(
            "everything",
            vec![group("everything", &[zone_light.as_str()])],
        )]);
        let parts = [part("Fan 1", ChainPartKind::Fan, 10), part("Fan 2", ChainPartKind::Fan, 10), part("Fan 3", ChainPartKind::Fan, 10)];

        let plan = plan_commit(&lights, &scenes, &d, ZONE, &parts, 40).expect("the plan fits");

        assert_eq!(plan.final_leds, 30);
        assert_eq!(ids(&plan.lights.lights), part_ids(&d, ZONE, 3));
        assert_eq!(plan.scenes.scenes[0].groups[0].members, part_ids(&d, ZONE, 3));
        assert_eq!(
            plan.lights.chains,
            vec![ChainRecord { device_id: d.clone(), zone_index: ZONE, state: ChainState::SetUp }]
        );
    }

    #[test]
    fn the_first_run_everything_scene_keeps_lighting_every_part() {
        let d = device();
        let other = other_device();
        let zone_light = zone_light_id(&d, ZONE);
        let other_light = whole(&other, 0);
        let lights = lights_file(vec![other_light.clone(), whole(&d, ZONE)]);
        let scenes = scenes(vec![scene(
            "everything",
            vec![group("everything", &[other_light.id.as_str(), zone_light.as_str()])],
        )]);
        let parts = [
            part("Fan 1", ChainPartKind::Fan, 8),
            part("Fan 2", ChainPartKind::Fan, 8),
            part("Strip 3", ChainPartKind::Strip, 8),
        ];

        let plan = plan_commit(&lights, &scenes, &d, ZONE, &parts, 24).expect("the plan fits");

        let mut expected = vec![other_light.id.clone()];
        expected.extend(part_ids(&d, ZONE, 3));
        assert_eq!(plan.scenes.scenes[0].groups[0].members, expected);
        assert!(canvas_proto::scene::unassigned(&plan.scenes.scenes[0], &plan.lights.lights)
            .iter()
            .all(|l| l.id != other_light.id));
        assert_eq!(
            canvas_proto::scene::unassigned(&plan.scenes.scenes[0], &plan.lights.lights).len(),
            0,
            "every part stays lit by the Everything scene"
        );
    }

    #[test]
    fn parts_keep_reversed_and_groups_by_position_when_lengths_change() {
        let d = device();
        let (p1, p2, p3) = (
            part_light_id(&d, ZONE, 1),
            part_light_id(&d, ZONE, 2),
            part_light_id(&d, ZONE, 3),
        );
        let lights = lights_file(vec![
            old_part(&d, ZONE, 1, (0, 10), "Fan 1", ChainPartKind::Fan, true),
            old_part(&d, ZONE, 2, (10, 10), "Fan 2", ChainPartKind::Fan, false),
            old_part(&d, ZONE, 3, (20, 10), "Fan 3", ChainPartKind::Fan, true),
        ]);
        let scenes = scenes(vec![scene(
            "desk",
            vec![group("intake", &[p1.as_str(), p3.as_str()]), group("exhaust", &[p2.as_str()])],
        )]);
        let mut with_chains = lights;
        with_chains.chains = vec![
            ChainRecord { device_id: d.clone(), zone_index: ZONE, state: ChainState::SetUp },
            ChainRecord {
                device_id: other_device(),
                zone_index: 2,
                state: ChainState::Empty,
            },
        ];
        let parts = [
            part("Intake", ChainPartKind::Fan, 8),
            part("Side strip", ChainPartKind::Strip, 12),
            part("Exhaust", ChainPartKind::Fan, 5),
        ];

        let plan = plan_commit(&with_chains, &scenes, &d, ZONE, &parts, 30).expect("the plan fits");

        let made = &plan.lights.lights;
        assert_eq!(ids(made), vec![p1.clone(), p2.clone(), p3.clone()]);
        assert_eq!(made[0].name.as_deref(), Some("Intake"));
        assert!(made[0].reversed);
        assert_eq!(made[1].name.as_deref(), Some("Side strip"));
        assert!(!made[1].reversed);
        assert_eq!(made[1].kind, Some(ChainPartKind::Strip));
        assert_eq!(made[1].arrangement, Arrangement::Strip);
        assert_eq!(made[2].name.as_deref(), Some("Exhaust"));
        assert!(made[2].reversed);
        assert_eq!(plan.scenes.scenes[0].groups[0].members, vec![p1, p3]);
        assert_eq!(plan.scenes.scenes[0].groups[1].members, vec![p2]);
        assert_eq!(plan.lights.chains.len(), 2);
        assert_eq!(
            plan.lights.chains[1],
            ChainRecord { device_id: other_device(), zone_index: 2, state: ChainState::Empty }
        );
    }

    #[test]
    fn shrinking_a_chain_removes_only_the_missing_parts_from_groups() {
        let d = device();
        let (p1, p2, p3, p4) = (
            part_light_id(&d, ZONE, 1),
            part_light_id(&d, ZONE, 2),
            part_light_id(&d, ZONE, 3),
            part_light_id(&d, ZONE, 4),
        );
        let lights = lights_file(vec![
            old_part(&d, ZONE, 1, (0, 5), "Fan 1", ChainPartKind::Fan, false),
            old_part(&d, ZONE, 2, (5, 5), "Fan 2", ChainPartKind::Fan, false),
            old_part(&d, ZONE, 3, (10, 5), "Fan 3", ChainPartKind::Fan, false),
            old_part(&d, ZONE, 4, (15, 5), "Fan 4", ChainPartKind::Fan, false),
        ]);
        let scenes = scenes(vec![scene(
            "desk",
            vec![
                group("front", &[p1.as_str(), p2.as_str()]),
                group("side", &[p3.as_str()]),
                group("rear", &[p4.as_str()]),
            ],
        )]);
        let parts = [part("Fan 1", ChainPartKind::Fan, 6), part("Fan 2", ChainPartKind::Fan, 6)];

        let plan = plan_commit(&lights, &scenes, &d, ZONE, &parts, 12).expect("the plan fits");

        assert_eq!(plan.lights.lights.len(), 2);
        assert_eq!(plan.scenes.scenes[0].groups.len(), 1);
        assert_eq!(plan.scenes.scenes[0].groups[0].id, "front");
        assert_eq!(plan.scenes.scenes[0].groups[0].members, vec![p1, p2]);
    }

    #[test]
    fn growing_a_chain_adds_unassigned_parts() {
        let d = device();
        let (p1, p2) = (part_light_id(&d, ZONE, 1), part_light_id(&d, ZONE, 2));
        let lights = lights_file(vec![
            old_part(&d, ZONE, 1, (0, 5), "Fan 1", ChainPartKind::Fan, false),
            old_part(&d, ZONE, 2, (5, 5), "Fan 2", ChainPartKind::Fan, false),
        ]);
        let scenes = scenes(vec![scene("desk", vec![group("front", &[p1.as_str(), p2.as_str()])])]);
        let parts = [
            part("Fan 1", ChainPartKind::Fan, 6),
            part("Fan 2", ChainPartKind::Fan, 6),
            part("Fan 3", ChainPartKind::Fan, 6),
            part("Fan 4", ChainPartKind::Fan, 6),
        ];

        let plan = plan_commit(&lights, &scenes, &d, ZONE, &parts, 24).expect("the plan fits");

        assert_eq!(plan.lights.lights.len(), 4);
        assert_eq!(plan.scenes.scenes[0].groups[0].members, vec![p1, p2]);
        let loose: Vec<LightId> = canvas_proto::scene::unassigned(
            &plan.scenes.scenes[0],
            &plan.lights.lights,
        )
        .iter()
        .map(|l| l.id.clone())
        .collect();
        assert_eq!(loose, part_ids(&d, ZONE, 4)[2..].to_vec());
    }

    #[test]
    fn a_group_left_empty_by_a_shrink_is_dropped() {
        let d = device();
        let (p1, p2) = (part_light_id(&d, ZONE, 1), part_light_id(&d, ZONE, 2));
        let lights = lights_file(vec![
            old_part(&d, ZONE, 1, (0, 5), "Fan 1", ChainPartKind::Fan, false),
            old_part(&d, ZONE, 2, (5, 5), "Fan 2", ChainPartKind::Fan, false),
        ]);
        let scenes = scenes(vec![scene(
            "desk",
            vec![group("front", &[p1.as_str()]), group("rear", &[p2.as_str()])],
        )]);
        let parts = [part("Fan 1", ChainPartKind::Fan, 5)];

        let plan = plan_commit(&lights, &scenes, &d, ZONE, &parts, 10).expect("the plan fits");

        assert_eq!(plan.scenes.scenes[0].groups.len(), 1);
        assert_eq!(plan.scenes.scenes[0].groups[0].id, "front");
        assert_eq!(plan.scenes.scenes[0].groups[0].members, vec![p1]);
    }

    #[test]
    fn changing_a_parts_kind_changes_its_arrangement_but_keeps_its_id() {
        let d = device();
        let (p1, p2) = (part_light_id(&d, ZONE, 1), part_light_id(&d, ZONE, 2));
        let lights = lights_file(vec![
            old_part(&d, ZONE, 1, (0, 10), "Fan 1", ChainPartKind::Fan, false),
            old_part(&d, ZONE, 2, (10, 10), "Fan 2", ChainPartKind::Fan, false),
        ]);
        let scenes = scenes(vec![scene("desk", vec![group("front", &[p1.as_str(), p2.as_str()])])]);
        let parts = [part("Fan 1", ChainPartKind::Fan, 10), part("Fan 2", ChainPartKind::Strip, 10)];

        let plan = plan_commit(&lights, &scenes, &d, ZONE, &parts, 20).expect("the plan fits");

        assert_eq!(plan.lights.lights[1].id, p2);
        assert_eq!(plan.lights.lights[1].kind, Some(ChainPartKind::Strip));
        assert_eq!(plan.lights.lights[1].arrangement, Arrangement::Strip);
        assert_eq!(plan.lights.lights[0].kind, Some(ChainPartKind::Fan));
        assert_eq!(plan.lights.lights[0].arrangement, Arrangement::Ring);
        assert_eq!(plan.scenes.scenes[0].groups[0].members, vec![p1, p2]);
    }

    #[test]
    fn other_is_stored_as_other_with_a_strip_arrangement() {
        let d = device();
        let lights = lights_file(vec![whole(&d, ZONE)]);
        let scenes = scenes(vec![scene("desk", vec![group("front", &[zone_light_id(&d, ZONE).as_str()])])]);
        let parts = [part("Pump", ChainPartKind::Other, 6)];

        let plan = plan_commit(&lights, &scenes, &d, ZONE, &parts, 10).expect("the plan fits");

        assert_eq!(plan.lights.lights[0].kind, Some(ChainPartKind::Other));
        assert_eq!(plan.lights.lights[0].arrangement, Arrangement::Strip);
    }

    #[test]
    fn a_row_of_zero_leds_is_refused() {
        let d = device();
        let lights = lights_file(vec![whole(&d, ZONE)]);
        let scenes = scenes(vec![scene("desk", vec![group("front", &[zone_light_id(&d, ZONE).as_str()])])]);
        let parts = [part("Fan 1", ChainPartKind::Fan, 10), part("Fan 2", ChainPartKind::Fan, 0)];

        assert_eq!(
            plan_commit(&lights, &scenes, &d, ZONE, &parts, 100),
            Err(ChainPlanError::PartUnderOneLed { index: 1 })
        );
    }

    #[test]
    fn a_sum_above_the_zone_maximum_is_refused() {
        let d = device();
        let lights = lights_file(vec![whole(&d, ZONE)]);
        let scenes = scenes(vec![scene("desk", vec![group("front", &[zone_light_id(&d, ZONE).as_str()])])]);
        let parts = [part("Fan 1", ChainPartKind::Fan, 10), part("Fan 2", ChainPartKind::Fan, 10)];

        assert_eq!(
            plan_commit(&lights, &scenes, &d, ZONE, &parts, 19),
            Err(ChainPlanError::SumAboveMax { sum: 20, max: 19 })
        );
        assert!(plan_commit(&lights, &scenes, &d, ZONE, &parts, 20).is_ok());
    }

    #[test]
    fn a_name_over_the_limit_is_refused() {
        let d = device();
        let lights = lights_file(vec![whole(&d, ZONE)]);
        let scenes = scenes(vec![scene("desk", vec![group("front", &[zone_light_id(&d, ZONE).as_str()])])]);
        let long = "a".repeat(PART_NAME_MAX + 1);
        let parts = [part(&long, ChainPartKind::Fan, 10)];

        assert_eq!(
            plan_commit(&lights, &scenes, &d, ZONE, &parts, 100),
            Err(ChainPlanError::NameTooLong { index: 0 })
        );
        let fits = "b".repeat(PART_NAME_MAX);
        assert!(plan_commit(
            &lights,
            &scenes,
            &d,
            ZONE,
            &[part(&fits, ChainPartKind::Fan, 10)],
            100
        )
        .is_ok());
    }

    #[test]
    fn an_empty_part_list_is_refused() {
        let d = device();
        let lights = lights_file(vec![whole(&d, ZONE)]);
        let scenes = scenes(vec![scene("desk", vec![group("front", &[zone_light_id(&d, ZONE).as_str()])])]);

        assert_eq!(
            plan_commit(&lights, &scenes, &d, ZONE, &[], 100),
            Err(ChainPlanError::NoParts)
        );
    }

    #[test]
    fn plan_empty_removes_the_zone_from_lights_and_groups_and_marks_it_empty() {
        let d = device();
        let other = other_device();
        let other_light = whole(&other, 0);
        let (p1, p2) = (part_light_id(&d, ZONE, 1), part_light_id(&d, ZONE, 2));
        let mut lights = lights_file(vec![
            other_light.clone(),
            old_part(&d, ZONE, 1, (0, 5), "Fan 1", ChainPartKind::Fan, false),
            old_part(&d, ZONE, 2, (5, 5), "Fan 2", ChainPartKind::Fan, false),
        ]);
        lights.chains = vec![ChainRecord {
            device_id: d.clone(),
            zone_index: ZONE,
            state: ChainState::SetUp,
        }];
        let scenes = scenes(vec![scene(
            "desk",
            vec![
                group("front", &[p1.as_str(), other_light.id.as_str()]),
                group("rear", &[p2.as_str()]),
            ],
        )]);

        let plan = plan_empty(&lights, &scenes, &d, ZONE);

        assert_eq!(plan.final_leds, 0);
        assert_eq!(ids(&plan.lights.lights), vec![other_light.id.clone()]);
        assert_eq!(plan.scenes.scenes[0].groups.len(), 1);
        assert_eq!(plan.scenes.scenes[0].groups[0].id, "front");
        assert_eq!(plan.scenes.scenes[0].groups[0].members, vec![other_light.id]);
        assert_eq!(
            plan.lights.chains,
            vec![ChainRecord { device_id: d.clone(), zone_index: ZONE, state: ChainState::Empty }]
        );
    }

    #[test]
    fn other_zones_lights_are_untouched() {
        let d = device();
        let other = other_device();
        let zone_one = whole(&d, 1);
        let other_light = whole(&other, 0);
        let lights = lights_file(vec![
            whole(&d, ZONE),
            zone_one.clone(),
            other_light.clone(),
        ]);
        let scenes = scenes(vec![scene(
            "desk",
            vec![group(
                "elsewhere",
                &[zone_one.id.as_str(), other_light.id.as_str()],
            )],
        )]);
        let parts = [part("Fan 1", ChainPartKind::Fan, 4), part("Fan 2", ChainPartKind::Fan, 4)];

        let plan = plan_commit(&lights, &scenes, &d, ZONE, &parts, 8).expect("the plan fits");

        let mut expected = part_ids(&d, ZONE, 2);
        expected.push(zone_one.id.clone());
        expected.push(other_light.id.clone());
        assert_eq!(ids(&plan.lights.lights), expected);
        assert_eq!(plan.lights.lights[2], zone_one);
        assert_eq!(plan.lights.lights[3], other_light.clone());
        assert_eq!(
            plan.scenes.scenes[0].groups[0].members,
            vec![zone_one.id, other_light.id]
        );
    }

    #[test]
    fn every_planned_scene_validates() {
        let d = device();
        let (p1, p2) = (part_light_id(&d, ZONE, 1), part_light_id(&d, ZONE, 2));
        let lights = lights_file(vec![
            old_part(&d, ZONE, 1, (0, 5), "Fan 1", ChainPartKind::Fan, false),
            old_part(&d, ZONE, 2, (5, 5), "Fan 2", ChainPartKind::Fan, false),
            whole(&other_device(), 0),
        ]);
        let scenes = scenes(vec![
            scene("desk", vec![group("front", &[p1.as_str()]), group("rear", &[p2.as_str()])]),
            scene("everything", vec![group("all", &[p1.as_str(), p2.as_str()])]),
        ]);
        let parts = [
            part("Fan 1", ChainPartKind::Fan, 3),
            part("Fan 2", ChainPartKind::Strip, 7),
            part("Fan 3", ChainPartKind::Other, 2),
        ];

        let plan = plan_commit(&lights, &scenes, &d, ZONE, &parts, 12).expect("the plan fits");
        for scene in &plan.scenes.scenes {
            validate_scene(scene, &plan.lights.lights).expect("a planned scene validates");
        }

        let empty = plan_empty(&lights, &scenes, &d, ZONE);
        for scene in &empty.scenes.scenes {
            validate_scene(scene, &empty.lights.lights).expect("an emptied scene validates");
        }
    }
}
