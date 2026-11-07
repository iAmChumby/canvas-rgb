//! The bulk bar over the Headers list, after `app/web/src/views/FanBulk.tsx`
//! and the rules in `app/web/src/lib/fanBulk.ts`.
//!
//! The daemon has one route per header, so a bulk change is the per-header
//! request sent once for each header in the selection, one after the other.
//! Each header answers for itself: the ones that took the change stay applied,
//! the ones that did not are named with the reason.
//!
//! The rules here are pure and are covered by the tests at the bottom; the bar
//! itself is drawn from the state the view keeps.

use canvas_proto::fans::{FanHeaderStatus, FanMode, FansStatus, TempSource};
use gpui::{div, prelude::*, px};

use crate::components::fan_bits::{fans_button, fans_number_field, live, live_soft, BtnVariant};
use crate::components::fan_curve::base_floor;
use crate::theme::{Theme, TokenRadius};

/// Why a tach-only header cannot be ticked.
pub const TACH_ONLY_WHY: &str = "Tach only: canvas-rgb reads its speed and cannot set it.";

/// Why a header the board does not route cannot be ticked (issue #128).
pub const UNAVAILABLE_WHY: &str = "Not on this board: this header is not fitted on your board.";

/// What a bulk run does.
#[derive(Debug, Clone, PartialEq)]
pub enum BulkAction {
    Bios,
    Fixed { percent: u8 },
    Curve { from: String },
}

/// How one header took the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkState {
    Applied,
    Skipped,
    Failed,
}

/// What one header did.
#[derive(Debug, Clone, PartialEq)]
pub struct BulkOutcome {
    pub id: String,
    pub label: String,
    pub state: BulkState,
    pub detail: String,
}

/// A header's label without the "(tach only)" the daemon appends.
pub fn plain_label(label: &str) -> String {
    match label.strip_suffix(" (tach only)") {
        Some(stripped) => stripped.to_string(),
        None => label.to_string(),
    }
}

/// A header the chip can drive. A header the board does not route and a
/// tach-only header cannot be driven.
pub fn can_drive(h: &FanHeaderStatus) -> bool {
    h.available && h.pwm_percent.is_some()
}

/// A header this board does not route (issue #128).
pub fn is_unavailable(h: &FanHeaderStatus) -> bool {
    !h.available
}

/// The daemon handed this header back to BIOS although a mode was asked for.
pub fn is_handed_back(h: &FanHeaderStatus) -> bool {
    h.on_bios && !matches!(h.mode, FanMode::Bios)
}

/// The words the summary says.
pub fn action_words(action: &BulkAction, from: Option<&str>) -> String {
    match action {
        BulkAction::Bios => "BIOS".to_string(),
        BulkAction::Fixed { percent } => format!("Fixed {percent}%"),
        BulkAction::Curve { .. } => match from {
            Some(from) => format!("the curve of {from}"),
            None => "a copied curve".to_string(),
        },
    }
}

/// The mode one header is asked for. `None` when the action has nothing for it.
pub fn mode_for(action: &BulkAction, h: &FanHeaderStatus, fans: &FansStatus) -> Option<FanMode> {
    match action {
        BulkAction::Bios => Some(FanMode::Bios),
        BulkAction::Fixed { percent } => {
            let floor = base_floor(&h.id, fans.floor_percent);
            Some(FanMode::Fixed {
                percent: (*percent).min(100).max(floor),
            })
        }
        BulkAction::Curve { from } => {
            let src = fans.headers.iter().find(|x| &x.id == from)?;
            let points = match &src.mode {
                FanMode::Curve { points, .. } => points.clone(),
                _ => return None,
            };
            let source = match &src.mode {
                FanMode::Curve { source, .. } => source.clone(),
                _ => TempSource::Cpu,
            };
            Some(FanMode::Curve { source, points })
        }
    }
}

/// What a run does with one header: skip it, send it, or fail it.
#[derive(Debug, Clone, PartialEq)]
pub enum BulkDecision {
    // The header is not written; the words say why.
    Skip { detail: String },
    // The header is written with this mode.
    Send(FanMode),
    // The header fails; the words say why.
    Fail { detail: String },
}

/// The rules of a bulk run, in order: an unavailable header is skipped and
/// never sent, a tach-only header is skipped, a header the daemon handed back
/// is skipped, the source of a copy is skipped, and a copy whose source is no
/// longer on a curve fails.
pub fn decide(action: &BulkAction, h: &FanHeaderStatus, fans: &FansStatus) -> BulkDecision {
    if is_unavailable(h) {
        return BulkDecision::Skip {
            detail: UNAVAILABLE_WHY.to_string(),
        };
    }
    if !can_drive(h) {
        return BulkDecision::Skip {
            detail: TACH_ONLY_WHY.to_string(),
        };
    }
    if is_handed_back(h) {
        let why = match &h.override_reason {
            Some(reason) => format!(": {reason}"),
            None => String::new(),
        };
        return BulkDecision::Skip {
            detail: format!("Handed back to BIOS by the daemon{why}. Left as it is."),
        };
    }
    if let BulkAction::Curve { from } = action {
        if from == &h.id {
            return BulkDecision::Skip {
                detail: "This is the curve being copied.".to_string(),
            };
        }
    }
    match mode_for(action, h, fans) {
        Some(mode) => BulkDecision::Send(mode),
        None => BulkDecision::Fail {
            detail: "The header to copy from is no longer on a curve.".to_string(),
        },
    }
}

/// The one line that sums an apply up: how many took it, how many did not.
pub fn summary_of(outcomes: &[BulkOutcome], words: &str, total: usize) -> String {
    let count = |state: BulkState| outcomes.iter().filter(|o| o.state == state).count();
    let applied = count(BulkState::Applied);
    let failed = count(BulkState::Failed);
    let skipped = count(BulkState::Skipped);
    let mut parts = vec![format!("{applied} set to {words}")];
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    if total > outcomes.len() {
        parts.push(format!("{} not sent", total - outcomes.len()));
    }
    parts.join(", ")
}

/// The words a sent header took, when the duty the user typed was under the
/// header's own floor.
pub fn applied_detail(action: &BulkAction, mode: &FanMode) -> String {
    match (action, mode) {
        (BulkAction::Fixed { percent }, FanMode::Fixed { percent: sent }) => {
            if *sent != *percent {
                format!("Raised to its floor, Fixed {sent}%.")
            } else {
                format!("Fixed {sent}%.")
            }
        }
        (_, FanMode::Bios) => "BIOS.".to_string(),
        (_, FanMode::Fixed { percent }) => format!("Fixed {percent}%."),
        (_, FanMode::Curve { .. }) => "Curve copied.".to_string(),
    }
}

/// The bar over the list: the idle line and the live bar share one cell, so
/// ticking the first header moves nothing.
#[allow(clippy::too_many_arguments)]
pub fn fan_bulk_bar(
    t: &Theme,
    id: &str,
    fans: &FansStatus,
    checked: &[FanHeaderStatus],
    percent: f32,
    picked_from: Option<&str>,
    busy: bool,
    result: Option<(String, Vec<BulkOutcome>)>,
    details_open: bool,
    on_clear: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_bios: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_fixed: impl Fn(f32, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_copy: impl Fn(&str, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
    on_details: impl Fn(&mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> impl gpui::IntoElement {
    let active = !checked.is_empty();
    let curves: Vec<&FanHeaderStatus> = fans
        .headers
        .iter()
        .filter(|h| matches!(h.mode, FanMode::Curve { .. }))
        .collect();
    let from_id: Option<String> = picked_from
        .filter(|id| curves.iter().any(|h| h.id == *id))
        .map(|id| id.to_string())
        .or_else(|| curves.first().map(|h| h.id.clone()));
    let from = from_id
        .as_ref()
        .and_then(|id| curves.iter().find(|h| &h.id == id));
    let floor = base_floor("", fans.floor_percent);
    let tach: Vec<String> = fans
        .headers
        .iter()
        .filter(|h| h.pwm_percent.is_none())
        .map(|h| plain_label(&h.label))
        .collect();

    // The idle line.
    let mut idle_text = String::from("Tick headers to set several at once.");
    if !tach.is_empty() {
        idle_text.push_str(&format!(" {}: tach only, read-only.", tach.join(", ")));
    }
    let idle = div()
        .flex()
        .items_center()
        .gap_2()
        .py(px(8.0))
        .px(px(12.0))
        .r_md()
        .bg(Theme::color(t.raised))
        .text_color(Theme::color(t.mute))
        .text_sm()
        .font_family(crate::fonts::UI_FAMILY)
        .child(
            gpui::svg()
                .path("icons/ListChecks.svg")
                .size_4()
                .text_color(Theme::color(t.mute)),
        )
        .child(idle_text);
    let idle = if active { idle.invisible() } else { idle };

    // The live bar.
    let failed = result
        .as_ref()
        .is_some_and(|(_, outcomes)| outcomes.iter().any(|o| o.state == BulkState::Failed));
    let count = div()
        .text_sm()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .font_family(crate::fonts::UI_FAMILY)
        .text_color(Theme::color(t.ink))
        .child(format!("{} selected", checked.len()));
    let result_text = if busy {
        "Applying".to_string()
    } else {
        result
            .as_ref()
            .map(|(words, outcomes)| summary_of(outcomes, words, outcomes.len()))
            .unwrap_or_default()
    };
    let result_line = div()
        .flex_1()
        .min_w_0()
        .text_sm()
        .font_family(crate::fonts::UI_FAMILY)
        .text_color(if failed {
            Theme::color(t.down)
        } else {
            Theme::color(t.dim)
        })
        .child(result_text);
    let mut top = div()
        .flex()
        .items_center()
        .gap_2()
        .min_h(px(28.0))
        .child(count)
        .child(result_line);
    if result.is_some() && !busy {
        top = top.child(fans_button(
            t,
            &format!("{id}-details"),
            "Details",
            BtnVariant::Ghost,
            true,
            false,
            None,
            {
                let on_details = on_details.clone();
                move |_, window, cx| on_details(window, cx)
            },
        ));
    }
    top = top.child(fans_button(
        t,
        &format!("{id}-clear"),
        "Clear",
        BtnVariant::Ghost,
        true,
        busy,
        Some("X"),
        move |_, window, cx| on_clear(window, cx),
    ));

    // The actions.
    let mut actions = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .gap_x_3()
        .child(fans_button(
            t,
            &format!("{id}-bios"),
            "BIOS",
            BtnVariant::Default,
            true,
            busy,
            None,
            {
                let on_bios = on_bios.clone();
                move |_, window, cx| on_bios(window, cx)
            },
        ));
    actions = actions.child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .min_h(px(32.0))
            .child(fans_number_field(
                t,
                &format!("{id}-percent"),
                percent,
                f32::from(floor),
                100.0,
                1.0,
                Some("%"),
                busy,
                {
                    let on_fixed = on_fixed.clone();
                    move |v, window, cx| on_fixed(v, window, cx)
                },
            ))
            .child(fans_button(
                t,
                &format!("{id}-set-fixed"),
                "Set fixed",
                BtnVariant::Default,
                true,
                busy,
                None,
                {
                    let on_fixed = on_fixed.clone();
                    let value = percent;
                    move |_, window, cx| on_fixed(value, window, cx)
                },
            )),
    );
    let mut curve_group = div().flex().items_center().gap_2().min_h(px(32.0));
    match from {
        Some(from) => {
            let options: Vec<(&str, &str)> = curves
                .iter()
                .map(|h| (h.id.as_str(), h.label.as_str()))
                .collect();
            let mut trigger = div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(32.0))
                .px(px(12.0))
                .r_md()
                .border_1()
                .border_color(Theme::color(t.line_2))
                .bg(Theme::color(t.raised))
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child(from.label.clone())
                .child(
                    gpui::svg()
                        .path("icons/CaretDown-bold.svg")
                        .w(px(14.0))
                        .h(px(14.0))
                        .flex_none()
                        .text_color(Theme::color(t.ink)),
                );
            if details_open {
                let mut list = div()
                    .absolute()
                    .mt(px(36.0))
                    .w(px(220.0))
                    .r_md()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .bg(Theme::color(t.panel));
                for (value, label) in &options {
                    let on_copy = on_copy.clone();
                    let value = value.to_string();
                    list = list.child(
                        div()
                            .id(gpui::SharedString::from(format!("{id}-option-{value}")))
                            .px_3()
                            .py_1()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .cursor_pointer()
                            .child(label.to_string())
                            .on_click(move |_, window, cx| on_copy(&value, window, cx)),
                    );
                }
                trigger = trigger.child(list);
            }
            curve_group = curve_group.child(trigger);
        }
        None => {
            curve_group = curve_group.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("No header is on a curve"),
            );
        }
    }
    let copy_disabled = busy || from.is_none();
    let from_id_for_copy = from.map(|h| h.id.clone());
    curve_group = curve_group.child(fans_button(
        t,
        &format!("{id}-copy-curve"),
        "Copy curve",
        BtnVariant::Default,
        true,
        copy_disabled,
        None,
        move |_, window, cx| {
            if let Some(id) = &from_id_for_copy {
                on_copy(id, window, cx);
            }
        },
    ));
    actions = actions.child(curve_group);

    let live = div()
        .flex()
        .flex_col()
        .gap_2()
        .py(px(8.0))
        .px(px(12.0))
        .r_md()
        .border_1()
        .border_color(live(t))
        .bg(live_soft(t))
        .child(top)
        .child(actions);
    let live = if active { live } else { live.invisible() };

    div()
        .id(gpui::SharedString::from(id.to_string()))
        .relative()
        .flex()
        .flex_col()
        .mb_2()
        // The idle line and the live bar share one cell, so the cell is as
        // tall as the taller of the two and the list under it does not move
        // when the first header is ticked. The live bar is in the flow (it
        // sets the cell's height) and the idle line lies over it.
        .child(live)
        .child(idle.absolute().top_0().left_0().right_0().bottom_0())
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::fans::{CurvePoint, TempSource};

    fn header(id: &str, label: &str, pwm: Option<u8>, mode: FanMode) -> FanHeaderStatus {
        FanHeaderStatus {
            id: id.to_string(),
            label: label.to_string(),
            rpm: Some(1000),
            pwm_percent: pwm,
            mode,
            on_bios: false,
            override_reason: None,
            available: true,
            override_available: None,
        }
    }

    fn fans(headers: Vec<FanHeaderStatus>) -> FansStatus {
        FansStatus {
            chip: Some("chip".into()),
            hardware_watchdog: Some(false),
            headers,
            temps: Vec::new(),
            active_profile: "Default".into(),
            profiles: vec!["Default".into()],
            floor_percent: 20,
            hot: Vec::new(),
        }
    }

    #[test]
    fn a_tach_only_header_cannot_be_driven() {
        let tach = header("sys_fan2", "System fan 2 (tach only)", None, FanMode::Bios);
        assert!(!can_drive(&tach));
        assert!(!is_unavailable(&tach));
        assert_eq!(plain_label(&tach.label), "System fan 2");
        // A header the board does not route cannot be driven either.
        let mut missing = header("sys_fan1", "System fan 1", Some(40), FanMode::Bios);
        missing.available = false;
        assert!(!can_drive(&missing));
        assert!(is_unavailable(&missing));
    }

    #[test]
    fn a_handed_back_header_is_one_the_daemon_took_back() {
        let mut h = header(
            "cpu_fan",
            "CPU fan",
            Some(40),
            FanMode::Fixed { percent: 50 },
        );
        assert!(!is_handed_back(&h));
        h.on_bios = true;
        assert!(is_handed_back(&h));
        // A header on BIOS is not handed back.
        let mut bios = header("cpu_fan", "CPU fan", Some(40), FanMode::Bios);
        bios.on_bios = true;
        assert!(!is_handed_back(&bios));
    }

    #[test]
    fn a_fixed_action_is_raised_to_the_headers_own_floor() {
        let cpu = header("cpu_fan", "CPU fan", Some(40), FanMode::Bios);
        let status = fans(vec![cpu.clone()]);
        let action = BulkAction::Fixed { percent: 20 };
        match mode_for(&action, &cpu, &status) {
            Some(FanMode::Fixed { percent }) => assert_eq!(percent, 30),
            other => panic!("expected a fixed mode, got {other:?}"),
        }
        // Another header keeps the default floor.
        let sys = header("sys_fan1", "System fan 1", Some(40), FanMode::Bios);
        let status = fans(vec![sys.clone()]);
        match mode_for(&action, &sys, &status) {
            Some(FanMode::Fixed { percent }) => assert_eq!(percent, 20),
            other => panic!("expected a fixed mode, got {other:?}"),
        }
    }

    #[test]
    fn the_skip_rules_run_in_order() {
        let mut missing = header("sys_fan1", "System fan 1", Some(40), FanMode::Bios);
        missing.available = false;
        let tach = header("sys_fan2", "System fan 2 (tach only)", None, FanMode::Bios);
        let mut handed = header(
            "cpu_fan",
            "CPU fan",
            Some(40),
            FanMode::Fixed { percent: 50 },
        );
        handed.on_bios = true;
        handed.override_reason = Some("at the safety ceiling".into());
        let plain = header("sys_fan3", "System fan 3", Some(40), FanMode::Bios);
        let status = fans(vec![
            missing.clone(),
            tach.clone(),
            handed.clone(),
            plain.clone(),
        ]);
        let action = BulkAction::Bios;

        match decide(&action, &missing, &status) {
            BulkDecision::Skip { detail } => assert_eq!(detail, UNAVAILABLE_WHY),
            other => panic!("expected a skip, got {other:?}"),
        }
        match decide(&action, &tach, &status) {
            BulkDecision::Skip { detail } => assert_eq!(detail, TACH_ONLY_WHY),
            other => panic!("expected a skip, got {other:?}"),
        }
        match decide(&action, &handed, &status) {
            BulkDecision::Skip { detail } => assert_eq!(
                detail,
                "Handed back to BIOS by the daemon: at the safety ceiling. Left as it is."
            ),
            other => panic!("expected a skip, got {other:?}"),
        }
        assert!(matches!(
            decide(&action, &plain, &status),
            BulkDecision::Send(_)
        ));
    }

    #[test]
    fn a_copy_skips_its_own_source() {
        let source = header(
            "cpu_fan",
            "CPU fan",
            Some(40),
            FanMode::Curve {
                source: TempSource::Cpu,
                points: vec![
                    CurvePoint {
                        temp_c: 30.0,
                        percent: 30,
                    },
                    CurvePoint {
                        temp_c: 70.0,
                        percent: 100,
                    },
                ],
            },
        );
        let other = header("sys_fan1", "System fan 1", Some(40), FanMode::Bios);
        let status = fans(vec![source.clone(), other.clone()]);
        let action = BulkAction::Curve {
            from: "cpu_fan".into(),
        };
        match decide(&action, &source, &status) {
            BulkDecision::Skip { detail } => assert_eq!(detail, "This is the curve being copied."),
            other => panic!("expected a skip, got {other:?}"),
        }
        match decide(&action, &other, &status) {
            BulkDecision::Send(FanMode::Curve { points, .. }) => assert_eq!(points.len(), 2),
            other => panic!("expected a curve, got {other:?}"),
        }
    }

    #[test]
    fn a_copy_whose_source_left_the_curve_fails() {
        let source = header("cpu_fan", "CPU fan", Some(40), FanMode::Bios);
        let other = header("sys_fan1", "System fan 1", Some(40), FanMode::Bios);
        let status = fans(vec![source, other.clone()]);
        let action = BulkAction::Curve {
            from: "cpu_fan".into(),
        };
        match decide(&action, &other, &status) {
            BulkDecision::Fail { detail } => {
                assert_eq!(detail, "The header to copy from is no longer on a curve.")
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn the_summary_counts_each_outcome() {
        let outcomes = vec![
            BulkOutcome {
                id: "a".into(),
                label: "A".into(),
                state: BulkState::Applied,
                detail: String::new(),
            },
            BulkOutcome {
                id: "b".into(),
                label: "B".into(),
                state: BulkState::Applied,
                detail: String::new(),
            },
            BulkOutcome {
                id: "c".into(),
                label: "C".into(),
                state: BulkState::Failed,
                detail: String::new(),
            },
            BulkOutcome {
                id: "d".into(),
                label: "D".into(),
                state: BulkState::Skipped,
                detail: String::new(),
            },
        ];
        assert_eq!(
            summary_of(&outcomes, "BIOS", 4),
            "2 set to BIOS, 1 failed, 1 skipped"
        );
        // Headers that were not sent are counted.
        assert_eq!(
            summary_of(&outcomes, "BIOS", 6),
            "2 set to BIOS, 1 failed, 1 skipped, 2 not sent"
        );
        assert_eq!(
            summary_of(&outcomes[..2], "Fixed 50%", 2),
            "2 set to Fixed 50%"
        );
    }

    #[test]
    fn a_raised_duty_says_so() {
        let action = BulkAction::Fixed { percent: 20 };
        assert_eq!(
            applied_detail(&action, &FanMode::Fixed { percent: 30 }),
            "Raised to its floor, Fixed 30%."
        );
        assert_eq!(
            applied_detail(&action, &FanMode::Fixed { percent: 20 }),
            "Fixed 20%."
        );
        assert_eq!(applied_detail(&BulkAction::Bios, &FanMode::Bios), "BIOS.");
    }

    #[test]
    fn the_words_name_the_action() {
        assert_eq!(action_words(&BulkAction::Bios, None), "BIOS");
        assert_eq!(
            action_words(&BulkAction::Fixed { percent: 50 }, None),
            "Fixed 50%"
        );
        assert_eq!(
            action_words(
                &BulkAction::Curve {
                    from: "cpu_fan".into()
                },
                Some("CPU fan")
            ),
            "the curve of CPU fan"
        );
    }
}
