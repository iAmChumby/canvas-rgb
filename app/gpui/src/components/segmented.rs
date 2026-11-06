//! `Segmented`, after `app/web/src/components/Segmented.tsx` and the
//! `.seg` / `.seg-item` / `.seg-thumb` rules in `app.css` with the
//! hardware layer's dot (`hardware.css`: a 5 px LED before each label,
//! lit in `--live` on the checked option).
//!
//! A compact pill group that takes the width it needs, not the width of
//! its column: the well is sunk with a line border, the checked option is
//! a raised thumb, and every option carries its dot.

use gpui::{IntoElement, Window, div, prelude::*, px};

use crate::theme::Theme;

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
        .flex()
        .gap(px(2.0))
        .p(px(2.0))
        .rounded_md()
        .border_1()
        .border_color(Theme::color(t.line))
        .bg(Theme::color(t.sunk));
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
        let cell = div()
            .id(gpui::SharedString::from(format!("{id}-{value}")))
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(26.0))
            .px(px(10.0))
            .rounded_md()
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(dot)
            .child(label.to_string());
        let cell = if is_selected {
            cell.bg(Theme::color(t.raised))
                .text_color(Theme::color(t.ink))
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
