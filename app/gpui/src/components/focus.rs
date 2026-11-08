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

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, BoxShadow, FocusHandle, Hsla, KeyDownEvent, StyleRefinement, Window, point, px, prelude::*};

use crate::theme::Theme;

/// `:focus-visible` (`app.css:95`): the ring shows for keyboard focus only. A
/// press with the pointer focuses a button or a slider and draws no ring
/// (`Slider.tsx:107` says so: "the keyboard keeps working after a mouse press,
/// without claiming the keyboard's focus ring"). The page notes which of the two
/// moved the focus last.
static KEYBOARD_FOCUS: AtomicBool = AtomicBool::new(false);

/// Was the last input a key? The ring draws only then.
pub fn keyboard_focus() -> bool {
    KEYBOARD_FOCUS.load(Ordering::Relaxed)
}

/// Note the modality of an input; returns true when it changed (the page repaints then).
pub fn note_modality(keyboard: bool) -> bool {
    KEYBOARD_FOCUS.swap(keyboard, Ordering::Relaxed) != keyboard
}

thread_local! {
    static BUTTONS_TAKE_FOCUS: Cell<bool> = const { Cell::new(false) };
}

/// While alive, every button the page builds is in the tab order, takes the
/// focus on a press, and fires on Enter and Space (a native `<button>`). A page
/// holds one for the length of its `render`. Pages that have not opted in draw
/// the plain mouse-only buttons they always did.
pub struct ButtonKeys(());

impl ButtonKeys {
    pub fn on() -> Self {
        BUTTONS_TAKE_FOCUS.with(|c| c.set(true));
        ButtonKeys(())
    }
}

impl Drop for ButtonKeys {
    fn drop(&mut self) {
        BUTTONS_TAKE_FOCUS.with(|c| c.set(false));
    }
}

/// Do buttons being built now take the keyboard?
pub fn buttons_take_focus() -> bool {
    BUTTONS_TAKE_FOCUS.with(Cell::get)
}

/// A page with no control in focus takes the focus itself, so the first Tab (a key
/// event reaches only the focused element's ancestors) has somewhere to start. The
/// browser's document does the same: Tab from nothing lands on the first control.
pub fn claim_when_idle(page: &FocusHandle, window: &mut Window, cx: &App) {
    if window.focused(cx).is_none() {
        window.focus(page);
    }
}

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

/// `:focus-visible` (`app.css:95`): a 2 px ring in `--focus`, 3 px off the control
/// (`outline-offset: 3px`). GPUI paints a shadow's spread, not an outline, so the
/// gap is a second shadow in the panel's colour laid over the ring's inner part.
/// A control sitting on another ground shows that ground's colour wrong in the gap.
pub fn ring(t: &Theme) -> impl FnOnce(StyleRefinement) -> StyleRefinement {
    let color: Hsla = Hsla::from(t.live());
    let gap: Hsla = Hsla::from(Theme::color(t.panel));
    // Only a key press draws the ring; after a pointer press the style is empty.
    let visible = keyboard_focus();
    move |s| {
        if !visible {
            return s;
        }
        let around = |color: Hsla, spread: f32| BoxShadow {
            color,
            offset: point(px(0.0), px(0.0)),
            // A zero blur draws nothing on the DirectX shadow shader; a fraction of a pixel
            // reads as a hard edge.
            blur_radius: px(0.5),
            spread_radius: px(spread),
        };
        s.shadow(vec![around(color, RING_GAP + RING_WIDTH), around(gap, RING_GAP)])
    }
}

/// The ring's width and its distance from the control, in px (`app.css:95-96`).
const RING_WIDTH: f32 = 2.0;
const RING_GAP: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// `:focus-visible`: a key press shows the ring, a pointer press hides it, and
    /// the page repaints only when the modality changes.
    #[test]
    fn focus_ring_follows_the_input_modality() {
        note_modality(false);
        assert!(!keyboard_focus());
        assert!(note_modality(true), "a key after a pointer press changes it");
        assert!(keyboard_focus());
        assert!(!note_modality(true), "a second key changes nothing");
        assert!(note_modality(false));
        assert!(!keyboard_focus());
    }

    /// Buttons take the keyboard only while a page holds the guard.
    #[test]
    fn buttons_take_focus_only_inside_a_page_that_opted_in() {
        assert!(!buttons_take_focus());
        {
            let _keys = ButtonKeys::on();
            assert!(buttons_take_focus());
        }
        assert!(!buttons_take_focus(), "the guard ends with the page's render");
    }

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
