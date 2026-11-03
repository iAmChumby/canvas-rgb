//! Reading effect properties safely.
//!
//! Every effect declares its properties once in its [`PropertySchema`]. At `init` time
//! the effect reads the values it declared, falling back to the schema default when a
//! key is missing or holds the wrong type. Nothing here ever panics.

use canvas_proto::{PropValue, Props, PropertySchema};

/// Reads a color property, falling back to the schema default.
pub fn color(schema: &PropertySchema, props: &Props, key: &str) -> canvas_proto::Rgb8 {
    match props.get(key) {
        Some(PropValue::Color(c)) => *c,
        _ => default_color(schema, key),
    }
}

/// The schema's default for a color property.
fn default_color(schema: &PropertySchema, key: &str) -> canvas_proto::Rgb8 {
    schema
        .properties
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.kind.default_value())
        .and_then(|v| match v {
            PropValue::Color(c) => Some(c),
            _ => None,
        })
        .unwrap_or(canvas_proto::Rgb8::BLACK)
}

/// Reads a number property, clamped to the schema's range, falling back to the default.
pub fn number(schema: &PropertySchema, props: &Props, key: &str) -> f64 {
    let def = schema
        .properties
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.kind.default_value())
        .and_then(|v| match v {
            PropValue::Number(n) => Some(n),
            _ => None,
        })
        .unwrap_or(0.0);
    let raw = match props.get(key) {
        Some(PropValue::Number(n)) if n.is_finite() => *n,
        _ => def,
    };
    clamp_to_schema(schema, key, raw)
}

/// Reads a boolean property, falling back to the schema default.
pub fn boolean(schema: &PropertySchema, props: &Props, key: &str) -> bool {
    let def = schema
        .properties
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.kind.default_value())
        .and_then(|v| match v {
            PropValue::Bool(b) => Some(b),
            _ => None,
        })
        .unwrap_or(false);
    match props.get(key) {
        Some(PropValue::Bool(b)) => *b,
        _ => def,
    }
}

/// Reads a choice property, falling back to the schema default when the value is not one
/// of the declared options.
pub fn choice(schema: &PropertySchema, props: &Props, key: &str) -> String {
    let def = schema
        .properties
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.kind.default_value())
        .and_then(|v| match v {
            PropValue::Choice(s) => Some(s),
            _ => None,
        })
        .unwrap_or_default();
    match props.get(key) {
        Some(PropValue::Choice(s))
            if schema
                .properties
                .iter()
                .find(|p| p.key == key)
                .map(|p| matches!(&p.kind, canvas_proto::PropKind::Choice { options, .. } if options.contains(s)))
                .unwrap_or(false) =>
        {
            s.clone()
        }
        _ => def,
    }
}

/// The seed the library tile path asks for, if the props carry one.
///
/// The tile path renders every effect with a fixed seed (see [`crate::TILE_SEED`]), so a
/// tile is the same picture every time it is rendered. An effect that draws from its own
/// randomness starts its generator from this seed instead of the clock's.
pub fn tile_seed(props: &Props) -> Option<u64> {
    match props.get(crate::TILE_SEED_PROP) {
        Some(PropValue::Number(n)) if n.is_finite() && *n >= 0.0 => Some(*n as u64),
        _ => None,
    }
}

/// Reads a hue property (degrees), falling back to the schema default.
pub fn hue(schema: &PropertySchema, props: &Props, key: &str) -> f64 {
    let def = schema
        .properties
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.kind.default_value())
        .and_then(|v| match v {
            PropValue::Hue(h) => Some(h),
            _ => None,
        })
        .unwrap_or(0.0);
    let raw = match props.get(key) {
        Some(PropValue::Hue(h)) if h.is_finite() => *h,
        _ => def,
    };
    raw.rem_euclid(360.0)
}

/// Reads an angle property (degrees, 0..360: a direction), falling back to the schema
/// default.
///
/// A saved `Hue` value reads as the same number of degrees: effects saved before the angle
/// kind existed stored their angle as a hue.
pub fn angle(schema: &PropertySchema, props: &Props, key: &str) -> f64 {
    let def = schema
        .properties
        .iter()
        .find(|p| p.key == key)
        .map(|p| p.kind.default_value())
        .and_then(|v| match v {
            PropValue::Angle(a) | PropValue::Hue(a) => Some(a),
            _ => None,
        })
        .unwrap_or(0.0);
    let raw = match props.get(key) {
        Some(PropValue::Angle(a) | PropValue::Hue(a)) if a.is_finite() => *a,
        _ => def,
    };
    raw.rem_euclid(360.0)
}

/// Reads an angle property that older saves stored as a plain `Number` of degrees (Rainbow
/// Wave's direction): a saved number reads as the same direction, and every other value reads
/// as [`angle`] reads it. Only an effect whose angle used to be a number should call this; the
/// other effects keep the strict reader, which takes a mistyped number for a stray value.
pub fn angle_or_saved_number(schema: &PropertySchema, props: &Props, key: &str) -> f64 {
    match props.get(key) {
        // The old reader clamped a number to 0..360, so 405 and -90 read as 360 and 0: the
        // same direction, then 360 wraps to 0.
        Some(PropValue::Number(n)) if n.is_finite() => n.clamp(0.0, 360.0).rem_euclid(360.0),
        _ => angle(schema, props, key),
    }
}

/// Clamps `value` to the range the schema declares for `key`, if it declares one.
fn clamp_to_schema(schema: &PropertySchema, key: &str, value: f64) -> f64 {
    for p in &schema.properties {
        if p.key != key {
            continue;
        }
        if let canvas_proto::PropKind::Number { min, max, .. } = p.kind {
            return value.clamp(min, max);
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::{PropKind, PropertyDef};

    fn schema() -> PropertySchema {
        PropertySchema {
            id: "t".into(),
            name: "t".into(),
            description: String::new(),
            needs: Default::default(),
            properties: vec![
                PropertyDef {
                    key: "c".into(),
                    label: "c".into(),
                    kind: PropKind::Color { default: canvas_proto::Rgb8::new(1, 2, 3) },
                },
                PropertyDef {
                    key: "n".into(),
                    label: "n".into(),
                    kind: PropKind::Number { min: 0.0, max: 10.0, step: 1.0, default: 5.0 },
                },
                PropertyDef {
                    key: "b".into(),
                    label: "b".into(),
                    kind: PropKind::Bool { default: true },
                },
                PropertyDef {
                    key: "ch".into(),
                    label: "ch".into(),
                    kind: PropKind::Choice {
                        options: vec!["x".into(), "y".into()],
                        default: "x".into(),
                    },
                },
                PropertyDef {
                    key: "h".into(),
                    label: "h".into(),
                    kind: PropKind::Hue { default: 30.0 },
                },
                PropertyDef {
                    key: "a".into(),
                    label: "a".into(),
                    kind: PropKind::Angle { default: 90.0 },
                },
            ],
            presets: vec![],
            thumbnail: None,
        }
    }

    #[test]
    fn missing_keys_take_defaults() {
        let s = schema();
        let p = Props::new();
        assert_eq!(color(&s, &p, "c"), canvas_proto::Rgb8::new(1, 2, 3));
        assert_eq!(number(&s, &p, "n"), 5.0);
        assert!(boolean(&s, &p, "b"));
        assert_eq!(choice(&s, &p, "ch"), "x");
        assert_eq!(hue(&s, &p, "h"), 30.0);
        assert_eq!(angle(&s, &p, "a"), 90.0);
    }

    #[test]
    fn mistyped_values_take_defaults() {
        let s = schema();
        let mut p = Props::new();
        p.insert("c".into(), PropValue::Number(9.0));
        p.insert("n".into(), PropValue::Bool(true));
        p.insert("b".into(), PropValue::Number(1.0));
        p.insert("ch".into(), PropValue::Number(1.0));
        p.insert("h".into(), PropValue::Number(1.0));
        p.insert("a".into(), PropValue::Number(1.0));
        assert_eq!(color(&s, &p, "c"), canvas_proto::Rgb8::new(1, 2, 3));
        assert_eq!(number(&s, &p, "n"), 5.0);
        assert!(boolean(&s, &p, "b"));
        assert_eq!(choice(&s, &p, "ch"), "x");
        assert_eq!(hue(&s, &p, "h"), 30.0);
        assert_eq!(angle(&s, &p, "a"), 90.0);
    }

    #[test]
    fn out_of_range_numbers_are_clamped() {
        let s = schema();
        let mut p = Props::new();
        p.insert("n".into(), PropValue::Number(999.0));
        assert_eq!(number(&s, &p, "n"), 10.0);
        p.insert("n".into(), PropValue::Number(-999.0));
        assert_eq!(number(&s, &p, "n"), 0.0);
    }

    #[test]
    fn non_finite_numbers_fall_back() {
        let s = schema();
        let mut p = Props::new();
        p.insert("n".into(), PropValue::Number(f64::NAN));
        assert_eq!(number(&s, &p, "n"), 5.0);
        // An infinite value is not a usable number, so it falls back too.
        p.insert("n".into(), PropValue::Number(f64::INFINITY));
        assert_eq!(number(&s, &p, "n"), 5.0);
        p.insert("n".into(), PropValue::Number(f64::NEG_INFINITY));
        assert_eq!(number(&s, &p, "n"), 5.0);
    }

    #[test]
    fn choice_outside_the_options_takes_the_default() {
        let s = schema();
        let mut p = Props::new();
        p.insert("ch".into(), PropValue::Choice("zzz".into()));
        assert_eq!(choice(&s, &p, "ch"), "x");
        p.insert("ch".into(), PropValue::Choice("y".into()));
        assert_eq!(choice(&s, &p, "ch"), "y");
    }

    #[test]
    fn hue_wraps() {
        let s = schema();
        let mut p = Props::new();
        p.insert("h".into(), PropValue::Hue(-30.0));
        assert_eq!(hue(&s, &p, "h"), 330.0);
        p.insert("h".into(), PropValue::Hue(400.0));
        assert_eq!(hue(&s, &p, "h"), 40.0);
    }

    /// A gradient saved before the angle kind existed stored its angle as a hue: the same
    /// number of degrees reads on, not the default.
    #[test]
    fn a_saved_hue_reads_as_the_same_angle() {
        let s = schema();
        let mut p = Props::new();
        p.insert("a".into(), PropValue::Hue(120.0));
        assert_eq!(angle(&s, &p, "a"), 120.0);
        p.insert("a".into(), PropValue::Angle(120.0));
        assert_eq!(angle(&s, &p, "a"), 120.0);
    }

    /// Rainbow Wave's direction was a plain number before it became an angle.
    #[test]
    fn a_saved_number_reads_as_the_same_angle() {
        let s = schema();
        let mut p = Props::new();
        p.insert("a".into(), PropValue::Number(45.0));
        assert_eq!(angle_or_saved_number(&s, &p, "a"), 45.0);
        // The old number reader clamped to 0..360: 405 read as 360 (0 degrees), -90 as 0.
        p.insert("a".into(), PropValue::Number(405.0));
        assert_eq!(angle_or_saved_number(&s, &p, "a"), 0.0);
        p.insert("a".into(), PropValue::Number(-90.0));
        assert_eq!(angle_or_saved_number(&s, &p, "a"), 0.0);
        p.insert("a".into(), PropValue::Number(f64::NAN));
        assert_eq!(angle_or_saved_number(&s, &p, "a"), 90.0);
        p.insert("a".into(), PropValue::Angle(10.0));
        assert_eq!(angle_or_saved_number(&s, &p, "a"), 10.0);
    }

    #[test]
    fn a_non_finite_saved_hue_takes_the_angle_default() {
        let s = schema();
        let mut p = Props::new();
        p.insert("a".into(), PropValue::Hue(f64::NAN));
        assert_eq!(angle(&s, &p, "a"), 90.0);
    }

    #[test]
    fn angle_wraps() {
        let s = schema();
        let mut p = Props::new();
        p.insert("a".into(), PropValue::Hue(-30.0));
        assert_eq!(angle(&s, &p, "a"), 330.0);
        p.insert("a".into(), PropValue::Angle(400.0));
        assert_eq!(angle(&s, &p, "a"), 40.0);
    }

}
