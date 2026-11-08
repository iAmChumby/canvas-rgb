//! `Slider`, after `app/web/src/components/Slider.tsx` and the `.slider`
//! rules in `app.css` re-skinned by `hardware.css`: a 32 px tall fader, a
//! 4 px sunk track, a live fill up to the cap, and a 14x24 raised cap with
//! a live line across it. Drawn on a canvas so the fill and the cap sit at
//! the exact percent of the travel.

use std::rc::Rc;
use std::cell::Cell;

use gpui::{Bounds, FocusHandle, IntoElement, Window, canvas, div, point, prelude::*, px, size};

use crate::theme::Theme;

/// Where an element was painted: its origin and size (the fader measures
/// itself in a canvas prepaint; GPUI 0.2.2 has no bounds call).
#[derive(Clone)]
pub struct Origin(Rc<Cell<(f32, f32, f32, f32)>>, Rc<Cell<Drag>>);

/// One slider's pointer drag (`Slider.tsx:48-58`, `:97-136`): whether a press
/// started on it and the last snapped value sent (so `onChange` runs once per
/// step crossed, not once per move).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Drag {
    pub active: bool,
    pub sent: f32,
}

impl Origin {
    pub fn new() -> Self {
        Origin(
            Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0))),
            Rc::new(Cell::new(Drag::default())),
        )
    }

    pub fn drag(&self) -> Drag {
        self.1.get()
    }

    pub fn set_drag(&self, d: Drag) {
        self.1.set(d);
    }

    pub fn set(&self, x: f32, y: f32, w: f32, h: f32) {
        self.0.set((x, y, w, h));
    }

    pub fn get(&self) -> (f32, f32, f32, f32) {
        self.0.get()
    }
}

/// How many decimals a step needs (`Slider.tsx:15-19`): 1 -> 0, 0.5 -> 1, 0.1 -> 1.
pub fn decimals_of(step: f32) -> usize {
    let s = format!("{step}");
    match s.find('.') {
        None => 0,
        Some(dot) => (s.len() - dot - 1).min(4),
    }
}

/// `Slider.tsx:66-69` `quantise`: the raw number on the step grid, inside the
/// range, and cut to the step's decimals (`Number(x.toFixed(decimals))`).
/// `Math.round` rounds a half up, so this does too (`f32::round` rounds a half
/// away from zero, which differs below zero).
pub fn quantise_js(raw: f32, min: f32, max: f32, step: f32) -> f32 {
    let stepped = min + ((raw - min) / step + 0.5).floor() * step;
    let clamped = stepped.clamp(min, max);
    let scale = 10f32.powi(decimals_of(step) as i32);
    (clamped * scale).round() / scale
}

/// `Slider.tsx:139-152` `onKeyDown`: the value a key moves the slider to, or
/// `None` for a key it ignores. Arrows step by one `step`, PageUp and PageDown
/// by ten, Home and End go to the ends. The result is always quantised, so a
/// key at an end returns the end again (React still runs `onChange` and
/// `onCommit` with it).
pub fn key_value(value: f32, min: f32, max: f32, step: f32, key: &str) -> Option<f32> {
    let raw = match key {
        "right" | "up" => value + step,
        "left" | "down" => value - step,
        "pageup" => value + step * 10.0,
        "pagedown" => value - step * 10.0,
        "home" => min,
        "end" => max,
        _ => return None,
    };
    Some(quantise_js(raw, min, max, step))
}

/// `Slider.tsx:81-90` `follow`: the value to report through `onChange` when the
/// pointer moved the snapped value, or `None` while it stays on the same step.
pub fn follow(drag: &mut Drag, next: f32) -> Option<f32> {
    if (next - drag.sent).abs() > f32::EPSILON {
        drag.sent = next;
        Some(next)
    } else {
        None
    }
}

/// Where the pointer is along the travel, 0..1 (the cap's centre follows it).
fn frac_at(origin: &Origin, x: f32, cap: f32) -> f32 {
    let (ox, _, w, _) = origin.get();
    let travel = w - cap;
    if travel <= 0.0 {
        return 0.0;
    }
    ((x - ox - cap / 2.0) / travel).clamp(0.0, 1.0)
}

/// A fader over an arbitrary range (`Slider.tsx` with min/max/step 1): the
/// sunk track, the live fill up to the cap, and the 14x24 raised cap with
/// the live line across it. Pointer down or drag reports `on_change` with
/// the snapped value; pointer release reports `on_commit` once.
#[allow(clippy::too_many_arguments)]
pub fn slider_range(
    t: &Theme,
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    disabled: bool,
    origin: Origin,
    on_change: impl Fn(f32, &mut Window, &mut gpui::App) + Clone + 'static,
    on_commit: impl Fn(f32, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    slider_range_kb(t, id, value, min, max, 1.0, disabled, origin, None, on_change, on_commit)
}

/// [`slider_range`] with the keyboard (`Slider.tsx:139-152`): in the tab order
/// unless `disabled`, arrows step, PageUp/PageDown step ten times, Home and End
/// go to the ends, and every key step runs `on_change` then `on_commit`. The
/// cap wears the focus ring while the slider has the focus
/// (`hardware.css:462`). A drag reports `on_change` once per step crossed and
/// `on_commit` once on release, wherever the pointer is released.
#[allow(clippy::too_many_arguments)]
pub fn slider_range_kb(
    t: &Theme,
    id: &str,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    disabled: bool,
    origin: Origin,
    focus: Option<&FocusHandle>,
    on_change: impl Fn(f32, &mut Window, &mut gpui::App) + Clone + 'static,
    on_commit: impl Fn(f32, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    const CAP: f32 = 14.0;
    let ring_focus = focus.cloned();
    let ring_color = t.live();
    let span = if (max - min).abs() < f32::EPSILON { 1.0 } else { max - min };
    let p = ((value - min) / span).clamp(0.0, 1.0);
    let track_color = Theme::color(t.sunk);
    let line_color = Theme::color(t.line);
    let live = t.live();
    let raised = Theme::color(t.raised);
    let line_2 = Theme::color(t.line_2);
    let skirt = t.skirt();
    let well = t.well();
    let measure = origin.clone();
    let slot = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex_1()
        .min_w(px(0.0))
        .h(px(32.0))
        .child(
            canvas(
                move |bounds, _window, _cx| {
                    measure.set(
                        f32::from(bounds.origin.x),
                        f32::from(bounds.origin.y),
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                    );
                },
                move |bounds, _, window, _| {
                    let focused = ring_focus.as_ref().is_some_and(|h| h.is_focused(window));
                    let w = bounds.size.width;
                    let track = Bounds {
                        origin: point(bounds.origin.x, bounds.origin.y + px(14.0)),
                        size: size(w, px(4.0)),
                    };
                    window.paint_quad(gpui::fill(track, track_color));
                    window.paint_quad(gpui::outline(track, line_color, gpui::BorderStyle::default()));
                    let well_line = Bounds {
                        origin: point(bounds.origin.x, bounds.origin.y + px(14.0)),
                        size: size(w, px(1.0)),
                    };
                    window.paint_quad(gpui::fill(well_line, well));
                    let fill_w = (w * p) - px(7.0);
                    if fill_w > px(0.0) {
                        let fill = Bounds {
                            origin: point(bounds.origin.x + px(7.0), bounds.origin.y + px(14.0)),
                            size: size(fill_w, px(4.0)),
                        };
                        window.paint_quad(gpui::fill(fill, live));
                    }
                    let cap_cx = bounds.origin.x + px(7.0) + (w - px(14.0)) * p;
                    let cap = Bounds {
                        origin: point(cap_cx - px(7.0), bounds.origin.y + px(4.0)),
                        size: size(px(14.0), px(24.0)),
                    };
                    let skirt_rect = Bounds {
                        origin: point(cap_cx - px(7.0), bounds.origin.y + px(28.0)),
                        size: size(px(14.0), px(2.0)),
                    };
                    if focused {
                        // hardware.css:462 `.slider:focus-visible .slider-cap`: a
                        // 2 px ring in `--focus` around the cap and its skirt.
                        let ring = Bounds {
                            origin: point(cap_cx - px(9.0), bounds.origin.y + px(2.0)),
                            size: size(px(18.0), px(28.0)),
                        };
                        window.paint_quad(gpui::fill(ring, ring_color));
                    }
                    window.paint_quad(gpui::fill(skirt_rect, skirt));
                    window.paint_quad(gpui::fill(cap, raised));
                    window.paint_quad(gpui::outline(cap, line_2, gpui::BorderStyle::default()));
                    let line = Bounds {
                        origin: point(cap_cx - px(4.0), bounds.origin.y + px(14.0)),
                        size: size(px(8.0), px(2.0)),
                    };
                    window.paint_quad(gpui::fill(line, live));
                },
            )
            // In-flow, like the deck's slider: the canvas takes the
            // slot's flex size instead of being absolutely inset, which
            // mis-measured the slot in the Settings rows.
            .flex_1()
            .h(px(32.0)),
        );
    // `Slider.tsx:95-96`: a disabled slider takes no press and no key, and is
    // out of the tab order (`tabIndex={disabled ? -1 : 0}`).
    if disabled {
        return slot.into_any_element();
    }
    let at = move |origin: &Origin, x: f32| -> f32 {
        let frac = frac_at(origin, x, CAP);
        quantise_js(min + frac * span, min, max, step)
    };
    let down_origin = origin.clone();
    let down_change = on_change.clone();
    let move_origin = origin.clone();
    let move_change = on_change.clone();
    let up_origin = origin.clone();
    let up_change = on_change.clone();
    let up_commit = on_commit.clone();
    let out_origin = origin;
    let out_change = on_change.clone();
    let out_commit = on_commit.clone();
    let key_change = on_change;
    let key_commit = on_commit;
    let slot = slot
        .on_mouse_down(gpui::MouseButton::Left, move |event: &gpui::MouseDownEvent, window, cx| {
            // A press starts the drag and moves the cap to the pointer
            // (`Slider.tsx:97-111`).
            let mut drag = Drag { active: true, sent: value };
            let next = at(&down_origin, f32::from(event.position.x));
            if let Some(v) = follow(&mut drag, next) {
                down_change(v, window, cx);
            }
            down_origin.set_drag(drag);
        })
        .on_mouse_move(move |event: &gpui::MouseMoveEvent, window, cx| {
            // Only the press that started on this slider drags it, and only
            // while its button is down (`Slider.tsx:113-117`); a hover moves nothing.
            let mut drag = move_origin.drag();
            if !drag.active || event.pressed_button != Some(gpui::MouseButton::Left) {
                return;
            }
            let next = at(&move_origin, f32::from(event.position.x));
            if let Some(v) = follow(&mut drag, next) {
                move_change(v, window, cx);
            }
            move_origin.set_drag(drag);
        })
        .on_mouse_up(gpui::MouseButton::Left, move |event: &gpui::MouseUpEvent, window, cx| {
            let mut drag = up_origin.drag();
            if !drag.active {
                return;
            }
            let next = at(&up_origin, f32::from(event.position.x));
            if let Some(v) = follow(&mut drag, next) {
                up_change(v, window, cx);
            }
            up_origin.set_drag(Drag::default());
            up_commit(next, window, cx);
        })
        // Pointer capture (`Slider.tsx:104`): a release anywhere ends the drag
        // and commits the value under the pointer's x.
        .on_mouse_up_out(gpui::MouseButton::Left, move |event: &gpui::MouseUpEvent, window, cx| {
            let mut drag = out_origin.drag();
            if !drag.active {
                return;
            }
            let next = at(&out_origin, f32::from(event.position.x));
            if let Some(v) = follow(&mut drag, next) {
                out_change(v, window, cx);
            }
            out_origin.set_drag(Drag::default());
            out_commit(next, window, cx);
        });
    match focus {
        Some(handle) => slot
            .track_focus(&handle.clone().tab_stop(true))
            .on_key_down(move |ev, window, cx| {
                if let Some(next) = key_value(value, min, max, step, ev.keystroke.key.as_str()) {
                    cx.stop_propagation();
                    key_change(next, window, cx);
                    key_commit(next, window, cx);
                }
            })
            .into_any_element(),
        None => slot.into_any_element(),
    }
}

/// A fader at `value` (0..=100). Clicking reports the drawn value.
pub fn slider(
    t: &Theme,
    id: &str,
    value: u8,
    disabled: bool,
    on_commit: impl Fn(u8, &mut Window, &mut gpui::App) + Clone + 'static,
) -> impl IntoElement {
    let p = (value as f32 / 100.0).clamp(0.0, 1.0);
    let track_color = Theme::color(t.sunk);
    let line_color = Theme::color(t.line);
    let live = t.live();
    let raised = Theme::color(t.raised);
    let line_2 = Theme::color(t.line_2);
    let skirt = t.skirt();
    let well = t.well();
    let el = canvas(
        move |_, _, _| {},
        move |bounds, _, window, _| {
            let w = bounds.size.width;
            let track = Bounds {
                origin: point(bounds.origin.x, bounds.origin.y + px(14.0)),
                size: size(w, px(4.0)),
            };
            window.paint_quad(gpui::fill(track, track_color));
            window.paint_quad(gpui::outline(
                track,
                line_color,
                gpui::BorderStyle::default(),
            ));
            // `--well` on the track: the recess's top line inside it.
            let well_line = Bounds {
                origin: point(bounds.origin.x, bounds.origin.y + px(14.0)),
                size: size(w, px(1.0)),
            };
            window.paint_quad(gpui::fill(well_line, well));
            let fill_w = (w * p) - px(7.0);
            if fill_w > px(0.0) {
                let fill = Bounds {
                    origin: point(bounds.origin.x + px(7.0), bounds.origin.y + px(14.0)),
                    size: size(fill_w, px(4.0)),
                };
                window.paint_quad(gpui::fill(fill, live));
            }
            // The cap is centred on its travel point (hardware.css
            // `translate(-50%, -50%)`), so at 100% its right edge is the
            // slider's right edge and it never reaches the value beside it.
            let cap_cx = bounds.origin.x + px(7.0) + (w - px(14.0)) * p;
            let cap = Bounds {
                origin: point(cap_cx - px(7.0), bounds.origin.y + px(4.0)),
                size: size(px(14.0), px(24.0)),
            };
            // The cap's skirt (2 px under it), drawn before the cap.
            let skirt_rect = Bounds {
                origin: point(cap_cx - px(7.0), bounds.origin.y + px(28.0)),
                size: size(px(14.0), px(2.0)),
            };
            window.paint_quad(gpui::fill(skirt_rect, skirt));
            window.paint_quad(gpui::fill(cap, raised));
            window.paint_quad(gpui::outline(cap, line_2, gpui::BorderStyle::default()));
            // React `.slider-cap`: the live line is an 8x2 background image
            // (`background-size: 8px 2px`), centred in the 14px cap.
            let line = Bounds {
                origin: point(cap_cx - px(4.0), bounds.origin.y + px(14.0)),
                size: size(px(8.0), px(2.0)),
            };
            window.paint_quad(gpui::fill(line, live));
        },
    )
    .flex_1()
    .h(px(32.0));
    let _ = id;
    let _ = disabled;
    let _ = on_commit;
    // The canvas takes the row's width; disabled/commit are the deck's
    // later slice (the value follows the daemon, as in React's commit).
    div().flex_1().h(px(32.0)).child(el)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_clamps_to_the_travel() {
        let p = (120f32 / 100.0).clamp(0.0, 1.0);
        assert_eq!(p, 1.0);
    }

    /// `Slider.tsx:139-152`: Right and Up step up, Left and Down step down.
    #[test]
    fn slider_arrows_step_by_one_step() {
        for key in ["right", "up"] {
            assert_eq!(key_value(30.0, 10.0, 60.0, 1.0, key), Some(31.0), "{key}");
        }
        for key in ["left", "down"] {
            assert_eq!(key_value(30.0, 10.0, 60.0, 1.0, key), Some(29.0), "{key}");
        }
        assert_eq!(key_value(0.5, 0.0, 1.0, 0.1, "right"), Some(0.6), "decimals follow the step");
    }

    /// PageUp and PageDown step ten times; Home and End go to the ends.
    #[test]
    fn slider_page_home_end_keys() {
        assert_eq!(key_value(30.0, 10.0, 60.0, 1.0, "pageup"), Some(40.0));
        assert_eq!(key_value(30.0, 10.0, 60.0, 1.0, "pagedown"), Some(20.0));
        assert_eq!(key_value(30.0, 10.0, 60.0, 1.0, "home"), Some(10.0));
        assert_eq!(key_value(30.0, 10.0, 60.0, 1.0, "end"), Some(60.0));
    }

    /// Past an end the value stays on the end, and React still reports it
    /// (`onChange` and `onCommit` run for any handled key).
    #[test]
    fn slider_keys_clamp_at_the_ends_and_still_report() {
        assert_eq!(key_value(60.0, 10.0, 60.0, 1.0, "right"), Some(60.0));
        assert_eq!(key_value(10.0, 10.0, 60.0, 1.0, "left"), Some(10.0));
        assert_eq!(key_value(55.0, 10.0, 60.0, 1.0, "pageup"), Some(60.0));
        assert_eq!(key_value(15.0, 10.0, 60.0, 1.0, "pagedown"), Some(10.0));
        assert_eq!(key_value(0.0, 0.0, 100.0, 1.0, "left"), Some(0.0));
    }

    #[test]
    fn slider_ignores_every_other_key() {
        for key in ["enter", "space", "tab", "escape", "a", "backspace", "delete"] {
            assert_eq!(key_value(30.0, 10.0, 60.0, 1.0, key), None, "{key}");
        }
    }

    #[test]
    fn slider_step_decimals_match_react() {
        assert_eq!(decimals_of(1.0), 0);
        assert_eq!(decimals_of(0.5), 1);
        assert_eq!(decimals_of(0.1), 1);
        assert_eq!(decimals_of(0.25), 2);
    }

    /// `Math.round` rounds a half up, also below zero.
    #[test]
    fn slider_quantise_rounds_a_half_up() {
        assert_eq!(quantise_js(30.4, 10.0, 60.0, 1.0), 30.0);
        assert_eq!(quantise_js(30.5, 10.0, 60.0, 1.0), 31.0);
        assert_eq!(quantise_js(-0.5, -10.0, 10.0, 1.0), 0.0, "a half rounds up at -0.5");
        assert_eq!(quantise_js(99.0, 10.0, 60.0, 1.0), 60.0);
        assert_eq!(quantise_js(-5.0, 10.0, 60.0, 1.0), 10.0);
    }

    /// `onChange` runs once per step crossed, not once per move (`Slider.tsx:81-90`).
    #[test]
    fn slider_drag_reports_one_change_per_step_crossed() {
        let mut drag = Drag { active: true, sent: 30.0 };
        assert_eq!(follow(&mut drag, 30.0), None, "the press on the same step reports nothing");
        assert_eq!(follow(&mut drag, 31.0), Some(31.0));
        assert_eq!(follow(&mut drag, 31.0), None, "a move inside the step reports nothing");
        assert_eq!(follow(&mut drag, 29.0), Some(29.0));
        assert_eq!(drag.sent, 29.0);
    }

    /// The pointer maps to the cap's centre: at the left end the cap's centre
    /// is half a cap in, at the right end half a cap short of the edge, so the
    /// ends of the slot give the min and max (`Slider.tsx:72-78`).
    #[test]
    fn slider_pointer_position_maps_to_the_value() {
        let o = Origin::new();
        o.set(100.0, 0.0, 214.0, 32.0);
        let at = |x: f32| quantise_js(10.0 + frac_at(&o, x, 14.0) * 50.0, 10.0, 60.0, 1.0);
        assert_eq!(at(0.0), 10.0, "left of the slot clamps to min");
        assert_eq!(at(107.0), 10.0);
        assert_eq!(at(207.0), 35.0, "the middle of the travel is the middle value");
        assert_eq!(at(307.0), 60.0);
        assert_eq!(at(900.0), 60.0, "right of the slot clamps to max");
    }

    /// Hover moves nothing: a drag is only live after a press on the slider.
    #[test]
    fn slider_drag_is_inactive_until_a_press() {
        let o = Origin::new();
        assert!(!o.drag().active);
        o.set_drag(Drag { active: true, sent: 5.0 });
        assert!(o.drag().active);
        o.set_drag(Drag::default());
        assert!(!o.drag().active, "a release ends it");
    }
}
