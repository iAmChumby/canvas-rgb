//! `Panel` and `Field`, after `app/web/src/components/Panel.tsx` and the
//! `.panel` / `.panel-head` / `.panel-title` / `.field` rules in `app.css`
//! re-skinned by `hardware.css` (titles are printed on the case: mono,
//! small capitals, spaced).
//!
//! The panel is a 16 px padding, line-bordered, 16 px radius block; its
//! head is a wrapping row with the title left and the actions right. A
//! field is label-left / control-right with a divider, or (in the wide and
//! phone layouts, via [`field_stack`]) label over control.

use gpui::{FontWeight, IntoElement, Rgba, div, prelude::*, px};

use crate::theme::{Theme, TokenRadius};

/// A text with CSS `letter-spacing` (GPUI text has no tracking), drawn as
/// words of per-character cells: each word is a row of characters separated
/// by `tracking_em` of the font size, and words wrap at word boundaries as
/// in CSS. Used for the panel title (0.08em) and the readout captions
/// (0.07em).
pub fn tracked_text(
    text: &str,
    font_size: f32,
    tracking_em: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
) -> gpui::AnyElement {
    let tracking = px(font_size * tracking_em);
    // A mono space advances a full 0.6em, like the browser's (plus the
    // tracking after it); a quarter-em made spaced titles fall short.
    let space_w = px(font_size * 0.6) + tracking;
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .text_size(px(font_size))
        .font_weight(weight)
        .font_family(family)
        .text_color(color);
    let words: Vec<&str> = text.split_whitespace().collect();
    let n = words.len();
    for (wi, word) in words.iter().enumerate() {
        let mut cell = div()
            .flex()
            .items_center()
            .gap(tracking)
            .whitespace_nowrap();
        for ch in word.chars() {
            cell = cell.child(ch.to_string());
        }
        row = row.child(cell);
        if wi + 1 < n {
            row = row.child(div().w(space_w).h(px(1.0)));
        }
    }
    row.into_any_element()
}

/// A panel with a title. `actions` render beside the title, on the right.
///
/// The panel is returned as a styled div, so a caller that needs it to fill
/// its cell (React's `.rd-headers`, `.rd-fans-right > .panel`) can say so.
#[allow(dead_code)]
pub fn panel(
    t: &Theme,
    id: &str,
    title: &str,
    actions: impl IntoElement,
    body: impl IntoElement,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .p_4()
        .r_lg()
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
                // React `.panel-head { flex: 0 0 auto; }`: the head keeps its
                // height when the panel's body scrolls.
                .flex_shrink_0()
                .child(tracked_text(
                    &title.to_uppercase(),
                    13.0,
                    0.08,
                    gpui::FontWeight::MEDIUM,
                    crate::fonts::MONO_FAMILY,
                    Theme::color(t.mute),
                ))
                .child(actions),
        )
        .child(body)
}

/// A labelled field in row form (the intermediate width): label left,
/// control right, divider under the row. React `.field`: min-height 48,
/// vertical padding s2 (8), border-bottom, except the last field which has
/// none (`.field:last-child`).
pub fn field(
    t: &Theme,
    id: &str,
    label: &str,
    control: impl IntoElement,
    last: bool,
) -> gpui::Stateful<gpui::Div> {
    let el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_2()
        .min_h(px(48.0))
        .py_2();
    let el = if last {
        el
    } else {
        el.border_b_1().border_color(Theme::color(t.line))
    };
    el
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
/// phone): label over control, control justified to the start. React
/// `.field` keeps its 48 min-height and s2 vertical padding in both stacked
/// forms; the wide form drops the divider (`border-bottom: 0`), the phone
/// keeps it except on the last field.
pub fn field_stack(
    t: &Theme,
    id: &str,
    label: &str,
    control: impl IntoElement,
    wide: bool,
    last: bool,
) -> gpui::Stateful<gpui::Div> {
    let el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .gap_1()
        .min_h(px(48.0))
        .py_2();
    let el = if wide || last {
        el
    } else {
        el.border_b_1().border_color(Theme::color(t.line))
    };
    el
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
                .w_full()
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
