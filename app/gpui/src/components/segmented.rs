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
    segmented_padded(t, id, options, selected, 10.0, on_change)
}

/// The same control with the option's own horizontal padding: React's
/// `.rd-actions .seg-item` is `--s2` (8 px) on a phone and 6 px below 431 px,
/// where the strip's profile picker has to fit beside Pause and Search
/// (`app.css:5174,6971,6986`). The well, the dot, the gap and the height are
/// unchanged.
pub fn segmented_padded(
    t: &Theme,
    id: &str,
    options: &[(&str, &str)],
    selected: &str,
    pad_x: f32,
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
        // checked one (hardware.css `seg-item::before`). The lit dot carries
        // a 2 px halo of live at 22% (`box-shadow: 0 0 0 2px`): box-shadow
        // is paint-only, so the halo draws as a 9 px disc centred on the
        // 5 px core, absolutely placed, and the cell keeps React's width
        // (#240).
        let live = t.live();
        let dot = if is_selected {
            let halo = gpui::Rgba { a: 0.22, ..live };
            div()
                .relative()
                .flex_none()
                .size(px(5.0))
                .child(
                    div()
                        .absolute()
                        .left(px(-2.0))
                        .top(px(-2.0))
                        .size(px(9.0))
                        .rounded_full()
                        .bg(halo),
                )
                .child(div().size(px(5.0)).rounded_full().bg(live))
        } else {
            div()
                .flex_none()
                .size(px(5.0))
                .rounded_full()
                .bg(Theme::color(t.dot))
        };
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
            .gap(px(6.0))
            .h(px(26.0))
            .px(px(pad_x))
            .text_sm()
            .font_family(family)
            .font_weight(gpui::FontWeight::MEDIUM);
        let cell = if is_selected {
            // hardware.css `.seg-thumb`: a raised key in the well, inset
            // `line-2` edge plus the 2 px skirt under it. The skirt follows
            // the thumb's radius: `.seg-thumb{border-radius:calc(var(--r-md)
            // - 3px)}` = 7 px.
            cell.text_color(Theme::color(t.ink)).child(
                crate::components::button::skirt_layers(
                    t.skirt(),
                    7.0,
                    26.0,
                    Theme::color(t.raised),
                    Theme::color(t.line_2),
                ),
            )
        } else {
            cell.r_sm().text_color(Theme::color(t.dim))
        };
        let cell = cell.child(dot).child(label.to_string());
        row = row.child(
            cell
                .cursor_pointer()
                .on_click(move |_, window, cx| change(value.clone(), window, cx)),
        );
    }
    row
}

/// A segmented control that cannot be pressed (`disabled`): React's
/// `.seg-item` buttons carry the native `disabled` attribute and no stylesheet
/// dims them (no `.seg-item:disabled` rule anywhere in `app.css` or
/// `hardware.css`), so the control draws at full brightness — only the click
/// is inert. The `on_change` is deliberately unused.
#[allow(dead_code)]
pub fn segmented_disabled(
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
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(1.0))
                .bg(t.well()),
        );
    let pad_x = 10.0;
    for (value, label) in options {
        let is_selected = *value == selected;
        let family = if label.chars().any(|c| c.is_ascii_digit()) {
            crate::fonts::MONO_FAMILY
        } else {
            crate::fonts::UI_FAMILY
        };
        let live = t.live();
        let dot = if is_selected {
            let halo = gpui::Rgba { a: 0.22, ..live };
            div()
                .relative()
                .flex_none()
                .size(px(5.0))
                .child(
                    div()
                        .absolute()
                        .left(px(-2.0))
                        .top(px(-2.0))
                        .size(px(9.0))
                        .rounded_full()
                        .bg(halo),
                )
                .child(div().size(px(5.0)).rounded_full().bg(live))
        } else {
            div()
                .flex_none()
                .size(px(5.0))
                .rounded_full()
                .bg(Theme::color(t.dot))
        };
        let cell = div()
            .id(gpui::SharedString::from(format!("{id}-{value}")))
            .relative()
            .flex()
            .items_center()
            .gap(px(6.0))
            .h(px(26.0))
            .px(px(pad_x))
            .text_sm()
            .font_family(family)
            .font_weight(gpui::FontWeight::MEDIUM);
        let cell = if is_selected {
            cell.text_color(Theme::color(t.ink)).child(
                crate::components::button::skirt_layers(
                    t.skirt(),
                    7.0,
                    26.0,
                    Theme::color(t.raised),
                    Theme::color(t.line_2),
                ),
            )
        } else {
            cell.r_sm().text_color(Theme::color(t.dim))
        };
        row = row.child(cell.child(dot).child(label.to_string()));
    }
    let _ = on_change;
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
