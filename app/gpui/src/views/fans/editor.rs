//! The header editor: the panel beside the list (a sheet on a phone).
//!
//! After `HeaderEditor` in `app/web/src/views/FansView.tsx` and
//! `FanCurveEditor` in `app/web/src/views/FanCurveEditor.tsx`: the mode
//! control (BIOS, Fixed %, Curve), the floor note with the "don't exceed"
//! line, the duty slider and field, the curve chart with its point rows, and
//! the owner's word on whether the header is on this board.

use canvas_proto::fans::{CurvePoint, FanHeaderStatus, FanMode, FansStatus, HotKind};
use gpui::{div, prelude::*, px, Context, IntoElement};

use crate::components::fan_bits::{
    fans_button, fans_icon_button, fans_number_field, rule_box, slider, warn_wash, BtnVariant,
    Origin,
};
use crate::components::fan_bulk::is_unavailable;
use crate::components::fan_curve::{base_floor, duty_for, fan_curve_chart, hot_rule_text, HOT_MIN};
use crate::components::panel::panel;
use crate::theme::{Theme, TokenRadius};

use super::FansView;

/// The number of points a curve may have.
pub const MAX_POINTS: usize = 8;
/// The number of points a curve needs.
pub const MIN_POINTS: usize = 2;

impl FansView {
    /// The editor panel for one header.
    pub(super) fn editor(
        &mut self,
        header: &FanHeaderStatus,
        page: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = self.theme;
        let fans = self.fans.clone().unwrap_or_else(super::empty_fans);
        let base = base_floor(&header.id, fans.floor_percent);
        let unavailable = is_unavailable(header);

        // The panel's head: the availability key and the speed. They wrap
        // rather than push the panel wider.
        let mut actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap_2()
            .min_w_0()
            .flex_1();
        if !unavailable {
            actions = actions.child(self.availability_key(header, cx));
        }
        actions = actions.child(
            div()
                .flex()
                .items_baseline()
                .gap_1()
                // React's `.rpm-readout` takes the body's size (15 px).
                .text_size(px(15.0))
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.ink))
                .child(format!("{}", header.rpm.unwrap_or(0)))
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child("RPM"),
                ),
        );

        let body = if unavailable {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(super::unavailable_body(header)),
                )
                .child(self.availability_key(header, cx))
                .into_any_element()
        } else {
            self.editor_body(header, &fans, base, page, cx)
                .into_any_element()
        };

        // The one live sentence for the editor: it names the kind of reason,
        // never the live temperature. It is one pixel wide, as the React
        // `.sr-only` node is, and it stands out of the flow so it takes no
        // room in the panel.
        let announce = div()
            .id(gpui::SharedString::from(format!(
                "fans-announce-{}",
                header.id
            )))
            .absolute()
            .w(px(1.0))
            .h(px(1.0))
            .overflow_hidden()
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .child(super::reason_kind_of(header));

        panel(
            &t,
            &format!("fans-editor-{}", header.id),
            &header.label,
            actions,
            div().flex().flex_col().gap_2().child(announce).child(body),
        )
        // React `.rd-fans .rd-fans-right > .panel { flex: 1 1 auto; display:
        // flex; flex-direction: column; min-height: 0; overflow-y: auto; }`:
        // the editor takes the room the column has left and scrolls inside it.
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        // React `scrollbar-gutter: stable` (16 px) plus the panel's 16 px
        // right padding: 26 px of the editor's width is spoken for.
        .pr(px(26.0))
    }

    /// The availability keys (issue #146).
    fn availability_key(
        &self,
        header: &FanHeaderStatus,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = self.theme;
        let unavailable = is_unavailable(header);
        let mut row = div().flex().items_center().gap_2();
        let entity = cx.entity();
        let id = header.id.clone();
        let (label, icon) = if unavailable {
            ("Show anyway", "Eye")
        } else {
            ("Not on my board", "EyeSlash")
        };
        row = row.child(fans_button(
            &t,
            &format!("fans-availability-{id}"),
            label,
            BtnVariant::Ghost,
            true,
            self.editor_busy,
            Some(icon),
            move |_, _, cx| {
                entity.update(cx, |view, cx| {
                    let available = if unavailable { Some(true) } else { Some(false) };
                    view.set_availability(id.clone(), available, cx);
                });
            },
        ));
        if header.override_available.is_some() {
            let entity = cx.entity();
            let id = header.id.clone();
            row = row.child(fans_button(
                &t,
                &format!("fans-availability-board-{id}"),
                "Use the board list",
                BtnVariant::Ghost,
                true,
                self.editor_busy,
                Some("ListChecks"),
                move |_, _, cx| {
                    entity.update(cx, |view, cx| {
                        view.set_availability(id.clone(), None, cx);
                    });
                },
            ));
        }
        row
    }

    /// The editor's body: the mode control and what the mode shows.
    fn editor_body(
        &mut self,
        header: &FanHeaderStatus,
        fans: &FansStatus,
        base: u8,
        page: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = self.theme;
        let mode = header.mode.clone();
        let entity = cx.entity();
        let seg = crate::components::segmented::segmented(
            &t,
            &format!("fans-mode-{}", header.id),
            &[("bios", "BIOS"), ("fixed", "Fixed %"), ("curve", "Curve")],
            super::mode_word(&mode),
            {
                let entity = entity.clone();
                let header_id = header.id.clone();
                move |value, _, cx| {
                    entity.update(cx, |view, cx| view.set_mode(&header_id, &value, cx));
                }
            },
        );

        let mut body = div().flex().flex_col().gap_2().child(
            // React `.stack > .seg { align-self: flex-start; }`: the
            // segmented control takes the width it needs, not the column's.
            div().flex().items_start().child(seg),
        );
        let manual = !matches!(mode, FanMode::Bios);

        if !manual {
            body = body.child(
                div()
                    .text_size(px(15.0))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("Chip auto control. Floors off."),
            );
            body = body.child(self.status(header, fans, &mode, cx));
        }

        if manual {
            // The floor note, with the "don't exceed" line inside it.
            let hot_kind = match &mode {
                FanMode::Curve { source, .. } => source.hot_kind(),
                _ => HotKind::Cpu,
            };
            let hot_row = crate::components::fan_curve::hot_row(&fans.hot, hot_kind).copied();
            let rule_text = hot_rule_text(hot_row.map(|h| h.limit_c));
            // React's `.rule`: the floor percent is a mono ink `.fig` span
            // inside the dim sentence.
            // One wrapping text run, like React's inline sentence: a flex row of three
            // pieces never wrapped and ran past the box (#204 review).
            let head = "Floor ".to_string();
            let fig = format!("{base}%");
            let tail = format!(". {rule_text}.");
            let dim: gpui::Hsla = Theme::color(t.dim).into();
            let ink: gpui::Hsla = Theme::color(t.ink).into();
            let run = |len: usize, family: &'static str, color: gpui::Hsla| gpui::TextRun {
                len,
                font: gpui::font(family),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let runs = vec![
                run(head.len(), crate::fonts::UI_FAMILY, dim),
                run(fig.len(), crate::fonts::MONO_FAMILY, ink),
                run(tail.len(), crate::fonts::UI_FAMILY, dim),
            ];
            let sentence = gpui::StyledText::new(format!("{head}{fig}{tail}")).with_runs(runs);
            let mut rule = div().flex().flex_col().min_w(px(0.0)).child(
                div().min_w(px(0.0)).text_sm().child(sentence),
            );
            if let Some(hot_row) = hot_row {
                rule = rule.child(self.hot_controls(hot_row, cx));
            }
            if let Some(error) = &self.hot_error {
                rule = rule.child(
                    div()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.down))
                        .child(error.clone()),
                );
            }
            body = body.child(rule_box(
                &t,
                &format!("fans-floor-{}", header.id),
                false,
                rule,
            ));
        }

        match &mode {
            FanMode::Fixed { .. } => {
                let entity = cx.entity();
                let header_id = header.id.clone();
                let origin = Origin::new();
                let duty = self.editor_percent;
                let slider_entity = entity.clone();
                let slider_id = header_id.clone();
                let fader = slider(
                    &t,
                    &format!("fans-duty-{}", header.id),
                    duty,
                    f32::from(base),
                    100.0,
                    Some(f32::from(base)),
                    self.editor_busy,
                    origin,
                    {
                        let slider_entity = slider_entity.clone();
                        move |v, _, cx| {
                            slider_entity.update(cx, |view, cx| {
                                view.editor_percent = v;
                                cx.notify();
                            });
                        }
                    },
                    {
                        let slider_entity = slider_entity.clone();
                        move |v, _, cx| {
                            slider_entity.update(cx, |view, cx| {
                                view.commit_percent(&slider_id, v, cx);
                            });
                        }
                    },
                );
                let entity = cx.entity();
                let header_id = header.id.clone();
                let number = fans_number_field(
                    &t,
                    &format!("fans-duty-value-{}", header.id),
                    duty,
                    f32::from(base),
                    100.0,
                    1.0,
                    Some("%"),
                    self.editor_busy,
                    move |v, _, cx| {
                        entity.update(cx, |view, cx| {
                            view.commit_percent(&header_id, v, cx);
                        });
                    },
                );
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child("Duty"),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .flex_1()
                                .child(div().flex_1().child(fader))
                                .child(number),
                        ),
                );
                body = body.child(self.status(header, fans, &mode, cx));
                body = body.child(
                    div()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child("Duty ")
                        .child(
                            div()
                                .font_family(crate::fonts::MONO_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child(match header.pwm_percent {
                                    None => "\u{2014}".to_string(),
                                    Some(pwm) => format!("{pwm}%"),
                                }),
                        ),
                );
            }
            FanMode::Curve { .. } => {
                // The temperature source.
                let options: Vec<(String, String)> = fans
                    .temps
                    .iter()
                    .map(|temp| (super::source_key(&temp.source), temp.label.clone()))
                    .collect();
                let entity = cx.entity();
                let toggle_entity = entity.clone();
                let header_id = header.id.clone();
                let source_select = crate::components::fan_bits::fans_select(
                    &t,
                    &format!("fans-source-{}", header.id),
                    &format!("{} temperature source", header.label),
                    &options,
                    &super::source_key(&self.editor_source),
                    self.source_select_open,
                    move |_, cx| {
                        toggle_entity.update(cx, |view, cx| {
                            view.source_select_open = !view.source_select_open;
                            cx.notify();
                        });
                    },
                    {
                        let entity = entity.clone();
                        let header_id = header_id.clone();
                        move |key, _, cx| {
                            entity.update(cx, |view, cx| {
                                view.set_source(&header_id, &key, cx);
                            });
                        }
                    },
                );
                body = body.child(crate::components::panel::field(
                    &t,
                    &format!("fans-source-field-{}", header.id),
                    "Temperature source",
                    div().flex().items_center().gap_2().child(source_select),
                    false,
                ));
                // The chart.
                let points = self.editor_points.clone();
                let current_temp = super::temp_for(
                    &FanMode::Curve {
                        source: self.editor_source.clone(),
                        points: points.clone(),
                    },
                    &fans.temps,
                );
                let current_duty = duty_for(&points, current_temp);
                let hot_row =
                    crate::components::fan_curve::hot_row(&fans.hot, self.editor_source.hot_kind())
                        .copied();
                let hot_value = self.hot_drag.or_else(|| hot_row.map(|h| h.limit_c));
                let entity = cx.entity();
                // The chart's width, from the page: the panel's border, its
                // padding and the 10 px scrollbar gutter (React's
                // `scrollbar-gutter: stable` measured at 10 px here).
                let chart_width = if page <= 640.0 {
                    page - 24.0 - 44.0
                } else {
                    (page - 16.0) / 2.1 - 44.0
                };
                let chart = fan_curve_chart(
                    &t,
                    &format!("fans-curve-{}", header.id),
                    &points,
                    base,
                    current_temp,
                    current_duty,
                    hot_row,
                    hot_value,
                    &header.id,
                    self.chart_box.clone(),
                    chart_width,
                    {
                        let entity = entity.clone();
                        move |index, _, cx| {
                            entity.update(cx, |view, cx| {
                                view.drag_point = Some(index);
                                view.drag_start = view.editor_points.clone();
                                cx.notify();
                            });
                        }
                    },
                    {
                        let entity = entity.clone();
                        move |x, y, _, cx| {
                            entity.update(cx, |view, cx| {
                                view.move_dragged_point(x, y, cx);
                            });
                        }
                    },
                    {
                        let entity = entity.clone();
                        move |x, y, _, cx| {
                            entity.update(cx, |view, cx| {
                                view.drop_dragged_point(x, y, cx);
                            });
                        }
                    },
                    {
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |view, cx| {
                                view.hot_drag = view.hot_value();
                                cx.notify();
                            });
                        }
                    },
                    {
                        let entity = entity.clone();
                        move |x, _, cx| {
                            entity.update(cx, |view, cx| {
                                view.move_hot(x, cx);
                            });
                        }
                    },
                    {
                        let entity = entity.clone();
                        move |x, _, cx| {
                            entity.update(cx, |view, cx| {
                                view.drop_hot(x, cx);
                            });
                        }
                    },
                );
                body = body.child(chart);
                body = body.child(self.status(header, fans, &mode, cx));
                // The point rows: React `.curve-rows` is a grid of two
                // columns, so two points stand on one line.
                let editor_col = (page - 16.0) / 2.1;
                let two_up = editor_col > 559.0;
                let entity = cx.entity();
                let add = fans_button(
                    &t,
                    &format!("fans-add-point-{}", header.id),
                    "Add point",
                    BtnVariant::Default,
                    true,
                    points.len() >= MAX_POINTS,
                    Some("Plus"),
                    move |_, _, cx| {
                        entity.update(cx, |view, cx| view.add_point(cx));
                    },
                );
                let count = div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(format!("{} of {MAX_POINTS} points", points.len()));
                let add_row = div().flex().items_center().gap_2().child(add).child(count);
                let rows = if two_up {
                    // Two points to a row, with a 12 px gap between them.
                    let mut grid = div().flex().flex_wrap().gap_2().gap_x_3();
                    for (index, point) in points.iter().enumerate() {
                        let row = self.point_row(index, point, base, cx);
                        grid = grid.child(div().flex_1().min_w_0().child(row));
                    }
                    grid.child(div().w_full().child(add_row))
                } else {
                    let mut column = div().flex().flex_col().gap_2();
                    for (index, point) in points.iter().enumerate() {
                        let row = self.point_row(index, point, base, cx);
                        column = column.child(row);
                    }
                    column.child(add_row)
                };
                body = body.child(rows);
            }
            FanMode::Bios => {}
        }
        body.into_any_element()
    }

    /// One point's row: its index, its temperature, its percent and the key
    /// that removes it.
    fn point_row(
        &mut self,
        index: usize,
        point: &CurvePoint,
        base: u8,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = self.theme;
        let temp_field = self.text_field(
            &format!("fans-point-{index}-temp"),
            point.temp_c,
            20.0,
            100.0,
            0,
            Some("\u{00B0}C"),
            super::FieldWidth::Fill,
            false,
            {
                let entity = cx.entity();
                move |v, cx| {
                    entity.update(cx, |view, cx| view.commit_point_temp(index, v, cx));
                }
            },
            cx,
        );
        let percent_field = self.text_field(
            &format!("fans-point-{index}-percent"),
            f32::from(point.percent),
            f32::from(base),
            100.0,
            0,
            Some("%"),
            super::FieldWidth::Fill,
            false,
            {
                let entity = cx.entity();
                move |v, cx| {
                    entity.update(cx, |view, cx| view.commit_point_percent(index, v, cx));
                }
            },
            cx,
        );
        let entity = cx.entity();
        let remove_disabled = self.editor_points.len() <= MIN_POINTS;
        let remove = fans_icon_button(
            &t,
            &format!("fans-point-{index}-remove"),
            &format!("Remove point {}", index + 1),
            "X",
            28.0,
            14.0,
            remove_disabled,
            move |_, _, cx| {
                entity.update(cx, |view, cx| view.remove_point(index, cx));
            },
        );
        div()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .child(
                div()
                    .w(px(20.0))
                    .flex_shrink_0()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(format!("{}", index + 1)),
            )
            .child(temp_field)
            .child(percent_field)
            .child(remove)
    }

    /// The "don't exceed" control, inside the floor note.
    fn hot_controls(
        &mut self,
        hot: canvas_proto::fans::HotLimitStatus,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = self.theme;
        let field = self.text_field(
            "fans-hot-value",
            hot.limit_c,
            HOT_MIN,
            hot.max_settable_c,
            0,
            Some("\u{00B0}C"),
            super::FieldWidth::Fixed(92.0),
            self.hot_busy,
            {
                let entity = cx.entity();
                move |v, cx| {
                    entity.update(cx, |view, cx| view.commit_hot(Some(v), cx));
                }
            },
            cx,
        );
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("Don't exceed"),
            )
            .child(field);
        if hot.user_set {
            let entity = cx.entity();
            row = row.child(fans_button(
                &t,
                "fans-hot-reset",
                "Reset to automatic",
                BtnVariant::Ghost,
                true,
                self.hot_busy,
                Some("ArrowClockwise"),
                move |_, _, cx| {
                    entity.update(cx, |view, cx| view.commit_hot(None, cx));
                },
            ));
        } else {
            row = row.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("automatic"),
            );
        }
        row
    }

    /// The status: the daemon's reason, then the error and "Applying".
    fn status(
        &self,
        header: &FanHeaderStatus,
        fans: &FansStatus,
        mode: &FanMode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = self.theme;
        let reason = super::reason_text(header);
        let handed_back = crate::components::fan_bulk::is_handed_back(header);
        let mut block = div().flex().flex_col().gap_1();
        // React's `.fan-status-reason` keeps the room of two lines, so the
        // panel does not move when a reason appears.
        let mut region = div().flex().flex_col().min_h(px(58.6));
        match reason {
            Some(text) => {
                // The reason box: two lines, with a More key when the text is
                // cut (GPUI cannot measure the text, so the length decides).
                let mut box_body = div().flex().items_start().gap_2().child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child(text.clone()),
                );
                if text.chars().count() > 110 || self.reason_open {
                    box_body = box_body.child(
                        div()
                            .id(gpui::SharedString::from("fans-reason-more"))
                            .flex_none()
                            .child({
                                let entity = cx.entity();
                                fans_button(
                                    &t,
                                    "fans-reason-toggle",
                                    if self.reason_open { "Less" } else { "More" },
                                    BtnVariant::Ghost,
                                    true,
                                    false,
                                    None,
                                    move |_, _, cx| {
                                        entity.update(cx, |view, cx| {
                                            view.reason_open = !view.reason_open;
                                            cx.notify();
                                        });
                                    },
                                )
                            }),
                    );
                }
                region = region.child(
                    div()
                        .id(gpui::SharedString::from("fans-reason"))
                        .flex()
                        .flex_col()
                        .flex_1()
                        .justify_center()
                        .gap_2()
                        .py(px(8.0))
                        .px(px(12.0))
                        .r_md()
                        .border_1()
                        .border_color(crate::components::fan_bits::tint(
                            Theme::color(t.warn),
                            0.45,
                        ))
                        .bg(warn_wash(&t))
                        .child(box_body),
                );
            }
            None => {
                region = region.child(
                    div()
                        .flex()
                        .flex_1()
                        // React `.fan-status-quiet { align-items: center }`: centred
                        // vertically, left-aligned (the text was centred both ways).
                        .items_center()
                        .py(px(8.0))
                        .px(px(12.0))
                        .r_md()
                        .border_1()
                        .border_color(Theme::color(t.line))
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(super::quiet_line(mode)),
                );
            }
        }
        block = block.child(region);
        let _ = handed_back;
        let _ = fans;
        block = block.child(
            div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap_3()
                .min_h(px(16.0))
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(Theme::color(t.down))
                        .child(self.editor_error.clone().unwrap_or_default()),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(80.0))
                        .text_right()
                        .text_color(Theme::color(t.mute))
                        .child(if self.editor_busy { "Applying" } else { "" }),
                ),
        );
        block
    }
}

/// The curve's points, sorted, as the daemon keeps them.
#[cfg(test)]
fn sorted(points: &[CurvePoint]) -> Vec<CurvePoint> {
    let mut out = points.to_vec();
    out.sort_by(|a, b| {
        a.temp_c
            .partial_cmp(&b.temp_c)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// The default curve for a header's floor.
#[cfg(test)]
fn curve_for(base: u8) -> Vec<CurvePoint> {
    crate::components::fan_curve::default_curve(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::fan_curve::bounds_for;
    #[test]
    fn the_points_sort_by_temperature() {
        let points = vec![
            CurvePoint {
                temp_c: 70.0,
                percent: 100,
            },
            CurvePoint {
                temp_c: 30.0,
                percent: 30,
            },
        ];
        let sorted = super::sorted(&points);
        assert_eq!(sorted[0].temp_c, 30.0);
        assert_eq!(sorted[1].temp_c, 70.0);
    }

    #[test]
    fn the_default_curve_follows_the_floor() {
        let curve = super::curve_for(20);
        assert_eq!(curve.len(), 4);
        assert_eq!(curve[0].percent, 30);
        let curve = super::curve_for(40);
        assert_eq!(curve[0].percent, 40);
    }

    #[test]
    fn the_bounds_keep_a_point_between_its_neighbours() {
        let points = vec![
            CurvePoint {
                temp_c: 30.0,
                percent: 30,
            },
            CurvePoint {
                temp_c: 50.0,
                percent: 50,
            },
        ];
        assert_eq!(bounds_for(&points, 0), (20.0, 49.0));
    }
}
