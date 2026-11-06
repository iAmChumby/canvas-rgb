//! `EmptyState`, after `app/web/src/components/EmptyState.tsx`: a centred
//! icon, title and body, with an optional action.

use gpui::{IntoElement, div, prelude::*};

use crate::theme::Theme;

pub fn empty_state(
    t: &Theme,
    id: &str,
    icon_name: &str,
    title: &str,
    body: &str,
    action: Option<impl IntoElement>,
) -> impl IntoElement {
    let mut col = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .items_center()
        .gap_3()
        .p_8()
        .child(
            gpui::svg()
                .path(format!("icons/{icon_name}.svg"))
                .size_6()
                .text_color(Theme::color(t.mute)),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child(body.to_string()),
        );
    if let Some(action) = action {
        col = col.child(action);
    }
    col
}
