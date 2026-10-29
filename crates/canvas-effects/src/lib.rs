//! canvas-rgb's effect library: 11 native effects drawn straight onto the canvas.
//!
//! Every effect declares its properties once in a [`PropertySchema`]; the editor
//! renders its controls from that declaration, so there is no per-effect UI code
//! anywhere in the app.
//!
//! HTML effects are not in this crate. They are user files in
//! `%APPDATA%\canvas-rgb\effects\*.html`, run by `canvas-html` and listed by the daemon
//! after the built-ins (see `canvas-daemon`'s `user_effects`).
//!
//! The two entry points the spine fixes are [`registry`] (the built-in effects, in the
//! order of [`canvas_proto::EFFECT_IDS`]) and [`create`].

pub mod audio_spectrum;
pub mod aurora;
pub mod breathing;
pub mod color;
pub mod color_cycle;
pub mod fire;
pub mod gradient;
pub mod onboard;
pub mod props_util;
pub mod rain;
pub mod rainbow_wave;
pub mod raster;
pub mod rng;
pub mod schema;
pub mod screen_ambience;
pub mod solid;
pub mod spiral;

use canvas_proto::{Effect, EffectFactory};

/// The built-in effects, in `canvas_proto::EFFECT_IDS` order.
///
/// These are the 11 effects that are ours. A user HTML effect is not one of them: it
/// lives on the owner's machine as a file in `%APPDATA%\canvas-rgb\effects\`, with the
/// id `html:` plus the file's stem.
pub fn registry() -> Vec<EffectFactory> {
    vec![
        solid::create as EffectFactory,
        gradient::create as EffectFactory,
        rainbow_wave::create as EffectFactory,
        spiral::create as EffectFactory,
        breathing::create as EffectFactory,
        color_cycle::create as EffectFactory,
        aurora::create as EffectFactory,
        rain::create as EffectFactory,
        fire::create as EffectFactory,
        screen_ambience::create as EffectFactory,
        audio_spectrum::create as EffectFactory,
    ]
}

/// Creates the effect with this id, or `None`. Only the requested effect is built.
///
/// The spine's id list holds the 11 effects that are ours, so every id in it builds.
/// An id that is not in the list is not a built-in: `create("html:poison")` is `None`,
/// because Poison is a user HTML effect now.
pub fn create(id: &str) -> Option<Box<dyn Effect>> {
    // The hidden onboarding effects build by id but are not in the registry.
    if let Some(effect) = onboard::create_hidden(id) {
        return Some(effect);
    }
    let index = canvas_proto::EFFECT_IDS.iter().position(|known| *known == id)?;
    let registry = registry();
    let factory = registry.get(index)?;
    let effect = factory();
    // The registry and the spine must agree on the order. If they ever do not, the id
    // check stops one effect from being built under another id.
    (effect.schema().id == id).then_some(effect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::{Canvas, EFFECT_IDS, Inputs, Props};

    /// The display names, in registry order.
    const NAMES: [&str; 11] = [
        "Solid",
        "Gradient",
        "Rainbow Wave",
        "Spiral",
        "Breathing",
        "Color Cycle",
        "Aurora",
        "Rain",
        "Fire",
        "Screen Ambience",
        "Audio Spectrum",
    ];

    /// The ids of the built-in effects, in registry order: the spine's ids.
    fn native_ids() -> Vec<&'static str> {
        EFFECT_IDS.to_vec()
    }

    /// The registry lists the built-in effects, in order.
    #[test]
    fn the_registry_holds_the_eleven_native_effects_in_order() {
        let reg = registry();
        assert_eq!(reg.len(), 11);
        let ids: Vec<String> = reg.iter().map(|f| f().schema().id).collect();
        assert_eq!(ids, native_ids());
    }

    #[test]
    fn every_registry_entry_builds_with_its_id_and_name() {
        let reg = registry();
        let ids = native_ids();
        for (i, make) in reg.iter().enumerate() {
            let s = make().schema();
            assert_eq!(s.id, ids[i]);
            assert_eq!(s.name, NAMES[i]);
        }
    }

    /// Poison is SignalRGB's effect, so it is not one of ours: it is a user HTML effect
    /// in `%APPDATA%\canvas-rgb\effects\`, with the id `html:poison`.
    #[test]
    fn poison_is_not_a_built_in() {
        assert!(create("poison").is_none());
        assert!(create("html:poison").is_none());
        assert!(registry().iter().all(|f| f().schema().id != "poison"));
    }

    #[test]
    fn the_schema_names_are_the_display_names() {
        for (id, name) in native_ids().iter().zip(NAMES.iter()) {
            let e = create(id).expect("effect");
            assert_eq!(&e.schema().name, name, "wrong name for {id}");
        }
    }

    #[test]
    fn create_returns_none_for_an_unknown_id() {
        assert!(create("nope").is_none());
        assert!(create("").is_none());
        assert!(create("Poison").is_none(), "ids are case sensitive");
    }

    /// Every property is declared once, and every preset names every property.
    fn check_declares(ids: &[&str]) {
        for id in ids {
            let e = create(id).expect("effect");
            let s = e.schema();
            assert!(!s.properties.is_empty(), "{id} declares no properties");
            assert!(!s.presets.is_empty(), "{id} has no presets");
            let mut keys: Vec<&str> = s.properties.iter().map(|p| p.key.as_str()).collect();
            keys.sort_unstable();
            let count = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), count, "{id} has duplicate property keys");
            for preset in &s.presets {
                for key in preset.props.keys() {
                    assert!(
                        s.properties.iter().any(|p| &p.key == key),
                        "{id} preset {} names undeclared property {key}",
                        preset.name
                    );
                }
                assert_eq!(
                    preset.props.len(),
                    s.properties.len(),
                    "{id} preset {} does not cover every property",
                    preset.name
                );
            }
        }
    }

    #[test]
    fn every_effect_declares_its_properties_once_and_has_presets() {
        check_declares(&native_ids());
    }

    /// Only Screen Ambience needs the screen and only Audio Spectrum needs the audio.
    fn check_needs(ids: &[&str]) {
        for id in ids {
            let s = create(id).expect("effect").schema();
            assert_eq!(s.needs.screen, *id == "screen_ambience", "{id} screen need");
            assert_eq!(s.needs.audio, *id == "audio_spectrum", "{id} audio need");
        }
    }

    #[test]
    fn only_the_capture_effects_need_inputs() {
        check_needs(&native_ids());
    }

    /// The contract every effect must meet: `init` with the schema defaults, then 120
    /// frames at 1/30 s, all values finite and in 0..=1, at every render scale.
    fn check_renders_at_every_scale(ids: &[&str]) {
        for id in ids {
            for factor in [1.0f32, 0.5, 0.25] {
                let mut e = create(id).expect("effect");
                let schema = e.schema();
                e.init(&schema.defaults());
                let mut canvas = Canvas::with_scale(factor);
                let inputs = Inputs::default();
                for _ in 0..120 {
                    e.update(1.0 / 30.0, &mut canvas, &inputs);
                    for p in canvas.pixels.iter() {
                        for v in p {
                            assert!(
                                v.is_finite() && (0.0..=1.0).contains(v),
                                "{id} at scale {factor} produced {v:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_effect_renders_finite_values_in_range_at_every_scale() {
        check_renders_at_every_scale(&native_ids());
    }

    /// The same contract, with the inputs the capture effects ask for.
    fn check_renders_with_inputs(ids: &[&str]) {
        let screen = std::sync::Arc::new(canvas_proto::ScreenFrame {
            width: 64,
            height: 40,
            pixels: vec![canvas_proto::Rgb8::new(200, 100, 50); 64 * 40],
        });
        let audio = std::sync::Arc::new(canvas_proto::AudioSpectrum {
            centers_hz: (0..16).map(|i| 60.0 * 2f32.powf(i as f32 / 4.0)).collect(),
            magnitudes: vec![0.5; 16],
        });
        for id in ids {
            let mut e = create(id).expect("effect");
            let schema = e.schema();
            e.init(&schema.defaults());
            let mut canvas = Canvas::new(320, 200);
            let mut inputs = Inputs::default();
            if schema.needs.screen {
                inputs.screen = Some(screen.clone());
            }
            if schema.needs.audio {
                inputs.audio = Some(audio.clone());
            }
            for _ in 0..120 {
                e.update(1.0 / 30.0, &mut canvas, &inputs);
                for p in canvas.pixels.iter() {
                    for v in p {
                        assert!(
                            v.is_finite() && (0.0..=1.0).contains(v),
                            "{id} with inputs produced {v:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_effect_renders_finite_values_with_its_inputs() {
        check_renders_with_inputs(&native_ids());
    }

    /// `init` must only read the properties the effect declares: a props map with
    /// unrelated or mistyped keys must not change anything or panic.
    fn check_tolerates_junk(ids: &[&str]) {
        for id in ids {
            let mut e = create(id).expect("effect");
            let schema = e.schema();
            let mut junk = Props::new();
            junk.insert("not_a_property".into(), canvas_proto::PropValue::Number(1.0));
            for p in &schema.properties {
                junk.insert(p.key.clone(), canvas_proto::PropValue::Bool(true));
            }
            e.init(&junk);
            let mut canvas = Canvas::new(64, 40);
            for _ in 0..30 {
                e.update(1.0 / 30.0, &mut canvas, &Inputs::default());
            }
            for p in canvas.pixels.iter() {
                for v in p {
                    assert!(v.is_finite() && (0.0..=1.0).contains(v), "{id}: {v:?}");
                }
            }
        }
    }

    #[test]
    fn every_effect_tolerates_missing_and_mistyped_properties() {
        check_tolerates_junk(&native_ids());
    }

    /// A zero, negative or non-finite `dt` must not panic or produce garbage.
    fn check_bad_dt(ids: &[&str]) {
        for id in ids {
            let mut e = create(id).expect("effect");
            let schema = e.schema();
            e.init(&schema.defaults());
            let mut canvas = Canvas::new(32, 20);
            for dt in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
                e.update(dt, &mut canvas, &Inputs::default());
            }
            for p in canvas.pixels.iter() {
                for v in p {
                    assert!(v.is_finite() && (0.0..=1.0).contains(v), "{id}: {v:?}");
                }
            }
        }
    }

    #[test]
    fn every_effect_ignores_a_bad_dt() {
        check_bad_dt(&native_ids());
    }
}
