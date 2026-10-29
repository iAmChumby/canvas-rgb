//! The one-time migration from the built-in Poison to the user HTML effect that
//! replaces it.
//!
//! Poison is SignalRGB's own effect, so the repo ships no copy of it. It lives on the
//! owner's machine as a user HTML effect in `%APPDATA%\canvas-rgb\effects\poison.html`.
//! A user effect's id is `html:` plus the file's stem, so that file's id is
//! `html:poison`.
//!
//! The owner's saved layout, saved property values and favourites name the effect
//! `poison`, the id it had while it was a built-in. This module rewrites those ids to
//! `html:poison`, so the saved setup comes back exactly as it was: the same effect
//! active, with the same values (the background `#2d0075`, the three colours and the
//! speed 35).
//!
//! The migration is idempotent and never touches an id that is not the built-in's.
//!
//! The second migration is the case: the editor's case picker is gone, and the case is
//! one box the owner sizes. A layout saved with a case form gets a case landmark of
//! that form's size, and the form is written back as `None`. The zones are not touched.

use canvas_proto::config::{EffectsFile, LayoutsFile};
use canvas_proto::layout::{CaseForm, PartKind};

/// The id Poison had while it was a built-in. The effect library does not hold it any
/// more (it is not one of ours), so the constant lives here, with the migration that
/// needs it.
pub const LEGACY_POISON_ID: &str = "poison";

/// The id of the user HTML effect that replaces it.
pub const USER_POISON_ID: &str = "html:poison";

/// The file the user effect has to be installed as, in `%APPDATA%\canvas-rgb\effects\`.
pub const USER_POISON_FILE: &str = "poison.html";

/// Rewrites every saved id that names the built-in Poison to the user effect's id.
///
/// Returns true when something changed. The property values are moved, never changed:
/// the effect the owner's file declares is the same effect, so the values still fit.
pub fn migrate_poison(layouts: &mut LayoutsFile, effects: &mut EffectsFile) -> bool {
    let mut changed = false;

    // The effect on every layout, the active one and the others.
    for layout in &mut layouts.layouts {
        if let Some(active) = &mut layout.effect {
            if active.id == LEGACY_POISON_ID {
                active.id = USER_POISON_ID.to_string();
                changed = true;
            }
        }
    }

    // The saved property values.
    if let Some(props) = effects.saved.remove(LEGACY_POISON_ID) {
        effects.saved.entry(USER_POISON_ID.to_string()).or_insert(props);
        changed = true;
    }

    // The owner's own presets for the effect.
    if let Some(presets) = effects.presets.remove(LEGACY_POISON_ID) {
        effects.presets.entry(USER_POISON_ID.to_string()).or_insert(presets);
        changed = true;
    }

    // The favourites.
    if effects.favorites.iter().any(|f| f == LEGACY_POISON_ID) {
        effects.favorites.retain(|f| f != LEGACY_POISON_ID);
        if !effects.favorites.iter().any(|f| f == USER_POISON_ID) {
            effects.favorites.push(USER_POISON_ID.to_string());
        }
        effects.favorites.sort();
        changed = true;
    }

    changed
}

/// Moves every saved case form to a case landmark.
///
/// The editor's case picker is gone: the case is one box the owner sizes, so a layout
/// saved with a form keeps that form's size as a case landmark and the form itself is
/// written back as `None`. A layout that already has a case landmark keeps it, and a
/// layout with no case form and no zones is not touched. A layout with zones and no case
/// landmark at all gets the default mid-tower case. The zones are never touched.
///
/// Returns true when something changed.
pub fn migrate_case_form(layouts: &mut LayoutsFile) -> bool {
    let mut changed = false;
    for layout in &mut layouts.layouts {
        if layout.case_form == CaseForm::None {
            // No form and no case box, but zones to place against one: give the layout the
            // default mid-tower case so there is a box to size. An empty layout waits for
            // first-run to make its own.
            let has_case = layout.landmarks.iter().any(|l| l.kind == PartKind::Case);
            if !has_case && !layout.zones.is_empty() {
                layout.landmarks.push(crate::first_run::case_landmark());
                changed = true;
            }
            continue;
        }
        let has_case = layout.landmarks.iter().any(|l| l.kind == PartKind::Case);
        if !has_case {
            if let Some(case) = crate::first_run::case_landmark_of(layout.case_form) {
                layout.landmarks.push(case);
            }
        }
        layout.case_form = CaseForm::None;
        changed = true;
    }
    changed
}

/// The same migration, on the daemon's own store.
pub fn migrate_store(store: &mut crate::storage::Store) -> bool {
    let poison = migrate_poison(&mut store.layouts, &mut store.effects);
    let cases = migrate_case_form(&mut store.layouts);
    poison || cases
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::layout::{ActiveEffect, Layout};
    use canvas_proto::props::{PropValue, Props};

    /// The property values the owner has stored for the effect.
    fn stored_props() -> Props {
        let mut props = Props::new();
        for (key, hex) in [
            ("bgColor", "#2d0075"),
            ("color1", "#6000fc"),
            ("color2", "#b300ff"),
            ("color3", "#8a42ff"),
        ] {
            props.insert(
                key.to_string(),
                PropValue::Color(hex.parse().expect("a hex color")),
            );
        }
        props.insert("speedRaw".to_string(), PropValue::Number(35.0));
        props
    }

    fn layouts_with(effect_id: &str) -> LayoutsFile {
        LayoutsFile {
            schema_version: canvas_proto::config::LAYOUTS_SCHEMA,
            active: "default".into(),
            layouts: vec![Layout {
                id: "default".into(),
                name: "Default".into(),
                render_scale: Default::default(),
                sampling: Default::default(),
                zones: Vec::new(),
                effect: Some(ActiveEffect { id: effect_id.to_string(), props: stored_props() }),
                case_form: canvas_proto::layout::CaseForm::None,
                landmarks: Vec::new(),
            }],
        }
    }

    fn effects_with(effect_id: &str) -> EffectsFile {
        EffectsFile {
            schema_version: canvas_proto::config::EFFECTS_SCHEMA,
            saved: [(effect_id.to_string(), stored_props())].into_iter().collect(),
            presets: Default::default(),
            favorites: vec![effect_id.to_string()],
        }
    }

    #[test]
    fn the_saved_effect_id_becomes_the_user_effect_id() {
        let mut layouts = layouts_with(LEGACY_POISON_ID);
        let mut effects = effects_with(LEGACY_POISON_ID);

        assert!(migrate_poison(&mut layouts, &mut effects));

        // The same effect, active, with the same values.
        let active = layouts.layouts[0].effect.as_ref().expect("the layout has an effect");
        assert_eq!(active.id, USER_POISON_ID);
        assert_eq!(active.props, stored_props());

        // The saved values and the favourite moved with it.
        assert_eq!(effects.saved.get(USER_POISON_ID), Some(&stored_props()));
        assert!(!effects.saved.contains_key(LEGACY_POISON_ID));
        assert_eq!(effects.favorites, vec![USER_POISON_ID.to_string()]);
    }

    #[test]
    fn the_migration_runs_only_once() {
        let mut layouts = layouts_with(LEGACY_POISON_ID);
        let mut effects = effects_with(LEGACY_POISON_ID);
        assert!(migrate_poison(&mut layouts, &mut effects));
        assert!(!migrate_poison(&mut layouts, &mut effects), "the second run changes nothing");
    }

    #[test]
    fn another_effect_is_left_alone() {
        let mut layouts = layouts_with("aurora");
        let mut effects = effects_with("aurora");
        assert!(!migrate_poison(&mut layouts, &mut effects));
        assert_eq!(layouts.layouts[0].effect.as_ref().unwrap().id, "aurora");
        assert!(effects.saved.contains_key("aurora"));
        assert_eq!(effects.favorites, vec!["aurora".to_string()]);
    }

    #[test]
    fn a_mid_tower_layout_gets_a_case_landmark_and_no_form() {
        let mut layouts = layouts_with("aurora");
        layouts.layouts[0].case_form = CaseForm::MidTower;

        assert!(migrate_case_form(&mut layouts));

        let layout = &layouts.layouts[0];
        assert_eq!(layout.case_form, CaseForm::None, "the picker is gone");
        let case = layout
            .landmarks
            .iter()
            .find(|l| l.kind == PartKind::Case)
            .expect("the case is a landmark now");
        // 488 mm deep and 480 mm tall, at 4 mm per unit: 122 x 120 units.
        assert_eq!((case.width, case.height), (122.0, 120.0));
        assert_eq!((case.x, case.y), (99.0, 29.0));
        assert_eq!(case.rotation_deg, 0.0);
    }

    #[test]
    fn the_case_migration_leaves_the_zones_alone() {
        let mut layouts = layouts_with("aurora");
        layouts.layouts[0].case_form = CaseForm::FullTower;
        let zone = canvas_proto::layout::PlacedZone {
            device_id: canvas_proto::DeviceId("d0".into()),
            zone_index: 0,
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            rotation_deg: 5.0,
            reversed: true,
            visible: true,
            shape: canvas_proto::layout::ZoneShape::Ring,
            label: None,
            leds: None,
            part: None,
            locked: false,
        };
        layouts.layouts[0].zones.push(zone.clone());

        assert!(migrate_case_form(&mut layouts));

        assert_eq!(layouts.layouts[0].zones, vec![zone], "the zones are not touched");
        // A full tower is 576 mm deep and 584 mm tall: 144 x 146 units.
        let case = layouts.layouts[0]
            .landmarks
            .iter()
            .find(|l| l.kind == PartKind::Case)
            .expect("the case is a landmark now");
        assert_eq!((case.width, case.height), (144.0, 146.0));
        assert_eq!((case.x, case.y), (88.0, 3.0));
    }

    #[test]
    fn a_layout_with_no_case_form_is_not_touched() {
        let mut layouts = layouts_with("aurora");
        assert!(!migrate_case_form(&mut layouts));
        assert!(layouts.layouts[0].landmarks.is_empty());
        assert_eq!(layouts.layouts[0].case_form, CaseForm::None);
    }

    #[test]
    fn a_layout_with_zones_and_no_case_gets_the_default_case_and_keeps_its_zones() {
        let mut layouts = layouts_with("aurora");
        let zone = canvas_proto::layout::PlacedZone {
            device_id: canvas_proto::DeviceId("d0".into()),
            zone_index: 0,
            x: 11.0,
            y: 22.0,
            width: 33.0,
            height: 44.0,
            rotation_deg: 5.0,
            reversed: false,
            visible: true,
            shape: canvas_proto::layout::ZoneShape::Ring,
            label: None,
            leds: None,
            part: None,
            locked: false,
        };
        layouts.layouts[0].zones.push(zone.clone());
        assert_eq!(layouts.layouts[0].case_form, CaseForm::None);
        assert!(layouts.layouts[0].landmarks.is_empty());

        assert!(migrate_case_form(&mut layouts));

        let layout = &layouts.layouts[0];
        assert_eq!(layout.zones, vec![zone], "the zones are not moved");
        assert_eq!(layout.landmarks.len(), 1);
        assert_eq!(layout.landmarks[0], crate::first_run::case_landmark());
        assert!(!migrate_case_form(&mut layouts), "the second run changes nothing");
        assert_eq!(layouts.layouts[0].landmarks.len(), 1);
    }

    #[test]
    fn the_case_migration_runs_only_once() {
        let mut layouts = layouts_with("aurora");
        layouts.layouts[0].case_form = CaseForm::MidTower;
        assert!(migrate_case_form(&mut layouts));
        assert!(!migrate_case_form(&mut layouts), "the second run changes nothing");
        assert_eq!(layouts.layouts[0].landmarks.len(), 1, "one case landmark");
    }

    /// A file named `poison.html` is the effect the migration points at.
    #[test]
    fn the_file_poison_html_has_the_id_the_migration_points_at() {
        let dir = std::env::temp_dir().join(format!("canvas-rgb-migration-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the folder is created");
        std::fs::write(
            dir.join(USER_POISON_FILE),
            "Poison\n<canvas id=\"exCanvas\" width=\"320\" height=\"200\"></canvas>",
        )
        .expect("the file is written");

        // The builder is a fake, so the real HTML runtime is not needed here: the id a
        // user effect gets comes from the file's name.
        struct FakeEffect {
            schema: canvas_proto::props::PropertySchema,
        }
        impl canvas_proto::traits::Effect for FakeEffect {
            fn schema(&self) -> canvas_proto::props::PropertySchema {
                self.schema.clone()
            }
            fn init(&mut self, _props: &Props) {}
            fn update(
                &mut self,
                _dt: f32,
                _canvas: &mut canvas_proto::Canvas,
                _inputs: &canvas_proto::Inputs,
            ) {
            }
        }
        let builder: crate::user_effects::Builder = std::sync::Arc::new(|id: &str, _source: &str| {
            Ok(Box::new(FakeEffect {
                schema: canvas_proto::props::PropertySchema {
                    id: id.to_string(),
                    name: id.to_string(),
                    description: String::new(),
                    needs: Default::default(),
                    properties: Vec::new(),
                    presets: Vec::new(),
                },
            }) as Box<dyn canvas_proto::traits::Effect>)
        });
        let effects = crate::user_effects::UserEffects::new(builder);
        effects.rescan(&dir);

        let ids: Vec<String> = effects.schemas().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![USER_POISON_ID.to_string()]);
        assert!(effects.create(USER_POISON_ID).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
