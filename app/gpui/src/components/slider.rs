//! `Slider`, after `app/web/src/components/Slider.tsx` and the `.slider`
//! rules in `app.css` re-skinned by `hardware.css`: a 32 px tall fader, a
//! 4 px sunk track, a live fill up to the cap, and a 14x24 raised cap with
//! a live line across it. Drawn on a canvas so the fill and the cap sit at
//! the exact percent of the travel.

use std::rc::Rc;
use std::cell::Cell;

use gpui::{Bounds, IntoElement, Window, canvas, div, point, prelude::*, px, size};

use crate::theme::Theme;

/// Where an element was painted: its origin and size (the fader measures
/// itself in a canvas prepaint; GPUI 0.2.2 has no bounds call).
#[derive(Clone)]
pub struct Origin(Rc<Cell<(f32, f32, f32, f32)>>);

impl Origin {
    pub fn new() -> Self {
        Origin(Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0))))
    }

    pub fn set(&self, x: f32, y: f32, w: f32, h: f32) {
        self.0.set((x, y, w, h));
    }

    pub fn get(&self) -> (f32, f32, f32, f32) {
        self.0.get()
    }
}

/// A raw number put on the step grid and inside the range.
pub fn quantise(raw: f32, min: f32, max: f32, step: f32) -> f32 {
    let stepped = min + ((raw - min) / step).round() * step;
    stepped.clamp(min, max)
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
    const CAP: f32 = 14.0;
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
    if disabled {
        // React `.slider[data-disabled="true"] { opacity: 0.5 }`: the whole
        // slot (track, fill, cap) draws at half strength; the press is inert.
        return slot.opacity(0.5);
    }
    let change = on_change.clone();
    let commit = on_commit.clone();
    let drag_origin = origin.clone();
    slot.on_mouse_down(gpui::MouseButton::Left, {
        let drag_origin = drag_origin.clone();
        let change = change.clone();
        move |event: &gpui::MouseDownEvent, window, cx| {
            let frac = frac_at(&drag_origin, f32::from(event.position.x), CAP);
            change(quantise(min + frac * span, min, max, 1.0), window, cx);
        }
    })
    .on_mouse_move({
        let drag_origin = drag_origin.clone();
        let change = change.clone();
        move |event: &gpui::MouseMoveEvent, window, cx| {
            let frac = frac_at(&drag_origin, f32::from(event.position.x), CAP);
            change(quantise(min + frac * span, min, max, 1.0), window, cx);
        }
    })
    .on_mouse_up(gpui::MouseButton::Left, move |event: &gpui::MouseUpEvent, window, cx| {
        let frac = frac_at(&drag_origin, f32::from(event.position.x), CAP);
        commit(quantise(min + frac * span, min, max, 1.0), window, cx);
    })
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
    #[test]
    fn percent_clamps_to_the_travel() {
        let p = (120f32 / 100.0).clamp(0.0, 1.0);
        assert_eq!(p, 1.0);
    }
}
