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
    tracked_text_lh(text, font_size, tracking_em, weight, family, color, 1.45)
}

/// A tracked title in the `h2.panel-title` metrics: React's `Panel.tsx`
/// renders the title as an `h2`, whose `line-height: 1.25` overrides the
/// body 1.45 that `.panel-title` (hardware.css) inherits — 13 px font in a
/// 16.25 px box, not 18.85. A title that wraps therefore grows the head by
/// 2.6 px less per wrapped line in React, which put the 390 head 6 px high
/// in GPUI at 1.45 (#204).
pub fn tracked_title(
    text: &str,
    font_size: f32,
    tracking_em: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
) -> gpui::AnyElement {
    tracked_text_lh(text, font_size, tracking_em, weight, family, color, 1.25)
}

/// A tracked title that stays one line (React `.audio-compact
/// .panel-title`: `flex: none; white-space: nowrap`).
pub fn tracked_title_nowrap(
    text: &str,
    font_size: f32,
    tracking_em: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
) -> gpui::AnyElement {
    tracked_text_lh_nowrap(text, font_size, tracking_em, weight, family, color, 1.25)
}

fn tracked_text_lh_nowrap(
    text: &str,
    font_size: f32,
    tracking_em: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
    line_height: f32,
) -> gpui::AnyElement {
    let tracking = px(font_size * tracking_em);
    let space_w = px(font_size * 0.6) + tracking;
    let mut row = div()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .text_size(px(font_size))
        .line_height(gpui::relative(line_height))
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
            .whitespace_nowrap()
            .flex_none();
        for ch in word.chars() {
            cell = cell.child(ch.to_string());
        }
        row = row.child(cell);
        if wi + 1 < n {
            row = row.child(div().w(space_w).h(px(1.0)).flex_none());
        }
    }
    row.into_any_element()
}

fn tracked_text_lh(
    text: &str,
    font_size: f32,
    tracking_em: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
    line_height: f32,
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
        // React captions inherit the body 1.45; titles take the h2 1.25
        // (see `tracked_title`); without this GPUI lays the tracked lines
        // out at its 1.618 default.
        .line_height(gpui::relative(line_height))
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
                .child(tracked_title(
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
            // React `.field-label` sets no colour: it inherits the body ink
            // (`--ink`), not dim (#204 G8, measured in both themes).
            div()
                .flex_1()
                // React `.field-label` inherits the body 15 px (`--t15`,
                // `app.css:23`); `text_sm()` (14 px) drew Source 46 against
                // 50 and Decimals 58 against 65 (#204 G8-r3).
                .text_size(px(15.0))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                // The 15 px body at line-height 1.45 is a 21.75 px box, the
                // same box React's label takes (the G8 1.6 at 14 px was
                // 22.4 and grew every wide field by 0.65 px).
                .line_height(gpui::relative(1.45))
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

/// A row field with a hint under the label (React `Field` with `hint`):
/// the label column holds the 15 px label over the 14 px mute hint.
pub fn field_hint(
    t: &Theme,
    id: &str,
    label: &str,
    hint: Option<&str>,
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
    let el = if last { el } else { el.border_b_1().border_color(Theme::color(t.line)) };
    // React `.field-label`: a flex column, flex 1 1 140px. The basis is
    // 140 px, not the content width: a content basis let the Setup hint
    // stay on one line at 1920 (React wraps it onto two).
    let mut label_col = div()
        .flex()
        .flex_col()
        .flex_1()
        .w(px(140.0))
        .min_w(px(140.0))
        .child(
            div()
                .text_size(px(15.0))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .line_height(gpui::relative(1.45))
                .child(label.to_string()),
        );
    if let Some(h) = hint {
        label_col = label_col.child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.mute))
                .line_height(gpui::relative(1.45))
                .child(h.to_string()),
        );
    }
    el.child(label_col).child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .justify_end()
            .child(control),
    )
}

/// A row field whose hint is a figure (React `Field` with a `.fig` hint,
/// e.g. Frame rate's "<n> fps measured"): the hint is in the mono face.
pub fn field_hint_fig(
    t: &Theme,
    id: &str,
    label: &str,
    hint: Option<&str>,
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
    let el = if last { el } else { el.border_b_1().border_color(Theme::color(t.line)) };
    let mut label_col = div()
        .flex()
        .flex_col()
        .flex_1()
        .w(px(140.0))
        .min_w(px(140.0))
        .child(
            div()
                .text_size(px(15.0))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .line_height(gpui::relative(1.45))
                .child(label.to_string()),
        );
    if let Some(h) = hint {
        label_col = label_col.child(
            div()
                .text_sm()
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.mute))
                .line_height(gpui::relative(1.45))
                .child(h.to_string()),
        );
    }
    el.child(label_col).child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .justify_end()
            .child(control),
    )
}

/// A stacked field with a figure hint (the phone form of `field_hint_fig`).
pub fn field_stack_hint_fig(
    t: &Theme,
    id: &str,
    label: &str,
    hint: Option<&str>,
    control: impl IntoElement,
    last: bool,
) -> gpui::Stateful<gpui::Div> {
    let el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .gap_2()
        .min_h(px(48.0))
        .py_2();
    let el = if last { el } else { el.border_b_1().border_color(Theme::color(t.line)) };
    let mut label_col = div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(px(15.0))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .line_height(gpui::relative(1.45))
                .child(label.to_string()),
        );
    if let Some(h) = hint {
        label_col = label_col.child(
            div()
                .text_sm()
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.mute))
                .line_height(gpui::relative(1.45))
                .child(h.to_string()),
        );
    }
    el.child(label_col).child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(control),
    )
}

/// A stacked field with a hint (the phone form of `field_hint`): label
/// over control, control at the start and full width.
pub fn field_stack_hint(
    t: &Theme,
    id: &str,
    label: &str,
    hint: Option<&str>,
    control: impl IntoElement,
    last: bool,
) -> gpui::Stateful<gpui::Div> {
    let el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        // React `.field` wraps with `gap: var(--s2) var(--s4)`: the
        // stacked rows sit 8 px apart.
        .gap_2()
        .min_h(px(48.0))
        .py_2();
    let el = if last { el } else { el.border_b_1().border_color(Theme::color(t.line)) };
    let mut label_col = div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(px(15.0))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .line_height(gpui::relative(1.45))
                .child(label.to_string()),
        );
    if let Some(h) = hint {
        label_col = label_col.child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.mute))
                .line_height(gpui::relative(1.45))
                .child(h.to_string()),
        );
    }
    el.child(label_col).child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
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
    // The stacked label is the same `.field-label`: ink, not dim, 15 px at
    // the body line-height 1.45 (a 21.75 px box in both the wide grid and
    // the phone).
    let _ = wide;
    let label_el = div()
        .text_size(px(15.0))
        .font_family(crate::fonts::UI_FAMILY)
        .text_color(Theme::color(t.ink))
        .line_height(gpui::relative(1.45));
    el.child(label_el.child(label.to_string()))
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
