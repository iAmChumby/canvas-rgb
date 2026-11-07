//! When live property values count as a saved preset, and how they differ
//! when they do not — after `app/web/src/lib/presetMatch.ts`.
//!
//! The rule: fill every missing key with the schema default, round a number
//! to its step, round a hue to a whole degree modulo 360, compare colours
//! without regard to case, ignore keys the schema does not name, then
//! compare key by key. An angle compares as its degrees, and a hue saved
//! for an angle property reads as the same degrees.

use canvas_proto::props::{Preset, PropKind, PropValue, Props, PropertyDef};

fn tidy(n: f64) -> f64 {
    (n * 1e4).round() / 1e4
}

/// The value the rule compares: rounded to the control's step, colours in
/// lower case.
fn normal_value(kind: &PropKind, value: &PropValue) -> PropValue {
    match kind {
        PropKind::Number { step, .. } if matches!(value, PropValue::Number(_)) => {
            let PropValue::Number(n) = value else { unreachable!() };
            let step = if *step > 0.0 { *step } else { 1.0 };
            PropValue::Number(tidy((*n / step).round() * step))
        }
        PropKind::Hue { .. } if matches!(value, PropValue::Hue(_)) => {
            let PropValue::Hue(n) = value else { unreachable!() };
            let whole = n.round() as i64;
            PropValue::Hue(whole.rem_euclid(360) as f64)
        }
        PropKind::Angle { .. }
            if matches!(value, PropValue::Angle(_) | PropValue::Hue(_) | PropValue::Number(_)) =>
        {
            let raw = match value {
                PropValue::Number(n) => n.clamp(0.0, 360.0),
                PropValue::Hue(n) => *n,
                PropValue::Angle(n) => *n,
                _ => 0.0,
            };
            let whole = raw.round() as i64;
            PropValue::Angle(whole.rem_euclid(360) as f64)
        }
        // Colours are bytes, so case cannot differ after a round trip.
        _ => value.clone(),
    }
}

/// Every schema key, in schema order, with the default for any the values leave out.
pub fn fill_defaults(defs: &[PropertyDef], props: &Props) -> Props {
    defs.iter()
        .map(|def| (def.key.clone(), props.get(&def.key).cloned().unwrap_or_else(|| def.kind.default_value())))
        .collect()
}

/// Every schema key, in schema order, filled with defaults and rounded.
pub fn normalise(defs: &[PropertyDef], props: &Props) -> Props {
    defs.iter()
        .map(|def| {
            let raw = props.get(&def.key).cloned().unwrap_or_else(|| def.kind.default_value());
            (def.key.clone(), normal_value(&def.kind, &raw))
        })
        .collect()
}

fn same_value(a: &PropValue, b: &PropValue) -> bool {
    match (a, b) {
        (PropValue::Color(x), PropValue::Color(y)) => x.to_hex() == y.to_hex(),
        (PropValue::Number(x), PropValue::Number(y)) => x == y,
        (PropValue::Bool(x), PropValue::Bool(y)) => x == y,
        (PropValue::Choice(x), PropValue::Choice(y)) => x == y,
        (PropValue::Hue(x), PropValue::Hue(y)) => x == y,
        (PropValue::Angle(x), PropValue::Angle(y)) => x == y,
        _ => false,
    }
}

/// True when the two sets of values are the same preset.
pub fn same_props(defs: &[PropertyDef], a: &Props, b: &Props) -> bool {
    let na = normalise(defs, a);
    let nb = normalise(defs, b);
    defs.iter().all(|d| match (na.get(&d.key), nb.get(&d.key)) {
        (Some(x), Some(y)) => same_value(x, y),
        _ => false,
    })
}

/// The first preset the values equal, or null. `presets` is searched in order.
pub fn matching_preset<'a>(defs: &[PropertyDef], presets: &'a [Preset], live: &Props) -> Option<&'a Preset> {
    presets.iter().find(|p| same_props(defs, &p.props, live))
}

/// A value in the words of the diff line: "40", "on", "#ff5e00", "120°".
pub fn format_value(v: &PropValue) -> String {
    match v {
        PropValue::Bool(b) => if *b { "on".to_string() } else { "off".to_string() },
        PropValue::Hue(n) => format!("{n}°"),
        PropValue::Angle(n) => format!("{n}°"),
        PropValue::Number(n) => format!("{}", tidy(*n)),
        PropValue::Color(c) => c.to_hex(),
        PropValue::Choice(s) => s.clone(),
    }
}

/// One property whose live value differs from the preset's.
#[derive(Debug, Clone)]
pub struct Change {
    #[allow(dead_code)]
    pub key: String,
    pub label: String,
    pub from: String,
    pub to: String,
}

/// The properties whose live value differs from the preset's, as "Speed 20 to 35".
pub fn diff_props(defs: &[PropertyDef], preset: &Props, live: &Props) -> Vec<Change> {
    let np = normalise(defs, preset);
    let nl = normalise(defs, live);
    let mut out = Vec::new();
    for def in defs {
        let (Some(from), Some(to)) = (np.get(&def.key), nl.get(&def.key)) else {
            continue;
        };
        if same_value(from, to) {
            continue;
        }
        out.push(Change {
            key: def.key.clone(),
            label: def.label.clone(),
            from: format_value(from),
            to: format_value(to),
        });
    }
    out
}

/// Folds the letters A to Z and nothing else: the daemon compares names with `eq_ignore_ascii_case`.
pub fn ascii_fold(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c }).collect()
}

/// True when the name is taken by one of the lists, ignoring ASCII case.
/// `except` may keep its own.
pub fn name_taken(name: &str, lists: &[&[Preset]], except: Option<&str>) -> bool {
    let n = ascii_fold(name.trim());
    let own = except.map(ascii_fold);
    lists.iter().any(|list| {
        list.iter().any(|p| {
            let folded = ascii_fold(&p.name);
            folded == n && Some(&folded) != own.as_ref()
        })
    })
}

/// The next free copy name: "Storm 2", then "Storm 3".
pub fn next_free_name(base: &str, lists: &[&[Preset]]) -> String {
    // Strip a trailing " N" (React `/\s+\d+$/`).
    let stem = match base.rfind(|c: char| c.is_whitespace()) {
        Some(at) if base[at + 1..].chars().all(|c| c.is_ascii_digit()) && base[at + 1..].chars().next().is_some() => {
            &base[..at]
        }
        _ => base,
    };
    for i in 2..1000 {
        let candidate = format!("{stem} {i}");
        if !name_taken(&candidate, lists, None) {
            return candidate;
        }
    }
    format!("{stem} copy")
}

/// Which preset the detail's live values mean: a match, else the one
/// applied last, modified (`runningOf`).
#[derive(Debug, Clone, PartialEq)]
pub struct RunningPreset {
    pub name: String,
    pub modified: bool,
}

pub fn running_of(
    defs: &[PropertyDef],
    built_in: &[Preset],
    user: &[Preset],
    live: &Props,
    applied: Option<&str>,
) -> Option<RunningPreset> {
    let mut all: Vec<Preset> = built_in.to_vec();
    all.extend(user.iter().cloned());
    let last_preset = applied.and_then(|name| all.iter().find(|p| p.name == name));
    // The one applied last wins a tie, so two presets with equal values do not
    // swap names.
    if let Some(last) = last_preset {
        if same_props(defs, &last.props, live) {
            return Some(RunningPreset { name: last.name.clone(), modified: false });
        }
    }
    // React searches the joined list `[...builtIn, ...user]` in order, so a
    // built-in wins a tie over a user preset with the same values.
    if let Some(matched) = matching_preset(defs, &all, live) {
        return Some(RunningPreset { name: matched.name.clone(), modified: false });
    }
    last_preset.map(|p| RunningPreset { name: p.name.clone(), modified: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> Vec<PropertyDef> {
        vec![
            PropertyDef { key: "speed".into(), label: "Speed".into(), kind: PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 20.0 } },
            PropertyDef { key: "ink".into(), label: "Ink".into(), kind: PropKind::Color { default: "#ffffff".parse().unwrap() } },
        ]
    }

    fn props(pairs: &[(&str, PropValue)]) -> Props {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn missing_keys_fill_with_the_schema_default() {
        let filled = fill_defaults(&defs(), &props(&[]));
        assert_eq!(filled.get("speed"), Some(&PropValue::Number(20.0)));
    }

    #[test]
    fn numbers_compare_on_the_step_grid() {
        let a = props(&[("speed", PropValue::Number(20.4))]);
        let b = props(&[("speed", PropValue::Number(20.0))]);
        assert!(same_props(&defs(), &a, &b));
    }

    #[test]
    fn colours_compare_regardless_of_case() {
        let a = props(&[("ink", PropValue::Color("#FF0000".parse().unwrap()))]);
        let b = props(&[("ink", PropValue::Color("#ff0000".parse().unwrap()))]);
        assert!(same_props(&defs(), &a, &b));
    }

    #[test]
    fn the_applied_preset_wins_a_tie() {
        let built = vec![Preset { name: "Calm".into(), props: props(&[("speed", PropValue::Number(20.0))]) }];
        let user = vec![Preset { name: "Mine".into(), props: props(&[("speed", PropValue::Number(20.0))]) }];
        let live = props(&[("speed", PropValue::Number(20.0))]);
        let running = running_of(&defs(), &built, &user, &live, Some("Mine")).expect("running");
        assert_eq!(running.name, "Mine");
        assert!(!running.modified);
    }

    #[test]
    fn a_modified_preset_keeps_its_name() {
        let built = vec![Preset { name: "Calm".into(), props: props(&[("speed", PropValue::Number(20.0))]) }];
        let live = props(&[("speed", PropValue::Number(35.0))]);
        let running = running_of(&defs(), &built, &[], &live, Some("Calm")).expect("running");
        assert_eq!(running.name, "Calm");
        assert!(running.modified);
    }

    #[test]
    fn moved_values_read_as_custom() {
        let built = vec![Preset { name: "Calm".into(), props: props(&[("speed", PropValue::Number(20.0))]) }];
        let live = props(&[("speed", PropValue::Number(35.0))]);
        assert!(running_of(&defs(), &built, &[], &live, None).is_none());
    }

    #[test]
    fn diff_spells_the_changes_in_schema_order() {
        let preset = props(&[("speed", PropValue::Number(20.0))]);
        let live = props(&[("speed", PropValue::Number(35.0))]);
        let changes = diff_props(&defs(), &preset, &live);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].label, "Speed");
        assert_eq!((changes[0].from.as_str(), changes[0].to.as_str()), ("20", "35"));
    }

    #[test]
    fn names_clash_without_regard_to_ascii_case() {
        let presets = vec![Preset { name: "Calm".into(), props: props(&[]) }];
        assert!(name_taken("calm", &[&presets], None));
        assert!(name_taken("CALM", &[&presets], None));
        assert!(!name_taken("calm", &[&presets], Some("Calm")));
    }

    #[test]
    fn duplicates_number_their_copies() {
        let presets = vec![Preset { name: "Storm".into(), props: props(&[]) }];
        assert_eq!(next_free_name("Storm", &[&presets]), "Storm 2");
        let presets = vec![
            Preset { name: "Storm".into(), props: props(&[]) },
            Preset { name: "Storm 2".into(), props: props(&[]) },
        ];
        assert_eq!(next_free_name("Storm", &[&presets]), "Storm 3");
    }
}
