//! `Button` and `IconButton`, after `app/web/src/components/Button.tsx`
//! and `app/web/src/styles/app.css` (`.btn`, `.btn-primary`, `.btn-ghost`,
//! `.btn-sm`, `.ibtn`, `.ibtn-sm`) with the hardware layer's re-skin
//! (`hardware.css`: skirt, live primary, ghost transparent).
//!
//! Drawn from `Theme` only. Primary is the live accent (`--live`) with
//! `on-accent` text and no border; default is raised with a `line-2`
//! border; ghost is transparent with dim text. Small is 28 px tall with
//! 10 px side padding; default is the 32 px control height.

use gpui::{ClickEvent, IntoElement, Rgba, Window, div, prelude::*, px};

use crate::theme::{Theme, TokenRadius};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Variant {
    Primary,
    Default,
    Ghost,
}

fn colors(t: &Theme, variant: Variant, disabled: bool) -> (Rgba, Rgba, Rgba) {
    let transparent = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    let (bg, fg, border) = match variant {
        Variant::Primary => (t.live(), Theme::color(t.on_accent), transparent),
        Variant::Default => (
            Theme::color(t.raised),
            Theme::color(t.ink),
            Theme::color(t.line_2),
        ),
        Variant::Ghost => (transparent, Theme::color(t.dim), transparent),
    };
    if disabled {
        // Hardware `.btn:disabled`: raised face, line border, mute text, at
        // half strength (opacity 0.5 in hardware.css).
        let bg = if variant == Variant::Primary {
            Theme::color(t.raised)
        } else {
            bg
        };
        let border = if variant == Variant::Primary || variant == Variant::Ghost {
            Theme::color(t.line)
        } else {
            border
        };
        (
            Rgba { a: 0.5, ..bg },
            Rgba { a: 0.5, ..Theme::color(t.mute) },
            Rgba { a: 0.5, ..border },
        )
    } else {
        (bg, fg, border)
    }
}

/// The hardware skirt as a solid 2 px strip below a key: GPUI 0.2.2's
/// box-shadow paints nothing on this build (measured in #204: the strip under
/// Pause lights is panel colour in GPUI, ink-at-0.22 in React), so the band
/// is drawn instead. `bottom: -2` hangs it below the face exactly like the
/// shadow did; it is clipped to the face's bottom corners' curve by the
/// radius clamp.
pub fn skirt_strip(color: Rgba) -> impl IntoElement {
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(-2.0))
        .h(px(3.0))
        .bg(color)
}

fn base(
    t: &Theme,
    id: &str,
    variant: Variant,
    disabled: bool,
    small: bool,
) -> gpui::Stateful<gpui::Div> {
    let (bg, fg, border) = colors(t, variant, disabled);
    let skirt_color = match variant {
        Variant::Primary => t.live_skirt(),
        _ => t.skirt(),
    };
    let el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .r_md()
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY);
    let el = if disabled { el } else { el.child(skirt_strip(skirt_color)) };
    if small {
        el.h(px(28.0)).px(px(10.0))
    } else {
        el.h(px(32.0)).px_3()
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
    let el = base(t, id, variant, disabled, false).child(label.to_string());
    if disabled {
        el
    } else {
        el.cursor_pointer().on_click(on_click)
    }
}

/// A small (28 px) button, as the strip's Pause lights uses (`size="sm"`).
#[allow(dead_code)]
pub fn button_sm(
    t: &Theme,
    id: &str,
    label: &str,
    variant: Variant,
    disabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let el = base(t, id, variant, disabled, true).child(label.to_string());
    if disabled {
        el
    } else {
        el.cursor_pointer().on_click(on_click)
    }
}

/// A button with a Phosphor icon before its label (`icon` is the asset name
/// without weight, e.g. `FloppyDisk` with `bold`, or `Pause` with `fill`).
#[allow(clippy::too_many_arguments)]
pub fn button_icon(
    t: &Theme,
    id: &str,
    label: &str,
    variant: Variant,
    disabled: bool,
    small: bool,
    icon: &str,
    weight: &str,
    icon_px: f32,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let path = if weight.is_empty() {
        format!("icons/{icon}.svg")
    } else {
        format!("icons/{icon}-{weight}.svg")
    };
    let (_, fg, _) = colors(t, variant, disabled);
    let glyph = gpui::svg().path(path).size(px(icon_px)).text_color(fg);
    let base_el = base(t, id, variant, disabled, small);
    // An empty label draws no gap after the icon (the phone strip pause is
    // the row's button with its words hidden).
    let el = if label.is_empty() {
        base_el.child(glyph)
    } else {
        base_el.child(glyph).child(label.to_string())
    };
    // The icon takes the button's text colour, except primary where it is
    // on-accent; both are the same token here, so set it after building.
    if disabled {
        el
    } else {
        el.cursor_pointer().on_click(on_click)
    }
}

/// An icon-only button (`.ibtn`): 32 px, or 28 px when `small`. `bordered`
/// draws the deck transport's raised key face; the strip's search is bare.
#[allow(clippy::too_many_arguments)]
pub fn icon_button(
    t: &Theme,
    id: &str,
    icon: &str,
    weight: &str,
    small: bool,
    bordered: bool,
    disabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let path = if weight.is_empty() {
        format!("icons/{icon}.svg")
    } else {
        format!("icons/{icon}-{weight}.svg")
    };
    let size = if small { 28.0 } else { 32.0 };
    let el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(px(size))
        .r_md()
        .text_color(Theme::color(t.dim))
        .child(
            gpui::svg()
                .path(path)
                .size(px(16.0))
                .text_color(Theme::color(t.dim)),
        );
    let el = if bordered {
        el.border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.raised))
            .child(skirt_strip(t.skirt()))
    } else {
        el
    };
    if disabled {
        el.opacity(0.4)
    } else {
        el.cursor_pointer().on_click(on_click)
    }
}
