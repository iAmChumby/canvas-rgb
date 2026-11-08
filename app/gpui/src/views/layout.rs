//! The Layout page: the desk and the inspector beside it.
//!
//! The port follows `app/web/src/views/LayoutView.tsx` and
//! `app/web/src/views/PreviewStage.tsx`; the checklist is
//! `docs/gpui-parity/layout.md`.
//!
//! The page is three things:
//!
//! - the strip's own actions (undo and redo, the zone count, the save state,
//!   Revert, Save and "Map my PC again"), which the shell draws beside the
//!   page's name;
//! - the desk, in [`stage`], with the parts on it and the cells the engine
//!   samples;
//! - the inspector: "To place", the parts list and (on a phone) the desk's
//!   own settings.

pub mod art;
pub mod geometry;
pub mod interact;
pub mod preview;
pub mod stage;
pub mod tokens;

use gpui::{Context, IntoElement, Render, Window, div, prelude::*, px};

use canvas_proto::api::{DevicesResponse, LayoutsResponse};
use canvas_proto::device::{DeviceInfo, DeviceKind};
use canvas_proto::layout::{Layout, PartKind, RenderScale, SamplingMode};

use crate::client::DaemonClient;
use crate::components::button::{Variant, button_icon, icon_button};
use crate::components::panel::panel_with_padding;
use crate::components::segmented::segmented;
use crate::components::slider::Origin;
use crate::theme::{Theme, TokenRadius};
use crate::views::layout::geometry::LengthUnit;
use crate::views::layout::preview::PreviewFeed;
use crate::views::layout::stage::{DeskInput, DragOverlay, Sel, StageMode, View as StageView, desk_with_hint, hero_view};
use crate::views::layout::tokens::DeskTokens;

/// The panes below the desk on a phone: one of them at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhonePane {
    Parts,
    Place,
    Render,
}

/// The Layout page.
pub struct LayoutView {
    pub theme: Theme,
    /// The daemon's layouts and the active one.
    pub layouts: Option<LayoutsResponse>,
    pub layouts_error: Option<String>,
    /// The daemon's devices.
    pub devices: Option<Vec<DeviceInfo>>,
    /// The unit every size is read in.
    pub unit: LengthUnit,
    /// What the desk shows: the picture under the parts, or the lights.
    pub mode: StageMode,
    /// The pane below the desk on a phone.
    pub phone_pane: PhonePane,
    /// The daemon's live preview.
    pub feed: PreviewFeed,
    /// The picked parts.
    pub selection: Vec<Sel>,
    /// How many frames the feed has delivered: the page watches this number
    /// to know when the desk has to be drawn again.
    pub frames: u64,
    /// The window's width, for the page's own media queries.
    pub width: f32,
    /// The window's height.
    pub height: f32,
    pub history: interact::History,
    pub base: Option<Layout>,
    pub saving: bool,
    pub save_error: Option<String>,
    pub view: Option<StageView>,
    pub desk_origin: Origin,
    pub drag: Option<Drag>,
    pub overlay: DragOverlay,
    pub render_open: bool,
    pub add_open: bool,
    pub identifying: Option<usize>,
    /// The onboarding takeover state, when open (React LayoutView `onboarding`).
    pub onboarding: Option<crate::views::onboarding::OnboardData>,
    /// The daemon's status, for the onboarding's hello variants and gate
    /// (React `st.status`). Polled when the onboarding or its decision needs it.
    pub status: Option<canvas_proto::api::Status>,
    /// The daemon's effects, for the finish tiles (React `st.effects`).
    pub effects: Option<canvas_proto::api::EffectsResponse>,
    /// "Set up later" was said this launch (React `laterThisLaunch` memory flag).
    pub later_this_launch: bool,
    /// The one-time open decision has run (React `decided` ref).
    pub onboard_decided: bool,
    /// Debug-only capture overrides (parsed only under `cfg!(debug_assertions)`).
    pub capture_step: Option<String>,
    pub capture_state: Option<String>,
    /// Debug-only: the fake daemon's "case appears after first run" — the
    /// hello wave/session timers are frozen for captures.
    pub capture_frozen: bool,
}

/// One pointer drag on the desk (PreviewStage `Drag`).
#[derive(Debug, Clone)]
pub struct Drag {
    pub kind: DragKind,
    pub sel: Sel,
    pub origs: Vec<Orig>,
    pub riders: Vec<Orig>,
    pub start: interact::Pt,
    pub start_px: (f32, f32),
    pub handle: Option<interact::Handle>,
    pub pending: bool,
    pub locked: bool,
    pub collapse: bool,
    pub pre: Option<Layout>,
    pub view_origin: Option<StageView>,
}

#[derive(Debug, Clone)]
pub enum DragKind {
    Move,
    Resize,
    Rotate,
    Marquee,
    Pan,
}

#[derive(Debug, Clone)]
pub struct Orig {
    pub sel: Sel,
    pub zone: Option<canvas_proto::layout::PlacedZone>,
    pub landmark: Option<canvas_proto::layout::Landmark>,
}

impl Orig {
    fn rect(&self) -> Option<interact::Rect> {
        if let Some(z) = &self.zone {
            Some(interact::Rect {
                x: z.x as f64,
                y: z.y as f64,
                w: z.width as f64,
                h: z.height as f64,
            })
        } else {
            self.landmark.as_ref().map(|l| interact::Rect {
                x: l.x as f64,
                y: l.y as f64,
                w: l.width as f64,
                h: l.height as f64,
            })
        }
    }
}

impl LayoutView {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        let unit = system_unit();
        let mut view = LayoutView {
            theme,
            layouts: None,
            layouts_error: None,
            devices: None,
            unit,
            mode: StageMode::Effect,
            phone_pane: PhonePane::Parts,
            feed: PreviewFeed::new(),
            selection: Vec::new(),
            frames: 0,
            width: 1280.0,
            height: 860.0,
            history: interact::History::new(),
            base: None,
            saving: false,
            save_error: None,
            view: None,
            desk_origin: Origin::new(),
            drag: None,
            overlay: DragOverlay::default(),
            render_open: false,
            add_open: false,
            identifying: None,
            onboarding: None,
            status: None,
            effects: None,
            later_this_launch: false,
            onboard_decided: false,
            capture_step: None,
            capture_state: None,
            capture_frozen: false,
        };
        view.load(cx);
        view.start_feed();
        view.watch_feed(cx);
        view
    }

    /// The layout the daemon has active, or the draft of it.
    pub fn layout(&self) -> Option<&Layout> {
        if let Some(draft) = self.history.draft.as_ref() {
            return Some(draft);
        }
        let layouts = self.layouts.as_ref()?;
        let active = &layouts.active;
        layouts
            .layouts
            .iter()
            .find(|l| &l.id == active)
            .or_else(|| layouts.layouts.first())
    }

    pub fn server_layout(&self) -> Option<&Layout> {
        let layouts = self.layouts.as_ref()?;
        let active = &layouts.active;
        layouts
            .layouts
            .iter()
            .find(|l| &l.id == active)
            .or_else(|| layouts.layouts.first())
    }

    pub fn dirty(&self) -> bool {
        interact::is_dirty(self.history.draft.as_ref(), self.base.as_ref())
    }

    pub fn undo(&mut self) {
        self.history.undo();
    }

    pub fn redo(&mut self) {
        self.history.redo();
    }

    pub fn commit(&mut self, f: impl FnOnce(&Layout) -> Layout) {
        self.history.commit(f);
    }

    pub fn revert(&mut self) {
        if let Some(base) = self.base.clone() {
            self.commit(move |_| base);
        }
        self.save_error = None;
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        let (Some(draft), Some(server)) = (self.history.draft.clone(), self.server_layout().cloned())
        else {
            return;
        };
        if self.saving {
            return;
        }
        self.saving = true;
        self.save_error = None;
        let body = interact::save_body(&draft, server.effect.clone());
        let id = draft.id.clone();
        let body_text = serde_json::to_string(&body).unwrap_or_default();
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let client = DaemonClient::new();
                    client.put_json(&format!("/api/v1/layouts/{id}"), &body_text)
                })
                .await;
            let read = cx
                .background_executor()
                .spawn(async move {
                    let client = DaemonClient::new();
                    client.get("/api/v1/layouts")
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(_) => {
                        this.base = Some(body.clone());
                        this.history.draft = Some(body);
                        if let Ok(text) = read {
                            if let Ok(parsed) = serde_json::from_str::<LayoutsResponse>(&text) {
                                this.layouts = Some(parsed);
                            }
                        }
                    }
                    Err(e) => {
                        this.save_error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn sync_server(&mut self, server: Layout) {
        if interact::is_dirty(self.history.draft.as_ref(), self.base.as_ref()) {
            return;
        }
        if self.drag.is_some() {
            return;
        }
        self.base = Some(server.clone());
        self.history.reset(server);
    }

    #[allow(clippy::collapsible_if)]
    pub fn select(&mut self, sel: Vec<Sel>) {
        let first = sel.first().copied();
        if let Some(id) = self.identifying {
            if !matches!(first, Some(Sel::Zone(i)) if i == id) {
                self.stop_identify();
            }
        }
        self.selection = sel;
    }

    pub fn stop_identify(&mut self) {
        let Some(index) = self.identifying else { return };
        self.identifying = None;
        let draft = self.history.draft.clone();
        if let Some(layout) = draft {
            if let Some(z) = layout.zones.get(index) {
                let device_id = z.device_id.0.clone();
                let zone = z.zone_index;
                std::thread::spawn(move || {
                    let client = DaemonClient::new();
                    let body = serde_json::json!({"zone": zone, "led": 0, "stop": true}).to_string();
                    let _ = client.post_json(
                        &format!("/api/v1/devices/{device_id}/identify"),
                        &body,
                    );
                });
            }
        }
    }

    #[allow(dead_code)]
    pub fn start_identify(&mut self, index: usize, cx: &mut Context<Self>) {
        self.stop_identify();
        let Some(layout) = self.history.draft.clone() else { return };
        let Some(z) = layout.zones.get(index).cloned() else { return };
        let device_id = z.device_id.0.clone();
        let zone_index = z.zone_index;
        let count = self
            .devices()
            .iter()
            .find(|d| d.id.0 == device_id)
            .and_then(|d| d.zones.iter().find(|zz| zz.index == zone_index))
            .map(|zz| zz.led_count.max(1))
            .unwrap_or(1);
        self.identifying = Some(index);
        let mut tick = 0u32;
        cx.spawn(async move |view, cx| {
            loop {
                if tick >= 40 {
                    break;
                }
                let did = device_id.clone();
                let led = tick % count;
                cx.background_executor()
                    .spawn(async move {
                        let client = DaemonClient::new();
                        let body =
                            serde_json::json!({"zone": zone_index, "led": led, "stop": false}).to_string();
                        let _ = client.post_json(
                            &format!("/api/v1/devices/{did}/identify"),
                            &body,
                        );
                    })
                    .await;
                tick += 1;
                let still = view
                    .update(cx, |this: &mut LayoutView, _cx| this.identifying == Some(index))
                    .unwrap_or(false);
                if !still {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(125))
                    .await;
            }
            let _ = view.update(cx, |this: &mut LayoutView, cx| {
                if this.identifying == Some(index) {
                    this.identifying = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn zoom_by(&mut self, factor: f64, desk: (f64, f64)) {
        let (w, h) = desk;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let centre = stage::content_centre(
            &self.history.draft.as_ref().map(|l| l.zones.clone()).unwrap_or_default(),
            &self.history.draft.as_ref().map(|l| l.landmarks.clone()).unwrap_or_default(),
        );
        let cur = self.view.unwrap_or_else(|| hero_view(w, h, centre));
        let base = w / geometry::CANVAS_W;
        let scale = interact::zoom_scale(cur.scale, factor);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let lx = cx / (base * cur.scale) - cur.pan_x;
        let ly = cy / (base * cur.scale) - cur.pan_y;
        let (scale, pan_x, pan_y) =
            interact::clamp_view(scale, cx / (base * scale) - lx, cy / (base * scale) - ly);
        self.view = Some(StageView { scale, pan_x, pan_y });
    }

    pub fn zoom_fit(&mut self, desk: (f64, f64)) {
        let (w, h) = desk;
        let (scale, pan_x, pan_y) = interact::whole_view(w, h);
        self.view = Some(StageView { scale, pan_x, pan_y });
    }

    pub fn wheel_zoom(&mut self, delta_y: f64, at_px: (f64, f64), desk: (f64, f64)) {
        let (w, h) = desk;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let centre = stage::content_centre(
            &self.history.draft.as_ref().map(|l| l.zones.clone()).unwrap_or_default(),
            &self.history.draft.as_ref().map(|l| l.landmarks.clone()).unwrap_or_default(),
        );
        let cur = self.view.unwrap_or_else(|| hero_view(w, h, centre));
        let base = w / geometry::CANVAS_W;
        let scale = interact::zoom_scale(cur.scale, interact::wheel_factor(delta_y));
        let (px_x, px_y) = at_px;
        let lx = px_x / (base * cur.scale) - cur.pan_x;
        let ly = px_y / (base * cur.scale) - cur.pan_y;
        let (scale, pan_x, pan_y) =
            interact::clamp_view(scale, px_x / (base * scale) - lx, px_y / (base * scale) - ly);
        self.view = Some(StageView { scale, pan_x, pan_y });
    }

    pub fn place_zone(&mut self, device_id: &str, zone_index: u32, at: Option<(f64, f64)>) {
        let info = self
            .devices()
            .iter()
            .find(|d| d.id.0 == device_id)
            .and_then(|d| d.zones.iter().find(|z| z.index == zone_index))
            .map(|z| (z.clone(), self.devices().iter().find(|d| d.id.0 == device_id).unwrap().kind));
        let Some((zone_info, device_kind)) = info else { return };
        let shape = geometry::default_shape(device_kind, &zone_info);
        let (w, h) = geometry::default_rect(&zone_info, device_kind, &shape);
        let placed = interact::placed_zone(device_id.to_string(), zone_index, shape, w, h, at);
        let index = self.history.draft.as_ref().map(|l| l.zones.len()).unwrap_or(0);
        self.commit(|l| Layout {
            zones: {
                let mut zones = l.zones.clone();
                zones.push(placed.clone());
                zones
            },
            ..l.clone()
        });
        self.selection = vec![Sel::Zone(index)];
    }

    pub fn add_landmark(&mut self, kind: PartKind) {
        let landmark = interact::landmark_at_centre(kind);
        let index = self.history.draft.as_ref().map(|l| l.landmarks.len()).unwrap_or(0);
        self.commit(|l| Layout {
            landmarks: {
                let mut landmarks = l.landmarks.clone();
                landmarks.push(landmark);
                landmarks
            },
            ..l.clone()
        });
        self.selection = vec![Sel::Landmark(index)];
    }

    /// Every part with lights is unconfirmed again (React `remapAll`).
    pub fn remap_all(&mut self) {
        self.commit(|l| {
            let mut l = l.clone();
            l.zones = l
                .zones
                .iter()
                .map(|z| {
                    let mut z = z.clone();
                    if let Some(p) = &mut z.part {
                        p.mapped = false;
                    }
                    z
                })
                .collect();
            l
        });
        self.selection.clear();
        self.onboarding = Some(crate::views::onboarding::OnboardData::new(true, None));
        self.sync_onboarding_match_run();
    }

    /// One device again, from its row (React `remapOne`).
    #[allow(dead_code)]
    pub fn remap_one(&mut self, zone_index: usize) {
        let Some(draft) = self.history.draft.clone() else { return };
        let Some(z) = draft.zones.get(zone_index) else { return };
        let scope = crate::views::onboarding::Scope { device_id: z.device_id.0.clone(), zone_index: z.zone_index };
        self.commit(|l| {
            let mut l = l.clone();
            l.zones = l
                .zones
                .iter()
                .map(|p| {
                    let mut p = p.clone();
                    if p.device_id.0 == scope.device_id && p.zone_index == scope.zone_index && p.part.is_some() {
                        if let Some(pt) = &mut p.part {
                            pt.mapped = false;
                        }
                    }
                    p
                })
                .collect();
            l
        });
        self.selection.clear();
        self.onboarding = Some(crate::views::onboarding::OnboardData::new(true, Some(scope)));
        self.sync_onboarding_match_run();
    }

    /// Open the onboarding without clearing `mapped` (first run / Settings).
    #[allow(dead_code)]
    pub fn open_onboarding_with(&mut self, again: bool, scope: Option<crate::views::onboarding::Scope>) {
        self.onboarding = Some(crate::views::onboarding::OnboardData::new(again, scope));
        self.sync_onboarding_match_run();
    }

    /// Close the flow, save if dirty (React `closeOnboarding`).
    pub fn close_onboarding(&mut self, cx: &mut Context<Self>) {
        self.stop_identify();
        self.selection.clear();
        self.onboarding = None;
        if self.dirty() && !self.saving {
            self.save(cx);
        }
        cx.notify();
    }

    /// "Set up later" (React `laterOnboarding`).
    pub fn later_onboarding(&mut self, cx: &mut Context<Self>) {
        self.later_this_launch = true;
        self.close_onboarding(cx);
    }

    /// The case box the flow sizes (React `ensureCase`).
    pub fn ensure_case(&mut self) {
        let (w, h) = (120.0f32, 122.0f32);
        let lms = self.history.draft.as_ref().map(|l| l.landmarks.clone()).unwrap_or_default();
        let found = lms.iter().position(|l| l.kind == PartKind::Case);
        let idx = match found {
            Some(i) => i,
            None => {
                let i = lms.len();
                self.commit(|l| {
                    let mut l = l.clone();
                    l.landmarks.push(canvas_proto::layout::Landmark {
                        kind: PartKind::Case,
                        x: 100.0,
                        y: 27.0,
                        width: w,
                        height: h,
                        rotation_deg: 0.0,
                        facing: canvas_proto::layout::Facing::Viewer,
                        visible: true,
                        label: None,
                        locked: false,
                    });
                    l
                });
                i
            }
        };
        self.selection = vec![Sel::Landmark(idx)];
    }

    /// Sets the case's box, standing on the same floor line (Onboarding.tsx `setCaseBox`).
    pub fn set_case_box(&mut self, width: Option<f32>, height: Option<f32>, locked: Option<bool>) {
        self.commit(|l| {
            let mut l = l.clone();
            l.landmarks = l
                .landmarks
                .iter()
                .map(|m| {
                    let mut m = m.clone();
                    if m.kind != PartKind::Case {
                        return m;
                    }
                    if let (Some(w), Some(h)) = (width, height) {
                        m.width = w;
                        m.height = h;
                        m.x = (crate::views::onboarding::CASE_ANCHOR_X - f64::from(w) / 2.0) as f32;
                        m.y = (crate::views::onboarding::CASE_ANCHOR_Y - f64::from(h)) as f32;
                    }
                    if let Some(locked) = locked {
                        m.locked = locked;
                    }
                    m
                })
                .collect();
            l
        });
    }

    pub fn set_case_locked(&mut self, locked: bool) {
        let cur = self
            .history
            .draft
            .as_ref()
            .and_then(|l| l.landmarks.iter().find(|m| m.kind == PartKind::Case).cloned());
        if let Some(cur) = cur {
            if cur.locked == locked {
                return;
            }
        }
        self.set_case_box(None, None, Some(locked));
    }

    /// Locate the Match run at/after (zi, off) (MatchStep.tsx).
    pub fn sync_onboarding_match_run(&mut self) {
        let Some(data) = self.onboarding.as_ref() else { return };
        if data.step() != crate::views::onboarding::StepKind::Match {
            return;
        }
        let layout = self.history.draft.clone();
        let devices = self.devices.clone();
        let scope = data.scope.clone();
        let Some(layout) = layout else { return };
        let zones = {
            let mut zones = crate::views::onboarding::setup_zones(&layout, devices.as_deref());
            zones.retain(|z| z.led_count > 0 && scope.as_ref().is_none_or(|s| z.device_id == s.device_id && z.zone_index == s.zone_index));
            zones
        };
        let run = crate::views::onboarding::locate_run(&layout, &zones, &std::collections::HashSet::new(), 0, 0);
        if let Some(data) = self.onboarding.as_mut() {
            data.count = run.as_ref().map(|r| r.count).unwrap_or(0);
            data.run = run;
        }
    }

    /// Sync the Match count from the current run.
    pub fn onboarding_match_zones(&self) -> Vec<crate::views::onboarding::SetupZone> {
        let Some(layout) = self.history.draft.clone() else { return Vec::new() };
        let devices = self.devices.clone();
        let scope = self.onboarding.as_ref().and_then(|d| d.scope.clone());
        let mut zones = crate::views::onboarding::setup_zones(&layout, devices.as_deref());
        zones.retain(|z| z.led_count > 0 && scope.as_ref().is_none_or(|s| z.device_id == s.device_id && z.zone_index == s.zone_index));
        zones
    }

    /// Whether the onboarding is open (the shell hides the rail/strip).
    pub fn is_onboarding_open(&self) -> bool {
        self.onboarding.is_some()
    }

    /// Mirror a React demo `?state=` token for capture pairs (debug only).
    #[cfg(debug_assertions)]
    fn apply_capture_state(&mut self) {
        let state = self.capture_state.clone().unwrap_or_default();
        let mut base = self.status.clone().unwrap_or(canvas_proto::api::Status {
            version: "0.0.0".into(),
            power: canvas_proto::api::Power::Running,
            brightness: 100,
            fps_target: 60,
            fps_measured: 59.6,
            frame_time_ms: 16.7,
            frames_rendered: 100,
            parked: false,
            devices_connected: 0,
            devices_total: 0,
            preview_subscribers: 0,
            active_layout: None,
            active_effect: None,
            openrgb: canvas_proto::api::LinkState::Connected,
            service: canvas_proto::api::LinkState::Connected,
            service_down_ms: None,
            conflicts: Vec::new(),
            show_zone_panel: false,
            onboarding_complete: false,
        });
        base.onboarding_complete = state == "complete";
        match state.as_str() {
            "connecting" | "waking" => base.openrgb = canvas_proto::api::LinkState::Connecting,
            "openrgb-down" => base.openrgb = canvas_proto::api::LinkState::Down,
            "conflict" | "conflict-clears" => {
                base.conflicts = vec![canvas_proto::api::Conflict::SignalRgb { process: true, service: false }];
            }
            _ => {}
        }
        self.status = Some(base);
        if state == "no-devices" || state == "found-on-rescan" {
            self.devices = Some(Vec::new());
        }
    }

    /// The one-time open decision (LayoutView.tsx:733-756): once the layout,
    /// devices and status are all in, open when `shouldOpen` holds.
    pub fn try_open_onboarding(&mut self, cx: &mut Context<Self>) {
        if self.onboard_decided || self.onboarding.is_some() {
            return;
        }
        let Some(layout) = self.layout().cloned() else { return };
        let _ = layout;
        if self.devices.is_none() || self.status.is_none() {
            return;
        }
        // Debug capture: the gate opens regardless of daemon truth.
        #[cfg(debug_assertions)]
        if self.capture_step.is_some() {
            self.onboard_decided = true;
            // React `?state=complete`: no onboarding — the Layout page alone.
            if self.capture_state.as_deref() == Some("complete") {
                cx.notify();
                return;
            }
            self.apply_capture_state();
            let again = self.capture_step.as_deref() != Some("hello");
            let step = match self.capture_step.as_deref() {
                Some("case") => crate::views::onboarding::StepKind::Case,
                Some("parts") => crate::views::onboarding::StepKind::Parts,
                Some("match") => crate::views::onboarding::StepKind::Match,
                Some("finish") | Some("finish-sweep") => crate::views::onboarding::StepKind::Finish,
                _ => crate::views::onboarding::StepKind::Hello,
            };
            let mut data = crate::views::onboarding::OnboardData::new(again, None);
            let at = match step {
                crate::views::onboarding::StepKind::Hello => 0,
                crate::views::onboarding::StepKind::Case => 1,
                crate::views::onboarding::StepKind::Parts => 2,
                crate::views::onboarding::StepKind::Match => 3,
                crate::views::onboarding::StepKind::Finish => 4,
            };
            if at < data.steps.len() {
                data.at = at;
            }
            // React's capture of the Finish screen is taken while the sweep is
            // still crossing (the sweep takes three seconds and the shot is
            // about one second after the screen arrives), so the screen it
            // shows is "Watch your PC." with the band on the bar and the Done
            // key disabled. The swept screen ("Your PC is mapped." with the
            // three tiles) is what the owner sees three seconds later.
            if step == crate::views::onboarding::StepKind::Finish {
                data.swept = false;
            }
            self.onboarding = Some(data);
            self.sync_onboarding_match_run();
            self.poll_status_effects(cx);
            cx.notify();
            return;
        }
        self.onboard_decided = true;
        let complete = self.status.as_ref().map(|s| s.onboarding_complete).unwrap_or(true);
        if crate::views::onboarding::should_open(complete, self.later_this_launch) {
            self.onboarding = Some(crate::views::onboarding::OnboardData::new(false, None));
            self.sync_onboarding_match_run();
            self.poll_status_effects(cx);
            cx.notify();
        }
    }

    /// Poll status and effects for the onboarding (off the UI thread).
    pub fn poll_status_effects(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            (client.get("/api/v1/status"), client.get("/api/v1/effects"))
        });
        cx.spawn(async move |view, cx| {
            let (status, effects) = bg.await;
            let _ = view.update(cx, |this, cx| {
                if let Ok(body) = status {
                    if let Ok(parsed) = serde_json::from_str::<canvas_proto::api::Status>(&body) {
                        this.status = Some(parsed);
                    }
                }
                if let Ok(body) = effects {
                    if let Ok(parsed) = serde_json::from_str::<canvas_proto::api::EffectsResponse>(&body) {
                        this.effects = Some(parsed);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Advance the onboarding by one (Case/Parts/Match "next").
    pub fn goto_next(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.onboarding.as_mut() {
            d.next_step();
        }
        self.sync_onboarding_match_run();
        self.poll_status_effects(cx);
        cx.notify();
    }

    /// Ladder/Parts Rescan (HelloStep/PartsStep `rescan`).
    pub fn start_rescan(&mut self, cx: &mut Context<Self>) {
        let Some(data) = self.onboarding.as_ref() else { return };
        if data.busy {
            return;
        }
        if let Some(d) = self.onboarding.as_mut() {
            d.busy = true;
        }
        cx.notify();
        let bg = cx.background_executor().spawn(async move { crate::views::onboarding::rescan_devices() });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                if let Some(d) = this.onboarding.as_mut() {
                    d.asked = true;
                    d.busy = false;
                }
                if let Ok(devices) = result {
                    this.devices = Some(devices);
                }
                this.poll_status_effects(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Parts: add a lit part on the first run nothing owns (PartsStep `addLit`).
    pub fn add_lit_part(&mut self, kind: PartKind, cx: &mut Context<Self>) {
        let Some(layout) = self.history.draft.clone() else { return };
        let devices = self.devices.clone();
        let zones = crate::views::onboarding::setup_zones(&layout, devices.as_deref())
            .into_iter()
            .filter(|z| z.led_count > 0)
            .collect::<Vec<_>>();
        let spare = crate::views::onboarding::all_runs(&zones).into_iter().find(|r| {
            let zone = &zones[r.zi];
            crate::views::onboarding::placements_of_run(&layout, zone, r.start, r.count).is_empty()
        });
        let Some(spare) = spare else {
            if let Some(d) = self.onboarding.as_mut() {
                d.note = Some("Every light already has a part. Drag one, or rescan.".to_string());
            }
            cx.notify();
            return;
        };
        let zone = zones[spare.zi].clone();
        let template = canvas_proto::layout::PlacedZone {
            device_id: canvas_proto::device::DeviceId(zone.device_id.clone()),
            zone_index: zone.zone_index,
            x: 0.0,
            y: 0.0,
            width: 30.0,
            height: 30.0,
            rotation_deg: 0.0,
            reversed: false,
            visible: true,
            shape: crate::views::onboarding::shape_for_kind_pub(kind),
            label: None,
            leds: if spare.whole { None } else { Some(canvas_proto::layout::LedSlice { start: spare.start, count: spare.count }) },
            part: Some(canvas_proto::layout::Part { kind, facing: canvas_proto::layout::Facing::Viewer, mapped: false }),
            locked: false,
        };
        let index = layout.zones.len();
        self.commit(|l| {
            let mut l = l.clone();
            let box_ = crate::views::onboarding::place_for_part(&l, kind, &template);
            l.zones.push(canvas_proto::layout::PlacedZone {
                x: box_.0 as f32,
                y: box_.1 as f32,
                width: box_.2 as f32,
                height: box_.3 as f32,
                ..template.clone()
            });
            l
        });
        self.selection = vec![Sel::Zone(index)];
        if let Some(d) = self.onboarding.as_mut() {
            d.note = None;
        }
        cx.notify();
    }

    /// Parts: remove the selected parts (PartsStep `remove`).
    pub fn remove_selected_parts(&mut self) {
        let layout = self.history.draft.clone();
        let Some(layout) = layout else { return };
        let removable: Vec<Sel> = self
            .selection
            .iter()
            .copied()
            .filter(|s| match s {
                Sel::Zone(_) => true,
                Sel::Landmark(i) => layout.landmarks.get(*i).map(|l| l.kind) != Some(PartKind::Case),
            })
            .collect();
        let zi: std::collections::HashSet<usize> = removable.iter().filter_map(|s| match s { Sel::Zone(i) => Some(*i), _ => None }).collect();
        let li: std::collections::HashSet<usize> = removable.iter().filter_map(|s| match s { Sel::Landmark(i) => Some(*i), _ => None }).collect();
        self.commit(|l| {
            let mut l = l.clone();
            let mut zones = Vec::new();
            for (i, z) in l.zones.iter().enumerate() {
                if !zi.contains(&i) {
                    zones.push(z.clone());
                }
            }
            l.zones = zones;
            let mut landmarks = Vec::new();
            for (i, lm) in l.landmarks.iter().enumerate() {
                if !li.contains(&i) {
                    landmarks.push(lm.clone());
                }
            }
            l.landmarks = landmarks;
            l
        });
        self.selection.clear();
    }

    /// Match: tap a part (MatchStep `assign`).
    #[allow(dead_code)]
    pub fn match_assign(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(data) = self.onboarding.clone() else { return };
        let Some(run) = data.run.clone() else { return };
        let zones = self.onboarding_match_zones();
        let Some(zone) = zones.get(run.zi).cloned() else { return };
        let slice = if run.whole { None } else { Some(canvas_proto::layout::LedSlice { start: run.start, count: data.count }) };
        let target = self.history.draft.as_ref().and_then(|l| l.zones.get(index).cloned());
        let Some(target) = target else { return };
        if target.part.map(|p| p.mapped).unwrap_or(false) {
            return;
        }
        self.commit(|l| {
            let mut l = l.clone();
            let other = {
                let placements = crate::views::onboarding::placements_of_run(&l, &zone, run.start, data.count);
                placements.into_iter().find(|i| *i != index && !l.zones[*i].part.map(|p| p.mapped).unwrap_or(false))
            };
            let old = (target.device_id.clone(), target.zone_index, target.leds);
            if let Some(z) = l.zones.get_mut(index) {
                z.device_id = canvas_proto::device::DeviceId(zone.device_id.clone());
                z.zone_index = zone.zone_index;
                z.leds = slice;
                if let Some(p) = &mut z.part {
                    p.mapped = true;
                } else {
                    z.part = Some(canvas_proto::layout::Part { kind: PartKind::Generic, facing: canvas_proto::layout::Facing::Viewer, mapped: true });
                }
            }
            if let Some(other) = other {
                if let Some(z) = l.zones.get_mut(other) {
                    z.device_id = old.0.clone();
                    z.zone_index = old.1;
                    z.leds = old.2;
                }
            }
            l
        });
        if let Some(d) = self.onboarding.as_mut() {
            d.tapped = Some(index);
            d.focus = Some(index);
        }
        // Identify the run (MatchStep.tsx:60-79).
        if !self.capture_frozen {
            crate::views::onboarding::identify(&zone.device_id, zone.zone_index, run.start, if data.count > 1 { Some(data.count) } else { None }, false);
        }
        cx.notify();
    }

    /// Match: "Yes"/"That's all of it" (MatchStep `advance`).
    pub fn advance_match(&mut self, cx: &mut Context<Self>) {
        let (run, zones, count) = match self.onboarding.clone() {
            Some(d) => (d.run.clone(), self.onboarding_match_zones(), d.count),
            None => return,
        };
        let Some(run) = run else { return };
        let layout = self.history.draft.clone();
        let Some(layout) = layout else { return };
        let next = crate::views::onboarding::locate_run(&layout, &zones, &std::collections::HashSet::new(), run.zi, run.start + count);
        let next_count = next.as_ref().map(|r| r.count).unwrap_or(0);
        if let Some(d) = self.onboarding.as_mut() {
            d.tapped = None;
            d.resizing = false;
            d.focus = None;
            d.run = next;
            d.count = next_count;
        }
        cx.notify();
    }

    /// Match: "No" on a fixed run (MatchStep `undo`).
    pub fn undo_match_tap(&mut self) {
        let Some(tapped) = self.onboarding.as_ref().and_then(|d| d.tapped) else { return };
        self.commit(|l| {
            let mut l = l.clone();
            if let Some(z) = l.zones.get_mut(tapped) {
                if let Some(p) = &mut z.part {
                    p.mapped = false;
                }
            }
            l
        });
        if let Some(d) = self.onboarding.as_mut() {
            d.tapped = None;
            d.resizing = false;
            d.focus = None;
        }
    }

    /// Match: "Nothing's lit" (MatchStep skip).
    pub fn skip_match_run(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.onboarding.clone() else { return };
        let Some(run) = d.run.clone() else { return };
        let zones = self.onboarding_match_zones();
        let Some(zone) = zones.get(run.zi).cloned() else { return };
        let _key = crate::views::onboarding::run_key(&zone, run.start);
        // Skipped runs stay todo (no persistent skipped set on the onboarding
        // shell — React keeps a local set; the next run advances past this one).
        let layout = self.history.draft.clone();
        let Some(layout) = layout else { return };
        let mut skipped = std::collections::HashSet::new();
        skipped.insert(_key);
        let next = crate::views::onboarding::locate_run(&layout, &zones, &skipped, run.zi, run.start + run.count);
        let next_count = next.as_ref().map(|r| r.count).unwrap_or(0);
        if let Some(d) = self.onboarding.as_mut() {
            d.tapped = None;
            d.resizing = false;
            d.focus = None;
            d.run = next;
            d.count = next_count;
        }
        cx.notify();
    }

    /// Match: Fewer/More lights (MatchStep `resize`).
    pub fn resize_match_count(&mut self, n: u32) {
        let (run, tapped) = match self.onboarding.clone() {
            Some(d) => (d.run.clone(), d.tapped),
            None => return,
        };
        let (Some(run), Some(tapped)) = (run, tapped) else { return };
        let zones = self.onboarding_match_zones();
        let Some(zone) = zones.get(run.zi).cloned() else { return };
        let clamped = n.clamp(1, zone.max_leds.min(zone.led_count.saturating_sub(run.start).max(1)));
        if let Some(d) = self.onboarding.as_mut() {
            d.count = clamped;
        }
        self.commit(|l| {
            let mut l = l.clone();
            if let Some(z) = l.zones.get_mut(tapped) {
                z.leds = Some(canvas_proto::layout::LedSlice { start: run.start, count: clamped });
            }
            l
        });
    }

    /// Finish: pick a tile (Onboarding.tsx `choose`).
    pub fn choose_effect(&mut self, id: &str) {
        if let Some(d) = self.onboarding.as_mut() {
            d.before_effect = None;
            d.picked = Some(id.to_string());
        }
        crate::views::onboarding::activate_effect(id, &canvas_proto::props::Props::default());
    }

    /// Finish: "Done" (Onboarding.tsx `finish`).
    pub fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        let picked = self.onboarding.as_ref().and_then(|d| d.picked.clone());
        if picked.is_none() {
            // restore the replaced effect
            let before = self.onboarding.as_mut().and_then(|d| d.before_effect.take());
            if let Some(Some((id, props))) = before {
                crate::views::onboarding::activate_effect(&id, &props);
            }
        }
        crate::views::onboarding::complete_onboarding();
        let devices = self.devices.clone();
        let _ = devices;
        self.poll_status_effects(cx);
        self.close_onboarding(cx);
    }

    /// The devices, or none yet.
    pub fn devices(&self) -> &[DeviceInfo] {
        self.devices.as_deref().unwrap_or(&[])
    }

    /// `GET /api/v1/layouts` and `GET /api/v1/devices`, off the UI thread.
    fn load(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            let layouts = client.get("/api/v1/layouts");
            let devices = client.get("/api/v1/devices");
            (layouts, devices)
        });
        cx.spawn(async move |view, cx| {
            let (layouts, devices) = bg.await;
            let _ = view.update(cx, |this, cx| {
                match layouts {
                    Ok(body) => match serde_json::from_str::<LayoutsResponse>(&body) {
                        Ok(parsed) => {
                            let server = parsed
                                .layouts
                                .iter()
                                .find(|l| l.id == parsed.active)
                                .or_else(|| parsed.layouts.first())
                                .cloned();
                            this.layouts = Some(parsed);
                            this.layouts_error = None;
                            if let Some(server) = server {
                                this.sync_server(server);
                            }
                        }
                        Err(e) => this.layouts_error = Some(e.to_string()),
                    },
                    Err(e) => this.layouts_error = Some(e.to_string()),
                }
                if let Ok(body) = devices {
                    if let Ok(parsed) = serde_json::from_str::<DevicesResponse>(&body) {
                        this.devices = Some(parsed.devices);
                    }
                }
                cx.notify();
            });
            let timer = cx
                .background_executor()
                .timer(std::time::Duration::from_secs(4));
            timer.await;
            let _ = view.update(cx, |this, cx| {
                this.load(cx);
            });
        })
        .detach();
    }

    /// The live preview feed: the daemon's WebSocket, read in the background.
    fn start_feed(&self) {
        let port = crate::client::api_port();
        let Some(token) = crate::client::api_token() else {
            return;
        };
        preview::spawn(port, token, self.feed.clone());
    }

    /// Watches the feed: the desk is drawn again when a frame arrives, the
    /// way React repaints its overlay for every frame.
    fn watch_feed(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            loop {
                let timer = cx
                    .background_executor()
                    .timer(std::time::Duration::from_millis(100));
                timer.await;
                let _ = view.update(cx, |this, cx| {
                    let count = this.feed.count();
                    if count != this.frames {
                        this.frames = count;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// The page's name and its one-line context, for the strip.
    pub fn strip_title(&self) -> (String, Option<String>) {
        let name = self.layout().map(|l| l.name.clone());
        ("Layout".to_string(), name)
    }

    /// The strip's page actions (React `PageActions`).
    pub fn strip_actions(view: &LayoutView, cx: &mut Context<LayoutView>) -> gpui::AnyElement {
        view.actions_row(cx).into_any_element()
    }

    /// The page's own actions: undo and redo, the zone count, the save state,
    /// Revert, Save and "Map my PC again".
    fn actions_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let narrow = self.width <= 1439.0;
        let no_count = self.width <= 1099.0;
        let phone = self.width <= 719.0;
        let (visible, total) = match self.layout() {
            Some(l) => (
                l.zones.iter().filter(|z| z.visible).count(),
                l.zones.len(),
            ),
            None => (0, 0),
        };
        let dirty = self.dirty();
        let saving = self.saving;
        let can_save = dirty && !saving;
        let entity = cx.entity();
        let undo_entity = entity.clone();
        let redo_entity = entity.clone();
        let revert_entity = entity.clone();
        let save_entity = entity.clone();
        let remap_entity = entity;
        let mut row = div().flex().items_center().gap_2();
        if !phone {
            let past_empty = self.history.past.is_empty();
            let future_empty = self.history.future.is_empty();
            row = row
                .child(icon_button(
                    &t, "lay-undo", "ArrowUUpLeft", "", true, false, past_empty,
                    move |_, _window, app| {
                        undo_entity.update(app, |this: &mut LayoutView, cx| {
                            this.undo();
                            cx.notify();
                        });
                    },
                ))
                .child(icon_button(
                    &t, "lay-redo", "ArrowUUpRight", "", true, false, future_empty,
                    move |_, _window, app| {
                        redo_entity.update(app, |this: &mut LayoutView, cx| {
                            this.redo();
                            cx.notify();
                        });
                    },
                ));
        }
        if !no_count {
            row = row.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(format!("{visible}/{total} zones")),
            );
        }
        row = row
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(if dirty {
                        Theme::color(t.warn)
                    } else {
                        Theme::color(t.mute)
                    })
                    .child(if dirty { "Unsaved" } else { "Saved" }),
            )
            .child(button_icon(
                &t,
                "lay-revert",
                if phone { "" } else { "Revert" },
                Variant::Ghost,
                !can_save,
                true,
                "ArrowCounterClockwise",
                "",
                16.0,
                move |_, _window, app| {
                    revert_entity.update(app, |this: &mut LayoutView, cx| {
                        this.revert();
                        cx.notify();
                    });
                },
            ))
            .child(button_icon(
                &t,
                "lay-save",
                if phone {
                    ""
                } else if saving {
                    "Saving"
                } else {
                    "Save layout"
                },
                Variant::Primary,
                !can_save,
                true,
                "FloppyDisk",
                "bold",
                16.0,
                move |_, _window, app| {
                    save_entity.update(app, |this: &mut LayoutView, cx| {
                        this.save(cx);
                    });
                },
            ));
        if !phone {
            row = row.child(button_icon(
                &t,
                "lay-remap",
                if narrow { "" } else { "Map my PC again" },
                Variant::Ghost,
                false,
                true,
                "CompassTool",
                "",
                16.0,
                move |_, _window, app| {
                    remap_entity.update(app, |this: &mut LayoutView, cx| {
                        this.remap_all();
                        this.poll_status_effects(cx);
                        cx.notify();
                    });
                },
            ));
        }
        row
    }
}

/// The unit this machine's region uses, until the owner picks one
/// (`systemUnit` in `lib/zones.ts`).
pub fn system_unit() -> LengthUnit {
    locale_region()
        .map(|region| LengthUnit::for_region(&region))
        .unwrap_or(LengthUnit::Cm)
}

/// The user's region, as Windows has it ("en-US").
fn locale_region() -> Option<String> {
    #[cfg(windows)]
    {
        const CAPACITY: i32 = 85;
        let mut buf = [0u16; CAPACITY as usize];
        // SAFETY: `buf` is CAPACITY wide, which is what the call is told,
        // and the call writes at most that many code units into it.
        let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), CAPACITY) };
        if len <= 0 {
            return None;
        }
        // The return includes the null terminator: "en-US" is 6 with the
        // terminating zero, and the region must not carry it.
        let mut end = len as usize;
        if end > 0 && buf[end - 1] == 0 {
            end -= 1;
        }
        let name = String::from_utf16_lossy(&buf[..end]);
        // The region is the tag after the language: "en-US" -> "US".
        let parts: Vec<&str> = name.split(['-', '_']).collect();
        let region = parts
            .get(1)
            .filter(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))?;
        Some(region.to_string())
    }
    #[cfg(not(windows))]
    None
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultLocaleName(lpLocaleName: *mut u16, cchLocaleName: i32) -> i32;
}

/// The page's own container width: the content box, which is what the page's
/// container queries measure.
pub fn container_width(window_width: f32) -> f32 {
    let phone = window_width <= 720.0;
    let rail = if phone {
        0.0
    } else if window_width < 1280.0 {
        64.0
    } else {
        200.0
    };
    let pad = if phone { 16.0 } else { 24.0 };
    window_width - rail - pad * 2.0
}

/// The inspector's width, by the page's container queries.
pub fn inspector_width(container: f32) -> f32 {
    if container >= 1500.0 {
        360.0
    } else if container <= 999.0 {
        280.0
    } else {
        320.0
    }
}

/// The desk's own box, in css pixels: the size the desk's view is measured
/// from.
///
/// The desk fills the frame, and the frame fills the desk's column of the
/// page's grid. On a phone the column is 46 vh tall and the panes below the
/// desk are the switcher and the bar; on a desktop the column is the page's
/// own height and the bar is one row.
pub fn desk_size(
    _window_w: f32,
    window_h: f32,
    container: f32,
    inspector_w: f32,
    one_column: bool,
) -> (f32, f32) {
    let width = if one_column {
        container - 2.0
    } else {
        container - inspector_w - 16.0 - 2.0
    };
    let height = if one_column {
        // The desk's column is 46 vh; the bar wraps to two rows under it.
        window_h * 0.46 - 32.0 - 2.0 - 91.0
    } else {
        // The strip, the page's padding and the bar.
        window_h - 55.0 - 16.0 - 24.0 - 2.0 - 51.0
    };
    (width, height)
}

/// One zone that is not on the desk yet.
pub struct Unplaced {
    pub device_id: String,
    pub device_name: String,
    pub zone_index: u32,
    pub zone_name: String,
    pub led_count: u32,
    pub connected: bool,
}

/// The zones that are not on the desk yet (`unplaced` in LayoutView).
pub fn unplaced(layout: Option<&Layout>, devices: &[DeviceInfo]) -> Vec<Unplaced> {
    let mut out = Vec::new();
    let Some(layout) = layout else {
        return out;
    };
    let placed: Vec<String> = layout
        .zones
        .iter()
        .map(|z| format!("{}#{}", z.device_id.0, z.zone_index))
        .collect();
    for device in devices {
        for zone in &device.zones {
            if placed.contains(&format!("{}#{}", device.id.0, zone.index)) {
                continue;
            }
            out.push(Unplaced {
                device_id: device.id.0.clone(),
                device_name: device.name.clone(),
                zone_index: zone.index,
                zone_name: zone.name.clone(),
                led_count: zone.led_count,
                connected: device.connected,
            });
        }
    }
    out
}

/// The icon of a kind of device (`KindIcon` in `components/KindIcon.tsx`).
pub fn kind_icon(kind: Option<DeviceKind>, fan: bool) -> &'static str {
    if fan {
        return "Fan";
    }
    match kind {
        Some(DeviceKind::Dram) => "Memory",
        Some(DeviceKind::Keyboard) => "Keyboard",
        Some(DeviceKind::Mouse) => "Mouse",
        Some(DeviceKind::Microphone) => "Microphone",
        Some(DeviceKind::Headset) => "Headphones",
        Some(DeviceKind::Gpu) => "GraphicsCard",
        Some(DeviceKind::Motherboard) => "Cpu",
        _ => "Lightbulb",
    }
}

/// The row of a part in the parts list.
pub struct PartRow {
    pub name: String,
    pub sub: String,
    pub leds: Option<u32>,
    pub icon: &'static str,
    pub sel: Sel,
    pub width_text: Option<String>,
}

/// The parts list: the zones, then the landmarks.
#[allow(dead_code)]
pub fn part_rows(layout: Option<&Layout>, devices: &[DeviceInfo]) -> Vec<PartRow> {
    part_rows_unit(layout, devices, LengthUnit::Cm)
}

pub fn part_rows_unit(
    layout: Option<&Layout>,
    devices: &[DeviceInfo],
    unit: LengthUnit,
) -> Vec<PartRow> {
    let mut out = Vec::new();
    let Some(layout) = layout else {
        return out;
    };
    for (zi, z) in layout.zones.iter().enumerate() {
        let device = devices.iter().find(|d| d.id.0 == z.device_id.0);
        let info = device.and_then(|d| d.zones.iter().find(|zone| zone.index == z.zone_index));
        let plain = geometry::plain_name(devices, &z.device_id.0, z.zone_index);
        let friendly = geometry::placement_name(devices, z, &layout.zones);
        let leds = z
            .leds
            .as_ref()
            .map(|s| s.count)
            .or_else(|| info.map(|zone| zone.led_count));
        let device_name = device.map(|d| d.name.clone());
        let mut parts: Vec<String> = Vec::new();
        if !friendly.is_empty() {
            parts.push(plain.name.clone());
        }
        if let Some(name) = device_name {
            if !parts.contains(&name) {
                parts.push(name);
            }
        }
        out.push(PartRow {
            name: if friendly.is_empty() {
                plain.name.clone()
            } else {
                friendly
            },
            sub: parts.join(" · "),
            leds,
            icon: kind_icon(plain.kind, plain.fan),
            sel: Sel::Zone(zi),
            width_text: None,
        });
    }
    for (li, l) in layout.landmarks.iter().enumerate() {
        let name = l
            .label
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| interact::part_label(l.kind).to_string());
        out.push(PartRow {
            name,
            sub: "no lights of its own".to_string(),
            leds: None,
            icon: "",
            sel: Sel::Landmark(li),
            width_text: Some(geometry::format_units(l.width as f64, unit)),
        });
    }
    out
}

/// The page, at rest.
pub fn render_page(
    view: &mut LayoutView,
    window: &mut Window,
    cx: &mut Context<LayoutView>,
) -> gpui::AnyElement {
    let t = view.theme;
    let size = window.viewport_size();
    view.width = f32::from(size.width);
    view.height = f32::from(size.height);
    let container = container_width(view.width);
    let phone = view.width <= 720.0;
    let one_column = container <= 639.0;
    let inspector_w = inspector_width(container);
    let colours = DeskTokens::for_theme(&t);
    let layout = view.layout().cloned();
    let devices = view.devices().to_vec();

    let page = div().flex().flex_1().flex_col().gap_4().min_h_0().h_full();

    // The onboarding takeover (LayoutView.tsx:971): the window is the
    // onboarding alone — the stage over the preview, one panel beside it.
    if view.onboarding.is_some() {
        let layout = view.layout().cloned();
        if layout.is_some() {
            let page = div().flex().flex_1().flex_col().min_h_0().h_full().min_w_0();
            return page.child(crate::views::onboarding::render_onboarding(view, window, cx)).into_any_element();
        }
    }

    let Some(layout) = layout else {
        // Nothing to draw yet: the page says it is reading the layout.
        return page.child(reading(t)).into_any_element();
    };

    // The desk, in its frame, with the bar along the frame's bottom edge.
    let desk_input = DeskInput {
        zones: layout.zones.clone(),
        landmarks: layout.landmarks.clone(),
        case_form: layout.case_form,
        devices: devices.clone(),
        unit: view.unit,
        mode: view.mode,
        sampling: layout.sampling,
        frame: view.feed.frame(),
        selection: view.selection.clone(),
        view: view.view,
        overlay: view.overlay.clone(),
        unlock_badge: false,
    };
    let desk_entity = cx.entity();
    let desk_origin = view.desk_origin.clone();
    let desk_el = desk_with_hint(&t, &colours, desk_input, desk_origin, true);
    let desk_el = desk_handlers(desk_el, desk_entity);
    let frame = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .r_lg()
        .bg(Theme::color(t.sunk))
        .overflow_hidden()
        .child(desk_el)
        .child(stage_toolbar(view, &t, container, phone, cx));
    let mut stage_column = if one_column {
        // The stage column is 46 vh tall on a phone (`.rd-lay-stage` at the
        // 639 px container query); the frame inside it takes the rest. It
        // cannot carry `flex_1`: that sets a zero flex basis, and the
        // explicit height would lose.
        div().flex().flex_col().min_w_0().min_h_0().child(frame)
    } else {
        div().flex().flex_col().flex_1().min_w_0().min_h_0().child(frame)
    };
    if one_column {
        stage_column = stage_column.flex_none().flex_basis(px(view.height * 0.46));
    }
    if one_column {
        stage_column = stage_column.child(phone_switcher(view, &t, cx));
    }

    // The inspector: the unfinished work first, then the parts. On a phone
    // it is the full width of the page, not a fixed column.
    let inspector = if one_column {
        div().flex().flex_col().gap_4().min_w_0().min_h_0().w_full()
    } else {
        div().flex().flex_col().gap_4().min_w_0().min_h_0().w(px(inspector_w))
    }
    .child(if one_column && view.phone_pane != PhonePane::Place {
            div().hidden().into_any_element()
        } else {
            place_panel(view, &t, &devices, &layout, cx)
        })
        .child(if one_column && view.phone_pane != PhonePane::Parts {
            div().hidden().into_any_element()
        } else {
            parts_panel(view, &t, &devices, &layout, one_column, cx, window)
        });
    let inspector = if one_column && view.phone_pane == PhonePane::Render {
        inspector.child(render_panel(view, &t, cx))
    } else {
        inspector
    };

    let grid = if one_column {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_h_0()
            .child(stage_column)
            .child(inspector)
    } else {
        div()
            .flex()
            .gap_4()
            .flex_1()
            .min_h_0()
            .child(stage_column)
            .child(inspector)
    };
    page.child(grid).into_any_element()
}

/// The desk's pointer and wheel handlers: picking, dragging, rubber band,
/// marquee, middle pan and wheel zoom (PreviewStage's stage handlers).
/// Used by the Layout page and the onboarding takeover (same stage).
pub fn desk_handlers_pub(el: gpui::AnyElement, entity: gpui::Entity<LayoutView>) -> gpui::AnyElement {
    desk_handlers(el, entity)
}

fn desk_handlers(
    el: gpui::AnyElement,
    entity: gpui::Entity<LayoutView>,
) -> gpui::AnyElement {
    let down_entity = entity.clone();
    let middle_entity = entity.clone();
    let move_entity = entity.clone();
    let up_entity = entity.clone();
    let wheel_entity = entity.clone();
    let key_entity = entity;
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .child(el)
        .on_mouse_down(
            gpui::MouseButton::Left,
            move |event: &gpui::MouseDownEvent, _window, app| {
                down_entity.update(app, |this: &mut LayoutView, cx| {
                    this.on_desk_down(
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                        event.modifiers.shift,
                        event.modifiers.alt,
                        false,
                        cx,
                    );
                    cx.notify();
                });
            },
        )
        .on_mouse_down(
            gpui::MouseButton::Middle,
            move |event: &gpui::MouseDownEvent, _window, app| {
                middle_entity.update(app, |this: &mut LayoutView, cx| {
                    this.on_desk_pan_down(
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                        cx,
                    );
                    cx.notify();
                });
            },
        )
        .on_mouse_move(move |event: &gpui::MouseMoveEvent, _window, app| {
            move_entity.update(app, |this: &mut LayoutView, cx| {
                this.on_desk_move(
                    f32::from(event.position.x),
                    f32::from(event.position.y),
                    event.modifiers.shift,
                    event.modifiers.alt,
                    event.modifiers.control || event.modifiers.platform,
                    cx,
                );
                cx.notify();
            });
        })
        .on_mouse_up(
            gpui::MouseButton::Left,
            move |event: &gpui::MouseUpEvent, _window, app| {
                let _ = event;
                up_entity.update(app, |this: &mut LayoutView, cx| {
                    this.on_desk_up(cx);
                    cx.notify();
                });
            },
        )
        .on_scroll_wheel(move |event: &gpui::ScrollWheelEvent, _window, app| {
            let delta_y = match event.delta {
                gpui::ScrollDelta::Pixels(p) => f32::from(p.y),
                gpui::ScrollDelta::Lines(p) => p.y * 24.0,
            } as f64;
            wheel_entity.update(app, |this: &mut LayoutView, cx| {
                this.on_desk_wheel(
                    delta_y,
                    f32::from(event.position.x) as f64,
                    f32::from(event.position.y) as f64,
                    cx,
                );
                cx.notify();
            });
        })
        .on_key_down(move |event: &gpui::KeyDownEvent, _window, app| {
            key_entity.update(app, |this: &mut LayoutView, cx| {
                this.on_desk_key(event, cx);
                cx.notify();
            });
        })
        .into_any_element()
}

impl LayoutView {
    fn hit_parts(&self) -> Vec<interact::HitPart> {
        let Some(layout) = self.history.draft.clone() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (i, z) in layout.zones.iter().enumerate() {
            out.push(interact::HitPart {
                r: interact::Ref::Zone(i),
                x: z.x as f64,
                y: z.y as f64,
                w: z.width as f64,
                h: z.height as f64,
                rot: z.rotation_deg as f64,
                is_case: false,
                locked: z.locked,
            });
        }
        for (i, l) in layout.landmarks.iter().enumerate() {
            if !l.visible {
                continue;
            }
            out.push(interact::HitPart {
                r: interact::Ref::Landmark(i),
                x: l.x as f64,
                y: l.y as f64,
                w: l.width as f64,
                h: l.height as f64,
                rot: l.rotation_deg as f64,
                is_case: l.kind == PartKind::Case,
                locked: l.locked,
            });
        }
        out
    }

    fn sel_to_ref(sel: Sel) -> interact::Ref {
        match sel {
            Sel::Zone(i) => interact::Ref::Zone(i),
            Sel::Landmark(i) => interact::Ref::Landmark(i),
        }
    }

    fn ref_to_sel(r: interact::Ref) -> Sel {
        match r {
            interact::Ref::Zone(i) => Sel::Zone(i),
            interact::Ref::Landmark(i) => Sel::Landmark(i),
        }
    }

    fn desk_box(&self) -> Option<(f32, f32, f32, f32, StageView, f64)> {
        let (ox, oy, w, h) = self.desk_origin.get();
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let centre = stage::content_centre(
            &self.history.draft.as_ref().map(|l| l.zones.clone()).unwrap_or_default(),
            &self.history.draft.as_ref().map(|l| l.landmarks.clone()).unwrap_or_default(),
        );
        let view = self.view.unwrap_or_else(|| hero_view(w as f64, h as f64, centre));
        let k = w as f64 / geometry::CANVAS_W * view.scale;
        Some((ox, oy, w, h, view, k))
    }

    fn to_canvas(&self, x: f32, y: f32) -> Option<interact::Pt> {
        let (ox, oy, _w, _h, view, k) = self.desk_box()?;
        Some(interact::Pt {
            x: (x - ox) as f64 / k - view.pan_x,
            y: (y - oy) as f64 / k - view.pan_y,
        })
    }

    fn on_desk_pan_down(&mut self, x: f32, y: f32, _cx: &mut Context<Self>) {
        let Some(pt) = self.to_canvas(x, y) else { return };
        let orig = self.desk_box().map(|b| b.4).unwrap_or(StageView {
            scale: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        });
        self.drag = Some(Drag {
            kind: DragKind::Pan,
            sel: Sel::Zone(0),
            origs: Vec::new(),
            riders: Vec::new(),
            start: pt,
            start_px: (x - self.desk_origin.get().0, y - self.desk_origin.get().1),
            handle: None,
            pending: false,
            locked: false,
            collapse: false,
            pre: None,
            view_origin: Some(orig),
        });
        self.view = Some(orig);
    }

    fn on_desk_down(
        &mut self,
        x: f32,
        y: f32,
        shift: bool,
        alt: bool,
        touch: bool,
        _cx: &mut Context<Self>,
    ) {
        let Some(point) = self.to_canvas(x, y) else { return };
        let Some((_ox, _oy, _w, _h, _view, k)) = self.desk_box() else { return };
        let parts = self.hit_parts();
        let selection_refs: Vec<interact::Ref> =
            self.selection.iter().map(|s| Self::sel_to_ref(*s)).collect();
        let hit = interact::hit_test(&parts, &selection_refs, point, k, touch, alt);
        match hit {
            interact::Hit::Handle(r, handle) => {
                self.begin_drag(DragKind::Resize, Self::ref_to_sel(r), point, Some(handle), x, y);
            }
            interact::Hit::Rotate(r) => {
                self.begin_drag(DragKind::Rotate, Self::ref_to_sel(r), point, None, x, y);
            }
            interact::Hit::Desk => {
                self.select(Vec::new());
                let (ox, oy, _, _, _, _) = self.desk_box().unwrap();
                self.drag = Some(Drag {
                    kind: DragKind::Marquee,
                    sel: Sel::Zone(0),
                    origs: Vec::new(),
                    riders: Vec::new(),
                    start: point,
                    start_px: (x - ox, y - oy),
                    handle: None,
                    pending: false,
                    locked: false,
                    collapse: false,
                    pre: None,
                    view_origin: None,
                });
                self.overlay.marquee = Some((x - ox, y - oy, 0.0, 0.0));
            }
            interact::Hit::Part(r, locked)
            | interact::Hit::CaseEdge(r, locked)
            | interact::Hit::CaseBody(r, locked) => {
                let sel = Self::ref_to_sel(r);
                let picked = self.selection.contains(&sel);
                if shift && !alt {
                    if picked {
                        self.selection.retain(|s| *s != sel);
                    } else {
                        self.selection.push(sel);
                    }
                    return;
                }
                if !picked {
                    self.select(vec![sel]);
                }
                let (ox, oy, _, _, _, _) = self.desk_box().unwrap();
                self.drag = Some(Drag {
                    kind: DragKind::Move,
                    sel,
                    origs: Vec::new(),
                    riders: Vec::new(),
                    start: point,
                    start_px: (x - ox, y - oy),
                    handle: None,
                    pending: true,
                    locked,
                    collapse: picked && self.selection.len() > 1,
                    pre: None,
                    view_origin: None,
                });
            }
        }
    }

    fn orig_of(&self, sel: Sel) -> Option<Orig> {
        let layout = self.history.draft.as_ref()?;
        match sel {
            Sel::Zone(i) => layout.zones.get(i).map(|z| Orig {
                sel,
                zone: Some(z.clone()),
                landmark: None,
            }),
            Sel::Landmark(i) => layout.landmarks.get(i).map(|l| Orig {
                sel,
                zone: None,
                landmark: Some(l.clone()),
            }),
        }
    }

    fn begin_drag(
        &mut self,
        kind: DragKind,
        sel: Sel,
        start: interact::Pt,
        handle: Option<interact::Handle>,
        x: f32,
        y: f32,
    ) {
        let already = self.selection.contains(&sel);
        let is_move = matches!(kind, DragKind::Move);
        if !already || !is_move {
            self.select(vec![sel]);
        }
        let members: Vec<Sel> = if already && is_move {
            self.selection.clone()
        } else {
            vec![sel]
        };
        let mut origs: Vec<Orig> = Vec::new();
        for m in &members {
            let Some(o) = self.orig_of(*m) else { continue };
            let locked = o
                .zone
                .as_ref()
                .map(|z| z.locked)
                .unwrap_or_else(|| o.landmark.as_ref().map(|l| l.locked).unwrap_or(false));
            let is_primary = *m == sel;
            if is_move && locked && !is_primary {
                continue;
            }
            origs.push(o);
        }
        if !origs.iter().any(|o| o.sel == sel) {
            if let Some(o) = self.orig_of(sel) {
                origs.push(o);
            }
        }
        let mut riders: Vec<Orig> = Vec::new();
        if is_move {
            let parts = self.hit_parts();
            for o in &origs {
                if o.landmark.as_ref().map(|l| l.kind) != Some(PartKind::Case) {
                    continue;
                }
                let Some(case) = parts.iter().find(|p| p.r == Self::sel_to_ref(o.sel)).copied() else {
                    continue;
                };
                for r in interact::parts_inside_case(&case, &parts) {
                    let sel_r = Self::ref_to_sel(r);
                    if origs.iter().any(|x| x.sel == sel_r) || riders.iter().any(|x| x.sel == sel_r) {
                        continue;
                    }
                    if let Some(ro) = self.orig_of(sel_r) {
                        riders.push(ro);
                    }
                }
            }
        }
        let pre = self.history.draft.clone();
        let (ox, oy) = (self.desk_origin.get().0, self.desk_origin.get().1);
        self.drag = Some(Drag {
            kind,
            sel,
            origs,
            riders,
            start,
            start_px: (x - ox, y - oy),
            handle,
            pending: false,
            locked: false,
            collapse: false,
            pre,
            view_origin: None,
        });
    }

    fn on_desk_move(
        &mut self,
        x: f32,
        y: f32,
        shift: bool,
        alt: bool,
        free: bool,
        _cx: &mut Context<Self>,
    ) {
        let Some((_ox, _oy, _w, _h, _view, k)) = self.desk_box() else { return };
        let Some(point) = self.to_canvas(x, y) else { return };
        if matches!(self.drag.as_ref().map(|d| &d.kind), Some(DragKind::Pan)) {
            let d = self.drag.clone().unwrap();
            let (ox, oy, dw, _dh, orig, _k0) = self.desk_box().unwrap();
            let orig = d.view_origin.unwrap_or(orig);
            let base = dw as f64 / geometry::CANVAS_W;
            let kk = base * orig.scale;
            let direct = StageView {
                scale: orig.scale,
                pan_x: orig.pan_x + (((x - ox) as f64) - d.start_px.0 as f64) / kk,
                pan_y: orig.pan_y + (((y - oy) as f64) - d.start_px.1 as f64) / kk,
            };
            let (scale, pan_x, pan_y) = interact::clamp_view(direct.scale, direct.pan_x, direct.pan_y);
            self.view = Some(StageView { scale, pan_x, pan_y });
            return;
        }
        if matches!(self.drag.as_ref().map(|d| d.pending), Some(true)) {
            let d = self.drag.clone().unwrap();
            if d.locked {
                return;
            }
            let dist = ((x - self.desk_origin.get().0) - d.start_px.0)
                .hypot((y - self.desk_origin.get().1) - d.start_px.1);
            if dist < interact::drag_threshold_px(false) as f32 {
                return;
            }
            self.begin_drag(DragKind::Move, d.sel, d.start, None, x, y);
        }
        if matches!(self.drag.as_ref().map(|d| &d.kind), Some(DragKind::Marquee)) {
            let d = self.drag.clone().unwrap();
            let (ox, oy, _, _, _, _) = self.desk_box().unwrap();
            let px = x - ox;
            let py = y - oy;
            self.overlay.marquee = Some((
                d.start_px.0.min(px),
                d.start_px.1.min(py),
                (px - d.start_px.0).abs(),
                (py - d.start_px.1).abs(),
            ));
            return;
        }
        let Some(drag) = self.drag.clone() else { return };
        let Some(orig) = drag.origs.iter().find(|o| o.sel == drag.sel) else { return };
        let Some(src) = orig.rect() else { return };
        let (dx, dy) = (point.x - drag.start.x, point.y - drag.start.y);
        match drag.kind {
            DragKind::Move => {
                let threshold = interact::SNAP_PIXELS / k;
                let moving: Vec<Sel> = drag
                    .origs
                    .iter()
                    .chain(drag.riders.iter())
                    .map(|o| o.sel)
                    .collect();
                let mut others: Vec<interact::Rect> = Vec::new();
                if let Some(layout) = self.history.draft.clone() {
                    for (i, z) in layout.zones.iter().enumerate() {
                        if moving.contains(&Sel::Zone(i)) {
                            continue;
                        }
                        others.push(interact::Rect { x: z.x as f64, y: z.y as f64, w: z.width as f64, h: z.height as f64 });
                    }
                    for (i, l) in layout.landmarks.iter().enumerate() {
                        if moving.contains(&Sel::Landmark(i)) {
                            continue;
                        }
                        others.push(interact::Rect { x: l.x as f64, y: l.y as f64, w: l.width as f64, h: l.height as f64 });
                    }
                }
                let case_moves = drag.origs.iter().any(|o| {
                    o.landmark.as_ref().map(|l| l.kind == PartKind::Case).unwrap_or(false)
                });
                let (panels_x, panels_y): (Vec<f64>, Vec<f64>) = if case_moves {
                    (Vec::new(), Vec::new())
                } else if let Some(layout) = self.history.draft.clone() {
                    let derived = geometry::case_landmark_for(layout.case_form);
                    let case_landmark = layout
                        .landmarks
                        .iter()
                        .find(|l| l.kind == PartKind::Case)
                        .or(derived.as_ref());
                    if let Some(case) = case_landmark {
                        let (cx, cy, cw, ch) = (case.x as f64, case.y as f64, case.width as f64, case.height as f64);
                        (vec![cx, cx + cw / 2.0, cx + cw], vec![cy, cy + ch / 2.0, cy + ch])
                    } else {
                        (Vec::new(), Vec::new())
                    }
                } else {
                    (Vec::new(), Vec::new())
                };
                let want = interact::Rect {
                    x: interact::rubber_band(src.x + dx, -src.w / 2.0, geometry::CANVAS_W - src.w / 2.0),
                    y: interact::rubber_band(src.y + dy, -src.h / 2.0, geometry::CANVAS_H - src.h / 2.0),
                    w: src.w,
                    h: src.h,
                };
                let canvas_box = interact::Rect { x: 0.0, y: 0.0, w: geometry::CANVAS_W, h: geometry::CANVAS_H };
                let mut all_others = vec![canvas_box];
                all_others.extend(others.clone());
                let snap = interact::snap_move(want, &all_others, &panels_x, &panels_y, threshold, free);
                let eq = if free { None } else { Some(interact::equal_spacing(want, &others, threshold)) };
                let next_x = eq.as_ref().and_then(|e| e.x).unwrap_or(snap.x);
                let next_y = eq.as_ref().and_then(|e| e.y).unwrap_or(snap.y);
                let (delta_x, delta_y) = (next_x - src.x, next_y - src.y);
                if let Some(layout) = self.history.draft.as_mut() {
                    for o in drag.origs.clone().iter().chain(drag.riders.clone().iter()) {
                        match o.sel {
                            Sel::Zone(i) => {
                                if let Some(z) = layout.zones.get_mut(i) {
                                    if let Some(oz) = o.zone.clone() {
                                        z.x = oz.x + delta_x as f32;
                                        z.y = oz.y + delta_y as f32;
                                    }
                                }
                            }
                            Sel::Landmark(i) => {
                                if let Some(l) = layout.landmarks.get_mut(i) {
                                    if let Some(ol) = o.landmark.clone() {
                                        l.x = ol.x + delta_x as f32;
                                        l.y = ol.y + delta_y as f32;
                                    }
                                }
                            }
                        }
                    }
                }
                self.overlay.guides = (
                    eq.as_ref().and_then(|e| e.line_a).or(snap.guide_x),
                    eq.as_ref().and_then(|e| e.line_b),
                    snap.guide_y,
                );
                let where_text = if eq.is_some_and(|e| e.x.is_some() || e.y.is_some()) {
                    " \u{b7} even spacing"
                } else if snap.gap_x.is_some_and(|g| g < 0.05) {
                    " \u{b7} on the guide"
                } else {
                    ""
                };
                let unit = self.unit;
                self.overlay.readout = Some((
                    format!(
                        "{} \u{b7} {}{}",
                        geometry::format_units(next_x, unit),
                        geometry::format_units(next_y, unit),
                        where_text
                    ),
                    (point.x * k) as f32 + 12.0,
                    (point.y * k) as f32 + 12.0,
                ));
            }
            DragKind::Resize => {
                let handle = drag.handle.unwrap_or(interact::Handle::Se);
                let q = interact::Pt {
                    x: interact::rubber_band(point.x, 0.0, geometry::CANVAS_W),
                    y: interact::rubber_band(point.y, 0.0, geometry::CANVAS_H),
                };
                let next = interact::resize_by(src, orig_rot(orig), handle, q, shift, alt);
                if let Some(layout) = self.history.draft.as_mut() {
                    match drag.sel {
                        Sel::Zone(i) => {
                            if let Some(z) = layout.zones.get_mut(i) {
                                z.x = next.x as f32;
                                z.y = next.y as f32;
                                z.width = next.w as f32;
                                z.height = next.h as f32;
                            }
                        }
                        Sel::Landmark(i) => {
                            if let Some(l) = layout.landmarks.get_mut(i) {
                                l.x = next.x as f32;
                                l.y = next.y as f32;
                                l.width = next.w as f32;
                                l.height = next.h as f32;
                            }
                        }
                    }
                }
                let unit = self.unit;
                self.overlay.readout = Some((
                    format!(
                        "{} \u{d7} {}{}",
                        geometry::format_units(next.w, unit),
                        geometry::format_units(next.h, unit),
                        if shift { " \u{b7} locked" } else { "" }
                    ),
                    (point.x * k) as f32 + 12.0,
                    (point.y * k) as f32 + 12.0,
                ));
                self.overlay.guides = (None, None, None);
            }
            DragKind::Rotate => {
                let centre = interact::Pt { x: src.x + src.w / 2.0, y: src.y + src.h / 2.0 };
                let deg = interact::rotate_angle(centre, point, shift);
                if let Some(layout) = self.history.draft.as_mut() {
                    match drag.sel {
                        Sel::Zone(i) => {
                            if let Some(z) = layout.zones.get_mut(i) {
                                z.rotation_deg = deg as f32;
                            }
                        }
                        Sel::Landmark(i) => {
                            if let Some(l) = layout.landmarks.get_mut(i) {
                                l.rotation_deg = deg as f32;
                            }
                        }
                    }
                }
                self.overlay.readout = Some((
                    format!("{deg:.1}\u{b0}{}", if shift { " \u{b7} 15\u{b0} steps" } else { "" }),
                    (point.x * k) as f32 + 12.0,
                    (point.y * k) as f32 + 12.0,
                ));
                self.overlay.guides = (None, None, None);
            }
            _ => {}
        }
    }

    fn on_desk_up(&mut self, _cx: &mut Context<Self>) {
        let Some(drag) = self.drag.clone() else {
            self.overlay = DragOverlay::default();
            return;
        };
        if drag.pending {
            if drag.collapse {
                self.select(vec![drag.sel]);
            }
            self.drag = None;
            self.overlay = DragOverlay::default();
            return;
        }
        if matches!(drag.kind, DragKind::Pan) {
            self.drag = None;
            return;
        }
        if matches!(drag.kind, DragKind::Marquee) {
            if let Some((x, y, w, h)) = self.overlay.marquee {
                if w > 4.0 || h > 4.0 {
                    if let Some((_ox, _oy, _dw, _dh, view, k)) = self.desk_box() {
                        let band = interact::Rect {
                            x: x as f64 / k - view.pan_x,
                            y: y as f64 / k - view.pan_y,
                            w: w as f64 / k,
                            h: h as f64 / k,
                        };
                        let parts = self.hit_parts();
                        let picked = interact::marquee_pick(band, &parts);
                        self.select(picked.iter().map(|r| Self::ref_to_sel(*r)).collect());
                    }
                }
            }
            self.drag = None;
            self.overlay = DragOverlay::default();
            return;
        }
        if let Some(layout) = self.history.draft.as_mut() {
            for o in drag.origs.iter().chain(drag.riders.iter()) {
                let now = match o.sel {
                    Sel::Zone(i) => layout.zones.get(i).map(|z| (z.x as f64, z.y as f64, z.width as f64, z.height as f64)),
                    Sel::Landmark(i) => layout.landmarks.get(i).map(|l| (l.x as f64, l.y as f64, l.width as f64, l.height as f64)),
                };
                let Some((x, y, w, h)) = now else { continue };
                let settled = interact::settle_rect(
                    interact::Rect { x, y, w, h },
                    geometry::CANVAS_W,
                    geometry::CANVAS_H,
                    interact::MIN_SIZE,
                );
                match o.sel {
                    Sel::Zone(i) => {
                        if let Some(z) = layout.zones.get_mut(i) {
                            z.x = settled.x as f32;
                            z.y = settled.y as f32;
                            z.width = settled.w as f32;
                            z.height = settled.h as f32;
                        }
                    }
                    Sel::Landmark(i) => {
                        if let Some(l) = layout.landmarks.get_mut(i) {
                            l.x = settled.x as f32;
                            l.y = settled.y as f32;
                            l.width = settled.w as f32;
                            l.height = settled.h as f32;
                        }
                    }
                }
            }
        }
        self.drag = None;
        self.overlay = DragOverlay::default();
        if let Some(pre) = drag.pre.clone() {
            if self.history.draft.is_some() {
                self.history.past.push(pre);
                self.history.future.clear();
            }
        }
    }

    fn on_desk_wheel(&mut self, delta_y: f64, x: f64, y: f64, _cx: &mut Context<Self>) {
        let (ox, oy, dw, dh, _view, _k) = self
            .desk_box()
            .unwrap_or((0.0, 0.0, 0.0, 0.0, StageView { scale: 1.0, pan_x: 0.0, pan_y: 0.0 }, 1.0));
        self.wheel_zoom(delta_y, (x - ox as f64, y - oy as f64), (dw as f64, dh as f64));
    }

    fn on_desk_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        if mods.control || mods.platform {
            let k = key.to_lowercase();
            if k == "z" && !mods.shift {
                self.undo();
                return;
            }
            if (k == "z" && mods.shift) || k == "y" {
                self.redo();
                return;
            }
        }
        let k = key.to_lowercase();
        if k == "l" && !mods.control && !mods.platform && !mods.alt {
            self.mode = StageMode::Lights;
            cx.notify();
            return;
        }
        if k == "e" && !mods.control && !mods.platform && !mods.alt {
            self.mode = StageMode::Effect;
            cx.notify();
            return;
        }
        if mods.control || mods.platform || mods.alt {
            return;
        }
        let Some(sel) = self.selection.first().copied() else {
            if k == "escape" {
                self.select(Vec::new());
            }
            return;
        };
        if key == "Escape" {
            self.select(Vec::new());
            return;
        }
        if key == "Enter" {
            return;
        }
        let step = interact::nudge_step(mods.shift);
        match sel {
            Sel::Zone(i) => {
                let Some(layout) = self.history.draft.clone() else { return };
                let Some(z) = layout.zones.get(i).cloned() else { return };
                #[allow(clippy::needless_late_init)]
                let patch: Option<(f32, f32, Option<bool>)>;
                match key {
                    "ArrowLeft" => patch = Some((
                        interact::clamp(z.x as f64 - step, -z.width as f64 / 2.0, geometry::CANVAS_W - z.width as f64 / 2.0) as f32,
                        z.y,
                        None,
                    )),
                    "ArrowRight" => patch = Some((
                        interact::clamp(z.x as f64 + step, -z.width as f64 / 2.0, geometry::CANVAS_W - z.width as f64 / 2.0) as f32,
                        z.y,
                        None,
                    )),
                    "ArrowUp" => patch = Some((
                        z.x,
                        interact::clamp(z.y as f64 - step, -z.height as f64 / 2.0, geometry::CANVAS_H - z.height as f64 / 2.0) as f32,
                        None,
                    )),
                    "ArrowDown" => patch = Some((
                        z.x,
                        interact::clamp(z.y as f64 + step, -z.height as f64 / 2.0, geometry::CANVAS_H - z.height as f64 / 2.0) as f32,
                        None,
                    )),
                    "Delete" | "Backspace" => patch = Some((z.x, z.y, Some(false))),
                    _ => return,
                }
                let Some((px, py, visible)) = patch else { return };
                self.commit(|l| Layout {
                    zones: l
                        .zones
                        .iter()
                        .enumerate()
                        .map(|(zi, zz)| {
                            if zi == i {
                                let mut zz = zz.clone();
                                zz.x = px;
                                zz.y = py;
                                if let Some(v) = visible {
                                    zz.visible = v;
                                }
                                zz
                            } else {
                                zz.clone()
                            }
                        })
                        .collect(),
                    ..l.clone()
                });
            }
            Sel::Landmark(i) => {
                let Some(layout) = self.history.draft.clone() else { return };
                let Some(l) = layout.landmarks.get(i).cloned() else { return };
                let patch = match key {
                    "ArrowLeft" => (l.x - step as f32, l.y),
                    "ArrowRight" => (l.x + step as f32, l.y),
                    "ArrowUp" => (l.x, l.y - step as f32),
                    "ArrowDown" => (l.x, l.y + step as f32),
                    _ => return,
                };
                self.commit(|l| Layout {
                    landmarks: l
                        .landmarks
                        .iter()
                        .enumerate()
                        .map(|(li, ll)| {
                            if li == i {
                                let mut ll = ll.clone();
                                ll.x = patch.0;
                                ll.y = patch.1;
                                ll
                            } else {
                                ll.clone()
                            }
                        })
                        .collect(),
                    ..l.clone()
                });
            }
        }
    }
}

fn orig_rot(o: &Orig) -> f64 {
    o.zone
        .as_ref()
        .map(|z| z.rotation_deg as f64)
        .unwrap_or_else(|| o.landmark.as_ref().map(|l| l.rotation_deg as f64).unwrap_or(0.0))
}

/// The loading frame: the page says it is reading the layout.
fn reading(t: Theme) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_base()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child("Reading the layout"),
        )
        .into_any_element()
}

/// The switcher between the panes below the desk, on a phone.
fn phone_switcher(
    view: &mut LayoutView,
    t: &Theme,
    cx: &mut Context<LayoutView>,
) -> gpui::AnyElement {
    let entity = cx.entity();
    let selected = view.phone_pane;
    segmented(
        t,
        "lay-tabs",
        &[("parts", "Parts"), ("place", "To place"), ("render", "Render")],
        match selected {
            PhonePane::Parts => "parts",
            PhonePane::Place => "place",
            PhonePane::Render => "render",
        },
        move |value, _window, cx| {
            entity.update(cx, |this, cx| {
                this.phone_pane = match value.as_str() {
                    "place" => PhonePane::Place,
                    "render" => PhonePane::Render,
                    _ => PhonePane::Parts,
                };
                cx.notify();
            });
        },
    )
    .into_any_element()
}

/// The bar along the frame's bottom edge: the scale, the segments and the
/// zoom.
/// The desk's toolbar as the onboarding takeover draws it.
///
/// React's `PreviewStage` puts its toolbar straight under the desk (the
/// `.toolbar` of `desk.module.css`): no rule above it, no frame around it,
/// and the whole row - the mode, the undo and redo keys, the desk's own
/// settings and the Render key - in one line. The Layout page's own toolbar
/// lives inside the stage's frame instead, so the two are drawn apart here.
/// The zoom is read from the onboarding's own desk, not the Layout page's.
pub fn stage_toolbar_onboarding_pub(
    view: &mut LayoutView,
    t: &Theme,
    desk: (f32, f32),
    cx: &mut Context<LayoutView>,
) -> gpui::AnyElement {
    let layout = view.layout().cloned();
    let centre = stage::content_centre(
        &layout.as_ref().map(|l| l.zones.clone()).unwrap_or_default(),
        &layout.as_ref().map(|l| l.landmarks.clone()).unwrap_or_default(),
    );
    let view_state = view.view.unwrap_or_else(|| hero_view(desk.0 as f64, desk.1 as f64, centre));
    let scale_percent = interact::zoom_percent(view_state.scale);
    let desk_box = (desk.0 as f64, desk.1 as f64);
    let entity = cx.entity();

    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(12.0))
        .flex_none()
        .w_full()
        // React's `.toolbar` has a margin-top of its own (`--s2`).
        .mt(px(8.0));
    row = row.child(scale_bar(view, t, view_state.scale, desk.0));

    let mut segs = div().flex().items_center().gap(px(8.0)).min_w_0();
    let mode = view.mode;
    segs = segs.child(segmented(
        t,
        "onb-mode",
        &[("effect", "Effect"), ("lights", "Lights")],
        match mode {
            StageMode::Effect => "effect",
            StageMode::Lights => "lights",
        },
        {
            let entity = entity.clone();
            move |value, _window, cx| {
                entity.update(cx, |this, cx| {
                    this.mode = if value == "lights" {
                        StageMode::Lights
                    } else {
                        StageMode::Effect
                    };
                    cx.notify();
                });
            }
        },
    ));
    // The undo and redo keys: they undo work done on the desk, and the
    // takeover has no strip to put them on.
    let past_empty = view.history.past.is_empty();
    let future_empty = view.history.future.is_empty();
    let undo_entity = entity.clone();
    let redo_entity = entity.clone();
    segs = segs.child(
        div()
            .flex()
            .items_center()
            .child(icon_button(
                t,
                "onb-stage-undo",
                "ArrowUUpLeft",
                "",
                true,
                false,
                past_empty,
                move |_, _window, app| {
                    undo_entity.update(app, |this: &mut LayoutView, cx| {
                        this.undo();
                        cx.notify();
                    });
                },
            ))
            .child(icon_button(
                t,
                "onb-stage-redo",
                "ArrowUUpRight",
                "",
                true,
                false,
                future_empty,
                move |_, _window, app| {
                    redo_entity.update(app, |this: &mut LayoutView, cx| {
                        this.redo();
                        cx.notify();
                    });
                },
            )),
    );
    // The desk's own settings: how much of the desk the effect is drawn at,
    // and how each LED reads the picture.
    let scale = layout.as_ref().map(|l| l.render_scale.percent()).unwrap_or(50);
    let scale_entity = entity.clone();
    let sampling = layout.as_ref().map(|l| sampling_key(l.sampling)).unwrap_or("nearest");
    let sampling_entity = entity.clone();
    segs = segs.child(
        div().flex().items_center().child(segmented(
            t,
            "onb-scale",
            &[("25", "¼"), ("50", "½"), ("100", "All")],
            &scale.to_string(),
            move |value, _window, cx| {
                scale_entity.update(cx, |this: &mut LayoutView, cx| {
                    let pct: u8 = value.parse().unwrap_or(50);
                    let rs: RenderScale = pct.try_into().unwrap_or(RenderScale::P50);
                    this.commit(|l| Layout { render_scale: rs, ..l.clone() });
                    cx.notify();
                });
            },
        ))
        .child(segmented(
            t,
            "onb-sampling",
            &[("nearest", "1"), ("bilinear", "4"), ("area_average", "All px")],
            sampling,
            move |value, _window, cx| {
                sampling_entity.update(cx, |this: &mut LayoutView, cx| {
                    let sm = match value.as_str() {
                        "nearest" => SamplingMode::Nearest,
                        "bilinear" => SamplingMode::Bilinear,
                        _ => SamplingMode::AreaAverage,
                    };
                    this.commit(|l| Layout { sampling: sm, ..l.clone() });
                    cx.notify();
                });
            },
        )),
    );
    // The Render key: the desk's own settings in one key.
    let render_entity = entity.clone();
    segs = segs.child(button_icon(
        t,
        "onb-render-key",
        "Render",
        Variant::Default,
        false,
        true,
        "SlidersHorizontal",
        "",
        15.0,
        move |_, _window, app| {
            render_entity.update(app, |this: &mut LayoutView, cx| {
                this.render_open = !this.render_open;
                cx.notify();
            });
        },
    ));
    row = row.child(segs);
    row = row.child(div().flex_1());
    // The zoom, on the right.
    let zoom_out_entity = entity.clone();
    let zoom_in_entity = entity.clone();
    let fit_entity = entity.clone();
    row = row.child(
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .ml_auto()
            .p(px(2.0))
            .border_1()
            .border_color(Theme::color(t.line_2))
            .r_md()
            .bg(Theme::color(t.panel))
            .child(icon_button(
                t,
                "onb-zoom-out",
                "Minus",
                "",
                true,
                false,
                false,
                move |_, _window, app| {
                    zoom_out_entity.update(app, |this: &mut LayoutView, cx| {
                        this.zoom_by(1.0 / 1.3, desk_box);
                        cx.notify();
                    });
                },
            ))
            .child(
                div()
                    .w(px(40.0))
                    .text_center()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(format!("{scale_percent}%")),
            )
            .child(icon_button(
                t,
                "onb-zoom-in",
                "Plus",
                "",
                true,
                false,
                false,
                move |_, _window, app| {
                    zoom_in_entity.update(app, |this: &mut LayoutView, cx| {
                        this.zoom_by(1.3, desk_box);
                        cx.notify();
                    });
                },
            ))
            .child(icon_button(
                t,
                "onb-zoom-fit",
                "ArrowsOut",
                "",
                true,
                false,
                false,
                move |_, _window, app| {
                    fit_entity.update(app, |this: &mut LayoutView, cx| {
                        this.zoom_fit(desk_box);
                        cx.notify();
                    });
                },
            )),
    );
    if view.render_open {
        row = row.child(render_popover(view, t, cx));
    }
    row.into_any_element()
}

fn stage_toolbar(
    view: &mut LayoutView,
    t: &Theme,
    container: f32,
    phone: bool,
    cx: &mut Context<LayoutView>,
) -> gpui::AnyElement {
    let layout = view.layout().cloned();
    let container_width = container;
    let one_column = container_width <= 639.0;
    let inspector_w = inspector_width(container_width);
    let (desk_w, desk_h) = desk_size(view.width, view.height, container_width, inspector_w, one_column);
    let centre = stage::content_centre(
        &layout.as_ref().map(|l| l.zones.clone()).unwrap_or_default(),
        &layout.as_ref().map(|l| l.landmarks.clone()).unwrap_or_default(),
    );
    let view_state = view
        .view
        .unwrap_or_else(|| hero_view(desk_w as f64, desk_h as f64, centre));
    let scale_percent = interact::zoom_percent(view_state.scale);
    let desk_box = (desk_w as f64, desk_h as f64);

    let mut row = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .flex_none()
        .w_full()
        .border_t_1()
        .border_color(Theme::color(t.line))
        .bg(Theme::color(t.panel))
        .p_2();
    // The scale bar: a real length, in the reader's unit.
    if view.width > 480.0 {
        row = row.child(scale_bar(view, t, view_state.scale, desk_w));
    }
    // The segments in the middle: what the canvas shows, and the desk's own
    // settings.
    let mut segs = div().flex().items_center().gap(px(8.0)).min_w_0();
    let entity = cx.entity();
    let mode = view.mode;
    segs = segs.child(segmented(
        t,
        "lay-mode",
        &[("effect", "Effect"), ("lights", "Lights")],
        match mode {
            StageMode::Effect => "effect",
            StageMode::Lights => "lights",
        },
        {
            let entity = entity.clone();
            move |value, _window, cx| {
                entity.update(cx, |this, cx| {
                    this.mode = if value == "lights" {
                        StageMode::Lights
                    } else {
                        StageMode::Effect
                    };
                    cx.notify();
                });
            }
        },
    ));
    if !phone {
        // The desk's own settings: how much of the desk the effect is drawn
        // at, and how each LED reads the picture. On a narrow desk they fold
        // into one Render key that opens both.
        if desk_w >= 780.0 {
            let scale = layout
                .as_ref()
                .map(|l| l.render_scale.percent())
                .unwrap_or(50);
            let scale_entity = entity.clone();
            segs = segs.child(segmented(
                t,
                "lay-scale",
                &[("25", "¼"), ("50", "½"), ("100", "All")],
                &scale.to_string(),
                move |value, _window, cx| {
                    scale_entity.update(cx, |this: &mut LayoutView, cx| {
                        let pct: u8 = value.parse().unwrap_or(50);
                        let rs: RenderScale = pct.try_into().unwrap_or(RenderScale::P50);
                        this.commit(|l| Layout { render_scale: rs, ..l.clone() });
                        cx.notify();
                    });
                },
            ));
            let sampling = layout
                .as_ref()
                .map(|l| sampling_key(l.sampling))
                .unwrap_or("nearest");
            let sampling_entity = entity.clone();
            segs = segs.child(segmented(
                t,
                "lay-sampling",
                &[
                    ("nearest", "1"),
                    ("bilinear", "4"),
                    ("area_average", "All px"),
                ],
                sampling,
                move |value, _window, cx| {
                    sampling_entity.update(cx, |this: &mut LayoutView, cx| {
                        let sm = match value.as_str() {
                            "nearest" => SamplingMode::Nearest,
                            "bilinear" => SamplingMode::Bilinear,
                            _ => SamplingMode::AreaAverage,
                        };
                        this.commit(|l| Layout { sampling: sm, ..l.clone() });
                        cx.notify();
                    });
                },
            ));
        } else {
            let render_entity = entity.clone();
            segs = segs.child(button_icon(
                t,
                "lay-render-key",
                "Render",
                Variant::Default,
                false,
                true,
                "SlidersHorizontal",
                "",
                15.0,
                move |_, _window, app| {
                    render_entity.update(app, |this: &mut LayoutView, cx| {
                        this.render_open = !this.render_open;
                        cx.notify();
                    });
                },
            ));
        }
    } else {
        // On a phone the undo and redo keys move here, to the desk where the
        // work they undo happens.
        let past_empty = view.history.past.is_empty();
        let future_empty = view.history.future.is_empty();
        let undo_entity = entity.clone();
        let redo_entity = entity.clone();
        segs = segs
            .child(icon_button(
                t,
                "lay-stage-undo",
                "ArrowUUpLeft",
                "",
                true,
                false,
                past_empty,
                move |_, _window, app| {
                    undo_entity.update(app, |this: &mut LayoutView, cx| {
                        this.undo();
                        cx.notify();
                    });
                },
            ))
            .child(icon_button(
                t,
                "lay-stage-redo",
                "ArrowUUpRight",
                "",
                true,
                false,
                future_empty,
                move |_, _window, app| {
                    redo_entity.update(app, |this: &mut LayoutView, cx| {
                        this.redo();
                        cx.notify();
                    });
                },
            ));
    }
    row = row.child(segs);
    row = row.child(div().flex_1());
    // The zoom, on the right.
    let zoom_out_entity = entity.clone();
    let zoom_in_entity = entity.clone();
    let fit_entity = entity.clone();
    row = row.child(
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .ml_auto()
            .p(px(2.0))
            .border_1()
            .border_color(Theme::color(t.line_2))
            .r_md()
            .bg(Theme::color(t.panel))
            .child(icon_button(
                t,
                "lay-zoom-out",
                "Minus",
                "",
                true,
                false,
                false,
                move |_, _window, app| {
                    zoom_out_entity.update(app, |this: &mut LayoutView, cx| {
                        this.zoom_by(1.0 / 1.3, desk_box);
                        cx.notify();
                    });
                },
            ))
            .child(
                div()
                    .w(px(40.0))
                    .text_center()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(format!("{scale_percent}%")),
            )
            .child(icon_button(
                t,
                "lay-zoom-in",
                "Plus",
                "",
                true,
                false,
                false,
                move |_, _window, app| {
                    zoom_in_entity.update(app, |this: &mut LayoutView, cx| {
                        this.zoom_by(1.3, desk_box);
                        cx.notify();
                    });
                },
            ))
            .child(icon_button(
                t,
                "lay-zoom-fit",
                "ArrowsOut",
                "",
                true,
                false,
                false,
                move |_, _window, app| {
                    fit_entity.update(app, |this: &mut LayoutView, cx| {
                        this.zoom_fit(desk_box);
                        cx.notify();
                    });
                },
            )),
    );
    if view.render_open {
        row = row.child(render_popover(view, t, cx));
    }
    row.into_any_element()
}

/// "How the desk is drawn and read": render scale and sampling, with notes.
fn render_popover(view: &mut LayoutView, t: &Theme, cx: &mut Context<LayoutView>) -> gpui::AnyElement {
    let layout = view.layout().cloned();
    let scale = layout.as_ref().map(|l| l.render_scale.percent()).unwrap_or(50);
    let sampling = layout.as_ref().map(|l| sampling_key(l.sampling)).unwrap_or("nearest");
    let render_note = match scale {
        25 => "A quarter of the desk: the effect is drawn small and then scaled up, so the edges are softer and the work is lighter.",
        100 => "The whole desk: every LED reads the picture at the desk's own size, which is the sharpest and the most work.",
        _ => "Half of the desk: a balance between the detail of the picture and the work of drawing it.",
    };
    let sampling_note = match sampling {
        "nearest" => "Each LED takes the colour of the one pixel it sits on: the sharpest, and the lightest.",
        "bilinear" => "Each LED blends the four pixels around it, so the edge between them fades.",
        _ => "Each LED takes the average of every pixel it covers, so it is lit evenly.",
    };
    let entity = cx.entity();
    let scale_entity = entity.clone();
    let sampling_entity = entity;
    let body = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p_2()
        .child(
            div().flex().items_center().gap(px(8.0))
                .child(div().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.mute)).child("Render"))
                .child(segmented(t, "lay-pop-scale", &[("25", "¼"), ("50", "½"), ("100", "All")], &scale.to_string(),
                    move |value, _window, cx| {
                        scale_entity.update(cx, |this: &mut LayoutView, cx| {
                            let pct: u8 = value.parse().unwrap_or(50);
                            let rs: RenderScale = pct.try_into().unwrap_or(RenderScale::P50);
                            this.commit(|l| Layout { render_scale: rs, ..l.clone() });
                            cx.notify();
                        });
                    })),
        )
        .child(div().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.mute)).child(render_note))
        .child(
            div().flex().items_center().gap(px(8.0))
                .child(div().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.mute)).child("Sampling"))
                .child(segmented(t, "lay-pop-sampling", &[("nearest", "1"), ("bilinear", "4"), ("area_average", "All px")], sampling,
                    move |value, _window, cx| {
                        sampling_entity.update(cx, |this: &mut LayoutView, cx| {
                            let sm = match value.as_str() {
                                "nearest" => SamplingMode::Nearest,
                                "bilinear" => SamplingMode::Bilinear,
                                _ => SamplingMode::AreaAverage,
                            };
                            this.commit(|l| Layout { sampling: sm, ..l.clone() });
                            cx.notify();
                        });
                    })),
        )
        .child(div().text_sm().font_family(crate::fonts::UI_FAMILY).text_color(Theme::color(t.mute)).child(sampling_note));
    div().w_full().child(body).into_any_element()
}

/// The sampling's name, as the segment says it.
fn sampling_key(sampling: SamplingMode) -> &'static str {
    match sampling {
        SamplingMode::Nearest => "nearest",
        SamplingMode::Bilinear => "bilinear",
        SamplingMode::AreaAverage => "area_average",
    }
}

/// The scale bar: how long a line on the desk is, in the reader's unit.
fn scale_bar(view: &LayoutView, t: &Theme, scale: f64, desk_w: f32) -> gpui::AnyElement {
    let unit = view.unit;
    let (value, px_wide) = scale_bar_length(unit, scale, desk_w);
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(6.0))
        .py(px(3.0))
        .r_sm()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .bg(Theme::color(t.panel))
        .child(
            div()
                .w(px(px_wide))
                .h(px(0.0))
                .border_t_2()
                .border_l_2()
                .border_r_2()
                .border_color(Theme::color(t.dim)),
        )
        .child(
            div()
                .text_sm()
                .font_family(crate::fonts::MONO_FAMILY)
                .text_color(Theme::color(t.dim))
                .child(value),
        )
        .into_any_element()
}

/// The scale bar's length: a round number of the reader's unit, about 120
/// css pixels wide.
pub fn scale_bar_length(unit: LengthUnit, scale: f64, desk_w: f32) -> (String, f32) {
    let px_per_unit = desk_w as f64 / 320.0 * scale;
    let px_per_cm = px_per_unit * 2.5;
    let steps: &[f64] = match unit {
        LengthUnit::In => &[1.0, 2.0, 5.0, 10.0],
        LengthUnit::Cm => &[1.0, 2.0, 5.0, 10.0, 20.0],
    };
    let raw = 120.0 / px_per_cm;
    let mut value = steps[0];
    for s in steps {
        if *s <= raw {
            value = *s;
        }
    }
    let wide = (value
        * match unit {
            LengthUnit::In => px_per_cm / 2.54,
            LengthUnit::Cm => px_per_cm,
        })
    .round() as f32;
    let text = match unit {
        LengthUnit::In => format!("{value} in"),
        LengthUnit::Cm => format!("{value} cm"),
    };
    (text, wide)
}

/// The "To place" panel: the zones that are not on the desk yet.
fn place_panel(
    view: &mut LayoutView,
    t: &Theme,
    devices: &[DeviceInfo],
    layout: &Layout,
    cx: &mut Context<LayoutView>,
) -> gpui::AnyElement {
    let entity_for_place = cx.entity();
    let _ = view;
    let rows = unplaced(Some(layout), devices);
    let count = rows.len();
    let actions = div()
        .text_sm()
        .font_family(crate::fonts::MONO_FAMILY)
        .text_color(Theme::color(t.mute))
        .child(format!("{count}"));
    let mut body = div().flex().flex_col();
    if count == 0 {
        body = body.child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.mute))
                .child("Every zone is placed."),
        );
    }
    for (i, u) in rows.iter().enumerate() {
        let device_name = u.device_name.clone();
        let connected = u.connected;
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .min_w_0()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(Theme::color(t.ink))
            .child(u.zone_name.clone())
            .child(
                div()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(format!("{}", u.led_count)),
            )
            .truncate();
        let sub = div()
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.mute))
            .child(if connected {
                device_name.clone()
            } else {
                format!("{device_name} · disconnected")
            })
            .truncate();
        let mut row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .py(px(8.0))
            .child(div().flex_1().min_w(px(0.0)).child(title).child(sub));
        if i + 1 < count {
            row = row.border_b_1().border_color(Theme::color(t.line));
        }
        let entity = entity_for_place.clone();
        let place_id = format!("lay-place-{}-{}", u.device_id, u.zone_index);
        let u_device = u.device_id.clone();
        let u_zone = u.zone_index;
        row = row.child(icon_button(
            t,
            &place_id,
            "Plus",
            "bold",
            true,
            false,
            false,
            move |_, _window, app| {
                entity.update(app, |this: &mut LayoutView, cx| {
                    this.place_zone(&u_device, u_zone, None);
                    cx.notify();
                });
            },
        ));
        body = body.child(row);
    }
    let panel = panel_with_padding(t, "lay-place", "To place", actions, body, None);
    div().flex_none().min_h_0().child(panel).into_any_element()
}

/// The parts panel: the list of what is on the desk.
fn parts_panel(
    view: &mut LayoutView,
    t: &Theme,
    devices: &[DeviceInfo],
    layout: &Layout,
    one_column: bool,
    cx: &mut Context<LayoutView>,
    window: &mut Window,
) -> gpui::AnyElement {
    let entity = cx.entity();
    let unit = view.unit;
    let unit_entity = entity.clone();
    let actions = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .child(segmented(
            t,
            "lay-unit",
            &[("cm", "cm"), ("in", "in")],
            match unit {
                LengthUnit::Cm => "cm",
                LengthUnit::In => "in",
            },
            move |value, _window, cx| {
                unit_entity.update(cx, |this, cx| {
                    this.unit = if value == "in" {
                        LengthUnit::In
                    } else {
                        LengthUnit::Cm
                    };
                    cx.notify();
                });
            },
        ))
        .child({
            let add_entity = entity.clone();
            button_icon(
                t,
                "lay-add-part",
                "Add part",
                Variant::Ghost,
                false,
                true,
                "Plus",
                "",
                14.0,
                move |_, _window, app| {
                    add_entity.update(app, |this: &mut LayoutView, cx| {
                        this.add_open = !this.add_open;
                        cx.notify();
                    });
                },
            )
        });
    let add_popover = if view.add_open {
        let add_entity = entity.clone();
        let mut chips = div()
            .flex()
            .flex_wrap()
            .gap(px(8.0))
            .mt_3()
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("Add a part to the desk"),
            );
        for kind in interact::LANDMARK_KINDS {
            let chip_entity = add_entity.clone();
            chips = chips.child(button_icon(
                t,
                &format!("lay-add-{}", interact::part_label(kind)),
                &format!("Add a {}", interact::part_label(kind)),
                Variant::Default,
                false,
                true,
                "Plus",
                "",
                14.0,
                move |_, _window, app| {
                    chip_entity.update(app, |this: &mut LayoutView, cx| {
                        this.add_open = false;
                        this.add_landmark(kind);
                        cx.notify();
                    });
                },
            ));
        }
        chips
    } else {
        div().hidden()
    };
    let rows = part_rows_unit(Some(layout), devices, unit);
    let mut body = div()
        .id("lay-parts-body")
        .flex()
        .flex_col()
        .gap_0()
        .mt_3();
    if !one_column {
        body = body.flex_1().min_h_0().overflow_y_scroll();    }
    body = body.child(add_popover);
    let inspector_w_now = inspector_width(container_width(view.width));
    for row in rows {
        let title_text_w = crate::shell::text_width(
            window,
            &row.name,
            15.0,
            gpui::FontWeight::SEMIBOLD,
            crate::fonts::UI_FAMILY,
        );
        let title_w = title_text_w + if row.icon.is_empty() { 0.0 } else { 23.0 };
        let count_text = row
            .width_text
            .clone()
            .or_else(|| row.leds.map(|n| format!("{n}")))
            .unwrap_or_else(|| "\u{2014}".to_string());
        let count_w = crate::shell::text_width(
            window,
            &count_text,
            14.0,
            gpui::FontWeight::NORMAL,
            crate::fonts::MONO_FAMILY,
        );
        let sub_w = (inspector_w_now - 32.0 - 24.0 - title_w - count_w - 24.0).max(40.0);
        let ellipsized = crate::shell::ellipsize(
            window,
            &row.sub,
            sub_w,
            14.0,
            gpui::FontWeight::NORMAL,
            crate::fonts::UI_FAMILY,
        );
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .min_w(px(0.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(Theme::color(t.ink))
            .child(if row.icon.is_empty() {
                div()
            } else {
                div().child(
                    gpui::svg()
                        .path(format!("icons/{}.svg", row.icon))
                        .size(px(15.0))
                        .flex_none()
                        .text_color(Theme::color(t.ink)),
                )
            })
            .child(row.name.clone())
            .truncate();
        let sub = div()
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.mute))
            .child(ellipsized)
            .truncate();
        let count = div()
            .text_sm()
            .font_family(crate::fonts::MONO_FAMILY)
            .text_color(Theme::color(t.mute))
            .child(count_text);
        let mut row_el = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .w_full()
            .min_h(px(if one_column { 32.0 } else { 52.0 }))
            .px(px(12.0))
            .py(px(if one_column { 4.0 } else { 8.0 }))
            .border_b_1()
            .border_color(Theme::color(t.line))
            .child(title);
        if !one_column {
            row_el = row_el.child(div().flex_1().min_w(px(0.0)).child(sub));
        }
        row_el = row_el.child(count);
        let sel = row.sel;
        let row_entity = entity.clone();
        let row_el = row_el
            .id(("lay-row", match sel { Sel::Zone(i) => 1000 + i, Sel::Landmark(i) => 2000 + i }))
            .cursor_pointer()
            .on_click(move |_, _window, app| {
                row_entity.update(app, |this: &mut LayoutView, cx| {
                    this.select(vec![sel]);
                    cx.notify();
                });
            });
        body = body.child(row_el);
    }
    let panel_title = if view.selection.is_empty() {
        "Parts"
    } else {
        match view.selection[0] {
            Sel::Zone(_) => "Zone",
            Sel::Landmark(_) => "Part",
        }
    };
    let panel = panel_with_padding(t, "lay-parts", panel_title, actions, body, None);
    let panel = if one_column {
        panel
    } else {
        // The parts pane fills the inspector's column and the panel fills the
        // pane, so the list scrolls inside its own panel (R1).
        panel.flex_1().min_h_0()
    };
    let pane = div()
        .flex()
        .flex_col()
        .min_h_0()
        .child(panel);
    let pane = if one_column {
        pane.flex_none()
    } else {
        pane.flex_1()
    };
    pane.into_any_element()
}

#[allow(dead_code)]
fn sel_debug(sel: Sel) -> &'static str {
    match sel {
        Sel::Zone(_) => "zone",
        Sel::Landmark(_) => "landmark",
    }
}

/// The Render pane: the desk's own settings, on a phone.
fn render_panel(view: &mut LayoutView, t: &Theme, cx: &mut Context<LayoutView>) -> gpui::AnyElement {
    let layout = view.layout().cloned();
    let scale = layout
        .as_ref()
        .map(|l| l.render_scale.percent())
        .unwrap_or(50);
    let sampling = layout
        .as_ref()
        .map(|l| sampling_key(l.sampling))
        .unwrap_or("nearest");
    let scale_entity = cx.entity();
    let sampling_entity = cx.entity();
    let body = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .flex_wrap()
        .child(segmented(
            t,
            "lay-pane-scale",
            &[("25", "¼"), ("50", "½"), ("100", "All")],
            &scale.to_string(),
            move |value, _window, cx| {
                scale_entity.update(cx, |this: &mut LayoutView, cx| {
                    let pct: u8 = value.parse().unwrap_or(50);
                    let rs: RenderScale = pct.try_into().unwrap_or(RenderScale::P50);
                    this.commit(|l| Layout { render_scale: rs, ..l.clone() });
                    cx.notify();
                });
            },
        ))
        .child(segmented(
            t,
            "lay-pane-sampling",
            &[
                ("nearest", "1"),
                ("bilinear", "4"),
                ("area_average", "All px"),
            ],
            sampling,
            move |value, _window, cx| {
                sampling_entity.update(cx, |this: &mut LayoutView, cx| {
                    let sm = match value.as_str() {
                        "nearest" => SamplingMode::Nearest,
                        "bilinear" => SamplingMode::Bilinear,
                        _ => SamplingMode::AreaAverage,
                    };
                    this.commit(|l| Layout { sampling: sm, ..l.clone() });
                    cx.notify();
                });
            },
        ));
    let panel = panel_with_padding(t, "lay-render", "Render", div(), body, None);
    panel.into_any_element()
}

impl Render for LayoutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_page(self, window, cx)
    }
}
