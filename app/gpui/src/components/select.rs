//! `Select`, after `app/web/src/components/Select.tsx` and the
//! `.select-trigger` / `.listbox` rules in `app.css` (trigger: 32 px,
//! min-width 160, raised face, bold caret; list: panel, line-2 border,
//! `--pop` shadow, 32 px rows).
//!
//! The React control is a combobox with a listbox, arrow keys, type-ahead and
//! an exit animation. GPUI 0.2.2 has no portalled list; this slice draws the
//! trigger and, when open, the option rows under it (click an option to
//! pick). Keyboard, type-ahead and ARIA items in the Displays checklist stay
//! unticked for that reason.

use std::rc::Rc;

use gpui::{FocusHandle, IntoElement, Window, div, prelude::*, px};

use crate::components::slider::Origin;
use crate::theme::{Theme, TokenRadius};

/// What a key did to an open or closed select (`Select.tsx:162-194`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The key is not the select's: it travels on (Tab to the next control).
    Ignored,
    /// The select used the key; it goes no further.
    Handled,
    /// Enter or Space picked this row (a flat index); the list is closed.
    Pick(usize),
}

/// The state a select keeps between frames (`Select.tsx:63-64`, `:90`): whether
/// the list is open, which row is active, and the type-ahead buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavState {
    pub open: bool,
    pub active: usize,
    typed: String,
    typed_at_ms: u64,
}

/// How long a typed prefix stays alive (`Select.tsx:188`, "last half second").
pub const TYPE_AHEAD_MS: u64 = 500;

impl NavState {
    /// `show` (`Select.tsx:94-98`): open on `at` (the chosen row). A disabled select never opens.
    pub fn show(&mut self, at: usize, disabled: bool) {
        if disabled {
            return;
        }
        self.active = at;
        self.open = true;
    }

    /// A click on the trigger (`Select.tsx:209`): closes an open list, opens a closed one.
    pub fn click_trigger(&mut self, chosen: usize, disabled: bool) {
        if self.open {
            self.open = false;
        } else {
            self.show(chosen, disabled);
        }
    }

    /// A pointer down anywhere but the list or the trigger (`Select.tsx:145-154`).
    pub fn dismiss(&mut self) {
        self.open = false;
    }

    /// The pointer moves over row `i` (`Select.tsx:254`): it becomes active.
    pub fn hover(&mut self, i: usize) {
        self.active = i;
    }

    /// A click on row `i` (`Select.tsx:99-106`): closes the list. A list that is
    /// not open (mid-exit) acts on nothing.
    pub fn click_row(&mut self, i: usize) -> Option<usize> {
        if !self.open {
            return None;
        }
        self.open = false;
        Some(i)
    }

    /// `onKey` (`Select.tsx:162-194`). `labels` are the flat rows' labels,
    /// `chosen` the row of the current value, `typed` the printable character
    /// of the key (if any), `now_ms` a monotonic clock.
    pub fn key(
        &mut self,
        key: &str,
        typed: Option<&str>,
        labels: &[&str],
        chosen: usize,
        disabled: bool,
        now_ms: u64,
    ) -> Outcome {
        let last = labels.len().saturating_sub(1);
        if !self.open {
            // A closed select opens on the four keys and ignores the rest
            // (typing included).
            if matches!(key, "down" | "up" | "enter" | "space") {
                self.show(chosen, disabled);
                return Outcome::Handled;
            }
            return Outcome::Ignored;
        }
        match key {
            // Escape closes and is used; Tab closes and goes on to the next control.
            "escape" => {
                self.open = false;
                Outcome::Handled
            }
            "tab" => {
                self.open = false;
                Outcome::Ignored
            }
            "down" => {
                self.active = (self.active + 1).min(last);
                Outcome::Handled
            }
            "up" => {
                self.active = self.active.saturating_sub(1);
                Outcome::Handled
            }
            "home" => {
                self.active = 0;
                Outcome::Handled
            }
            "end" => {
                self.active = last;
                Outcome::Handled
            }
            "enter" | "space" => {
                self.open = false;
                Outcome::Pick(self.active)
            }
            _ => {
                // `e.key.length === 1`: a printable character jumps to the first
                // row that starts with what was typed in the last half second.
                let Some(ch) = typed.filter(|c| c.chars().count() == 1) else {
                    return Outcome::Ignored;
                };
                let fresh = now_ms.saturating_sub(self.typed_at_ms) < TYPE_AHEAD_MS;
                let mut text = if fresh { self.typed.clone() } else { String::new() };
                text.push_str(&ch.to_lowercase());
                self.typed.clone_from(&text);
                self.typed_at_ms = now_ms;
                if let Some(hit) = labels.iter().position(|l| l.to_lowercase().starts_with(&text)) {
                    self.active = hit;
                }
                Outcome::Handled
            }
        }
    }
}

/// The select's keyboard hooks (`Select.tsx:198-211`): the trigger's focus
/// handle, the row the arrows are on, and the callbacks the owning view
/// answers. `trigger` is where the trigger was painted, so a click on it is
/// not also a click "outside" the list.
#[derive(Clone)]
pub struct SelectKb {
    pub focus: FocusHandle,
    pub active: usize,
    pub trigger: Origin,
    pub on_key: Rc<dyn Fn(&gpui::KeyDownEvent, &mut Window, &mut gpui::App)>,
    pub on_hover: Rc<dyn Fn(usize, &mut Window, &mut gpui::App)>,
    pub on_dismiss: Rc<dyn Fn(&mut Window, &mut gpui::App)>,
}

/// The flat row of the current value: `max(0, findIndex)` (`Select.tsx:92`).
pub fn chosen_index(values: &[&str], value: &str) -> usize {
    values.iter().position(|v| *v == value).unwrap_or(0)
}

/// The source trigger and (when `open`) its option list.
#[allow(clippy::too_many_arguments)]
pub fn select(
    t: &Theme,
    id: &str,
    label: &str,
    options: &[(&str, &str)],
    value: &str,
    open: bool,
    full_width: bool,
    on_toggle: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_pick: impl Fn(String, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let _ = label;
    let chosen = options
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(_, l)| *l)
        .unwrap_or("CPU temperature");
    let toggle = on_toggle.clone();
    // React `.select-value`: every option is laid out in the trigger and
    // only the chosen one is visible, so the trigger is as wide as its
    // widest option and picking one never resizes it. GPUI has no
    // `visibility: hidden` that keeps layout, so the labels are stacked in
    // one grid cell (all at row/column 1): the cell takes the widest label,
    // and the non-chosen labels are `invisible` (visibility hidden), exactly
    // as in React. The chosen label is drawn last, on top.
    let mut value_grid = div().grid().flex_1().min_w(px(0.0)).text_left();
    for (v, l) in options {
        let label = div()
            .col_start(1)
            .row_start(1)
            .whitespace_nowrap()
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(l.to_string());
        let label = if *v == value {
            label
        } else {
            label.invisible()
        };
        value_grid = value_grid.child(label);
    }
    let _ = chosen;
    let trigger = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .min_w(px(160.0))
        .h(px(32.0))
        .pl_3()
        .pr(px(10.0))
        .text_color(Theme::color(t.ink))
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY)
        .cursor_pointer()
        // hardware.css `.select-trigger`: a key on its 2 px skirt.
        .child(crate::components::button::skirt_layers(
            t.skirt(),
            10.0,
            32.0,
            Theme::color(t.raised),
            if open {
                t.live()
            } else {
                Theme::color(t.line_2)
            },
        ))
        .child(value_grid)
        .child(
            // React Select.tsx:219: CaretDown size 14 bold.
            gpui::svg()
                .path("icons/CaretDown-bold.svg")
                .size(px(14.0))
                .flex_none()
                .text_color(Theme::color(t.mute)),
        )
        .on_click(move |_, window, cx| toggle(window, cx));
    let trigger = if full_width { trigger.w_full() } else { trigger };
    let mut col = div()
        .id(gpui::SharedString::from(format!("{id}-col")))
        .flex()
        .flex_col()
        .child(trigger);
    // The trigger's full width is relative to this column, so the column fills too
    // (phone: React's trigger spans the card).
    if full_width {
        col = col.w_full();
    }
    if open {
        let mut list = div()
            .id(gpui::SharedString::from(format!("{id}-list")))
            .flex()
            .flex_col()
            .absolute()
            .mt(px(38.0))
            .min_w(px(160.0))
            .p(px(4.0))
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.panel));
        for (v, l) in options {
            let pick = on_pick.clone();
            let v_owned = v.to_string();
            let is_chosen = *v == value;
            let row = div()
                .id(gpui::SharedString::from(format!("{id}-option-{v}")))
                .flex()
                .items_center()
                .justify_between()
                .h(px(32.0))
                .px(px(10.0))
                .r_sm()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .cursor_pointer()
                .child(l.to_string())
                .child(if is_chosen { "✓" } else { "" });
            let row = if is_chosen {
                row.bg(Theme::color(t.raised))
            } else {
                row
            };
            list = list.child(row.on_click(move |_, window, cx| {
                pick(v_owned.clone(), window, cx)
            }));
        }
        col = col.child(list);
    }
    col
}

/// A select whose trigger can be disabled (`Select` with `disabled`): the
/// trigger draws at half strength with no list and no click, exactly as
/// React's disabled `.select-trigger`. `open` is ignored while disabled.
#[allow(clippy::too_many_arguments)]
pub fn select_field(
    t: &Theme,
    id: &str,
    label: &str,
    options: &[(&str, &str)],
    value: &str,
    open: bool,
    disabled: bool,
    full_width: bool,
    on_toggle: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_pick: impl Fn(String, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> gpui::AnyElement {
    if disabled {
        let flat: Vec<(&str, &str)> = options.to_vec();
        return select_grouped(t, id, &[("", &flat)], value, false, true, full_width, on_toggle, on_pick).into_any_element();
    }
    select(t, id, label, options, value, open, full_width, on_toggle, on_pick).into_any_element()
}

/// A grouped select (the audio source picker): `groups` are
/// (heading, options) pairs; an empty heading draws no heading row.
/// `disabled` draws the trigger at half strength with no list.
#[allow(clippy::too_many_arguments)]
pub fn select_grouped(
    t: &Theme,
    id: &str,
    groups: &[(&str, &[(&str, &str)])],
    value: &str,
    open: bool,
    disabled: bool,
    full_width: bool,
    on_toggle: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_pick: impl Fn(String, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    select_grouped_trunc(t, id, groups, value, open, disabled, full_width, false, on_toggle, on_pick)
}

/// [`select_grouped`] whose trigger can truncate its value with an ellipsis
/// (`truncate`, with `full_width`): React's `.select-value` + `.one-line` in a
/// trigger whose width the parent sets (the Audio panel's compact head). A
/// shrink-wrapped trigger keeps the widest label's width.
#[allow(clippy::too_many_arguments)]
pub fn select_grouped_trunc(
    t: &Theme,
    id: &str,
    groups: &[(&str, &[(&str, &str)])],
    value: &str,
    open: bool,
    disabled: bool,
    full_width: bool,
    truncate: bool,
    on_toggle: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_pick: impl Fn(String, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    select_grouped_inner(t, id, groups, value, open, disabled, full_width, truncate, None, on_toggle, on_pick)
}

/// [`select_grouped_trunc`] with the keyboard and the dismiss
/// (`Select.tsx:162-211`): the trigger takes the focus and is in the tab
/// order unless `disabled`, its keys go to `kb.on_key` (feed them to
/// [`NavState::key`]), the active row is drawn, hovering a row makes it
/// active, and a click outside the list and trigger dismisses it.
#[allow(clippy::too_many_arguments)]
pub fn select_grouped_kb(
    t: &Theme,
    id: &str,
    groups: &[(&str, &[(&str, &str)])],
    value: &str,
    open: bool,
    disabled: bool,
    full_width: bool,
    truncate: bool,
    kb: SelectKb,
    on_toggle: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_pick: impl Fn(String, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    select_grouped_inner(t, id, groups, value, open, disabled, full_width, truncate, Some(kb), on_toggle, on_pick)
}

#[allow(clippy::too_many_arguments)]
fn select_grouped_inner(
    t: &Theme,
    id: &str,
    groups: &[(&str, &[(&str, &str)])],
    value: &str,
    open: bool,
    disabled: bool,
    full_width: bool,
    truncate: bool,
    kb: Option<SelectKb>,
    on_toggle: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_pick: impl Fn(String, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let flat: Vec<(&str, &str)> = groups.iter().flat_map(|(_, opts)| opts.iter().copied()).collect();
    let mut value_grid = div().grid().flex_1().min_w(px(0.0)).text_left();
    for (v, l) in &flat {
        let label = div()
            .col_start(1)
            .row_start(1)
            .whitespace_nowrap()
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(l.to_string());
        let label = if *v == value { label } else { label.invisible() };
        value_grid = value_grid.child(label);
    }
    // React `.select-value` + `.one-line`: in a trigger of known width a value
    // wider than the room truncates with an ellipsis. The stacked grid above
    // only exists to give a shrink-wrapped trigger the widest label's width, so
    // a trigger of known width shows just the picked label. No `whitespace_nowrap`:
    // gpui only measures a truncation width for wrapping text and caches the
    // first (untruncated) measure of a nowrap one.
    let value_el = if full_width && truncate {
        let picked = flat.iter().find(|(v, _)| *v == value).map(|(_, l)| l.to_string()).unwrap_or_default();
        div()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .text_ellipsis()
            .text_left()
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(picked)
            .into_any_element()
    } else {
        value_grid.into_any_element()
    };
    let trigger = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .min_w(px(160.0))
        .h(px(32.0))
        // React `.select-trigger`: `padding: 0 10px 0 12px` inside a 1 px
        // border; the skirt layers draw the border here, so the border's
        // pixel is added to each side (the text sat 1 px left and the
        // trigger 2 px narrow against React, #205).
        .pl(px(13.0))
        .pr(px(11.0))
        .text_color(Theme::color(t.ink))
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .font_family(crate::fonts::UI_FAMILY)
        .child(crate::components::button::skirt_layers(
            t.skirt(),
            10.0,
            32.0,
            Theme::color(t.raised),
            if open { t.live() } else { Theme::color(t.line_2) },
        ))
        .child(value_el)
        .child(
            gpui::svg()
                .path("icons/CaretDown-bold.svg")
                .size(px(14.0))
                .flex_none()
                .text_color(Theme::color(t.mute)),
        );
    let trigger = if disabled {
        trigger.opacity(0.5)
    } else {
        let toggle = on_toggle.clone();
        trigger.cursor_pointer().on_click(move |_, window, cx| toggle(window, cx))
    };
    // The keyboard: focus on the trigger (a disabled `<button>` is out of the
    // tab order), the keys to the owner, and the trigger's place measured.
    let trigger = match (&kb, disabled) {
        (Some(kb), false) => {
            let on_key = kb.on_key.clone();
            let measure = kb.trigger.clone();
            let ring = crate::components::focus::ring(t);
            trigger
                .track_focus(&kb.focus.clone().tab_stop(true))
                .focus(ring)
                .on_key_down(move |ev, window, cx| on_key(ev, window, cx))
                .child(
                    gpui::canvas(
                        move |bounds, _w, _cx| {
                            measure.set(
                                f32::from(bounds.origin.x),
                                f32::from(bounds.origin.y),
                                f32::from(bounds.size.width),
                                f32::from(bounds.size.height),
                            );
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
        }
        _ => trigger,
    };
    let trigger = if full_width { trigger.w_full() } else { trigger };
    let mut col = div()
        .id(gpui::SharedString::from(format!("{id}-col")))
        .flex()
        .flex_col()
        .child(trigger);
    if full_width {
        col = col.w_full();
    }
    if open && !disabled {
        let mut list = div()
            .id(gpui::SharedString::from(format!("{id}-list")))
            .flex()
            .flex_col()
            .absolute()
            .mt(px(38.0))
            .min_w(px(160.0))
            .p(px(4.0))
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.panel));
        if let Some(kb) = &kb {
            // `Select.tsx:145-154`: a pointer down anywhere but the list or the
            // trigger closes the list.
            let dismiss = kb.on_dismiss.clone();
            let trigger_at = kb.trigger.clone();
            list = list.on_mouse_down_out(move |ev, window, cx| {
                if !trigger_hit(trigger_at.get(), f32::from(ev.position.x), f32::from(ev.position.y)) {
                    dismiss(window, cx);
                }
            });
        }
        let mut flat_i = 0usize;
        for (heading, opts) in groups {
            if !heading.is_empty() {
                let heading_el = div()
                    .px(px(10.0))
                    .pt(px(6.0))
                    .pb(px(2.0))
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(heading.to_string());
                list = list.child(heading_el);
            }
            for (v, l) in *opts {
                let pick = on_pick.clone();
                let v_owned = v.to_string();
                let is_chosen = *v == value;
                let i = flat_i;
                flat_i += 1;
                let is_active = kb.as_ref().is_some_and(|k| k.active == i);
                let row = div()
                    .id(gpui::SharedString::from(format!("{id}-option-{v}")))
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(32.0))
                    .px(px(10.0))
                    .r_sm()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .cursor_pointer()
                    .child(l.to_string())
                    .child(if is_chosen { "✓" } else { "" });
                // `.listbox-opt[data-active]` is `--press`; without the keyboard
                // hooks the chosen row keeps its raised face.
                let row = if kb.is_some() {
                    if is_active { row.bg(Theme::color(t.press)) } else { row }
                } else if is_chosen {
                    row.bg(Theme::color(t.raised))
                } else {
                    row
                };
                let row = match &kb {
                    Some(kb) => {
                        let hover = kb.on_hover.clone();
                        row.on_mouse_move(move |_, window, cx| hover(i, window, cx))
                    }
                    None => row,
                };
                list = list.child(row.on_click(move |_, window, cx| {
                    pick(v_owned.clone(), window, cx)
                }));
            }
        }
        col = col.child(list);
    }
    col
}

/// Is the point inside the trigger's painted box (`x, y, w, h`)? A pointer
/// down on the trigger is not "outside" the list.
pub fn trigger_hit(bounds: (f32, f32, f32, f32), px_: f32, py_: f32) -> bool {
    let (x, y, w, h) = bounds;
    px_ >= x && px_ <= x + w && py_ >= y && py_ <= y + h
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS: [&str; 5] = ["Follow Windows default output", "Speakers", "Headphones", "Line in", "Microphone"];

    fn open_at(active: usize) -> NavState {
        NavState { open: true, active, ..NavState::default() }
    }

    /// `Select.tsx:164-168`: a closed select opens on ArrowDown, ArrowUp, Enter
    /// and Space, on the row of the current value.
    #[test]
    fn select_opens_on_the_four_keys_at_the_chosen_row() {
        for key in ["down", "up", "enter", "space"] {
            let mut nav = NavState::default();
            assert_eq!(nav.key(key, None, &LABELS, 2, false, 0), Outcome::Handled, "{key}");
            assert!(nav.open, "{key} opens");
            assert_eq!(nav.active, 2, "{key} lands on the chosen row");
        }
    }

    /// The rest of the keys do nothing while closed, typing included, and
    /// Tab goes on.
    #[test]
    fn select_closed_ignores_other_keys() {
        for key in ["escape", "tab", "home", "end", "left", "right", "a"] {
            let mut nav = NavState::default();
            assert_eq!(nav.key(key, Some("a"), &LABELS, 1, false, 0), Outcome::Ignored, "{key}");
            assert!(!nav.open, "{key} leaves it closed");
        }
    }

    /// A disabled select never opens (`show` returns early, `Select.tsx:95`).
    #[test]
    fn select_disabled_does_not_open() {
        let mut nav = NavState::default();
        nav.key("enter", None, &LABELS, 0, true, 0);
        assert!(!nav.open);
        nav.click_trigger(0, true);
        assert!(!nav.open);
    }

    /// `Select.tsx:174-179`: the arrows move the active row by one and stop at the
    /// ends (no wrap).
    #[test]
    fn select_arrows_move_and_stop_at_the_ends() {
        let mut nav = open_at(0);
        nav.key("up", None, &LABELS, 0, false, 0);
        assert_eq!(nav.active, 0, "up stops at the first row");
        nav.key("down", None, &LABELS, 0, false, 0);
        assert_eq!(nav.active, 1);
        let mut nav = open_at(4);
        nav.key("down", None, &LABELS, 0, false, 0);
        assert_eq!(nav.active, 4, "down stops at the last row");
        nav.key("up", None, &LABELS, 0, false, 0);
        assert_eq!(nav.active, 3);
    }

    #[test]
    fn select_home_and_end_jump_to_the_ends() {
        let mut nav = open_at(2);
        nav.key("end", None, &LABELS, 0, false, 0);
        assert_eq!(nav.active, 4);
        nav.key("home", None, &LABELS, 0, false, 0);
        assert_eq!(nav.active, 0);
        assert!(nav.open, "the keys move, they do not close");
    }

    /// `Select.tsx:183-185`: Enter and Space pick the active row and close the list.
    #[test]
    fn select_enter_and_space_pick_the_active_row() {
        for key in ["enter", "space"] {
            let mut nav = open_at(3);
            assert_eq!(nav.key(key, None, &LABELS, 0, false, 0), Outcome::Pick(3), "{key}");
            assert!(!nav.open, "{key} closes");
        }
    }

    /// `Select.tsx:171-173`: Escape closes and is used; Tab closes and goes on
    /// (so focus moves to the next control).
    #[test]
    fn select_escape_closes_and_tab_closes_and_travels_on() {
        let mut nav = open_at(2);
        assert_eq!(nav.key("escape", None, &LABELS, 0, false, 0), Outcome::Handled);
        assert!(!nav.open);
        let mut nav = open_at(2);
        assert_eq!(nav.key("tab", None, &LABELS, 0, false, 0), Outcome::Ignored);
        assert!(!nav.open);
    }

    /// `Select.tsx:186-193`: a typed letter jumps to the first row starting with it.
    #[test]
    fn select_type_ahead_jumps_to_the_first_match() {
        let mut nav = open_at(0);
        assert_eq!(nav.key("h", Some("h"), &LABELS, 0, false, 1000), Outcome::Handled);
        assert_eq!(nav.active, 2, "h finds Headphones");
        // A letter nothing starts with leaves the active row where it is.
        nav.key("z", Some("z"), &LABELS, 0, false, 5000);
        assert_eq!(nav.active, 2);
    }

    /// Within half a second the letters build one prefix; after it the prefix
    /// starts again (`Select.tsx:188`, `now - typed.at < 500`).
    #[test]
    fn select_type_ahead_prefix_lasts_half_a_second() {
        let mut nav = open_at(0);
        nav.key("l", Some("l"), &LABELS, 0, false, 1000);
        assert_eq!(nav.active, 3, "l finds Line in");
        nav.key("i", Some("i"), &LABELS, 0, false, 1400);
        assert_eq!(nav.active, 3, "li still finds Line in");
        nav.key("s", Some("s"), &LABELS, 0, false, 1800);
        assert_eq!(nav.active, 3, "lis matches nothing, the row stays");
        // 500 ms or more later "s" starts a new prefix and finds Speakers.
        nav.key("s", Some("s"), &LABELS, 0, false, 2300);
        assert_eq!(nav.active, 1);
    }

    /// Type-ahead compares in lower case (`toLowerCase` on both sides).
    #[test]
    fn select_type_ahead_ignores_case() {
        let mut nav = open_at(0);
        nav.key("m", Some("M"), &LABELS, 0, false, 0);
        assert_eq!(nav.active, 4);
    }

    /// A key that is not a single character and not one of the select's keys
    /// goes on.
    #[test]
    fn select_open_other_keys_travel_on() {
        let mut nav = open_at(1);
        assert_eq!(nav.key("f5", None, &LABELS, 0, false, 0), Outcome::Ignored);
        assert_eq!(nav.key("left", None, &LABELS, 0, false, 0), Outcome::Ignored);
        assert!(nav.open);
        assert_eq!(nav.active, 1);
    }

    /// `Select.tsx:209`: a click on the trigger closes an open list and opens a closed one.
    #[test]
    fn select_trigger_click_toggles() {
        let mut nav = NavState::default();
        nav.click_trigger(2, false);
        assert!(nav.open);
        assert_eq!(nav.active, 2);
        nav.click_trigger(2, false);
        assert!(!nav.open);
    }

    /// `Select.tsx:145-154`: a pointer down anywhere else closes the list.
    #[test]
    fn select_outside_pointer_down_dismisses() {
        let mut nav = open_at(1);
        nav.dismiss();
        assert!(!nav.open);
    }

    /// The pointer-down "outside" test spares the trigger's own box.
    #[test]
    fn select_trigger_press_is_not_outside() {
        let b = (100.0, 40.0, 160.0, 32.0);
        assert!(trigger_hit(b, 120.0, 50.0));
        assert!(trigger_hit(b, 100.0, 40.0), "the corner counts");
        assert!(!trigger_hit(b, 99.0, 50.0));
        assert!(!trigger_hit(b, 120.0, 73.0));
    }

    /// `Select.tsx:254-256`: hovering a row makes it active; a click picks it
    /// and closes the list; a list that is not open acts on nothing (`:102`).
    #[test]
    fn select_hover_activates_and_click_picks_only_when_open() {
        let mut nav = open_at(0);
        nav.hover(3);
        assert_eq!(nav.active, 3);
        assert_eq!(nav.click_row(3), Some(3));
        assert!(!nav.open);
        assert_eq!(nav.click_row(2), None, "a closed list picks nothing");
    }

    /// `max(0, findIndex)`: a value that matches no row reads as the first.
    #[test]
    fn select_chosen_index_falls_back_to_the_first_row() {
        assert_eq!(chosen_index(&["a", "b", "c"], "c"), 2);
        assert_eq!(chosen_index(&["a", "b", "c"], "zz"), 0);
    }

    #[test]
    fn a_value_outside_the_options_reads_as_the_first_option() {
        let options = [("cpu_temp", "CPU temperature"), ("fixed", "A fixed number")];
        let chosen = options
            .iter()
            .find(|(v, _)| *v == "board:temp0")
            .map(|(_, l)| *l)
            .unwrap_or("CPU temperature");
        assert_eq!(chosen, "CPU temperature");
    }
}
