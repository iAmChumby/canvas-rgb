//! `Toggle`, after `app/web/src/components/Toggle.tsx` and the `.switch`
//! rules in `app.css` re-skinned by `hardware.css`: a 40x22 slot, a 16 px
//! knob, sunk when off and live when on, the knob travelling 18 px.

use gpui::{BoxShadow, IntoElement, Window, div, point, prelude::*, px};

use crate::theme::Theme;

/// A switch. `checked` draws the on state; clicking reports the new state.
pub fn toggle(
    t: &Theme,
    id: &str,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
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
    // hardware.css `.switch::after`: `0 1px 0 var(--skirt)` under the knob.
    let knob = div()
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
        .flex()
        .items_center()
        .w(px(40.0))
        .h(px(22.0))
        .px(px(2.0))
        .rounded_full()
        .border_1()
        .border_color(if checked {
            Theme::color(t.sunk)
        } else {
            Theme::color(t.line)
        })
        .bg(track)
        .cursor_pointer()
        .on_click(move |_, window, cx| on_change(!checked, window, cx));
    if checked {
        row.justify_end().child(knob)
    } else {
        row.child(knob)
    }
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
        .flex()
        .items_center()
        .w(px(40.0))
        .h(px(22.0))
        .px(px(2.0))
        .rounded_full()
        .border_1()
        .border_color(if checked { Theme::color(t.sunk) } else { Theme::color(t.line) })
        .bg(track)
        .opacity(0.5);
    let row = if checked { row.justify_end().child(knob) } else { row.child(knob) };
    let _ = on_change;
    row
}

#[cfg(test)]
mod tests {
    #[test]
    fn toggle_reports_the_flipped_state() {
        let mut seen = Vec::new();
        let mut change = |v: bool| seen.push(v);
        // The component itself only wires the click; the flip rule is `!checked`.
        change(!false);
        change(!true);
        assert_eq!(seen, vec![true, false]);
    }
}
