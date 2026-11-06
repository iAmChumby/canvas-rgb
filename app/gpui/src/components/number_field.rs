//! `NumberField`, after `app/web/src/components/NumberField.tsx`.
//!
//! The React field is a text input that commits on Enter, blur or an arrow
//! key, clamped to `min..=max`. GPUI 0.2.2's text input is a custom element;
//! for this slice the field is a value display with −/+ steppers that commit
//! the stepped value at once (the same `min..=max` clamp). The keyboard and
//! type-ahead items in the Displays checklist are therefore NOT ported yet
//! (see `docs/gpui-parity/displays.md`, Keyboard) — they stay unticked.

use gpui::{IntoElement, div, prelude::*, px};

use crate::theme::Theme;

/// Clamp a draft value to the field's range, matching `NumberField`'s commit.
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() {
        return min;
    }
    value.clamp(min, max)
}

/// The field: current value, optional unit suffix, and steppers.
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
    let commit = on_commit.clone();
    let down = commit.clone();
    let up = commit;
    let row = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .gap_2()
        .h(px(32.0))
        .px_2()
        .rounded_md()
        .border_1()
        .border_color(Theme::color(t.line_2))
        // The sunk well the preview frame and input tracks use.
        .bg(Theme::color(t.sunk))
        .text_color(Theme::color(t.ink))
        .text_sm()
        .font(crate::fonts::font_mono());
    let text = match unit {
        Some(u) => format!("{value} {u}"),
        None => format!("{value}"),
    };
    let minus = div()
        .id(gpui::SharedString::from(format!("{id}-minus")))
        .flex()
        .items_center()
        .justify_center()
        .w(px(22.0))
        .h(px(22.0))
        .rounded_sm()
        .bg(Theme::color(t.raised))
        .text_color(Theme::color(t.ink))
        .cursor_pointer()
        .child("−")
        .on_click(move |_, window, cx| down(clamp(value - step, min, max), window, cx));
    let plus = div()
        .id(gpui::SharedString::from(format!("{id}-plus")))
        .flex()
        .items_center()
        .justify_center()
        .w(px(22.0))
        .h(px(22.0))
        .rounded_sm()
        .bg(Theme::color(t.raised))
        .text_color(Theme::color(t.ink))
        .cursor_pointer()
        .child("+")
        .on_click(move |_, window, cx| up(clamp(value + step, min, max), window, cx));
    row.child(minus).child(text).child(plus)
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
