//! `Panel` and `Field`, after `app/web/src/components/Panel.tsx` and the
//! `.panel` / `.panel-head` / `.panel-title` / `.field` rules in `app.css`
//! re-skinned by `hardware.css` (titles are printed on the case: mono,
//! small capitals, spaced).
//!
//! The panel is a 16 px padding, line-bordered, 16 px radius block; its
//! head is a wrapping row with the title left and the actions right. A
//! field is label-left / control-right with a divider, or (in the wide and
//! phone layouts, via [`field_stack`]) label over control.

use gpui::{IntoElement, div, prelude::*, px};

use crate::theme::Theme;

/// A panel with a title. `actions` render beside the title, on the right.
#[allow(dead_code)]
pub fn panel(
    t: &Theme,
    id: &str,
    title: &str,
    actions: impl IntoElement,
    body: impl IntoElement,
) -> impl IntoElement {
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(Theme::color(t.line))
        .bg(Theme::color(t.panel))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .mb_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .font_family(crate::fonts::MONO_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(title.to_uppercase()),
                )
                .child(actions),
        )
        .child(body)
}

/// A labelled field in row form (the intermediate width): label left,
/// control right, divider under the row.
pub fn field(t: &Theme, id: &str, label: &str, control: impl IntoElement) -> gpui::Stateful<gpui::Div> {
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_2()
        .min_h(px(48.0))
        .py_2()
        .border_b_1()
        .border_color(Theme::color(t.line))
        .child(
            div()
                .flex_1()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child(label.to_string()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .justify_end()
                .child(control),
        )
}

/// A labelled field in stacked form (wide two-column controls and the
/// phone): label over control, control justified to the start, no divider.
pub fn field_stack(t: &Theme, id: &str, label: &str, control: impl IntoElement) -> gpui::Stateful<gpui::Div> {
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child(label.to_string()),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(control),
        )
}

/// The state words beside a panel's title: an 8 px dot in the tone's colour
/// and the words. Only the words carry meaning (the dot is `aria-hidden` in React).
pub fn state_dot(t: &Theme, tone: &str, words: &str) -> impl IntoElement {
    let color = match tone {
        "ok" => Theme::color(t.ok),
        "warn" => Theme::color(t.warn),
        "work" => Theme::color(t.work),
        _ => Theme::color(t.down),
    };
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().size(px(8.0)).rounded_full().bg(color))
        .child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child(words.to_string()),
        )
}
