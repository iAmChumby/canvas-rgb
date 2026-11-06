//! `Segmented`, after `app/web/src/components/Segmented.tsx` and the
//! `.seg` / `.seg-item` / `.seg-thumb` rules in `app.css` with the
//! hardware layer's dot (`hardware.css`: a 5 px LED before each label,
//! lit in `--live` on the checked option).
//!
//! A compact pill group that takes the width it needs, not the width of
//! its column: the well is sunk with a line border, the checked option is
//! a raised thumb, and every option carries its dot.

use gpui::{IntoElement, Window, div, prelude::*, px};

use crate::theme::{Theme, TokenRadius};

/// One segmented control: `options` are (value, label) pairs, `selected` the
/// current value. Clicking an option calls `on_change` with its value.
pub fn segmented(
    t: &Theme,
    id: &str,
    options: &[(&str, &str)],
    selected: &str,
    on_change: impl Fn(String, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let mut row = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .gap(px(2.0))
        .p(px(2.0))
        .r_md()
        .border_1()
        .border_color(Theme::color(t.line))
        .bg(Theme::color(t.sunk))
        // hardware.css `--well`: `inset 0 1px 0 …`, the recess's top line.
        // GPUI has no inset shadow, so the line is drawn inside the top edge.
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(1.0))
                .bg(t.well()),
        );
    for (value, label) in options {
        let is_selected = *value == selected;
        let value = value.to_string();
        let change = on_change.clone();
        // The LED dot: unlit (`--led-off`) on the quiet options, live on the
        // checked one (hardware.css `seg-item::before`).
        let dot_color = if is_selected {
            t.live()
        } else {
            Theme::color(t.dot)
        };
        let dot = div().size(px(5.0)).rounded_full().bg(dot_color);
        // React Segmented.tsx:129 wraps a label holding a digit in `.fig`
        // (the mono figures face, tabular-nums): the Decimals "0"/"1".
        let family = if label.chars().any(|c| c.is_ascii_digit()) {
            crate::fonts::MONO_FAMILY
        } else {
            crate::fonts::UI_FAMILY
        };
        let cell = div()
            .id(gpui::SharedString::from(format!("{id}-{value}")))
            .relative()
            .flex()
            .items_center()
            .gap(px(4.0))
            .h(px(26.0))
            .px(px(10.0))
            .r_sm()
            .text_sm()
            .font_family(family)
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(dot)
            .child(label.to_string());
        let cell = if is_selected {
            // hardware.css `.seg-thumb`: a raised key in the well, inset
            // `line-2` edge plus the 2 px skirt under it.
            cell.bg(Theme::color(t.raised))
                .text_color(Theme::color(t.ink))
                .border_1()
                .border_color(Theme::color(t.line_2))
                // The skirt follows the thumb's radius: hardware.css
                // `.seg-thumb{border-radius:calc(var(--r-md) - 3px)}` = 7 px.
                .child(crate::components::button::skirt_strip_r(t.skirt(), 7.0))
        } else {
            cell.text_color(Theme::color(t.dim))
        };
        row = row.child(
            cell
                .cursor_pointer()
                .on_click(move |_, window, cx| change(value.clone(), window, cx)),
        );
    }
    row
}

/// One option's index, for the roving-tab discussion in the parity checklist.
#[allow(dead_code)]
pub fn index_of(options: &[&str], value: &str) -> Option<usize> {
    options.iter().position(|v| *v == value)
}

#[cfg(test)]
#[allow(clippy::too_many_lines)]
mod tests {
    use super::*;

    #[test]
    fn index_of_finds_the_selected_option() {
        assert_eq!(index_of(&["celsius", "fahrenheit"], "fahrenheit"), Some(1));
        assert_eq!(index_of(&["0", "1"], "0"), Some(0));
        assert_eq!(index_of(&["a"], "z"), None);
    }
}
