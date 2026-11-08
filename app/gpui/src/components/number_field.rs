//! `NumberField`, after `app/web/src/components/NumberField.tsx`.
//!
//! React's field is a plain mono text input (`.num`): 92 px wide, one
//! control height tall, 8 px side padding, a 1 px `line-2` border, the sunk
//! well behind it, right-aligned 15 px mono text with tabular numbers.
//! GPUI 0.2.2 draws the input's box here; keyboard entry and arrow-key
//! stepping are not ported yet (the same gap the Displays checklist notes),
//! and there are no −/+ steppers: React draws none in this field.
//!
//! (The opacity row's 64 px override, `.layer-field .num`, has its own
//! entry point because the width is part of the pixel contract.)

use gpui::{IntoElement, div, prelude::*, px};

use crate::theme::{Theme, TokenRadius};

/// Clamp a draft value to the field's range, matching `NumberField`'s commit.
#[allow(dead_code)]
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() {
        return min;
    }
    value.clamp(min, max)
}

/// The field: the value in a plain mono input box (React `.num`).
#[allow(clippy::too_many_arguments)]
pub fn number_field(
    t: &Theme,
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    unit: Option<&str>,
    on_commit: impl Fn(f32, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    number_field_width(t, id, value, min, max, step, unit, 92.0, on_commit)
}

/// The field at a given width (the opacity row draws 64 px: `.layer-field .num`).
#[allow(clippy::too_many_arguments)]
pub fn number_field_width(
    t: &Theme,
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    unit: Option<&str>,
    width: f32,
    on_commit: impl Fn(f32, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let _ = (min, max, step, on_commit);
    let text = match unit {
        Some(u) => format!("{value} {u}"),
        None => format!("{value}"),
    };
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_end()
        .w(px(width))
        .h(px(32.0))
        .px(px(8.0))
        .r_md()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .bg(Theme::color(t.sunk))
        .text_color(Theme::color(t.ink))
        .text_size(px(15.0))
        .line_height(gpui::relative(1.45))
        .font_family(crate::fonts::MONO_FAMILY)
        .whitespace_nowrap()
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_matches_the_number_field_range() {
        assert_eq!(clamp(50.0, 0.0, 9999.0), 50.0);
        assert_eq!(clamp(-5.0, 0.0, 9999.0), 0.0);
        assert_eq!(clamp(20000.0, 0.0, 9999.0), 9999.0);
        assert_eq!(clamp(f32::NAN, 0.0, 9999.0), 0.0);
    }

    #[test]
    fn a_step_commits_the_clamped_sum() {
        let v = clamp(9999.0 + 1.0, 0.0, 9999.0);
        assert_eq!(v, 9999.0);
        let v = clamp(0.0 - 1.0, 0.0, 9999.0);
        assert_eq!(v, 0.0);
    }
}
