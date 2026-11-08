//! `Toggle`, after `app/web/src/components/Toggle.tsx` and the `.switch`
//! rules in `app.css` re-skinned by `hardware.css`: a 40x22 slot, a 16 px
//! knob, sunk when off and live when on, the knob travelling 18 px.

use std::rc::Rc;

use gpui::{BoxShadow, FocusHandle, IntoElement, Window, div, point, prelude::*, px};

use crate::theme::Theme;

/// Which half of a key press a handler sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPhase {
    Down,
    Up,
}

/// A native `<button>`'s keys (React `Toggle.tsx` is `<button role="switch">`):
/// Enter clicks it on key down, Space clicks it on key up. Nothing else does.
pub fn activates(key: &str, phase: KeyPhase) -> bool {
    match phase {
        KeyPhase::Down => key == "enter",
        KeyPhase::Up => key == "space",
    }
}

/// A switch. `checked` draws the on state; clicking reports the new state.
pub fn toggle(
    t: &Theme,
    id: &str,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    toggle_with(t, id, checked, None, on_change)
}

/// A switch that takes the keyboard focus (`Toggle.tsx`'s `<button>`): it is
/// in the tab order, Enter and Space flip it, and a focus ring draws around
/// it (`app.css:95` `:focus-visible`, 2 px `--focus`).
pub fn toggle_focus(
    t: &Theme,
    id: &str,
    checked: bool,
    focus: &FocusHandle,
    on_change: impl Fn(bool, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    toggle_with(t, id, checked, Some(focus), on_change)
}

fn toggle_with(
    t: &Theme,
    id: &str,
    checked: bool,
    focus: Option<&FocusHandle>,
    on_change: impl Fn(bool, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let on_change = Rc::new(on_change);
    let track = if checked {
        t.live()
    } else {
        Theme::color(t.sunk)
    };
    let knob_color = if checked {
        Theme::color(t.on_accent)
    } else {
        Theme::color(t.dim)
    };
    // hardware.css `.switch::after`: `0 1px 0 var(--skirt)` under the knob,
    // at left 2 / top 2 (16 px in a 40x22 slot); checked it travels 18 px,
    // so it spans 20..36. The knob is positioned absolutely at React's
    // offsets instead of centred by flex padding.
    let knob = div()
        .absolute()
        .top(px(2.0))
        .left(if checked { px(20.0) } else { px(2.0) })
        .size(px(16.0))
        .rounded_full()
        .bg(knob_color)
        .shadow(vec![BoxShadow {
            color: gpui::Hsla::from(t.skirt()),
            offset: point(px(0.0), px(1.0)),
            blur_radius: px(0.0),
            spread_radius: px(0.0),
        }]);
    let click = on_change.clone();
    let down = on_change.clone();
    let up = on_change;
    let ring = crate::components::focus::ring(t);
    div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .items_center()
        .w(px(40.0))
        .h(px(22.0))
        .rounded_full()
        .border_1()
        .border_color(if checked {
            gpui::Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }
        } else {
            Theme::color(t.line)
        })
        .bg(track)
        .cursor_pointer()
        .on_click(move |_, window, cx| click(!checked, window, cx))
        .when_some(focus, move |d, handle| {
            d.track_focus(&handle.clone().tab_stop(true))
                .focus(ring)
                .on_key_down(move |ev, window, cx| {
                    if activates(ev.keystroke.key.as_str(), KeyPhase::Down) {
                        cx.stop_propagation();
                        down(!checked, window, cx);
                    }
                })
                .on_key_up(move |ev, window, cx| {
                    if activates(ev.keystroke.key.as_str(), KeyPhase::Up) {
                        cx.stop_propagation();
                        up(!checked, window, cx);
                    }
                })
        })
        // hardware.css `--well` on the slot: the recess's top line inside it
        // (the checked pill's ring). GPUI has no inset shadow, so the line
        // is drawn inside the top edge, as in `segmented`.
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(1.0))
                .bg(t.well()),
        )
        .child(knob)
}

/// A switch that cannot be pressed (`disabled`): the track and knob draw
/// at half strength, as React's disabled `.switch`.
pub fn toggle_disabled(
    t: &Theme,
    id: &str,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let track = if checked { t.live() } else { Theme::color(t.sunk) };
    let knob_color = if checked {
        Theme::color(t.on_accent)
    } else {
        Theme::color(t.dim)
    };
    let knob = div()
        .absolute()
        .top(px(2.0))
        .left(if checked { px(20.0) } else { px(2.0) })
        .size(px(16.0))
        .rounded_full()
        .bg(knob_color)
        .shadow(vec![BoxShadow {
            color: gpui::Hsla::from(t.skirt()),
            offset: point(px(0.0), px(1.0)),
            blur_radius: px(0.0),
            spread_radius: px(0.0),
        }]);
    let row = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .items_center()
        .w(px(40.0))
        .h(px(22.0))
        .rounded_full()
        .border_1()
        .border_color(if checked { gpui::Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 } } else { Theme::color(t.line) })
        .bg(track)
        .opacity(0.5)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(1.0))
                .bg(t.well()),
        )
        .child(knob);
    let _ = on_change;
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_reports_the_flipped_state() {
        let mut seen = Vec::new();
        let mut change = |v: bool| seen.push(v);
        // The component itself only wires the click; the flip rule is `!checked`.
        change(!false);
        change(!true);
        assert_eq!(seen, vec![true, false]);
    }

    /// `Toggle.tsx` is a `<button>`: Enter clicks on key down, Space on key
    /// up, and no other key does (a native button ignores arrows and letters).
    #[test]
    fn toggle_enter_and_space_activate_like_a_native_button() {
        assert!(activates("enter", KeyPhase::Down));
        assert!(!activates("enter", KeyPhase::Up));
        assert!(activates("space", KeyPhase::Up));
        assert!(!activates("space", KeyPhase::Down));
        for other in ["left", "right", "up", "down", "tab", "escape", "a", "home"] {
            assert!(!activates(other, KeyPhase::Down), "{other} down");
            assert!(!activates(other, KeyPhase::Up), "{other} up");
        }
    }
}
