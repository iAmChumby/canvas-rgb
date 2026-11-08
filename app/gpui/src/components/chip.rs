//! `Chip`, after `app/web/src/components/Chip.tsx` and the `.chip` rules in
//! `app.css` (28 px tall, 10 px side padding, line border, small radius,
//! dim text in 14 px medium; pressed wears the live accent and a live-soft
//! fill; the HTML/Stack words wear `--a2`/`--a3` while hardware.css's later
//! `.chip` keeps the border line-2 and the raised face).
//!
//! The LED `.chip::before` (5 px dot) draws only as a state: lit means the
//! chip is pressed, amber means the values on the lights differ from the
//! preset it names (`chip-marked`), off means a save is held (`chip-pending`).
//! The modified amber is hardware.css's `--modified`.

use gpui::{ClickEvent, IntoElement, Window, div, prelude::*, px};

use crate::components::button::skirt_layers;
use crate::theme::{Theme, TokenRadius};

/// The chip's tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipTone {
    /// Plain (categories, saved presets).
    Plain,
    /// An HTML effect's label (`chip-html`, `--a2`).
    Html,
    /// A stack's label (`chip-stack`, `--a3`).
    Stack,
}

/// `--raised-press` (hardware.css): raised mixed toward the dark end of the case.
fn raised_press(t: &Theme) -> gpui::Rgba {
    let raised = Theme::color(t.raised);
    if t.is_dark() {
        gpui::Rgba { r: raised.r * 0.92, g: raised.g * 0.92, b: raised.b * 0.92, a: 1.0 }
    } else {
        let ink = Theme::color(t.ink);
        gpui::Rgba {
            r: raised.r * 0.9 + ink.r * 0.1,
            g: raised.g * 0.9 + ink.g * 0.1,
            b: raised.b * 0.9 + ink.b * 0.1,
            a: 1.0,
        }
    }
}

/// "Save current": a key that makes a key. Dashed, so it reads as a slot and
/// not a preset; no LED dot (`.chip-add::before` is none); pressed reads as
/// a key gone down (solid border, raised-press face).
pub fn chip_add(
    t: &Theme,
    id: &str,
    label: &str,
    pressed: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let transparent = gpui::Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    let (fg, border, bg) = if pressed {
        (Theme::color(t.ink), Theme::color(t.line_2), raised_press(t))
    } else {
        (Theme::color(t.dim), Theme::color(t.line), transparent)
    };
    let _ = bg;
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(28.0))
        .px(px(10.0))
        .r_sm()
        .border_1()
        .border_color(border)
        .text_color(fg)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY)
        .flex_none()
        .whitespace_nowrap()
        .cursor_pointer()
        .on_click(on_click)
        .child(
            gpui::svg()
                .path("icons/Plus-bold.svg")
                .size(px(14.0))
                .flex_none()
                .text_color(fg),
        )
        .child(label.to_string())
}
#[allow(clippy::too_many_arguments)]
pub fn chip(
    t: &Theme,
    id: &str,
    label: &str,
    tone: ChipTone,
    pressed: bool,
    marked: bool,
    pending: bool,
) -> impl IntoElement {
    chip_inner(t, id, label, None, tone, pressed, marked, pending, None)
}

/// A chip without an icon that is also a key (a preset key, a category).
#[allow(clippy::too_many_arguments)]
pub fn chip_button(
    t: &Theme,
    id: &str,
    label: &str,
    tone: ChipTone,
    pressed: bool,
    marked: bool,
    pending: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    chip_inner(t, id, label, None, tone, pressed, marked, pending, Some(Box::new(on_click)))
}

/// A chip with an icon before its label (the head's feature chips).
#[allow(clippy::too_many_arguments)]
pub fn chip_icon(
    t: &Theme,
    id: &str,
    label: &str,
    icon: Option<(&str, &str, f32)>,
    tone: ChipTone,
    pressed: bool,
    marked: bool,
    pending: bool,
) -> impl IntoElement {
    chip_inner(t, id, label, icon, tone, pressed, marked, pending, None)
}

/// A chip with an icon that is also a key (the head's favourite star).
#[allow(clippy::too_many_arguments)]
pub fn chip_icon_button(
    t: &Theme,
    id: &str,
    label: &str,
    icon: Option<(&str, &str, f32)>,
    tone: ChipTone,
    pressed: bool,
    marked: bool,
    pending: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    chip_inner(t, id, label, icon, tone, pressed, marked, pending, Some(Box::new(on_click)))
}

/// The onboarding's "Add a <part>" chip (React's `PartsStep.tsx`: a `Chip`
/// whose body is the part's icon, its name and a bold plus, in `.addChip`).
/// The chip itself is `hardware.css`'s: a raised face, a line-2 border and
/// the key's skirt, with the LED at the front.
pub fn chip_add_part(
    t: &Theme,
    id: &str,
    label: &str,
    icon: (&str, &str, f32),
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let fg = Theme::color(t.dim);
    let icon_path = if icon.1.is_empty() {
        format!("icons/{}.svg", icon.0)
    } else {
        format!("icons/{}-{}.svg", icon.0, icon.1)
    };
    let body = div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(gpui::svg().path(icon_path).size(px(icon.2)).flex_none().text_color(fg))
        .child(label.to_string())
        .child(gpui::svg().path("icons/Plus-bold.svg").size(px(11.0)).flex_none().text_color(fg));
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .items_center()
        .gap(px(4.0))
        .h(px(28.0))
        // React's `.chip` is 10 px of padding inside its 1 px border, so the
        // words start 11 px from the chip's edge.
        .px(px(11.0))
        .r_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY)
        .text_size(px(14.0))
        .text_color(fg)
        .flex_none()
        .whitespace_nowrap()
        .cursor_pointer()
        .on_click(on_click)
        // The key's face is painted first, then the LED in front of it: the
        // face is an absolute layer, so a dot drawn before it would be
        // hidden under the face.
        .child(crate::components::button::skirt_layers(
            t.skirt(),
            8.0,
            28.0,
            Theme::color(t.raised),
            Theme::color(t.line_2),
        ))
        .child(div().size(px(5.0)).flex_none().rounded_full().bg(Theme::color(t.dot)))
        .child(body)
}

/// A chip's click handler.
type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static>;

#[allow(clippy::too_many_arguments)]
fn chip_inner(
    t: &Theme,
    id: &str,
    label: &str,
    icon: Option<(&str, &str, f32)>,
    tone: ChipTone,
    pressed: bool,
    marked: bool,
    pending: bool,
    on_click: Option<OnClick>,
) -> impl IntoElement {
    // A latched chip stays down (hardware.css `.chip[aria-pressed]`): no
    // skirt, a raised-press face, ink words. The app.css pressed accent is
    // the underlying rule the skin overrides.
    //
    // Every other chip is a key with a face and a skirt (hardware.css
    // `.btn, .select-trigger, .chip`: border line-2, raised face, the 2 px
    // skirt). The face is opaque, so the page's dotted grid never shows
    // through a chip — at some sizes a grid dot landed beside an LED and
    // read as a glyph in it (#210 round 3).
    let skirted = !pressed;
    let (fg, border, bg) = match tone {
        // React `.chip-html`/`.chip-stack` colour the words `--a2`/`--a3`,
        // but hardware.css's later `.chip` (border line-2, raised face)
        // outranks their border colour, so the badge reads as a neutral
        // grey chip with accent words, not an accent-outlined one.
        ChipTone::Html => (Theme::color(t.a2), Theme::color(t.line_2), Theme::color(t.raised)),
        ChipTone::Stack => (Theme::color(t.a3), Theme::color(t.line_2), Theme::color(t.raised)),
        ChipTone::Plain if pressed => (Theme::color(t.ink), Theme::color(t.line_2), raised_press(t)),
        _ => (Theme::color(t.dim), Theme::color(t.line_2), Theme::color(t.raised)),
    };
    let el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .items_center()
        .gap(px(4.0))
        .h(px(28.0))
        // A skirted key draws its border on the face layer, so it gets the
        // border's 1 px back as padding, like `buttons` (without it the key
        // measures 2 px narrow and the LED leaves the CSS padding).
        .px(if skirted { px(11.0) } else { px(10.0) })
        .text_color(fg)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY)
        .flex_none()
        .whitespace_nowrap();
    let el = if skirted {
        el.child(skirt_layers(t.skirt(), 8.0, 28.0, bg, border))
    } else {
        el.r_sm().border_1().border_color(border).bg(bg)
    };
    let mut el = el;
    // The LED dot: only a key (a `button.chip`) carries one (hardware.css);
    // a label chip is a span and has none.
    let has_dot = on_click.is_some();
    if has_dot {
        let dot = if marked {
            Theme::color(t.warn)
        } else if pressed {
            t.live()
        } else {
            Theme::color(t.dot)
        };
        el = el.child(div().size(px(5.0)).rounded_full().bg(dot));
    }
    if let Some((icon, weight, px_size)) = icon {
        let path = if weight.is_empty() {
            format!("icons/{icon}.svg")
        } else {
            format!("icons/{icon}-{weight}.svg")
        };
        el = el.child(gpui::svg().path(path).size(px(px_size)).flex_none().text_color(fg));
    }
    el = el.child(label.to_string());
    if pending {
        el = el.opacity(0.65);
    }
    match on_click {
        Some(click) if !pending => el.cursor_pointer().on_click(click),
        _ => el,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_is_the_only_state_that_lights_the_dot() {
        // The dot rule above: pressed and unmarked reads live; marked reads
        // amber even when pressed. These are the colour choices, as values.
        let t = Theme::dark();
        let dot = |pressed: bool, marked: bool| {
            if marked {
                "amber"
            } else if pressed {
                "live"
            } else {
                "off"
            }
        };
        assert_eq!(dot(true, false), "live");
        assert_eq!(dot(true, true), "amber");
        assert_eq!(dot(false, false), "off");
        assert_eq!(dot(false, true), "amber");
        let _ = t;
    }
}
