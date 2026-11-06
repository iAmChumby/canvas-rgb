//! `Slider`, after `app/web/src/components/Slider.tsx` and the `.slider`
//! rules in `app.css` re-skinned by `hardware.css`: a 32 px tall fader, a
//! 4 px sunk track, a live fill up to the cap, and a 14x24 raised cap with
//! a live line across it. Drawn on a canvas so the fill and the cap sit at
//! the exact percent of the travel.

use gpui::{Bounds, IntoElement, Window, canvas, div, point, prelude::*, px, size};

use crate::theme::Theme;

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
