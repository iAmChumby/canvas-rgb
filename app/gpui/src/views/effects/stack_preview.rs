//! The builder's picture, after `app/web/src/views/effects/StackPreviewCanvas.tsx`.
//!
//! The stack's own frames at 320 × 200, fitted to the stage without ever
//! changing its ratio, with the picked layer's area drawn on it when
//! `Show areas` is on.
//!
//! React draws the frames into a `<canvas>`; GPUI has no such element, so
//! the picture and the area frame are painted into one canvas element that
//! fills the stage, and the stage's own box is kept in a cell so a drag can
//! work in canvas units.

#![allow(dead_code)]

use std::cell::Cell;
use std::rc::Rc;

use gpui::{Bounds, IntoElement, MouseButton, Pixels, Window, canvas, div, point, prelude::*, px, size};

use crate::theme::{Theme, TokenRadius};

use super::stack_draft::Bounds as AreaBounds;

/// The picture's box inside the stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PictureBox {
    /// The picture's left edge, in window pixels.
    pub x: f32,
    /// The picture's top edge, in window pixels.
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PictureBox {
    /// The canvas units a window point is at.
    pub fn canvas_point(&self, at: gpui::Point<Pixels>) -> (f32, f32) {
        let k = self.width / super::stack_draft::CANVAS_W;
        if k <= 0.0 {
            return (0.0, 0.0);
        }
        ((f32::from(at.x) - self.x) / k, (f32::from(at.y) - self.y) / k)
    }

    /// The css pixels a canvas unit is at.
    pub fn scale(&self) -> f32 {
        self.width / super::stack_draft::CANVAS_W
    }
}

/// The picture's box inside the stage: the smaller of the stage's width and
/// 1.6 × its height, centred (`width: min(100%, 160cqh)`).
pub fn picture_box(stage: Bounds<Pixels>) -> PictureBox {
    let w = f32::from(stage.size.width);
    let h = f32::from(stage.size.height);
    let width = w.min(h * 1.6);
    let height = width / 1.6;
    PictureBox {
        x: f32::from(stage.origin.x) + (w - width) / 2.0,
        y: f32::from(stage.origin.y) + (h - height) / 2.0,
        width,
        height,
    }
}

/// The stage's box, kept for the drag.
#[derive(Clone)]
pub struct StageBox(pub Rc<Cell<(f32, f32, f32, f32)>>);

impl Default for StageBox {
    fn default() -> Self {
        Self(Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0))))
    }
}

impl StageBox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, x: f32, y: f32, w: f32, h: f32) {
        self.0.set((x, y, w, h));
    }

    pub fn get(&self) -> (f32, f32, f32, f32) {
        self.0.get()
    }
}

/// The picture: the daemon's frame and, over it, the picked layer's area.
#[allow(clippy::too_many_arguments)]
pub fn stack_preview(
    t: &Theme,
    id: &str,
    frame: Option<(u16, u16, Vec<u8>)>,
    area: Option<(AreaBounds, String)>,
    editable: bool,
    stage_box: StageBox,
    on_drag: impl Fn(gpui::Point<Pixels>, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let stage = StageBox::new();
    let _ = stage;
    let mut stage_el = div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex_1()
        .min_h(px(0.0))
        .r_md()
        .bg(Theme::color(t.sunk))
        .overflow_hidden();
    let picture = frame.clone();
    let area_frame = area.clone();
    let box_cell = stage_box.clone();
    let theme = *t;
    stage_el = stage_el.child(
        canvas(
            move |bounds, _window, _cx| {
                let pic = picture_box(bounds);
                box_cell.set(pic.x, pic.y, pic.width, pic.height);
                pic
            },
            move |bounds, pic, window, cx| {
                // The picture itself.
                if let Some((w, h, data)) = &picture {
                    paint_picture(window, pic, *w, *h, data);
                }
                // The picked layer's area over it.
                if let Some((bounds_in_canvas, label)) = &area_frame {
                    let k = pic.scale();
                    let x = pic.x + bounds_in_canvas.x * k;
                    let y = pic.y + bounds_in_canvas.y * k;
                    let w = bounds_in_canvas.width * k;
                    let h = bounds_in_canvas.height * k;
                    paint_area(window, cx, &theme, pic, x, y, w, h, label, editable);
                }
                let _ = bounds;
            },
        )
        .absolute()
        .inset_0(),
    );
    stage_el.on_mouse_move(move |event, window, cx| on_drag(event.position, window, cx))
}

/// Paints the frame into the picture's box, one source pixel at a time.
fn paint_picture(window: &mut Window, pic: PictureBox, w: u16, h: u16, data: &[u8]) {
    if w == 0 || h == 0 || pic.width <= 0.0 || pic.height <= 0.0 {
        return;
    }
    let dst_w = pic.width.ceil() as usize;
    let dst_h = pic.height.ceil() as usize;
    for dy in 0..dst_h {
        let sy = ((dy as f32 + 0.5) / pic.height * h as f32).floor() as usize;
        let sy = sy.min(h as usize - 1);
        let mut dx = 0;
        while dx < dst_w {
            let sx = ((dx as f32 + 0.5) / pic.width * w as f32).floor() as usize;
            let sx = sx.min(w as usize - 1);
            let i = (sy * w as usize + sx) * 3;
            let rgb = (data[i], data[i + 1], data[i + 2]);
            let mut end = dx + 1;
            while end < dst_w {
                let ex = ((end as f32 + 0.5) / pic.width * w as f32).floor() as usize;
                let ex = ex.min(w as usize - 1);
                let j = (sy * w as usize + ex) * 3;
                if (data[j], data[j + 1], data[j + 2]) != rgb {
                    break;
                }
                end += 1;
            }
            let color = gpui::Rgba {
                r: rgb.0 as f32 / 255.0,
                g: rgb.1 as f32 / 255.0,
                b: rgb.2 as f32 / 255.0,
                a: 1.0,
            };
            window.paint_quad(gpui::fill(
                Bounds {
                    origin: point(px(pic.x + dx as f32), px(pic.y + dy as f32)),
                    size: size(px((end - dx) as f32), px(1.0)),
                },
                color,
            ));
            dx = end;
        }
    }
}

/// The picked layer's area: an accent frame with four corner knobs and the
/// layer's name beside it.
#[allow(clippy::too_many_arguments)]
fn paint_area(
    window: &mut Window,
    cx: &mut gpui::App,
    t: &Theme,
    pic: PictureBox,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &str,
    editable: bool,
) {
    let accent = Theme::color(t.a1);
    // The tint over the picture: `color-mix(in srgb, var(--a1) 12%, transparent)`.
    let tint = gpui::Rgba { a: 0.12, ..accent };
    window.paint_quad(gpui::fill(
        Bounds { origin: point(px(x), px(y)), size: size(px(w), px(h)) },
        tint,
    ));
    // The 1.5 px border.
    let border = 1.5;
    for (ox, oy, bw, bh) in [
        (x, y, w, border),
        (x, y + h - border, w, border),
        (x, y, border, h),
        (x + w - border, y, border, h),
    ] {
        window.paint_quad(gpui::fill(
            Bounds { origin: point(px(ox), px(oy)), size: size(px(bw), px(bh)) },
            accent,
        ));
    }
    if editable {
        // The knob: a 12 px dot in a 2 px light ring and a 2 px dark ring.
        for (cx0, cy0) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
            window.paint_quad(gpui::fill(
                Bounds {
                    origin: point(px(cx0 - 6.0), px(cy0 - 6.0)),
                    size: size(px(12.0), px(12.0)),
                },
                gpui::Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            ));
            window.paint_quad(gpui::fill(
                Bounds {
                    origin: point(px(cx0 - 4.0), px(cy0 - 4.0)),
                    size: size(px(8.0), px(8.0)),
                },
                accent,
            ));
        }
    }
    // The label: "X · area", on the accent, in the mono face.
    let text = label.to_string();
    let mut style = window.text_style();
    style.font_family = crate::fonts::MONO_FAMILY.into();
    style.font_size = px(14.0).into();
    style.font_weight = gpui::FontWeight::MEDIUM;
    style.color = Theme::color(t.on_accent).into();
    let run = style.to_run(text.len());
    let length = text.len();
    let line = window.text_system().shape_line(text.into(), px(14.0), &[run], None);
    let width = f32::from(line.x_for_index(length));
    let height = 18.2;
    let lx = x + 12.0;
    let ly = y + 8.0;
    // The label's own plate.
    window.paint_quad(gpui::fill(
        Bounds { origin: point(px(lx - 5.0), px(ly - 1.0)), size: size(px(width + 10.0), px(height)) },
        accent,
    ));
    line.paint(point(px(lx), px(ly)), px(height), window, cx).unwrap();
    let _ = pic;
}

/// The mouse button a press carries.
pub fn is_left(button: MouseButton) -> bool {
    button == MouseButton::Left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_takes_the_smaller_of_the_stage_width_and_its_height_times_one_and_six() {
        let stage = Bounds {
            origin: point(px(100.0), px(200.0)),
            size: size(px(800.0), px(400.0)),
        };
        let pic = picture_box(stage);
        // 1.6 * 400 = 640 is the smaller, so the picture is 640 x 400.
        assert_eq!((pic.width, pic.height), (640.0, 400.0));
        // Centred in the stage.
        assert_eq!(pic.x, 100.0 + 80.0);
        assert_eq!(pic.y, 200.0);
    }

    #[test]
    fn a_wide_stage_lets_the_picture_fill_its_width() {
        let stage = Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(1000.0), px(300.0)),
        };
        let pic = picture_box(stage);
        // 1.6 * 300 = 480 is the smaller.
        assert_eq!((pic.width, pic.height), (480.0, 300.0));
    }

    #[test]
    fn a_window_point_maps_to_canvas_units() {
        let pic = PictureBox { x: 10.0, y: 20.0, width: 640.0, height: 400.0 };
        let at = point(px(10.0), px(20.0));
        assert_eq!(pic.canvas_point(at), (0.0, 0.0));
        let at = point(px(650.0), px(420.0));
        assert_eq!(pic.canvas_point(at), (320.0, 200.0));
    }
}
