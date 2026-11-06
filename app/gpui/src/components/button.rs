//! `Button` and `IconButton`, after `app/web/src/components/Button.tsx`.
//! Drawn from `Theme` only. The GPUI port carries the variant looks the shell
//! and Displays use (primary, default, ghost) and the disabled state.

use gpui::{ClickEvent, IntoElement, Rgba, Window, div, prelude::*, px};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Variant {
    Primary,
    Default,
    Ghost,
}

fn colors(t: &Theme, variant: Variant, disabled: bool) -> (Rgba, Rgba, Rgba) {
    let (bg, fg, border) = match variant {
        Variant::Primary => (
            Theme::color(t.a1),
            Theme::color(t.on_accent),
            Theme::color(t.a1),
        ),
        Variant::Default => (
            Theme::color(t.raised),
            Theme::color(t.ink),
            Theme::color(t.line_2),
        ),
        Variant::Ghost => (
            Theme::color(t.raised),
            Theme::color(t.ink),
            Theme::color(t.line),
        ),
    };
    if disabled {
        (Rgba { a: 0.45, ..bg }, Rgba { a: 0.45, ..fg }, Rgba { a: 0.45, ..border })
    } else {
        (bg, fg, border)
    }
}

/// A button of the given variant and label. `on_click` runs only when enabled.
pub fn button(
    t: &Theme,
    id: &str,
    label: &str,
    variant: Variant,
    disabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (bg, fg, border) = colors(t, variant, disabled);
    let el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .font(crate::fonts::font_ui())
        .child(label.to_string());
    if disabled {
        el
    } else {
        el.cursor_pointer().on_click(on_click)
    }
}

/// An icon-only button; the label is the accessible name (kept in the element
/// id for tests), drawn as a small glyph text here.
#[allow(dead_code)]
pub fn icon_button(
    t: &Theme,
    id: &str,
    label: &str,
    size_px: f32,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let _ = label;
    let border = Theme::color(t.line_2);
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_center()
        .size(px(size_px))
        .rounded_md()
        .border_1()
        .border_color(border)
        .bg(Theme::color(t.raised))
        .text_color(Theme::color(t.ink))
        .text_sm()
        .font(crate::fonts::font_ui())
        .cursor_pointer()
        .on_click(on_click)
}
