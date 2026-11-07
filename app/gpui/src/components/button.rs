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
        // Hardware `.btn:disabled`: at half strength (opacity 0.5). The
        // border stays the variant's own (ghost stays transparent, so a
        // disabled ghost is still just text: React's `.btn-ghost:disabled`
        // keeps `border-color: transparent`).
        if variant == Variant::Ghost {
            // A disabled ghost is bare text (React `.btn-ghost:disabled`:
            // background and border transparent, mute words): half-strength
            // black made it a bordered chip (Boot colour's Clear).
            return (bg, Theme::color(t.mute), border);
        }
        let bg = if variant == Variant::Primary {
            Theme::color(t.raised)
        } else {
            bg
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

/// A key drawn as two stacked rounded rectangles (#204 G8-r3): an outer
/// skirt-coloured rect with the full radius, 2 px taller than the key, and
/// the key's face on top, 2 px shorter at the bottom. The skirt then follows
/// the curve exactly. A 2 px tall strip cannot: GPUI clamps a radius to
/// half the rect's height, so the corners came out square and the band
/// measured wider than the key body (Save skirt 1189-1267 against the body
/// 1192-1264, 1 px high, at 1440).
pub fn skirt_layers(
    skirt_color: Rgba,
    radius: f32,
    height: f32,
    face_bg: Rgba,
    face_border: Rgba,
) -> impl IntoElement {
    let corner = gpui::AbsoluteLength::Pixels(px(radius));
    let skirt = div()
        .w_full()
        .h(px(height + 2.0))
        .bg(skirt_color);
    let mut skirt = skirt;
    skirt.style().corner_radii.top_left = Some(corner);
    skirt.style().corner_radii.top_right = Some(corner);
    skirt.style().corner_radii.bottom_right = Some(corner);
    skirt.style().corner_radii.bottom_left = Some(corner);
    let face = div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(height))
        .border_1()
        .border_color(face_border)
        .bg(face_bg);
    let mut face = face;
    face.style().corner_radii.top_left = Some(corner);
    face.style().corner_radii.top_right = Some(corner);
    face.style().corner_radii.bottom_right = Some(corner);
    face.style().corner_radii.bottom_left = Some(corner);
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(height + 2.0))
        .child(skirt)
        .child(face)
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
    let key_h = if small { 28.0 } else { 32.0 };
    let el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .text_color(fg)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY);
    // React `hardware.css`: `.btn-ghost{box-shadow:none}` — a ghost key has no
    // skirt; a disabled key loses it too (`.btn:disabled{box-shadow:none}`).
    // A skirted key carries no bg/border of its own: the face layer in
    // `skirt_layers` draws both, so the parent stays transparent over the
    // skirt's bottom 2 px.
    let bordered = disabled || variant == Variant::Ghost;
    let el = if bordered {
        el.r_md().border_1().border_color(border).bg(bg)
    } else {
        el.child(skirt_layers(skirt_color, 10.0, key_h, bg, border))
    };
    // React's `.btn` always has a 1 px border inside its box. A skirted key draws its
    // border on the face layer, not on this box, so it gets the border's 1 px back as
    // padding; without it every skirted key measured 2 px narrow (Save 79 vs 81 at 1440).
    let side = if bordered { 0.0 } else { 1.0 };
    if small {
        el.h(px(28.0)).px(px(10.0 + side))
    } else {
        el.h(px(32.0)).px(px(12.0 + side))
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
    // The glyph keeps its px size inside a flex row (React's svg has an
    // intrinsic size and never shrinks under flex overflow).
    let glyph = gpui::svg().path(path).size(px(icon_px)).flex_none().text_color(fg);
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

/// A button with an icon and a label whose cell reserves `text_w` px for
/// the text: React's `Swap` lays every label variant in one grid cell, so
/// the key keeps the widest label's width and the visible label is centred
/// in it (the strip's Pause lights / Resume lights). `text_w` is the widest
/// label's measured width, so the key never resizes under the pointer.
#[allow(clippy::too_many_arguments)]
pub fn button_icon_wide(
    t: &Theme,
    id: &str,
    label: &str,
    text_w: f32,
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
    let glyph = gpui::svg().path(path).size(px(icon_px)).flex_none().text_color(fg);
    let base_el = base(t, id, variant, disabled, small);
    let el = base_el
        .child(glyph)
        .child(div().w(px(text_w)).text_center().child(label.to_string()));
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
        .text_color(Theme::color(t.dim));
    let el = if bordered {
        el.child(skirt_layers(
            t.skirt(),
            10.0,
            size,
            Theme::color(t.raised),
            Theme::color(t.line_2),
        ))
    } else {
        el.r_md()
    };
    let el = el.child(
        gpui::svg()
            .path(path)
            .size(px(16.0))
            .text_color(Theme::color(t.dim)),
    );
    if disabled {
        el.opacity(0.4)
    } else {
        el.cursor_pointer().on_click(on_click)
    }
}
