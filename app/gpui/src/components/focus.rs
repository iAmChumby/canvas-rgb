//! Keyboard focus for the skirted controls: the ring, the tab order and the
//! keys that move between them.
//!
//! React leaves all of this to the browser: every `<button>`, `<input>` and
//! `[tabindex]` is in the tab order in document order, Tab and Shift+Tab move
//! through them, and `:focus-visible` draws `outline: 2px solid var(--focus);
//! outline-offset: 3px` (`app/web/src/styles/app.css:95`). GPUI has focus
//! handles and tab stops but no default key bindings, so a view that wants the
//! browser's behaviour owns one [`FocusHandle`] per control, hands it to the
//! component, and routes Tab through [`tab_move`] from its root.

use gpui::{App, BoxShadow, FocusHandle, Hsla, KeyDownEvent, StyleRefinement, Window, point, px, prelude::*};

use crate::theme::Theme;

/// A handle that is in the tab order (`tabIndex` 0).
pub fn tab_handle(cx: &App) -> FocusHandle {
    cx.focus_handle().tab_stop(true)
}

/// Which way Tab moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabMove {
    Next,
    Prev,
}

/// Tab goes to the next control, Shift+Tab to the previous one; nothing else
/// moves between controls (the arrow keys belong to the control in focus).
pub fn tab_move(key: &str, shift: bool) -> Option<TabMove> {
    match (key, shift) {
        ("tab", false) => Some(TabMove::Next),
        ("tab", true) => Some(TabMove::Prev),
        _ => None,
    }
}

/// The root of a page calls this from its `on_key_down`. Returns true when the
/// key moved the focus (and so must not travel further).
pub fn handle_tab(event: &KeyDownEvent, window: &mut Window) -> bool {
    match tab_move(event.keystroke.key.as_str(), event.keystroke.modifiers.shift) {
        Some(TabMove::Next) => {
            window.focus_next();
            true
        }
        Some(TabMove::Prev) => {
            window.focus_prev();
            true
        }
        None => false,
    }
}

/// `:focus-visible`: a 2 px ring in `--focus` hugging the control. CSS keeps a
/// 3 px gap (`outline-offset`); GPUI paints a shadow's spread, not an outline,
/// so the ring sits against the control with no gap.
pub fn ring(t: &Theme) -> impl FnOnce(StyleRefinement) -> StyleRefinement {
    let color: Hsla = Hsla::from(t.live());
    move |s| {
        s.shadow(vec![BoxShadow {
            color,
            offset: point(px(0.0), px(0.0)),
            blur_radius: px(0.0),
            spread_radius: px(2.0),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A browser: Tab forward, Shift+Tab back; every other key stays with the control.
    #[test]
    fn tab_moves_forward_and_shift_tab_back() {
        assert_eq!(tab_move("tab", false), Some(TabMove::Next));
        assert_eq!(tab_move("tab", true), Some(TabMove::Prev));
        for key in ["left", "right", "up", "down", "enter", "space", "escape", "home", "a"] {
            assert_eq!(tab_move(key, false), None, "{key}");
            assert_eq!(tab_move(key, true), None, "shift {key}");
        }
    }
}
