//! canvas-rgb's effect library: 11 native effects drawn straight onto the canvas, and
//! Poison, which runs SignalRGB's original `effect.html` through `canvas-html`.
//!
//! Every effect declares its properties once in a [`PropertySchema`]; the editor
//! renders its controls from that declaration, so there is no per-effect UI code
//! anywhere in the app.
//!
//! The two entry points the spine fixes are [`registry`] (exactly the 12 effects, in the
//! order of [`canvas_proto::EFFECT_IDS`]) and [`create`].

pub mod audio_spectrum;
pub mod aurora;
pub mod breathing;
pub mod color;
pub mod color_cycle;
pub mod fire;
pub mod gradient;
pub mod poison;
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

/// Exactly the 12 effects in `canvas_proto::EFFECT_IDS`, in that order.
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
        poison::create as EffectFactory,
    ]
}

/// Creates the effect with this id, or `None`. Only the requested effect is built.
pub fn create(id: &str) -> Option<Box<dyn Effect>> {
    let index = canvas_proto::EFFECT_IDS.iter().position(|known| *known == id)?;
    registry().get(index).map(|make| make())
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::{Canvas, EFFECT_IDS, Inputs, Props};

    /// The 12 display names, in `EFFECT_IDS` order.
    const NAMES: [&str; 12] = [
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
        "Poison",
    ];

    /// The 11 native effects: the ones whose tests do not touch `canvas-html`.
    fn native_ids() -> Vec<&'static str> {
        EFFECT_IDS.iter().copied().filter(|id| *id != "poison").collect()
    }

    /// The registry lists exactly the 12 ids in order. This builds only the 11 native
    /// effects; the all-twelve build needs `canvas-html` and is ignored below.
    #[test]
    fn the_registry_holds_exactly_the_twelve_effects_in_order() {
        let reg = registry();
        assert_eq!(reg.len(), 12);
        assert_eq!(EFFECT_IDS[11], "poison");
        let ids: Vec<String> = reg.iter().take(11).map(|f| f().schema().id).collect();
        assert_eq!(ids, native_ids());
    }

    #[test]
    #[ignore = "needs canvas-html"]
    fn every_registry_entry_builds_with_its_id_and_name() {
        let reg = registry();
        for (i, make) in reg.iter().enumerate() {
            let s = make().schema();
            assert_eq!(s.id, EFFECT_IDS[i]);
            assert_eq!(s.name, NAMES[i]);
        }
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
    fn native_effects_declare_their_properties_once_and_have_presets() {
        check_declares(&native_ids());
    }

    #[test]
    #[ignore = "needs canvas-html"]
    fn every_effect_declares_its_properties_once_and_has_presets() {
        check_declares(&EFFECT_IDS);
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

    #[test]
    #[ignore = "needs canvas-html"]
    fn only_the_capture_effects_need_inputs_all_twelve() {
        check_needs(&EFFECT_IDS);
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
    fn native_effects_render_finite_values_in_range_at_every_scale() {
        check_renders_at_every_scale(&native_ids());
    }

    #[test]
    #[ignore = "needs canvas-html"]
    fn every_effect_renders_finite_values_in_range_at_every_scale() {
        check_renders_at_every_scale(&EFFECT_IDS);
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
    fn native_effects_render_finite_values_with_their_inputs() {
        check_renders_with_inputs(&native_ids());
    }

    #[test]
    #[ignore = "needs canvas-html"]
    fn every_effect_renders_finite_values_with_its_inputs() {
        check_renders_with_inputs(&EFFECT_IDS);
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
    fn native_effects_tolerate_missing_and_mistyped_properties() {
        check_tolerates_junk(&native_ids());
    }

    #[test]
    #[ignore = "needs canvas-html"]
    fn every_effect_tolerates_missing_and_mistyped_properties() {
        check_tolerates_junk(&EFFECT_IDS);
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
    fn native_effects_ignore_a_bad_dt() {
        check_bad_dt(&native_ids());
    }

    #[test]
    #[ignore = "needs canvas-html"]
    fn every_effect_ignores_a_bad_dt() {
        check_bad_dt(&EFFECT_IDS);
    }
}
