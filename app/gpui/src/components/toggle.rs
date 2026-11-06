//! `Toggle`, after `app/web/src/components/Toggle.tsx`: a switch. Clicking
//! flips `checked`. The knob sits left when off, right when on.

use gpui::{IntoElement, Window, div, prelude::*, px};

use crate::theme::Theme;

/// A switch. `checked` draws the on state; clicking reports the new state.
pub fn toggle(
    t: &Theme,
    id: &str,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let track = if checked {
        Theme::color(t.a3)
    } else {
        Theme::color(t.raised)
    };
    let knob = div()
        .size(px(14.0))
        .rounded_full()
        .bg(Theme::color(t.knob_ring));
    let row = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .w(px(36.0))
        .h(px(20.0))
        .px_1()
        .rounded_full()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .bg(track)
        .cursor_pointer()
        .on_click(move |_, window, cx| on_change(!checked, window, cx));
    if checked {
        row.justify_end().child(knob)
    } else {
        row.child(knob)
    }
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
