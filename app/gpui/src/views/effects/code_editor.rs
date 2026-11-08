//! The code pane, after `app/web/src/views/effects/EffectCode.tsx` and the
//! `.eff-code` rules in `app.css`.
//!
//! React's pane is a CodeMirror editor (`app/web/src/components/CodeEditor.tsx`).
//! That is round 2 of #214: this round draws the pane as a plain read-only
//! monospace view of the source, so the frame around it (the head, the count,
//! the Revert key and the box) is at parity while the editor inside it is
//! still to come. The report says so.

#![allow(dead_code)]

use gpui::{FontWeight, IntoElement, Window, div, prelude::*, px};

use crate::components::button::{Variant, button_icon};
use crate::theme::{Theme, TokenRadius};

/// The code line's height (`--code-line` in `app.css`).
pub const CODE_LINE: f32 = 20.8;
/// The code's own face size (CodeMirror's theme in `CodeEditor.tsx`).
pub const CODE_FONT: f32 = 13.0;

/// The code pane: its head (the icon, the title, the line count and the
/// Revert key) over the source.
///
/// `narrow` is the phone layout, where the head carries a Back key and the
/// Save key instead of the icon.
#[allow(clippy::too_many_arguments)]
pub fn code_pane(
    t: &Theme,
    id: &str,
    source: &str,
    narrow: bool,
    can_revert: bool,
    on_back: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    on_revert: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let lines = source.split('\n').count();
    let mut head = div()
        .flex()
        .flex_nowrap()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(Theme::color(t.line))
        .min_w(px(0.0));
    if narrow {
        head = head.child(crate::components::button::icon_button(
            t,
            &format!("{id}-back"),
            "CaretLeft",
            "",
            false,
            false,
            false,
            on_back,
        ));
    } else {
        head = head.child(
            gpui::svg()
                .path("icons/Code.svg")
                .size(px(15.0))
                .flex_none()
                .text_color(Theme::color(t.dim)),
        );
    }
    head = head
        .child(
            div()
                .text_size(px(15.0))
                .line_height(gpui::relative(1.45))
                .font_weight(FontWeight::SEMIBOLD)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child("Code"),
        )
        // React `.eff-code-count { margin-left: auto }`: the count sits at
        // the head's right, before the actions.
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .justify_end()
                .child(
                    div()
                        .text_size(px(15.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::MONO_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(format!("{lines} lines")),
                ),
        )
        .child(
            div()
                .flex()
                .gap_1()
                .child(button_icon(
                    t,
                    &format!("{id}-revert"),
                    "Revert",
                    Variant::Ghost,
                    true,
                    !can_revert,
                    "ArrowCounterClockwise",
                    "",
                    14.0,
                    on_revert,
                )),
        );

    let body = div()
        .id(gpui::SharedString::from(format!("{id}-body")))
        .flex_1()
        .min_h(px(180.0))
        .overflow_hidden()
        .child(source_view(t, source));

    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .bg(Theme::color(t.sunk))
        .border_1()
        .border_color(Theme::color(t.line))
        .r_sm()
        .overflow_hidden()
        .child(head)
        .child(body)
}

/// The source as plain monospace text: round 1's stand-in for CodeMirror.
/// The line-number gutter matches CodeMirror's: a 36 px right-aligned
/// column with 8 px right padding, then the lint gutter's 18 px.
fn source_view(t: &Theme, source: &str) -> impl IntoElement {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut gutter = div()
        .flex()
        .flex_col()
        .flex_none()
        .border_r_1()
        .border_color(Theme::color(t.line));
    for (i, _) in lines.iter().enumerate() {
        gutter = gutter.child(
            div()
                .h(px(CODE_LINE))
                .whitespace_nowrap()
                .text_size(px(CODE_FONT))
                .line_height(px(CODE_LINE))
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.mute))
                .w(px(36.0))
                .pr(px(8.0))
                .text_right()
                .child((i + 1).to_string()),
        );
    }
    let mut col = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .text_size(px(CODE_FONT))
        .line_height(px(CODE_LINE))
        .font_family(crate::fonts::MONO_FAMILY)
        .text_color(Theme::color(t.dim))
        .pl_2();
    for line in lines {
        col = col.child(div().h(px(CODE_LINE)).whitespace_nowrap().child(line.to_string()));
    }
    div().flex().flex_1().min_h(px(0.0)).child(gutter).child(col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_line_height_is_the_css_one() {
        assert_eq!(CODE_LINE, 20.8);
        assert_eq!(CODE_FONT, 13.0);
    }
}
