//! `Segmented`, after `app/web/src/components/Segmented.tsx` and the
//! `.seg` / `.seg-item` / `.seg-thumb` rules in `app.css` with the
//! hardware layer's dot (`hardware.css`: a 5 px LED before each label,
//! lit in `--live` on the checked option).
//!
//! A compact pill group that takes the width it needs, not the width of
//! its column: the well is sunk with a line border, the checked option is
//! a raised thumb, and every option carries its dot.

use std::rc::Rc;

use gpui::{FocusHandle, IntoElement, Window, div, prelude::*, px};

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
    segmented_inner(t, id, options, selected, pad_x, None, on_change)
}

/// The radio group with the keyboard (`Segmented.tsx:84-94`, `:119-121`): one
/// `focus` handle per option, in option order. Only the selected option is in
/// the tab order (`tabIndex={checked ? 0 : -1}`), ArrowLeft and ArrowRight move
/// the selection one step and wrap at the ends, and the focus follows the
/// selection to the option it moved to.
pub fn segmented_focus(
    t: &Theme,
    id: &str,
    options: &[(&str, &str)],
    selected: &str,
    focus: &[FocusHandle],
    on_change: impl Fn(String, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    segmented_inner(t, id, options, selected, 10.0, Some(focus), on_change)
}

/// The option an arrow key moves to (`Segmented.tsx:84-94`): the selection plus
/// or minus one, wrapping. A selection that matches no option counts as -1,
/// as `findIndex` does. Any other key moves nothing.
pub fn key_target(len: usize, at: Option<usize>, key: &str) -> Option<usize> {
    let dir: isize = match key {
        "right" => 1,
        "left" => -1,
        _ => return None,
    };
    if len == 0 {
        return None;
    }
    let at = at.map_or(-1, |i| i as isize);
    Some((at + dir).rem_euclid(len as isize) as usize)
}

/// `tabIndex={checked ? 0 : -1}` (`Segmented.tsx:121`): is option `i` in the tab order?
pub fn is_tab_stop(selected: Option<usize>, i: usize) -> bool {
    selected == Some(i)
}

fn segmented_inner(
    t: &Theme,
    id: &str,
    options: &[(&str, &str)],
    selected: &str,
    pad_x: f32,
    focus: Option<&[FocusHandle]>,
    on_change: impl Fn(String, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let values: Rc<Vec<String>> = Rc::new(options.iter().map(|(v, _)| v.to_string()).collect());
    let at = options.iter().position(|(v, _)| *v == selected);
    let focus: Option<Rc<Vec<FocusHandle>>> = focus.map(|f| Rc::new(f.to_vec()));
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
    for (i, (value, label)) in options.iter().enumerate() {
        let is_selected = *value == selected;
        let value = value.to_string();
        let change = on_change.clone();
        let key_change = on_change.clone();
        let key_values = values.clone();
        let key_focus = focus.clone();
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
        let cell = cell
            .cursor_pointer()
            .on_click(move |_, window, cx| change(value.clone(), window, cx));
        let cell = match focus.as_ref().and_then(|f| f.get(i)) {
            Some(handle) => {
                let handle = handle.clone().tab_stop(is_tab_stop(at, i));
                let ring = crate::components::focus::ring(t);
                cell.track_focus(&handle).focus(ring).on_key_down(move |ev, window, cx| {
                    if let Some(next) = key_target(key_values.len(), at, ev.keystroke.key.as_str()) {
                        cx.stop_propagation();
                        key_change(key_values[next].clone(), window, cx);
                        if let Some(h) = key_focus.as_ref().and_then(|f| f.get(next)) {
                            window.focus(h);
                        }
                    }
                })
            }
            None => cell,
        };
        row = row.child(cell);
    }
    row
}

/// A segmented control that cannot be pressed (`disabled`): the well and
/// the checked thumb draw at half strength, as React's disabled `.seg`.
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
        .opacity(0.5)
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

    /// `Segmented.tsx:84-94`: ArrowRight/ArrowLeft move the selection by one and wrap.
    #[test]
    fn segmented_arrows_step_and_wrap() {
        assert_eq!(key_target(3, Some(0), "right"), Some(1));
        assert_eq!(key_target(3, Some(1), "right"), Some(2));
        assert_eq!(key_target(3, Some(2), "right"), Some(0), "right wraps to the first");
        assert_eq!(key_target(3, Some(0), "left"), Some(2), "left wraps to the last");
        assert_eq!(key_target(3, Some(2), "left"), Some(1));
        assert_eq!(key_target(2, Some(1), "right"), Some(0));
        assert_eq!(key_target(1, Some(0), "right"), Some(0), "one option stays put");
    }

    /// Up and Down, Home and End, Enter and Space do nothing in React's group
    /// (the handler returns for anything but the two horizontal arrows).
    #[test]
    fn segmented_other_keys_move_nothing() {
        for key in ["up", "down", "home", "end", "enter", "space", "tab", "escape", "a"] {
            assert_eq!(key_target(3, Some(1), key), None, "{key}");
        }
        assert_eq!(key_target(0, None, "right"), None, "an empty group has nowhere to go");
    }

    /// `findIndex` returns -1 for a value that matches no option, so Right goes
    /// to the first option and Left to the second to last.
    #[test]
    fn segmented_with_no_selection_follows_find_index() {
        assert_eq!(key_target(3, None, "right"), Some(0));
        assert_eq!(key_target(3, None, "left"), Some(1));
    }

    /// `tabIndex={checked ? 0 : -1}`: one stop in the group, the selected option.
    #[test]
    fn segmented_only_the_selected_option_is_a_tab_stop() {
        let stops: Vec<bool> = (0..3).map(|i| is_tab_stop(Some(1), i)).collect();
        assert_eq!(stops, vec![false, true, false]);
        assert!((0..3).all(|i| !is_tab_stop(None, i)), "no selection, no stop");
    }

    #[test]
    fn index_of_finds_the_selected_option() {
        assert_eq!(index_of(&["celsius", "fahrenheit"], "fahrenheit"), Some(1));
        assert_eq!(index_of(&["0", "1"], "0"), Some(0));
        assert_eq!(index_of(&["a"], "z"), None);
    }
}
