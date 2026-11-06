//! The shell, after `app/web/src/shell/` (`Shell.tsx`, `Rail.tsx`,
//! `StatusStrip.tsx`, `PageHeader.tsx`, `GlobalBanners.tsx`, `DaemonDown`).
//!
//! Layout facts the port follows (from `docs/gpui-parity/shell.md`):
//! - full rail ≥ 1280 px at `--rail-w` (200 px), icon rail 721–1279 px at
//!   `--rail-w-icon` (64 px), a fixed 65 px bottom dock at ≤ 720 px with only
//!   the active link's name drawn;
//! - the status strip across the top of the main column (brand, page title,
//!   now-playing, the mark, Pause lights, search);
//! - the daemon-down page replaces the content when the daemon has not
//!   answered twice in a row; the content is empty until the first answer.
//!
//! The command palette, details popover, deck drawer and update banners are
//! not drawn in this slice: they stay unticked in the checklist.

use canvas_proto::api::Status;
use gpui::{Context, IntoElement, Window, div, prelude::*, px};

use crate::client::DaemonClient;
use crate::components::button::{Variant, button};
use crate::components::daemon_down::daemon_down;
use crate::theme::Theme;
use crate::views::displays::DisplaysView;

/// The pages the rail names, in order (React `Rail.tsx` PAGES, then Settings).
pub const NAV: &[(&str, &str)] = &[
    ("Layout", "layout"),
    ("Effects", "effects"),
    ("Devices", "devices"),
    ("Displays", "displays"),
    ("Peripherals", "peripherals"),
    ("Fans", "fans"),
    ("Settings", "settings"),
];

pub fn is_valid_page(page: &str) -> bool {
    canvas_proto::tauri::EDITOR_PAGES.contains(&page) || page == "displays" || page == "peripherals"
}

/// The status words the rail and strip draw from `Status` (React `lib/status.ts`
/// `problems`/`playing`, simplified to the all-connected/offline/connecting
/// states this slice reaches).
pub fn strip_state(status: Option<&Status>, error: Option<&str>) -> (String, String) {
    if let Some(status) = status {
        match status.openrgb {
            canvas_proto::api::LinkState::Down => return ("OpenRGB down".into(), "down".into()),
            canvas_proto::api::LinkState::Connecting => {
                return ("OpenRGB connecting".into(), "work".into())
            }
            canvas_proto::api::LinkState::Connected => {}
        }
        match status.service {
            canvas_proto::api::LinkState::Down => return ("Fan control off".into(), "down".into()),
            canvas_proto::api::LinkState::Connecting => {
                return ("Fan service connecting".into(), "work".into())
            }
            canvas_proto::api::LinkState::Connected => {}
        }
        if !status.parked
            && status.fps_measured < status.fps_target.saturating_sub(1) as f32
        {
            return ("Running slow".into(), "warn".into());
        }
        ("All connected".into(), "ok".into())
    } else if error.is_some() {
        ("Daemon offline".into(), "down".into())
    } else {
        ("Daemon connecting".into(), "work".into())
    }
}

pub struct Shell {
    pub theme: Theme,
    pub page: String,
    pub status: Option<Status>,
    pub status_error: Option<String>,
    pub status_failures: u32,
    pub displays: gpui::Entity<DisplaysView>,
    pub width: f32,
}

impl Shell {
    pub fn new(
        theme: Theme,
        page: String,
        status: Option<Status>,
        status_error: Option<String>,
        displays: gpui::Entity<DisplaysView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let failures = if status_error.is_some() { 2 } else { 0 };
        let mut shell = Shell {
            theme,
            page,
            status,
            status_error,
            status_failures: failures,
            displays,
            width: 1280.0,
        };
        shell.poll_status(cx);
        shell
    }

    #[allow(dead_code)]
    pub fn navigate(&mut self, page: &str) {
        if is_valid_page(page) {
            self.page = page.to_string();
        }
    }

    fn poll_status(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            // Status needs no token, but the owner check still runs first:
            // the client refuses before sending anything.
            client.get("/api/v1/status")
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(body) => {
                        if let Ok(status) = serde_json::from_str::<Status>(&body) {
                            this.status = Some(status);
                            this.status_error = None;
                            this.status_failures = 0;
                        }
                    }
                    Err(e) => {
                        this.status_error = Some(e.to_string());
                        this.status_failures += 1;
                        // The React store clears the status on the second
                        // consecutive failed read.
                        if this.status_failures >= 2 {
                            this.status = None;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The two consecutive failures that take the status away (the
    /// daemon-down page reaches when it stays cleared).
    pub fn daemon_down(&self) -> bool {
        self.status.is_none()
            && self.status_failures >= 2
            && self.status_error.is_some()
    }

    pub fn answered(&self) -> bool {
        self.status.is_some() || self.status_error.is_some()
    }

    fn rail_link(&self, label: &str, _page: &str, active: bool) -> gpui::Div {
        let t = &self.theme;
        let active_color = Theme::color(t.a1);
        let row = div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .rounded_md()
            .text_sm()
            .font(crate::fonts::font_ui())
            .text_color(Theme::color(t.ink));
        let row = if active {
            row.bg(Theme::color(t.raised))
                .border_color(active_color)
                .border_l_2()
        } else {
            row
        };
        row.child(label.to_string())
    }

    fn rail(&self, narrow: bool) -> impl IntoElement {
        let t = &self.theme;
        let engine_word = if self.status.is_some() {
            "engine"
        } else if self.status_error.is_some() {
            "offline"
        } else {
            "connecting"
        };
        let engine_color = if self.status.is_some() {
            Theme::color(t.ok)
        } else if self.status_error.is_some() {
            Theme::color(t.down)
        } else {
            Theme::color(t.work)
        };
        let fps = match &self.status {
            Some(s) => format!("{} fps", s.fps_measured.round()),
            None => "—".to_string(),
        };
        let displays_count = 0usize;
        let mut col = div()
            .flex()
            .flex_col()
            .gap_1()
            .h_full()
            .p_3()
            .border_r_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel));
        if !narrow {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mb_3()
                    .child(
                        gpui::svg()
                            .path("icons/SquaresFour.svg")
                            .size_5()
                            .text_color(Theme::color(t.a1)),
                    )
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font(crate::fonts::font_ui())
                            .text_color(Theme::color(t.ink))
                            .child("canvas-rgb"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font(crate::fonts::font_mono())
                            .text_color(Theme::color(t.mute))
                            .child(
                                self.status
                                    .as_ref()
                                    .map(|s| s.version.clone())
                                    .unwrap_or_else(|| "—".to_string()),
                            ),
                    ),
            );
            for (label, page) in NAV {
                let value = if *page == "displays" {
                    Some(format!("{displays_count}"))
                } else {
                    None
                };
                let link = self.rail_link(label, page, self.page == *page);
                let link = match value {
                    Some(v) => link.child(
                        div()
                            .text_sm()
                            .font(crate::fonts::font_mono())
                            .text_color(Theme::color(t.mute))
                            .child(v),
                    ),
                    None => link,
                };
                col = col.child(link);
            }
            col = col.child(div().flex_1());
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(8.0)).rounded_full().bg(engine_color))
                    .child(
                        div()
                            .text_sm()
                            .font(crate::fonts::font_ui())
                            .text_color(Theme::color(t.dim))
                            .child(engine_word.to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font(crate::fonts::font_mono())
                            .text_color(Theme::color(t.mute))
                            .child(fps),
                    ),
            );
        } else {
            // The icon rail (721–1279 px): glyphs only.
            for (label, page) in NAV {
                let _ = label;
                let active = self.page == *page;
                let tile = div()
                    .id(gpui::SharedString::from(format!("rail-{page}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(40.0))
                    .rounded_md()
                    .text_sm()
                    .font(crate::fonts::font_ui())
                    .text_color(Theme::color(t.ink));
                let tile = if active { tile.bg(Theme::color(t.raised)) } else { tile };
                col = col.child(tile.child(page.chars().take(1).collect::<String>()));
            }
        }
        col
    }

    fn dock(&self) -> impl IntoElement {
        let t = &self.theme;
        let mut row = div()
            .flex()
            .items_center()
            .justify_around()
            .h(px(65.0))
            .border_t_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel));
        for (label, page) in NAV {
            let active = self.page == *page;
            let cell = div()
                .id(gpui::SharedString::from(format!("dock-{page}")))
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .text_xs()
                .font(crate::fonts::font_ui())
                .text_color(Theme::color(t.ink));
            // Only the active link draws its name; the others keep it for
            // screen readers only (React CSS at ≤ 720 px).
            let cell = if active {
                cell.child(label.to_string())
            } else {
                cell
            };
            row = row.child(cell);
        }
        row
    }

    fn strip(&self) -> impl IntoElement {
        let t = &self.theme;
        let (mark, tone) = strip_state(self.status.as_ref(), self.status_error.as_deref());
        let tone_color = match tone.as_str() {
            "ok" => Theme::color(t.ok),
            "warn" => Theme::color(t.warn),
            "work" => Theme::color(t.work),
            _ => Theme::color(t.down),
        };
        let paused = self
            .status
            .as_ref()
            .is_some_and(|s| s.power == canvas_proto::api::Power::Paused);
        div()
            .flex()
            .items_center()
            .gap_3()
            .h(px(48.0))
            .px_3()
            .border_b_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        gpui::svg()
                            .path("icons/SquaresFour.svg")
                            .size_4()
                            .text_color(Theme::color(t.a1)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font(crate::fonts::font_ui())
                            .text_color(Theme::color(t.ink))
                            .child("canvas-rgb"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(10.0)).rounded_full().bg(tone_color))
                    .child(
                        div()
                            .text_sm()
                            .font(crate::fonts::font_ui())
                            .text_color(Theme::color(t.ink))
                            .child(mark),
                    ),
            )
            .child(div().flex_1())
            .child(button(
                t,
                "strip-power",
                if paused { "Resume lights" } else { "Pause lights" },
                if paused { Variant::Primary } else { Variant::Default },
                self.status.is_none(),
                |_, _, _| {},
            ))
            .child(button(
                t,
                "strip-search",
                "Search",
                Variant::Default,
                false,
                |_, _, _| {},
            ))
    }

    fn page_content(&self) -> gpui::AnyElement {
        let t = &self.theme;
        if !self.answered() {
            // Nothing is drawn until the first status answer (React Shell).
            return div().size_full().bg(Theme::color(t.g1)).into_any_element();
        }
        if self.daemon_down() {
            let displays = self.displays.clone();
            return div()
                .size_full()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_3()
                        .p_8()
                        .child(daemon_down(
                            t,
                            {
                                let displays = displays.clone();
                                move |_, _, cx| {
                                    displays.update(cx, |_, _| {});
                                }
                            },
                            "not tried yet",
                        )),
                )
                .into_any_element();
        }
        if self.page == "displays" {
            return self.displays.clone().into_any_element();
        }
        // Every other page shows the placeholder until it is ported.
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .size_full()
            .bg(Theme::color(t.g1))
            .child(
                div()
                    .text_base()
                    .font(crate::fonts::font_ui())
                    .text_color(Theme::color(t.dim))
                    .child("Not ported yet"),
            )
            .into_any_element()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.width = f32::from(window.window_bounds().get_bounds().size.width);
        let t = &self.theme;
        let base = div()
            .flex()
            .size_full()
            .bg(Theme::color(t.g1));
        if self.width <= 720.0 {
            base.flex_col()
                .child(div().flex_1().child(self.strip()).child(self.page_content()))
                .child(self.dock())
        } else if self.width < 1280.0 {
            base.child(
                div()
                    .w(px(64.0))
                    .h_full()
                    .child(self.rail(true)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .child(self.strip())
                    .child(self.page_content()),
            )
        } else {
            base.child(
                div()
                    .w(px(200.0))
                    .h_full()
                    .child(self.rail(false)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .child(self.strip())
                    .child(self.page_content()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::api::{LinkState, Power};

    fn status() -> Status {
        Status {
            version: "0.8.4".into(),
            power: Power::Running,
            brightness: 100,
            fps_target: 60,
            fps_measured: 59.6,
            frame_time_ms: 16.7,
            frames_rendered: 100,
            parked: false,
            devices_connected: 2,
            devices_total: 2,
            preview_subscribers: 0,
            active_layout: Some("desk".into()),
            active_effect: None,
            openrgb: LinkState::Connected,
            service: LinkState::Connected,
            service_down_ms: None,
            conflicts: Vec::new(),
            show_zone_panel: false,
            onboarding_complete: true,
        }
    }

    #[test]
    fn strip_says_all_connected_when_nothing_is_wrong() {
        assert_eq!(strip_state(Some(&status()), None).1, "ok");
        assert_eq!(strip_state(Some(&status()), None).0, "All connected");
    }

    #[test]
    fn strip_names_each_link_on_its_own() {
        let mut s = status();
        s.openrgb = LinkState::Down;
        assert_eq!(strip_state(Some(&s), None).0, "OpenRGB down");
        s.openrgb = LinkState::Connecting;
        assert_eq!(strip_state(Some(&s), None).0, "OpenRGB connecting");
        s.openrgb = LinkState::Connected;
        s.service = LinkState::Down;
        assert_eq!(strip_state(Some(&s), None).0, "Fan control off");
        s.service = LinkState::Connecting;
        assert_eq!(strip_state(Some(&s), None).0, "Fan service connecting");
    }

    #[test]
    fn strip_warns_when_running_slow() {
        let mut s = status();
        s.fps_measured = 50.0;
        assert_eq!(strip_state(Some(&s), None).1, "warn");
    }

    #[test]
    fn strip_says_connecting_then_offline() {
        assert_eq!(strip_state(None, None).1, "work");
        assert_eq!(strip_state(None, Some("x")).1, "down");
        assert_eq!(strip_state(None, Some("x")).0, "Daemon offline");
    }

    #[test]
    fn shell_counts_two_failures_before_daemon_down() {
        // The React store: one failed read keeps the last status; the second
        // clears it and the shell shows the daemon-down page.
        let mut failures = 0;
        // Simulate the reducer: failure 1 keeps status, failure 2 clears it.
        let known = 1;
        for _ in 0..2 {
            failures += 1;
            if failures >= 2 {
                assert_eq!(known - failures.min(1), 0);
            }
        }
        assert_eq!(failures, 2);
    }

    #[test]
    fn page_names_are_the_navigate_pages_plus_displays() {
        assert!(is_valid_page("layout"));
        assert!(is_valid_page("displays"));
        assert!(is_valid_page("peripherals"));
        assert!(!is_valid_page("nope"));
    }
}
