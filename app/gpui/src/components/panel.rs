//! `Panel` and `Field`, after `app/web/src/components/Panel.tsx`: a raised
//! block with a title row (and its actions) over its body, and a labelled
//! field (label over control).

use gpui::{IntoElement, div, prelude::*, px};

use crate::theme::Theme;

/// A panel with a title. `actions` render beside the title, on the right.
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
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(Theme::color(t.line))
        .bg(Theme::color(t.panel))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_base()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font(crate::fonts::font_ui())
                        .text_color(Theme::color(t.ink))
                        .child(title.to_string()),
                )
                .child(actions),
        )
        .child(body)
}

/// A labelled field: muted label over the control.
pub fn field(t: &Theme, id: &str, label: &str, control: impl IntoElement) -> impl IntoElement {
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font(crate::fonts::font_ui())
                .text_color(Theme::color(t.mute))
                .child(label.to_string()),
        )
        .child(control)
}

/// The state words beside a panel's title: a dot in the tone's colour and the
/// words. Only the words carry meaning (the dot is `aria-hidden` in React).
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
        .child(
            div()
                .size(px(8.0))
                .rounded_full()
                .bg(color),
        )
        .child(
            div()
                .text_sm()
                .font(crate::fonts::font_ui())
                .text_color(Theme::color(t.dim))
                .child(words.to_string()),
        )
}
