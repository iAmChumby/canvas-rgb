//! Moving or copying one group from a scene to another (#368), as a pure plan.
//!
//! [`plan_move`] takes the two scenes as they are and answers the two scenes as they would be,
//! with what the move took and what it removed. It reads and writes nothing, so every rule of the
//! design is tested here alone; `api/scenes.rs` checks the revisions, runs the saveable checks,
//! saves once and tells the engine.
//!
//! The rules:
//! - A **move** takes the group out of the source; a **copy** leaves the source as it was,
//!   byte for byte, `rev` included.
//! - A light is in one group per scene, so every light the group brings is **taken** out of the
//!   target's other groups ([`GroupTaken`]); a target group left with no light is removed and
//!   named in `emptied`.
//! - The group lands at the **end** of the target. It keeps its id when the target (after the
//!   removals) has no group with that id, else it gets a fresh one (`all` becomes `all-2`); a
//!   **copy always** gets a fresh id, one the source does not hold either.
//! - Names are kept as they are: group names are not unique within a scene, so a name that
//!   collides is not renamed.
//! - The source may be left with no group (a scene with zero groups is valid); a move never
//!   deletes a scene, so the last scene is untouched.
//! - Each scene that changed is one `rev` higher; a copy's source is not a changed scene.

use canvas_proto::api::GroupTaken;
use canvas_proto::scene::{Group, Scene};

use crate::scenes::fresh_id;

/// Why a move cannot be planned. The handler turns each into its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveError {
    /// Source and target are one scene: a light is in one group per scene, so there is nowhere
    /// for the group to go.
    SameScene,
    /// The source has no group with this id.
    NoGroup(String),
}

/// The scenes after a move or a copy, and what it did.
#[derive(Debug, Clone, PartialEq)]
pub struct MovePlan {
    /// The id the group has in the target.
    pub group_id: String,
    /// The source after: unchanged (same `rev`) for a copy.
    pub from: Scene,
    /// The target after, its `rev` one higher.
    pub to: Scene,
    /// True when the source is a scene that changes (a move), so it is written and its `rev` rose.
    pub from_changed: bool,
    /// The lights taken out of the target's other groups, in the target's group order.
    pub taken: Vec<GroupTaken>,
    /// The names of the target groups left with no light and removed.
    pub emptied: Vec<String>,
}

/// Plans moving (or, with `copy`, copying) `group` of `from` to the end of `to`.
pub fn plan_move(from: &Scene, to: &Scene, group: &str, copy: bool) -> Result<MovePlan, MoveError> {
    if from.id == to.id {
        return Err(MoveError::SameScene);
    }
    let at = from
        .groups
        .iter()
        .position(|g| g.id == group)
        .ok_or_else(|| MoveError::NoGroup(group.to_string()))?;
    let moving = &from.groups[at];

    // Take the group's lights out of the target's groups; a group left with none is removed.
    let mut taken = Vec::new();
    let mut emptied = Vec::new();
    let mut kept: Vec<Group> = Vec::with_capacity(to.groups.len() + 1);
    for other in &to.groups {
        let (gone, left): (Vec<_>, Vec<_>) = other.members.iter().cloned().partition(|m| moving.members.contains(m));
        if gone.is_empty() {
            kept.push(other.clone());
            continue;
        }
        taken.push(GroupTaken { group_id: other.id.clone(), group_name: other.name.clone(), lights: gone });
        if left.is_empty() {
            emptied.push(other.name.clone());
        } else {
            kept.push(Group { members: left, ..other.clone() });
        }
    }

    // The id: a move keeps its own when the target is free of it; a copy always mints one that
    // neither scene holds.
    let group_id = if copy {
        let used = kept.iter().chain(&from.groups).map(|g| g.id.as_str());
        fresh_id(&moving.id, used)
    } else if kept.iter().any(|g| g.id == moving.id) {
        fresh_id(&moving.id, kept.iter().map(|g| g.id.as_str()))
    } else {
        moving.id.clone()
    };
    kept.push(Group { id: group_id.clone(), ..moving.clone() });

    let to_after = Scene { rev: to.rev + 1, groups: kept, ..to.clone() };
    let from_after = if copy {
        from.clone()
    } else {
        let mut groups = from.groups.clone();
        groups.remove(at);
        Scene { rev: from.rev + 1, groups, ..from.clone() }
    };
    Ok(MovePlan { group_id, from: from_after, to: to_after, from_changed: !copy, taken, emptied })
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::props::Props;
    use canvas_proto::scene::{GroupMode, Plays};

    fn group(id: &str, members: &[&str]) -> Group {
        Group {
            id: id.into(),
            name: format!("Name {id}"),
            members: members.iter().map(|m| m.to_string()).collect(),
            plays: Plays::Effect { effect: "solid".into(), props: Props::new() },
            mode: GroupMode::default(),
            brightness: 100,
        }
    }

    fn scene(id: &str, rev: u64, groups: Vec<Group>) -> Scene {
        Scene { id: id.into(), name: id.to_uppercase(), rev, groups }
    }

    fn gaming() -> Scene {
        scene("gaming", 3, vec![group("desk", &["a", "b"]), group("wall", &["c"])])
    }

    #[test]
    fn a_move_leaves_the_source_and_lands_at_the_end_of_the_target() {
        let to = scene("movie", 5, vec![group("lamp", &["x"])]);
        let plan = plan_move(&gaming(), &to, "desk", false).unwrap();
        assert_eq!(plan.group_id, "desk");
        assert_eq!(plan.from.groups.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["wall"]);
        assert_eq!(plan.to.groups.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["lamp", "desk"]);
        assert_eq!((plan.from.rev, plan.to.rev), (4, 6));
        assert!(plan.from_changed && plan.taken.is_empty() && plan.emptied.is_empty());
    }

    #[test]
    fn a_copy_leaves_the_source_exactly_as_it_was_and_mints_a_fresh_id() {
        let from = gaming();
        let to = scene("movie", 5, vec![group("lamp", &["x"])]);
        let plan = plan_move(&from, &to, "desk", true).unwrap();
        assert_eq!(plan.from, from, "the source is the same scene, rev included");
        assert!(!plan.from_changed);
        assert_eq!(plan.group_id, "desk-2", "a copy never keeps the id, even when the target is free of it");
        assert_eq!(plan.to.groups.last().unwrap().id, "desk-2");
        assert_eq!(plan.to.rev, 6);
    }

    #[test]
    fn a_taken_id_is_replaced_by_a_fresh_one_on_a_move() {
        let to = scene("movie", 1, vec![group("desk", &["x"]), group("desk-2", &["y"])]);
        let plan = plan_move(&gaming(), &to, "desk", false).unwrap();
        assert_eq!(plan.group_id, "desk-3");
        assert_eq!(plan.to.groups.last().unwrap().id, "desk-3");
    }

    #[test]
    fn an_id_freed_by_the_removal_is_kept() {
        // The target's `desk` holds only the light the move brings, so it is removed and its id is free.
        let to = scene("movie", 1, vec![group("desk", &["a"]), group("lamp", &["x"])]);
        let plan = plan_move(&gaming(), &to, "desk", false).unwrap();
        assert_eq!(plan.group_id, "desk");
        assert_eq!(plan.emptied, ["Name desk"]);
    }

    #[test]
    fn the_lights_it_brings_are_taken_from_the_targets_other_groups() {
        let to = scene("movie", 1, vec![group("wash", &["a", "x"]), group("lamp", &["y"])]);
        let plan = plan_move(&gaming(), &to, "desk", false).unwrap();
        assert_eq!(plan.taken, vec![GroupTaken { group_id: "wash".into(), group_name: "Name wash".into(), lights: vec!["a".into()] }]);
        let wash = plan.to.groups.iter().find(|g| g.id == "wash").unwrap();
        assert_eq!(wash.members, ["x"], "the group keeps the lights it still has");
        assert!(plan.emptied.is_empty());
    }

    #[test]
    fn lights_in_two_groups_of_the_target_are_taken_from_both() {
        let to = scene("movie", 1, vec![group("one", &["a", "x"]), group("two", &["b"]), group("three", &["z"])]);
        let plan = plan_move(&gaming(), &to, "desk", false).unwrap();
        assert_eq!(plan.taken.len(), 2);
        assert_eq!(plan.taken[0].group_id, "one");
        assert_eq!(plan.taken[1].group_id, "two");
        assert_eq!(plan.emptied, ["Name two"], "two had only b");
        let ids: Vec<_> = plan.to.groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, ["one", "three", "desk"]);
    }

    #[test]
    fn a_copy_takes_lights_too() {
        let to = scene("movie", 1, vec![group("wash", &["a"])]);
        let plan = plan_move(&gaming(), &to, "desk", true).unwrap();
        assert_eq!(plan.taken.len(), 1);
        assert_eq!(plan.emptied, ["Name wash"]);
        assert_eq!(plan.to.groups.len(), 1);
    }

    #[test]
    fn the_last_group_of_the_source_may_go() {
        let from = scene("solo", 2, vec![group("only", &["a"])]);
        let to = scene("movie", 1, vec![group("lamp", &["x"])]);
        let plan = plan_move(&from, &to, "only", false).unwrap();
        assert!(plan.from.groups.is_empty());
        assert_eq!(plan.from.rev, 3);
    }

    #[test]
    fn the_same_scene_and_an_unknown_group_are_refused() {
        let from = gaming();
        assert_eq!(plan_move(&from, &from, "desk", false), Err(MoveError::SameScene));
        let to = scene("movie", 1, vec![]);
        assert_eq!(plan_move(&from, &to, "nope", false), Err(MoveError::NoGroup("nope".into())));
    }

    #[test]
    fn a_name_that_collides_is_kept() {
        let mut twin = group("lamp", &["x"]);
        twin.name = "Name desk".into();
        let to = scene("movie", 1, vec![twin]);
        let plan = plan_move(&gaming(), &to, "desk", false).unwrap();
        let names: Vec<_> = plan.to.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Name desk", "Name desk"]);
    }

    #[test]
    fn the_group_keeps_its_effect_mode_and_brightness() {
        let mut from = gaming();
        from.groups[0].brightness = 42;
        let to = scene("movie", 1, vec![]);
        let plan = plan_move(&from, &to, "desk", false).unwrap();
        let landed = plan.to.groups.last().unwrap();
        assert_eq!((landed.brightness, &landed.plays, landed.mode), (42, &from.groups[0].plays, from.groups[0].mode));
        assert_eq!(landed.members, ["a", "b"]);
    }
}
