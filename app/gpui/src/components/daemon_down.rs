//! `DaemonDown`, after `app/web/src/components/DaemonDown.tsx`: the page the
//! shell shows when the daemon has not answered twice in a row.

use gpui::{ClickEvent, IntoElement, Window, div, prelude::*};

use crate::theme::Theme;
use crate::components::button::{Variant, button};

pub fn daemon_down(
    t: &Theme,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    detail: &str,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap_3()
        .p_8()
        .child(
            div()
                .text_xl()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child("canvas-rgb is not answering"),
        )
        .child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child("canvas-rgb draws the lights on 127.0.0.1:9430. It may still be starting, or it may have stopped."),
        )
        .child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child("The lights keep the frame they had."),
        )
        .child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child("The fans stay under BIOS control."),
        )
        .child(button(t, "daemon-down-retry", "Try again", Variant::Primary, false, on_retry))
        .child(
            div()
                .text_sm()
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.mute))
                .child(detail.to_string()),
        )
}
