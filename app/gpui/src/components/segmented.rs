//! `Segmented`, after `app/web/src/components/Segmented.tsx`: a radio group
//! where the chosen option is filled and the rest are quiet. Every option is
//! a tab stop target in React; GPUI gives each option its own click.

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
        .gap_1()
        .p_1()
        .rounded_lg()
        .border_1()
        .border_color(Theme::color(t.line))
        .bg(Theme::color(t.sunk));
    for (value, label) in options {
        let is_selected = *value == selected;
        let value = value.to_string();
        let change = on_change.clone();
        let cell = div()
            .id(gpui::SharedString::from(format!("{id}-{value}")))
            .flex()
            .items_center()
            .justify_center()
            .px_3()
            .py_1()
            .rounded_md()
            .flex_1()
            .text_sm()
            .font(crate::fonts::font_ui())
            .child(label.to_string());
        let cell = if is_selected {
            cell.bg(Theme::color(t.raised))
                .text_color(Theme::color(t.ink))
                .font_weight(gpui::FontWeight::SEMIBOLD)
        } else {
            cell.text_color(Theme::color(t.mute))
        };
        row = row.child(
            cell
                .cursor_pointer()
                .on_click(move |_, window, cx| change(value.clone(), window, cx)),
        );
    }
    let _ = px(0.0);
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
