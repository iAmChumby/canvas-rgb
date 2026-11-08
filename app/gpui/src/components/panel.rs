//! `Panel` and `Field`, after `app/web/src/components/Panel.tsx` and the
//! `.panel` / `.panel-head` / `.panel-title` / `.field` rules in `app.css`
//! re-skinned by `hardware.css` (titles are printed on the case: mono,
//! small capitals, spaced).
//!
//! The panel is a 16 px padding, line-bordered, 16 px radius block; its
//! head is a wrapping row with the title left and the actions right. A
//! field is label-left / control-right with a divider, or (in the wide and
//! phone layouts, via [`field_stack`]) label over control.

use gpui::{
    App, Bounds, Element, ElementId, FontWeight, GlobalElementId, IntoElement, LayoutId, Pixels,
    Point, Rgba, ShapedLine, SharedString, Style, TextRun, Window, div, point, prelude::*, px,
};

use crate::theme::{Theme, TokenRadius};

/// A `letter-spacing` element (#240): the run is shaped once with
/// `shape_line`, and painting walks the shaped glyphs with a per-character
/// tracking offset, so the font's kerning returns. `tracking` is in px and
/// may be negative (the strip title's -0.01em). `extra_w` is an unshaped
/// trailing allowance: the space cell carries the tracking that follows it,
/// as CSS places it after the space character.
#[derive(Clone)]
struct TrackedText {
    text: SharedString,
    font_size: f32,
    tracking: f32,
    extra_w: f32,
    line_height: f32,
    font: gpui::Font,
    color: Rgba,
}

struct TrackedPrepaint {
    line: ShapedLine,
    origin: Point<Pixels>,
    baseline_y: f32,
}

impl TrackedText {
    fn run(&self) -> TextRun {
        TextRun {
            len: self.text.len(),
            font: self.font.clone(),
            color: gpui::Hsla::from(self.color),
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    fn char_ordinal(text: &str, byte_index: usize) -> f32 {
        // Glyph indices are byte offsets; CSS letter-spacing advances per
        // character, not per byte.
        text[..byte_index.min(text.len())].chars().count() as f32
    }
}

impl IntoElement for TrackedText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TrackedText {
    type RequestLayoutState = ();
    type PrepaintState = TrackedPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let run = self.run();
        let layout = window
            .text_system()
            .layout_line(&self.text, px(self.font_size), std::slice::from_ref(&run), None);
        let gaps = self.text.chars().count().saturating_sub(1) as f32;
        let width = f32::from(layout.width) + self.tracking * gaps + self.extra_w;
        let height = self.font_size * self.line_height;
        let mut style = Style::default();
        style.size.width = px(width).into();
        style.size.height = px(height).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let run = self.run();
        let line = window
            .text_system()
            .shape_line(self.text.clone(), px(self.font_size), std::slice::from_ref(&run), None);
        let line_height_px = self.font_size * self.line_height;
        let ascent = f32::from(line.ascent);
        let descent = f32::from(line.descent);
        // gpui's text painter pads the ascent/descent into the line height
        // and paints the baseline below the top of the box.
        let padding_top = (line_height_px - ascent - descent) / 2.0;
        TrackedPrepaint {
            line,
            origin: bounds.origin,
            baseline_y: f32::from(bounds.origin.y) + padding_top + ascent,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let hsla = gpui::Hsla::from(self.color);
        for run in prepaint.line.runs.iter() {
            for glyph in run.glyphs.iter() {
                let ordinal = Self::char_ordinal(&self.text, glyph.index);
                let x = f32::from(glyph.position.x) + self.tracking * ordinal;
                let at = point(
                    px(f32::from(prepaint.origin.x) + x),
                    px(prepaint.baseline_y + f32::from(glyph.position.y)),
                );
                let _ = window.paint_glyph(at, run.font_id, glyph.id, px(self.font_size), hsla);
            }
        }
    }
}

/// One word of a tracked line: shaped once by [`TrackedText`], keeping the
/// font's kerning inside the word. The row lays the words out in a wrapping
/// flex, so a long title still wraps and the head keeps its height (#240:
/// one-element-per-title drew the whole line wide and pushed the rows below
/// up 16 px at 390).
fn tracked_word(
    word: &str,
    font_size: f32,
    tracking: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
    line_height: f32,
) -> gpui::AnyElement {
    let font = gpui::Font {
        family: family.to_string().into(),
        weight,
        style: gpui::FontStyle::Normal,
        // DirectWrite's text layout leaves `kern` off unless a typography is
        // set, so "STATUS" came out 3 px wide (no T-A / A-T pairs, -1.57 px
        // each at 15 px); naming `kern` brings Chrome's kerning back.
        features: gpui::FontFeatures(std::sync::Arc::new(vec![("kern".to_string(), 1)])),
        fallbacks: None,
    };
    TrackedText {
        text: word.to_string().into(),
        font_size,
        tracking,
        extra_w: 0.0,
        line_height,
        font,
        color,
    }
    .into_any()
}

/// A text with CSS `letter-spacing` (GPUI text has no tracking).
///
/// Drawn by [`TrackedText`]: the run is shaped once with
/// `window.text_system().shape_line` and every glyph is painted at its shaped
/// position plus `tracking` times its character index, so the font's own
/// kerning returns. The old per-character cells dropped kerning and ran about
/// 2 px wide on "Displays" and 4 px wide on PREVIEW (#240).
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
    // One shaped run per word (no wrapping): kerning inside each word, and
    // the space cells take the tracking that follows the space character.
    let tracking = font_size * tracking_em;
    let mut row = div()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .flex_none();
    let words: Vec<&str> = text.split_whitespace().collect();
    for (wi, word) in words.iter().enumerate() {
        row = row.child(tracked_word(word, font_size, tracking, weight, family, color, line_height));
        if wi + 1 < words.len() {
            row = row.child(tracked_space(font_size, tracking, weight, family, color, line_height));
        }
    }
    row.into_any_element()
}

fn tracked_space(
    font_size: f32,
    tracking: f32,
    _weight: FontWeight,
    family: &'static str,
    color: Rgba,
    line_height: f32,
) -> gpui::AnyElement {
    // A space character (no ink, just the mono/UI advance) plus the tracking
    // that follows it. `TrackedText` paints the space glyph without ink.
    let font = gpui::Font {
        family: family.to_string().into(),
        weight: _weight,
        style: gpui::FontStyle::Normal,
        features: gpui::FontFeatures::default(),
        fallbacks: None,
    };
    TrackedText {
        text: " ".to_string().into(),
        font_size,
        tracking: 0.0,
        extra_w: tracking * 2.0,
        line_height,
        font,
        color,
    }
    .into_any()
}

/// The line-height-aware tracked element (`tracked_text` at a caller-chosen
/// line-height; the strip title uses 1.0, `.rd-title h1`).
pub fn tracked_text_lh(
    text: &str,
    font_size: f32,
    tracking_em: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
    line_height: f32,
) -> gpui::AnyElement {
    // React captions inherit the body 1.45; titles take the h2 1.25 (see
    // `tracked_title`); without this GPUI lays the tracked lines out at its
    // 1.618 default. Words are shaped once each and wrap at spaces, as CSS.
    let tracking = font_size * tracking_em;
    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .text_size(px(font_size))
        .line_height(gpui::relative(line_height))
        .font_weight(weight)
        .font_family(family)
        .text_color(color);
    let words: Vec<&str> = text.split_whitespace().collect();
    for (wi, word) in words.iter().enumerate() {
        row = row.child(tracked_word(word, font_size, tracking, weight, family, color, line_height));
        if wi + 1 < words.len() {
            row = row.child(tracked_space(font_size, tracking, weight, family, color, line_height));
        }
    }
    row.into_any_element()
}

/// A tracked text whose word space is the font's own space, not the mono one
/// [`tracked_text`] assumes. CSS `letter-spacing` tightens every glyph and
/// the space with it; the UI font's space is a quarter of an em, so a mono
/// space would put the words back where they were and the line would wrap
/// where React's does not.
#[allow(clippy::too_many_arguments)]
pub fn tracked_text_space(
    text: &str,
    font_size: f32,
    tracking_em: f32,
    space_em: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
    line_height: f32,
) -> gpui::AnyElement {
    tracked_text_space_wrap(text, font_size, tracking_em, space_em, weight, family, color, line_height, true)
}

/// A tracked text that stays on one line: React's titles use
/// `text-wrap: balance`, and a title that is a little wider than the panel
/// must not break into two lines and push the rest of the screen down.
#[allow(clippy::too_many_arguments)]
pub fn tracked_text_space_nowrap(
    text: &str,
    font_size: f32,
    tracking_em: f32,
    space_em: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
    line_height: f32,
) -> gpui::AnyElement {
    tracked_text_space_wrap(text, font_size, tracking_em, space_em, weight, family, color, line_height, false)
}

#[allow(clippy::too_many_arguments)]
fn tracked_text_space_wrap(
    text: &str,
    font_size: f32,
    tracking_em: f32,
    space_em: f32,
    weight: FontWeight,
    family: &'static str,
    color: Rgba,
    line_height: f32,
    wrap: bool,
) -> gpui::AnyElement {
    let tracking = px(font_size * tracking_em);
    // CSS adds the tracking after every glyph, the space included; the word
    // cells below carry one step less each (between the glyphs), so the space
    // carries the step the last glyph of the word does not.
    let space_w = px(font_size * (space_em + tracking_em));
    let mut row = div()
        .flex()
        .items_center()
        .text_size(px(font_size))
        .line_height(gpui::relative(line_height))
        .font_weight(weight)
        .font_family(family)
        .text_color(color);
    if wrap {
        row = row.flex_wrap();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let n = words.len();
    for (wi, word) in words.iter().enumerate() {
        let cell = word_cell(word, tracking);
        row = row.child(cell);
        if wi + 1 < n {
            row = row.child(div().w(space_w).h(px(1.0)).flex_none());
        }
    }
    row.into_any_element()
}

/// One word of a tracked line: its glyphs, each one a step of `tracking`
/// apart.
///
/// A step that opens the line is a flex gap. A step that closes it (CSS
/// `letter-spacing: -0.02em` on a title) is a negative margin on every
/// glyph but the first: the layout engine ignores a negative gap (the
/// title came out 12 px wider than the browser's), and a negative margin
/// tightens the line the way the browser does.
fn word_cell(word: &str, tracking: gpui::Pixels) -> gpui::Div {
    let step = f32::from(tracking);
    let mut cell = div().flex().items_center().whitespace_nowrap().flex_none();
    if step < 0.0 {
        for (i, ch) in word.chars().enumerate() {
            let mut glyph = div().flex().items_center().whitespace_nowrap();
            if i > 0 {
                glyph = glyph.ml(tracking);
            }
            glyph = glyph.child(ch.to_string());
            cell = cell.child(glyph);
        }
    } else {
        cell = cell.gap(tracking);
        for ch in word.chars() {
            cell = cell.child(ch.to_string());
        }
    }
    cell
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
    panel_with_padding(t, id, title, actions, body, None)
}

/// A panel with an optional padding override (React `.rd-fans .panel {
/// padding: var(--s3) }` on a phone: 12 px instead of 16).
#[allow(dead_code)]
pub fn panel_with_padding(
    t: &Theme,
    id: &str,
    title: &str,
    actions: impl IntoElement,
    body: impl IntoElement,
    padding: Option<f32>,
) -> gpui::Stateful<gpui::Div> {
    panel_with_head_margin(t, id, title, actions, body, padding, 12.0)
}

/// A panel whose head carries `head_margin` px under it instead of the
/// base panel's `s3` (12 px): the stack stage's head has no margin
/// (`.rd-stack-stage .panel-head { margin-bottom: 0 }`) and the layers
/// head has `s2` (8 px). A negative margin on the body to cancel the base
/// margin instead collapsed the stage (measured at 390x844: a 55 px
/// head-only panel), so the margin lives on the head.
#[allow(dead_code)]
pub fn panel_with_head_margin(
    t: &Theme,
    id: &str,
    title: &str,
    actions: impl IntoElement,
    body: impl IntoElement,
    padding: Option<f32>,
    head_margin: f32,
) -> gpui::Stateful<gpui::Div> {
    let mut el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .r_lg()
        .border_1()
        .border_color(Theme::color(t.line))
        .bg(Theme::color(t.panel));
    el = match padding {
        Some(p) => el.p(px(p)),
        None => el.p_4(),
    };
    el.child(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .mb(px(head_margin))
            // React `.panel-head { flex: 0 0 auto; }`: the head keeps its
            // height when the panel's body scrolls.
            .flex_shrink_0()
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
        "mute" => Theme::color(t.mute),
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
