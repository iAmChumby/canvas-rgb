//! `Select`, after `app/web/src/components/Select.tsx` and the
//! `.select-trigger` / `.listbox` rules in `app.css` (trigger: 32 px,
//! min-width 160, raised face, bold caret; list: panel, line-2 border,
//! `--pop` shadow, 32 px rows).
//!
//! The React control is a combobox with a listbox, arrow keys, type-ahead and
//! an exit animation. GPUI 0.2.2 has no portalled list; this slice draws the
//! trigger and, when open, the option rows under it (click an option to
//! pick). Keyboard, type-ahead and ARIA items in the Displays checklist stay
//! unticked for that reason.

use gpui::{BoxShadow, IntoElement, div, point, prelude::*, px};

use crate::theme::Theme;

/// The source trigger and (when `open`) its option list.
#[allow(clippy::too_many_arguments)]
pub fn select(
    t: &Theme,
    id: &str,
    label: &str,
    options: &[(&str, &str)],
    value: &str,
    open: bool,
    full_width: bool,
    on_toggle: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_pick: impl Fn(String, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let _ = label;
    let chosen = options
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(_, l)| *l)
        .unwrap_or("CPU temperature");
    let toggle = on_toggle.clone();
    // React `.select-value`: every option is laid out in the trigger and
    // only the chosen one is visible, so the trigger is as wide as its
    // widest option and picking one never resizes it. GPUI has no
    // `visibility: hidden` that keeps layout, so the labels are stacked in
    // one grid cell (all at row/column 1): the cell takes the widest label,
    // and the non-chosen labels are `invisible` (visibility hidden), exactly
    // as in React. The chosen label is drawn last, on top.
    let mut value_grid = div().grid().flex_1().min_w(px(0.0)).text_left();
    for (v, l) in options {
        let label = div()
            .col_start(1)
            .row_start(1)
            .whitespace_nowrap()
            .child(l.to_string());
        let label = if *v == value {
            label
        } else {
            label.invisible()
        };
        value_grid = value_grid.child(label);
    }
    let _ = chosen;
    let trigger = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .min_w(px(160.0))
        .h(px(32.0))
        .pl_3()
        .pr(px(10.0))
        .rounded_md()
        .border_1()
        .border_color(if open {
            t.live()
        } else {
            Theme::color(t.line_2)
        })
        .bg(Theme::color(t.raised))
        .text_color(Theme::color(t.ink))
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY)
        .cursor_pointer()
        // hardware.css `.select-trigger`: a key on its 2 px skirt.
        .shadow(vec![BoxShadow {
            color: gpui::Hsla::from(t.skirt()),
            offset: point(px(0.0), px(2.0)),
            blur_radius: px(0.0),
            spread_radius: px(0.0),
        }])
        .child(value_grid)
        .child(
            gpui::svg()
                .path("icons/CaretDown-bold.svg")
                .size_4()
                .text_color(Theme::color(t.mute)),
        )
        .on_click(move |_, window, cx| toggle(window, cx));
    let trigger = if full_width { trigger.w_full() } else { trigger };
    let mut col = div()
        .id(gpui::SharedString::from(format!("{id}-col")))
        .flex()
        .flex_col()
        .child(trigger);
    if open {
        let mut list = div()
            .id(gpui::SharedString::from(format!("{id}-list")))
            .flex()
            .flex_col()
            .absolute()
            .mt(px(38.0))
            .min_w(px(160.0))
            .p(px(4.0))
            .rounded_lg()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.panel));
        for (v, l) in options {
            let pick = on_pick.clone();
            let v_owned = v.to_string();
            let is_chosen = *v == value;
            let row = div()
                .id(gpui::SharedString::from(format!("{id}-option-{v}")))
                .flex()
                .items_center()
                .justify_between()
                .h(px(32.0))
                .px(px(10.0))
                .rounded_md()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .cursor_pointer()
                .child(l.to_string())
                .child(if is_chosen { "✓" } else { "" });
            let row = if is_chosen {
                row.bg(Theme::color(t.raised))
            } else {
                row
            };
            list = list.child(row.on_click(move |_, window, cx| {
                pick(v_owned.clone(), window, cx)
            }));
        }
        col = col.child(list);
    }
    col
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_value_outside_the_options_reads_as_the_first_option() {
        let options = [("cpu_temp", "CPU temperature"), ("fixed", "A fixed number")];
        let chosen = options
            .iter()
            .find(|(v, _)| *v == "board:temp0")
            .map(|(_, l)| *l)
            .unwrap_or("CPU temperature");
        assert_eq!(chosen, "CPU temperature");
    }
}
