//! The stack builder, after `app/web/src/views/effects/StackBuilder.tsx`.
//!
#![allow(dead_code)]
//! The Effects route in build mode: the picture and its layers on the left,
//! the picked layer on the right. The preview is the daemon's second engine
//! (`?stack=1`); Try on the lights is a stack draft; Save writes the stack
//! and activates it.

use std::collections::HashMap;

use canvas_proto::api::{StackDetail, StackLayerStatus, StacksResponse};
use canvas_proto::props::Preset;
use canvas_proto::stack::{BlendMode, LayerArea, Stack, StackLayer};

use gpui::{Context, Entity, IntoElement, div, prelude::*, px};

use crate::client::DaemonClient;
use crate::components::button::{Variant, button, button_icon, icon_button};
use crate::components::panel::panel_with_padding;
use crate::components::thumb::{cached, thumb};
use crate::theme::{Theme, TokenRadius};

use crate::reactlib::stack_preview::{FeedEvent, FeedHandle};
use crate::reactlib::stack_route::StackRoute;

use super::stack_area_picker::{PickerMode, PickerState};
use super::stack_draft::{
    self, area_name, area_position, blend_label, blend_note, layer_values, Schemas, StackSession,
};
use super::stack_preview::stack_preview;

/// The builder's own state.
pub struct StackBuilder {
    pub theme: Theme,
    pub width: f32,
    /// The builder's own height: the page content box's height (window
    /// minus the strip and the content padding, as the editor's height is).
    pub height: f32,
    /// The `?stack=` value: a saved stack's name, or `new`.
    pub initial_name: String,
    /// The stack as it was saved.
    pub base: Option<Stack>,
    /// The stack as it is now.
    pub draft: Stack,
    /// The name a new stack is given.
    pub name_draft: String,
    pub picked: Option<String>,
    pub statuses: HashMap<String, StackLayerStatus>,
    pub schemas: Schemas,
    pub presets: Vec<Preset>,
    pub user_presets: Vec<Preset>,
    pub show_areas: bool,
    /// The area picker, when it is open.
    pub picker: Option<PickerState>,
    /// The phone's layer sheet.
    pub sheet_open: bool,
    pub saving: bool,
    pub save_error: Option<String>,
    pub clash: Option<String>,
    pub naming: bool,
    pub naming_error: Option<String>,
    pub leaving: bool,
    pub preview_error: Option<String>,
    pub feed_problem: Option<String>,
    pub draft_state: super::editor::DraftState,
    pub draft_error: Option<String>,
    pub draft_note: Option<String>,
    pub expires_in: u32,
    /// The last frame the preview feed sent.
    pub frame: Option<(u16, u16, Vec<u8>)>,
    /// The feed's events.
    pub feed_rx: Option<std::sync::mpsc::Receiver<FeedEvent>>,
    pub feed_handle: Option<FeedHandle>,
    /// The page that opened the builder.
    pub page: Option<Entity<super::EffectsView>>,
    /// The session's epoch and sequence.
    pub session: StackSession,
    /// The other stacks' names, for the name rule.
    pub other_names: Vec<String>,
    /// The layer the area is being dragged.
    pub drag: Option<AreaDrag>,
    /// The blend list is open.
    pub blend_open: bool,
    /// The opacity slider's own box.
    pub slider_origin: crate::components::slider::Origin,
    /// Thumbnail fetches in flight: one per effect per session (React
    /// `loadThumb`, which caches one request per effect).
    pub thumbs_pending: std::collections::HashSet<String>,
    /// The library's effects, for the Effect select (React's `effects` prop:
    /// every non-stack effect).
    pub effects: Vec<canvas_proto::api::EffectSummary>,
    /// The Effect select is open.
    pub effect_open: bool,
    /// The preset-save problem or the name field above the preset line.
    pub preset_problem: Option<String>,
    pub preset_name: Option<String>,
}

/// A drag on the picture's area frame.
#[derive(Debug, Clone, Copy)]
pub enum AreaDrag {
    Move { from: (f32, f32), start: stack_draft::Bounds },
    Resize { from: (f32, f32), start: stack_draft::Bounds, sx: i32, sy: i32 },
}

/// The builder's phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildPhase {
    Loading,
    Ready,
    Error,
}

impl StackBuilder {
    /// A new builder for the stack named `initial_name`.
    pub fn new(
        theme: Theme,
        initial_name: &str,
        page: Option<Entity<super::EffectsView>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_new = initial_name == crate::reactlib::stack_route::NEW_STACK;
        let mut builder = StackBuilder {
            theme,
            width: 1280.0,
            height: 805.0,
            initial_name: initial_name.to_string(),
            base: None,
            draft: Stack { id: "stack:".to_string(), layers: vec![] },
            name_draft: String::new(),
            picked: None,
            statuses: HashMap::new(),
            schemas: Schemas::new(),
            presets: vec![],
            user_presets: vec![],
            show_areas: true,
            picker: None,
            sheet_open: false,
            saving: false,
            save_error: None,
            clash: None,
            naming: false,
            naming_error: None,
            leaving: false,
            preview_error: None,
            feed_problem: None,
            draft_state: super::editor::DraftState::Off,
            draft_error: None,
            draft_note: None,
            expires_in: 0,
            frame: None,
            feed_rx: None,
            feed_handle: None,
            page,
            session: StackSession::new(stack_draft::new_epoch()),
            other_names: vec![],
            drag: None,
            blend_open: false,
            slider_origin: crate::components::slider::Origin::new(),
            thumbs_pending: std::collections::HashSet::new(),
            effects: Vec::new(),
            effect_open: false,
            preset_problem: None,
            preset_name: None,
        };
        if !is_new {
            builder.load(cx);
        }
        builder.load_effects(cx);
        builder.load_names(cx);
        builder.start_feed(cx);
        builder
    }

    /// Whether the builder is on a stack that has never been saved.
    pub fn is_new(&self) -> bool {
        self.initial_name == crate::reactlib::stack_route::NEW_STACK
    }

    /// The stack's name, as the title says it.
    pub fn name(&self) -> String {
        if self.is_new() {
            let typed = self.name_draft.trim().to_string();
            if typed.is_empty() {
                "New stack".to_string()
            } else {
                typed
            }
        } else {
            name_of(&self.base.as_ref().map(|b| b.id.clone()).unwrap_or_else(|| {
                if self.initial_name.starts_with("stack:") {
                    self.initial_name.clone()
                } else {
                    format!("stack:{}", self.initial_name)
                }
            }))
        }
    }

    /// How many layers the stack has.
    pub fn count(&self) -> usize {
        self.draft.layers.len()
    }

    /// The picked layer.
    pub fn picked_layer(&self) -> Option<&StackLayer> {
        let picked = self.picked.as_ref()?;
        self.draft.layers.iter().find(|layer| &layer.id == picked)
    }

    /// The stack is dirty when its layers differ from the saved copy.
    pub fn dirty(&self) -> bool {
        match &self.base {
            Some(base) => base.layers != self.draft.layers,
            None => !self.draft.layers.is_empty() || !self.name_draft.is_empty(),
        }
    }

    // -- reads ---------------------------------------------------------------

    /// `GET /api/v1/stacks/{id}`.
    fn load(&mut self, cx: &mut Context<Self>) {
        let name = self.initial_name.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.get(&format!("/api/v1/stacks/{}", encode(&name)))
        });
        cx.spawn(async move |builder, cx| {
            let result = bg.await;
            let _ = builder.update(cx, |this, cx| {
                match result {
                    Ok(body) => match serde_json::from_str::<StackDetail>(&body) {
                        Ok(detail) => {
                            this.base = Some(detail.stack.clone());
                            this.draft = detail.stack.clone();
                            this.statuses = detail
                                .layers
                                .iter()
                                .map(|l| (l.id.clone(), l.clone()))
                                .collect();
                            this.picked = detail.stack.layers.last().map(|l| l.id.clone());
                            this.load_details(cx);
                            // The first draft sends at once (StackBuilder.tsx:
                            // the preview's first PUT goes out on open, so the
                            // feed has an engine to show).
                            this.send_preview(cx);
                            this.fetch_thumbs(cx);
                        }
                        Err(e) => this.save_error = Some(e.to_string()),
                    },
                    Err(e) => this.save_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `GET /api/v1/stacks`: the other stacks' names.
    fn load_names(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.get("/api/v1/stacks")
        });
        cx.spawn(async move |builder, cx| {
            let result = bg.await;
            let _ = builder.update(cx, |this, cx| {
                if let Ok(body) = result {
                    if let Ok(parsed) = serde_json::from_str::<StacksResponse>(&body) {
                        this.other_names = parsed
                            .stacks
                            .iter()
                            .filter(|s| this.base.as_ref().map(|b| b.id != s.id).unwrap_or(true))
                            .map(|s| s.name.clone())
                            .collect();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `GET /api/v1/effects/{id}` for every layer's effect.
    fn load_details(&mut self, cx: &mut Context<Self>) {
        for layer in self.draft.layers.clone() {
            if self.schemas.contains_key(&layer.effect) {
                continue;
            }
            let id = layer.effect.clone();
            let id_bg = id.clone();
            let bg = cx.background_executor().spawn(async move {
                let client = DaemonClient::new();
                client.get(&format!("/api/v1/effects/{}", encode(&id_bg)))
            });
            cx.spawn(async move |builder, cx| {
                let result = bg.await;
                let _ = builder.update(cx, |this, cx| {
                    let schema = match result {
                        Ok(body) => serde_json::from_str::<canvas_proto::api::EffectDetail>(&body)
                            .ok()
                            .map(|d| d.schema),
                        Err(_) => None,
                    };
                    this.schemas.insert(id.clone(), schema);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// `GET /api/v1/effects`: the library, for the Effect select.
    fn load_effects(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.get("/api/v1/effects")
        });
        cx.spawn(async move |builder, cx| {
            let result = bg.await;
            let _ = builder.update(cx, |this, cx| {
                if let Ok(body) = result {
                    if let Ok(parsed) = serde_json::from_str::<canvas_proto::api::EffectsResponse>(&body) {
                        this.effects = parsed.effects;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Fetch a thumbnail for every layer's effect that has none yet, one
    /// request per effect per session (React `Thumb`, which loads on mount).
    fn fetch_thumbs(&mut self, cx: &mut Context<Self>) {
        use crate::components::thumb::{cached, fetch as fetch_thumb};
        let need: Vec<String> = self
            .draft
            .layers
            .iter()
            .map(|l| l.effect.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .filter(|id| cached(id).is_none() && !self.thumbs_pending.contains(id))
            .collect();
        for id in need {
            self.thumbs_pending.insert(id.clone());
            fetch_thumb(id, cx.entity(), cx, |this, cx, _id| {
                cx.notify();
                let _ = this;
            });
        }
    }

    // -- the preview feed ----------------------------------------------------

    /// Opens the preview feed and polls it.
    fn start_feed(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.feed_rx = Some(rx);
        let port = crate::client::api_port();
        let token = crate::client::api_token().unwrap_or_default();
        self.feed_handle = Some(crate::reactlib::stack_preview::start_feed(port, token, tx));
        cx.spawn(async move |builder, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                let _ = builder.update(cx, |this, cx| this.drain_feed(cx));
            }
        })
        .detach();
    }

    /// Takes the frames the feed has sent.
    fn drain_feed(&mut self, cx: &mut Context<Self>) {
        let Some(rx) = &self.feed_rx else {
            return;
        };
        let mut changed = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                FeedEvent::Frame(frame) => {
                    self.frame = Some((frame.width, frame.height, frame.rgb));
                    changed = true;
                }
                FeedEvent::Refused(reason) => {
                    self.feed_problem = Some(reason);
                    changed = true;
                }
                FeedEvent::Closed => {
                    changed = true;
                }
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// A change of the draft goes to the daemon's preview.
    pub fn send_preview(&mut self, cx: &mut Context<Self>) {
        let stack = self.stack_for_server();
        let (epoch, seq) = self.session.next();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            let body = serde_json::to_string(&canvas_proto::api::StackPreviewRequest {
                stack,
                epoch,
                seq,
            })
            .unwrap_or_else(|_| "{}".into());
            client.put_json("/api/v1/stacks/preview", &body)
        });
        cx.spawn(async move |builder, cx| {
            let result = bg.await;
            let _ = builder.update(cx, |this, cx| {
                match result {
                    Ok(body) => {
                        if let Ok(parsed) = serde_json::from_str::<canvas_proto::api::StackPreviewResponse>(&body) {
                            this.session.observe(Some(&parsed.epoch), Some(parsed.seq));
                            if parsed.applied {
                                for layer in parsed.layers {
                                    this.statuses.insert(
                                        layer.id.clone(),
                                        StackLayerStatus {
                                            id: layer.id.clone(),
                                            status: layer.status,
                                            last_ms: None,
                                        },
                                    );
                                }
                                this.preview_error = None;
                            }
                        }
                    }
                    Err(e) => this.preview_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The stack as the daemon must receive it.
    pub fn stack_for_server(&self) -> Stack {
        let id = if self.is_new() {
            format!("stack:{}", self.name_draft.trim())
        } else {
            self.base
                .as_ref()
                .map(|b| b.id.clone())
                .unwrap_or_else(|| format!("stack:{}", self.initial_name))
        };
        Stack { id, layers: stack_draft::layers_for_server(&self.draft.layers) }
    }

    // -- layer edits ----------------------------------------------------------

    /// A layer's area changed.
    pub fn set_area(&mut self, area: LayerArea, cx: &mut Context<Self>) {
        let Some(picked) = self.picked.clone() else {
            return;
        };
        let mut layers = self.draft.layers.clone();
        for layer in layers.iter_mut() {
            if layer.id == picked {
                layer.area = area.clone();
            }
        }
        self.draft.layers = layers;
        self.send_preview(cx);
        cx.notify();
    }

    /// A layer's blend changed.
    pub fn set_blend(&mut self, blend: BlendMode, cx: &mut Context<Self>) {
        let Some(picked) = self.picked.clone() else {
            return;
        };
        let mut layers = self.draft.layers.clone();
        for layer in layers.iter_mut() {
            if layer.id == picked {
                layer.blend = blend;
            }
        }
        self.draft.layers = layers;
        self.send_preview(cx);
        cx.notify();
    }

    /// A layer's opacity changed.
    pub fn set_opacity(&mut self, opacity: u8, cx: &mut Context<Self>) {
        let Some(picked) = self.picked.clone() else {
            return;
        };
        let mut layers = self.draft.layers.clone();
        for layer in layers.iter_mut() {
            if layer.id == picked {
                layer.opacity = opacity;
            }
        }
        self.draft.layers = layers;
        self.send_preview(cx);
        cx.notify();
    }

    /// A layer's visibility flipped.
    pub fn toggle_visible(&mut self, id: &str, cx: &mut Context<Self>) {
        self.draft.layers = stack_draft::toggle_layer(&self.draft.layers, id);
        self.send_preview(cx);
        cx.notify();
    }

    /// A layer is picked.
    pub fn pick(&mut self, id: &str, cx: &mut Context<Self>) {
        self.picked = Some(id.to_string());
        cx.notify();
    }

    // -- save ------------------------------------------------------------------

    /// Save (`save` in React): the stack's own route, then the activate.
    pub fn save(&mut self, replace_id: Option<String>, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if self.count() == 0 {
            self.save_error = Some("Add a layer before saving.".to_string());
            return;
        }
        let wanted = if self.is_new() {
            self.name_draft.trim().to_string()
        } else {
            self.name()
        };
        if wanted.is_empty() {
            self.naming = true;
            self.naming_error = None;
            return;
        }
        if replace_id.is_none() {
            if let Some(problem) = stack_draft::stack_name_problem(&wanted, &self.other_names) {
                self.save_error = Some(problem);
                return;
            }
        }
        self.saving = true;
        self.save_error = None;
        let page = self.page.clone();
        let id = replace_id.unwrap_or_else(|| format!("stack:{wanted}"));
        let stack = Stack { id: id.clone(), layers: stack_draft::layers_for_server(&self.draft.layers) };
        let stack_bg = stack.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            let body = serde_json::to_string(&stack_bg).unwrap_or_else(|_| "{}".into());
            client.put_json(&format!("/api/v1/stacks/{}", encode(&id)), &body)
        });
        cx.spawn(async move |builder, cx| {
            let result = bg.await;
            let saved: Option<Stack> = match result {
                Ok(body) => serde_json::from_str::<Stack>(&body).ok(),
                Err(crate::client::ClientError::Daemon { status: 409, .. }) => {
                    let _ = builder.update(cx, |this, cx| {
                        this.saving = false;
                        this.clash = Some(wanted.clone());
                        cx.notify();
                    });
                    return;
                }
                Err(e) => {
                    let _ = builder.update(cx, |this, cx| {
                        this.saving = false;
                        this.save_error = Some(e.to_string());
                        cx.notify();
                    });
                    return;
                }
            };
            let saved = saved.unwrap_or(stack);
            let activate_id = saved.id.clone();
            let _ = builder.update(cx, |this, cx| {
                this.saving = false;
                this.clash = None;
                this.base = Some(saved.clone());
                this.draft = saved.clone();
                this.name_draft = name_of(&saved.id);
                cx.notify();
            });
            let bg = cx.background_executor().spawn(async move {
                let client = DaemonClient::new();
                let body = serde_json::to_string(&serde_json::json!({ "props": {}, "preset": serde_json::Value::Null }))
                    .unwrap_or_else(|_| "{}".into());
                client.post_json(&format!("/api/v1/effects/{}/activate", encode(&activate_id)), &body)
            });
            let answer = bg.await;
            let _ = builder.update(cx, |this, cx| {
                if let Err(e) = answer {
                    this.save_error = Some(format!("Saved, but it did not start: {e}"));
                }
                cx.notify();
            });
            if let Some(page) = page {
                let _ = page.update(cx, |this, cx| this.load_list(cx));
            }
        })
        .detach();
    }

    // -- the draft on the lights -------------------------------------------------

    /// Starts the draft ("Try on the lights").
    pub fn start_draft(&mut self, cx: &mut Context<Self>) {
        if self.count() == 0 {
            return;
        }
        self.draft_state = super::editor::DraftState::Starting;
        self.draft_error = None;
        self.draft_note = None;
        let stack = self.stack_for_server();
        let (epoch, seq) = self.session.next();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            let body = serde_json::to_string(&canvas_proto::api::DraftRequest {
                html: String::new(),
                stack: Some(stack),
                epoch: Some(epoch),
                seq: Some(seq),
                props: Default::default(),
            })
            .unwrap_or_else(|_| "{}".into());
            client.put_json("/api/v1/effects/draft", &body)
        });
        cx.spawn(async move |builder, cx| {
            let result = bg.await;
            let _ = builder.update(cx, |this, cx| {
                match result {
                    Ok(body) => {
                        let parsed = serde_json::from_str::<canvas_proto::api::DraftStatus>(&body).ok();
                        this.draft_state = super::editor::DraftState::Running;
                        this.draft_error = None;
                        if let Some(parsed) = parsed {
                            this.expires_in = (parsed.lease_remaining_ms / 1000) as u32;
                        }
                    }
                    Err(e) => {
                        this.draft_state = super::editor::DraftState::Off;
                        this.draft_error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Stops the draft ("Stop").
    pub fn stop_draft(&mut self, cx: &mut Context<Self>) {
        self.draft_state = super::editor::DraftState::Restoring;
        self.draft_error = None;
        self.draft_note = None;
        let epoch = self.session.epoch.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.delete(&format!("/api/v1/effects/draft?epoch={epoch}"))
        });
        cx.spawn(async move |builder, cx| {
            let result = bg.await;
            let _ = builder.update(cx, |this, cx| {
                match result {
                    Ok(body) => {
                        this.draft_state = super::editor::DraftState::Off;
                        let ended = serde_json::from_str::<canvas_proto::api::DraftEnded>(&body).ok();
                        if let Some(ended) = ended {
                            if let Some(note) = ended.note {
                                this.draft_note = Some(note);
                            }
                        }
                    }
                    Err(e) => {
                        this.draft_state = super::editor::DraftState::RestoreError;
                        this.draft_error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A change the builder makes.
    pub fn notify(&self) {}

    /// The layer's name for a row (the schema's, then the library's).
    pub fn layer_label(&self, effect: &str) -> String {
        match self.schemas.get(effect) {
            Some(Some(schema)) if !schema.name.is_empty() => schema.name.clone(),
            _ => "Loading…".to_string(),
        }
    }

    /// The Effect select's options: every non-stack effect from the library,
    /// with the picked layer's own entry kept when its effect is gone
    /// ("Missing effect"), as React's `Select` for the picked layer does.
    fn effect_options(&self) -> Vec<(&str, &str)> {
        let picked_effect = self.picked_layer().map(|l| l.effect.as_str()).unwrap_or("");
        let mut out: Vec<(&str, &str)> = Vec::new();
        if !picked_effect.is_empty() && self.effects.iter().all(|e| e.id != picked_effect) {
            out.push((picked_effect, "Missing effect"));
        }
        for e in self.effects.iter().filter(|e| !e.id.starts_with("stack:")) {
            out.push((e.id.as_str(), e.name.as_str()));
        }
        out
    }

    /// The picked layer takes the chosen effect: its schema is read, the
    /// layer takes the effect's saved values and its own defaults, over the
    /// HTML-layer cap with the problem sentence (StackBuilder.tsx).
    pub fn choose_effect(&mut self, effect: &str, cx: &mut Context<Self>) {
        self.effect_open = false;
        let Some(picked_id) = self.picked.clone() else { return; };
        // HTML-layer cap: two HTML layers at most.
        let is_html = effect.starts_with("html:");
        let currently_html = self.picked_layer().map(|l| l.effect.starts_with("html:")).unwrap_or(false);
        if is_html && !currently_html {
            let html_count = self.draft.layers.iter().filter(|l| l.effect.starts_with("html:")).count();
            if html_count >= stack_draft::MAX_HTML_LAYERS {
                self.save_error = Some(format!("A stack holds {} HTML layers", stack_draft::MAX_HTML_LAYERS));
                cx.notify();
                return;
            }
        }
        self.save_error = None;
        let id_bg = effect.to_string();
        let id_for_fetch = id_bg.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.get(&format!("/api/v1/effects/{}", encode(&id_for_fetch)))
        });
        let picked_bg = picked_id.clone();
        cx.spawn(async move |builder, cx| {
            let result = bg.await;
            let _ = builder.update(cx, |this, cx| {
                let (schema, saved_props) = match result {
                    Ok(body) => match serde_json::from_str::<canvas_proto::api::EffectDetail>(&body) {
                        Ok(detail) => (Some(detail.schema), detail.props),
                        Err(_) => (None, Default::default()),
                    },
                    Err(_) => (None, Default::default()),
                };
                this.schemas.insert(id_bg.clone(), schema.clone());
                if let Some(layer) = this.draft.layers.iter_mut().find(|l| l.id == picked_bg) {
                    layer.effect = id_bg.clone();
                    layer.props = stack_draft::layer_values(schema.as_ref(), &saved_props);
                }
                this.fetch_thumbs(cx);
                this.send_preview(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // -- the render --------------------------------------------------------------

    /// The builder's body: the picture and its layers on the left, the picked
    /// layer on the right.
    pub fn render(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let narrow = self.width <= 720.0;
        let entity = cx.entity();
        let side_w = if self.width > 1150.0 { 360.0 } else { 320.0 };

        // The phone's column reads stage, layers, then the picked layer's
        // summary (React `rd-stack-main` + `rd-layer-summary`); the desktop
        // puts the picked panel in the side instead. React's phone gaps are
        // `s3` (12 px): the stack main, the body and the page all drop from
        // `s4` (16 px, desktop `.rd-stack-main`, `.rd-stack-body`,
        // `.rd-page`) to `s3` under `.rd-stacks-phone`.
        let col_gap = if narrow { gpui::px(12.0) } else { gpui::px(16.0) };
        let mut main = div().flex().flex_col().gap(col_gap).min_w(px(0.0)).min_h(px(0.0)).flex_1();
        main = main.child(self.stage(cx));
        main = main.child(self.layers_panel(cx));
        if narrow && self.picked_layer().is_some() {
            main = main.child(self.layer_summary(cx));
        }

        let mut side = div()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(side_w))
            .flex_none()
            .min_h(px(0.0))
            .child(self.picked_panel(cx));
        if !narrow {
            side = side.child(self.presets_panel(cx));
        }

        // The phone's body is the main column alone (the side's sheet is
        // hidden until Tune opens it, and the summary already sits in the
        // column); the desktop is main beside the side.
        let body = if narrow {
            div()
                .flex()
                .flex_col()
                .gap(col_gap)
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .child(main)
                .into_any_element()
        } else {
            div()
                .flex()
                .gap_4()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .child(main)
                .child(side)
                .into_any_element()
        };

        let mut page = div().flex().flex_col().gap(col_gap).flex_1().min_h(px(0.0)).min_w(px(0.0));
        if narrow {
            // React's phone `.content` for this page is `s2 s3` (8 px top,
            // 12 px sides: `.app:has(.rd-stacks-phone) .content`), but the
            // shell's phone content is 16 px on the sides, so the panels
            // sat at x 16-374 instead of React's x 12-378. Pull the page
            // out 4 px on each side to land on React's box.
            page = page.mx(px(-4.0)).mt(px(-8.0));
        }
        if !narrow && self.height > 0.0 {
            // The content box's height, as the editor's render.rs sets it:
            // without it the page is its content's height and the stage
            // comes up ~90 px short of React's.
            page = page.h(px(self.height));
        }
        if let Some(note) = self.draft_note.clone() {
            page = page.child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .border_l_2()
                    .border_color(Theme::color(t.warn))
                    .text_size(px(15.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.warn))
                    .child(note),
            );
        }
        if let Some(clash) = self.clash.clone() {
            let entity = entity.clone();
            page = page.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .r_md()
                    .border_1()
                    .border_color(Theme::color(t.warn))
                    .bg(Theme::color(t.warn_wash))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(15.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(format!(
                                "name exists  A stack named {clash} already exists. Replace it with this stack? Its layers are overwritten; its presets and favourite stay. If it is running, the lights change to this stack."
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(button(&t, "rd-clash-replace", "Replace", Variant::Default, self.saving, {
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.save(None, cx));
                                }
                            }))
                            .child(button(&t, "rd-clash-cancel", "Cancel", Variant::Ghost, false, {
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.clash = None;
                                        cx.notify();
                                    });
                                }
                            })),
                    ),
            );
        }
        page = page.child(body);
        // The area picker, when it is open.
        if let Some(picker) = self.picker.clone() {
            if let Some(layer) = self.picked_layer().cloned() {
                let entity = cx.entity();
                page = page.child(
                    div()
                        .id(gpui::SharedString::from("rd-picker-scrim"))
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_6()
                        .bg(Theme::color(t.scrim))
                        .child(super::stack_area_picker::area_picker(
                            &t,
                            "rd-area-picker",
                            &layer,
                            &picker,
                            None,
                            None,
                            {
                                let entity = entity.clone();
                                move |value: String, _window, cx| {
                                    entity.update(cx, |this, cx| this.set_mode(&value, cx));
                                }
                            },
                            {
                                let entity = entity.clone();
                                move |value: String, _window, cx| {
                                    entity.update(cx, |this, cx| this.set_device(&value, cx));
                                }
                            },
                            {
                                let entity = entity.clone();
                                move |value: String, _window, cx| {
                                    entity.update(cx, |this, cx| this.set_zone(&value, cx));
                                }
                            },
                            {
                                let entity = entity.clone();
                                move |at: (f32, f32), _last: Option<(f32, f32)>, _window, cx| {
                                    entity.update(cx, |this, cx| this.desk_press(at, cx));
                                }
                            },
                            {
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.use_area(cx));
                                }
                            },
                            {
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.close_picker(cx));
                                }
                            },
                        )),
                );
            }
        }
        page.into_any_element()
    }

    /// The stage: the picture and its foot.
    fn stage(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let entity = cx.entity();
        let count = self.count();
        // React's phone keys drop their words under `@media (max-width:
        // 640px)` (`.rd-btn-text { display: none }`): Try on the lights and
        // Add layer are icon-only keys at 390, not labelled ones.
        let icon_only = self.width <= 640.0;
        let narrow = self.width <= 720.0;
        // The phone stage's picture height: React's `.rd-canvas-wrap` takes
        // its height from its width (`aspect-ratio: 320 / 200`), so the
        // picture is (panel inner width) / 1.6 tall — about 212 px at 390
        // (panel x 12-378 less 12 px padding and the 1 px border each
        // side), not a flex row's leftover. The builder's width is the
        // window width less the shell's 16 px phone content padding on
        // each side, less the page's 4 px pull-out on each side.
        let canvas_h = if narrow {
            let inner_w = self.width - 32.0 - 26.0 + 8.0;
            inner_w / 1.6
        } else {
            0.0
        };
        let picked = self.picked_layer().cloned();
        let editable = picked
            .as_ref()
            .map(|layer| matches!(layer.area, LayerArea::Rect { .. }))
            .unwrap_or(false);
        let area = picked.as_ref().and_then(|layer| {
            let bounds = stack_draft::area_bounds(&layer.area, None)?;
            Some((
                bounds,
                format!(
                    "{} · {}",
                    layer.name,
                    area_name(&layer.area, None, None)
                ),
            ))
        });

        // The head's actions: the panel's title is "PREVIEW" on the left
        // and the meta on the right, on one row. On a phone the meta is
        // the size and the rate only (`.rd-stacks-phone .rd-show-areas {
        // display: none }`): the areas switch is a desktop control.
        let mut head = div()
            .flex()
            .items_center()
            .gap_3()
            .min_w(px(0.0))
            .child(
                div()
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("320 × 200 · 15 fps"),
            );
        if !narrow {
            head = head.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(14.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child("Show areas"),
                    )
                    .child(crate::components::toggle::toggle(
                        &t,
                        "rd-show-areas",
                        self.show_areas,
                        {
                            let entity = entity.clone();
                            move |checked, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.show_areas = checked;
                                    cx.notify();
                                });
                            }
                        },
                    )),
            );
        }

        // An empty stack's stage: the centred words (`StackPreviewCanvas`'s
        // `.rd-canvas-empty` replaces the canvas while the stack has no
        // layer: "Nothing to show yet" over "Add a layer and its picture
        // appears here.").
        let picture: gpui::AnyElement = if count == 0 {
            let mut empty = div()
                .r_md()
                .bg(Theme::color(t.sunk))
                .flex()
                .flex_col()
                .items_center()
                .gap_1();
            if narrow {
                // `justify_center` does not centre a column's children in
                // this GPUI build, so the centred words are held by two
                // equal flex spacers instead.
                empty = empty
                    .flex_none()
                    .h(px(canvas_h))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(15.0))
                            .line_height(gpui::relative(1.45))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child("Nothing to show yet"),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child("Add a layer and its picture appears here."),
                    )
                    .child(div().flex_1());
            } else {
                empty = empty
                    .flex_1()
                    .min_h(px(0.0))
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .line_height(gpui::relative(1.45))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child("Nothing to show yet"),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child("Add a layer and its picture appears here."),
                    );
            }
            empty.into_any_element()
        } else {
            let preview_el = stack_preview(
                &t,
                "rd-canvas",
                self.frame.clone(),
                area,
                editable,
                super::stack_preview::StageBox::new(),
                {
                    let entity = entity.clone();
                    move |_at, _window, _cx| {
                        let _ = &entity;
                    }
                },
            );
            // At phone width the wrap takes its height from its width
            // (React `.rd-stacks-phone .rd-canvas-wrap { aspect-ratio:
            // 320 / 200 }`), instead of the stage's leftover height.
            if narrow {
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .h(px(canvas_h))
                    .child(preview_el)
                    .into_any_element()
            } else {
                preview_el.into_any_element()
            }
        };

        let foot: gpui::AnyElement = if self.draft_state == super::editor::DraftState::Off {
            let note = if let Some(err) = self.preview_error.clone() {
                format!("The preview stopped: {err}")
            } else if let Some(err) = self.draft_error.clone() {
                format!("Try on the lights failed: {err}")
            } else if let Some(problem) = self.feed_problem.clone() {
                format!("The preview is not showing: {problem}")
            } else if count == 0 {
                "An empty stack runs nothing; the lights keep their effect.".to_string()
            } else {
                "Preview only — nothing is on the lights.".to_string()
            };
            // React's foot is `flex-wrap`: a long note (the empty stack's)
            // takes the full row and the Try key wraps below it; a short
            // note ("Preview only — …") leaves the key on the same row.
            // A flex key beside a shrinking note never wraps in GPUI, so
            // the long case is a column: note full width, key under it.
            let long_note = note.chars().count() > 44;
            let foot = div()
                .flex()
                .gap(if narrow { gpui::px(8.0) } else { gpui::px(12.0) })
                .flex_wrap()
                .min_w(px(0.0));
            let foot = if narrow && long_note {
                foot.flex_col().items_start().child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .text_size(px(14.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(note),
                )
            } else {
                foot.items_center().justify_between().child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(14.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(note),
                )
            };
            foot.child(button_icon(
                    &t,
                    "rd-try-lights",
                    if icon_only { "" } else { "Try on the lights" },
                    Variant::Default,
                    count == 0,
                    true,
                    "Play",
                    "",
                    15.0,
                    {
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| this.start_draft(cx));
                        }
                    },
                ))
                .into_any_element()
        } else {
            let text = match self.draft_state {
                super::editor::DraftState::Starting => "Starting the draft…".to_string(),
                super::editor::DraftState::Running => "Draft on the lights".to_string(),
                super::editor::DraftState::Expiring => {
                    format!("Draft on the lights, expiring in {}s", self.expires_in)
                }
                super::editor::DraftState::Restoring => "Restoring the previous effect…".to_string(),
                super::editor::DraftState::RestoreError => {
                    "The previous effect could not be restored.".to_string()
                }
                super::editor::DraftState::Off => String::new(),
            };
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .flex_wrap()
                .min_w(px(0.0))
                .border_t_1()
                .border_color(Theme::color(t.line))
                .pt_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(14.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(text),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(button(&t, "rd-draft-stop", "Stop", Variant::Ghost, self.draft_state == super::editor::DraftState::Restoring, {
                            let entity = entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| this.stop_draft(cx));
                            }
                        }))
                        .child(button(&t, "rd-draft-save", "Save", Variant::Primary, self.saving || count == 0, {
                            let entity = entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| this.save(None, cx));
                            }
                        })),
                )
                .into_any_element()
        };

        // The phone stage keeps its content height (`flex: 0 1 auto`,
        // `.rd-stacks-phone .rd-stack-stage`) and its gaps are `s2` (8 px),
        // not the desktop `s3` (12 px); the desktop stage takes the panel's
        // remaining height. The panel (and its body) must be `flex: none`
        // at phone width: left to flex, the panel grows into the main
        // column's leftover height and the picture sits at its top with
        // an empty well under it.
        let body_gap = if narrow { gpui::px(8.0) } else { gpui::px(12.0) };
        let mut body = div().flex().flex_col().gap(body_gap).min_h(px(0.0)).child(picture).child(foot);
        if narrow {
            body = body.flex_none();
        } else {
            body = body.flex_1();
        }
        let mut panel = panel_with_padding(
            &t,
            "rd-stack-stage",
            "Preview",
            head,
            body,
            Some(12.0),
        );
        if narrow {
            panel = panel.flex_none();
        } else {
            panel = panel.flex_1();
        }
        panel.into_any_element()
    }

    /// The layer list.
    fn layers_panel(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let entity = cx.entity();
        let count = self.count();
        let narrow = self.width <= 720.0;
        let icon_only = self.width <= 640.0;
        let head = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(format!("{count} / {}", stack_draft::MAX_LAYERS)),
            )
            .child(button_icon(
                &t,
                "rd-add-layer",
                if icon_only { "" } else { "Add layer" },
                Variant::Primary,
                count >= stack_draft::MAX_LAYERS,
                true,
                "Plus",
                "",
                15.0,
                {
                    let entity = entity.clone();
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.add_layer(cx);
                        });
                    }
                },
            ));

        let body: gpui::AnyElement = if count == 0 {
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_2()
                .px_2()
                .py_3()
                .min_w(px(0.0))
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .text_size(px(15.0))
                        .line_height(gpui::relative(1.45))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child("No layers yet. A stack is a list of effects drawn one over the other, bottom to top."),
                )
                // The dashes are drawn bars (`.rd-how-list li::before` is
                // an 8 x 2 px run of the divider colour, not a bullet).
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .w_full()
                        .min_w(px(0.0))
                        .text_size(px(14.0))
                        .line_height(gpui::relative(1.5))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(
                            div().flex().items_start().gap_2().min_w(px(0.0)).child(
                                div().flex_none().mt(px(9.0)).w(px(8.0)).h(px(2.0)).bg(Theme::color(t.line_2)),
                            ).child(
                                div().flex_1().min_w(px(0.0)).child("The bottom layer is painted first, then each layer over it."),
                            ),
                        )
                        .child(
                            div().flex().items_start().gap_2().min_w(px(0.0)).child(
                                div().flex_none().mt(px(9.0)).w(px(8.0)).h(px(2.0)).bg(Theme::color(t.line_2)),
                            ).child(
                                div().flex_1().min_w(px(0.0)).child("A layer paints on its canvas, a device, a zone or a rectangle."),
                            ),
                        )
                        .child(
                            div().flex().items_start().gap_2().min_w(px(0.0)).child(
                                div().flex_none().mt(px(9.0)).w(px(8.0)).h(px(2.0)).bg(Theme::color(t.line_2)),
                            ).child(
                                div().flex_1().min_w(px(0.0)).child("Blend and opacity mix it over what is under it."),
                            ),
                        ),
                )
                .child(button_icon(
                    &t,
                    "rd-add-first",
                    "Add the first layer",
                    Variant::Primary,
                    false,
                    false,
                    "Plus",
                    "",
                    15.0,
                    {
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.add_layer(cx);
                            });
                        }
                    },
                ))
                .into_any_element()
        } else {
            // React `.layer-list { margin: 0 calc(var(--s2) * -1) }`: the
            // list runs 8 px wider than the panel's content box on each
            // side, so its rows sit 8 px left of the head's text.
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .ml(px(-8.0))
                .mr(px(-8.0));
            for layer in self.draft.layers.iter().rev() {
                list = list.child(self.layer_row(layer, cx));
            }
            list.into_any_element()
        };

        // React `.rd-layers` padding is `s3` (12 px) on desktop and `s2`
        // (8 px) on a phone (`.rd-stacks-phone .rd-layers`).
        panel_with_padding(&t, "rd-layers", "Layers", head, body, Some(if narrow { 8.0 } else { 12.0 })).into_any_element()
    }

    /// One layer row.
    fn layer_row(&self, layer: &StackLayer, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let entity = cx.entity();
        let picked = self.picked.as_deref() == Some(layer.id.as_str());
        let status = self.statuses.get(&layer.id);
        let words = stack_draft::status_words(
            status.map(|s| s.status),
            status.and_then(|s| s.last_ms),
        );
        let meta = match &words {
            Some(words) => words.clone(),
            None => format!(
                "{} {}% · {}",
                blend_label(layer.blend),
                layer.opacity,
                area_name(&layer.area, None, None)
            ),
        };
        let id = layer.id.clone();
        let effect = layer.effect.clone();
        let name = layer.name.clone();

        let mut row = div()
            .id(gpui::SharedString::from(format!("rd-layer-{id}")))
            .flex()
            .items_center()
            .gap_2()
            // React `.layer-row` is 54 px, and 50 px on a phone
            // (`.rd-stacks-phone .layer-row`).
            .min_h(px(if self.width <= 720.0 { 50.0 } else { 54.0 }))
            .px_2()
            .py_1()
            .r_sm()
            .border_1();
        if picked {
            row = row.bg(Theme::color(t.raised)).border_color(Theme::color(t.line_2));
        } else {
            row = row.border_color(gpui::transparent_black());
        }
        // The grip.
        row = row.child(
            div()
                .w(px(20.0))
                .h(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .text_color(Theme::color(t.mute))
                .child(gpui::svg().path("icons/GripDots.svg").size(px(16.0))),
        );
        // The thumbnail.
        row = row.child(thumb(&t, &effect, 44.0, 28.0, cached(&effect), false));
        // The name and the meta.
        row = row.child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .child(
                    div()
                        .text_size(px(15.0))
                        .line_height(gpui::relative(1.45))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.ink))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(name.clone()),
                )
                .child(
                    div()
                        .text_size(px(14.0))
                        .line_height(gpui::relative(1.25))
                        .font_family(crate::fonts::MONO_FAMILY)
                        .text_color(Theme::color(if words.is_some() { t.warn } else { t.mute }))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(meta),
                ),
        );
        // The eye.
        let entity_eye = entity.clone();
        let eye_id = id.clone();
        let visible = layer.visible;
        row = row.child(icon_button(
            &t,
            &format!("rd-eye-{id}"),
            if visible { "Eye" } else { "EyeSlash" },
            if visible { "" } else { "fill" },
            false,
            false,
            false,
            move |_, _, cx| {
                entity_eye.update(cx, |this, cx| this.toggle_visible(&eye_id, cx));
            },
        ));
        let entity_row = entity.clone();
        row.cursor_pointer().on_click(move |_, _, cx| {
            entity_row.update(cx, |this, cx| this.pick(&id, cx));
        })
        .into_any_element()
    }

    /// The phone's picked-layer summary (`section.panel.rd-layer-summary`):
    /// the thumbnail, the name, "effect · area" and the Tune key that opens
    /// the layer sheet. The sheet is a dialog over the stack whose Escape
    /// and Tab behaviours the pair below cannot observe until it is open.
    fn layer_summary(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let entity = cx.entity();
        let Some(picked) = self.picked_layer().cloned() else {
            return div().into_any_element();
        };
        let label = self.layer_label(&picked.effect);
        // React's `.panel` without a title: the summary carries no tracked
        // title row, just the head (`layer-edit-head`).
        div()
            .id(gpui::SharedString::from("rd-layer-summary"))
            .p_4()
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel))
            .flex()
            .items_center()
            .gap_3()
            .child(thumb(&t, &picked.effect, 48.0, 30.0, cached(&picked.effect), false))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(16.0))
                            .line_height(gpui::relative(1.25))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(picked.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(14.0))
                            .line_height(gpui::relative(1.3))
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(format!("{} · {}", label, area_name(&picked.area, None, None))),
                    ),
            )
            .child(button_icon(
                &t,
                "rd-tune",
                "Tune",
                Variant::Default,
                false,
                true,
                "SlidersHorizontal",
                "",
                15.0,
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.sheet_open = true;
                        cx.notify();
                    });
                },
            ))
            .into_any_element()
    }

    /// The picked layer's panel.
    fn picked_panel(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let entity = cx.entity();
        let Some(picked) = self.picked_layer().cloned() else {
            return panel_with_padding(
                &t,
                "rd-layer-none",
                "No layer picked",
                div(),
                div()
                    .text_size(px(15.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child("Add a layer to tune it."),
                Some(12.0),
            )
            .into_any_element();
        };
        let index = self
            .draft
            .layers
            .iter()
            .position(|l| l.id == picked.id)
            .map(|at| at + 1)
            .unwrap_or(1);
        let count = self.count();
        let label = self.layer_label(&picked.effect);

        // The head: the thumbnail, the name and the keys.
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .mb_3()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(thumb(&t, &picked.effect, 48.0, 30.0, cached(&picked.effect), false))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .line_height(gpui::relative(1.25))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .font_family(crate::fonts::UI_FAMILY)
                                    .text_color(Theme::color(t.ink))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(picked.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .line_height(gpui::relative(1.3))
                                    .font_family(crate::fonts::MONO_FAMILY)
                                    .text_color(Theme::color(t.mute))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(format!("{label} · layer {index} of {count}")),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(2.0))
                    .child(icon_button(&t, "rd-duplicate", "CopySimple", "", false, false, false, {
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| this.duplicate(cx));
                        }
                    }))
                    .child(icon_button(&t, "rd-delete", "Trash", "", false, false, false, {
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| this.remove_layer(cx));
                        }
                    })),
            );

        // The fields: the effect, the blend and the opacity.
        let mut fields = div().grid().grid_cols(2).gap(px(8.0)).gap_x(px(12.0)).mb_1();
        fields = fields.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(px(14.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child("Effect"),
                )
                .child(crate::components::select::select_field(
                    &t,
                    "rd-effect",
                    "Effect",
                    &self.effect_options(),
                    &picked.effect,
                    self.effect_open,
                    false,
                    true,
                    {
                        let entity = entity.clone();
                        move |_w, cx| {
                            entity.update(cx, |this, _cx| {
                                this.effect_open = !this.effect_open;
                            });
                        }
                    },
                    {
                        let entity = entity.clone();
                        move |value: String, _window, cx| {
                            entity.update(cx, |this, cx| this.choose_effect(&value, cx));
                        }
                    },
                )),
        );
        let blends: Vec<(&str, &str)> = vec![
            ("normal", "Normal"),
            ("add", "Add"),
            ("multiply", "Multiply"),
            ("screen", "Screen"),
        ];
        let blend_value = match picked.blend {
            BlendMode::Normal => "normal",
            BlendMode::Add => "add",
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
        };
        fields = fields.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_size(px(14.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child("Blend"),
                )
                .child(crate::components::select::select_field(
                    &t,
                    "rd-blend",
                    "Blend",
                    &blends,
                    blend_value,
                    false,
                    false,
                    true,
                    {
                        let entity = entity.clone();
                        move |_w, _cx| {
                            let _ = &entity;
                        }
                    },
                    {
                        let entity = entity.clone();
                        move |value: String, _window, cx| {
                            entity.update(cx, |this, cx| {
                                let blend = match value.as_str() {
                                    "add" => BlendMode::Add,
                                    "multiply" => BlendMode::Multiply,
                                    "screen" => BlendMode::Screen,
                                    _ => BlendMode::Normal,
                                };
                                this.set_blend(blend, cx);
                            });
                        }
                    },
                )),
        );
        // The opacity, over the whole width.
        let opacity = picked.opacity as f32;
        let slider_origin = self.slider_origin.clone();
        let fields = fields
            .col_span(2)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.dim))
                            .child("Opacity"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(crate::components::slider::slider_range(
                                        &t,
                                        "rd-opacity",
                                        opacity,
                                        0.0,
                                        100.0,
                                        false,
                                        slider_origin,
                                        {
                                            let entity = entity.clone();
                                            move |value, _window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.set_opacity(value.round() as u8, cx);
                                                });
                                            }
                                        },
                                        {
                                            let entity = entity.clone();
                                            move |value, _window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.set_opacity(value.round() as u8, cx);
                                                });
                                            }
                                        },
                                    )),
                            )
                            .child(crate::components::number_field::number_field_width(
                                &t,
                                "rd-opacity-value",
                                opacity,
                                0.0,
                                100.0,
                                1.0,
                                None,
                                64.0,
                                {
                                    let entity = entity.clone();
                                    move |value, _window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.set_opacity(value.round() as u8, cx);
                                        });
                                    }
                                },
                            )),
                    ),
            );

        // The area line.
        let area_line = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .py_2()
            .border_t_1()
            .border_color(Theme::color(t.line))
            .border_b_1()
            .border_color(Theme::color(t.line))
            .mb_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w(px(0.0))
                    .child(
                        gpui::svg()
                            .path("icons/Monitor.svg")
                            .size(px(16.0))
                            .flex_none()
                            .text_color(Theme::color(t.mute)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .line_height(gpui::relative(1.45))
                                    .font_family(crate::fonts::UI_FAMILY)
                                    .text_color(Theme::color(t.ink))
                                    .child(area_name(&picked.area, None, None)),
                            )
                            .child(
                                area_position(&picked.area)
                                    .map(|at| {
                                        div()
                                            .text_size(px(14.0))
                                            .line_height(gpui::relative(1.3))
                                            .font_family(crate::fonts::MONO_FAMILY)
                                            .text_color(Theme::color(t.mute))
                                            .child(at)
                                    })
                                    .unwrap_or_else(|| div()),
                            ),
                    ),
            )
            .child(button(&t, "rd-change-area", "Change area", Variant::Ghost, false, {
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.open_picker(cx);
                    });
                }
            }));

        // The layer's own properties.
        let values = layer_values(
            self.schemas.get(&picked.effect).and_then(|s| s.as_ref()),
            &picked.props,
        );
        let schema_properties: Vec<canvas_proto::props::PropertyDef> = self
            .schemas
            .get(&picked.effect)
            .and_then(|s| s.as_ref())
            .map(|s| s.properties.clone())
            .unwrap_or_default();
        let mut props = div().flex().flex_col().min_w(px(0.0));
        let last = schema_properties.len().saturating_sub(1);
        for (i, def) in schema_properties.iter().enumerate() {
            let value = values.get(&def.key).cloned().unwrap_or_else(|| def.kind.default_value());
            // React `.prop-item`: 52 px tall, 8 px vertical padding, the
            // label and the control 16 px apart, a divider under every row
            // but the last.
            let mut row = div()
                .flex()
                .items_center()
                .gap_4()
                .min_h(px(52.0))
                .py_2();
            if i != last {
                row = row.border_b_1().border_color(Theme::color(t.line));
            }
            row = row.child(
                div()
                    .w(px(170.0))
                    .flex_none()
                    .text_size(px(15.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child(def.label.clone()),
            );
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(crate::components::prop_controls::control(
                                &t,
                                &def.key,
                                &def.label,
                                &def.kind,
                                &value,
                                &schema_properties,
                                &values,
                                false,
                                false,
                                false,
                                {
                                    let entity = entity.clone();
                                    let key = def.key.clone();
                                    std::rc::Rc::new(move |v, _window, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.set_value(&key, v, cx);
                                        });
                                    })
                                },
                                {
                                    let entity = entity.clone();
                                    std::rc::Rc::new(move |_w, _cx| {
                                        let _ = &entity;
                                    })
                                },
                                {
                                    let entity = entity.clone();
                                    std::rc::Rc::new(move |_w, _cx| {
                                        let _ = &entity;
                                    })
                                },
                            )),
            );
            props = props.child(row);
        }

        // The column's spacing is the children's own margins (the head's
        // 12, the fields' 4, the note's 8, the area's 12); React's layer
        // edit column has no flex gap of its own.
        let body = div()
            .flex()
            .flex_col()
            .child(head)
            .child(fields)
            .child(
                div()
                    .mb_2()
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.35))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(blend_note(picked.blend)),
            )
            .child(area_line)
            .child(props);

        // React's `<Panel className="rd-layer-edit">` has no title, so it has
        // no head either.
        div()
            .flex()
            .flex_col()
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel))
            .p_3()
            .child(body)
            .into_any_element()
    }

    /// The presets panel.
    fn presets_panel(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let entity = cx.entity();
        let mut chips = div().flex().flex_wrap().gap_2();
        for preset in self.presets.iter().chain(self.user_presets.iter()) {
            let name = preset.name.clone();
            let name_bg = name.clone();
            chips = chips.child(crate::components::chip::chip_button(
                &t,
                &format!("rd-preset-{name}"),
                &name,
                crate::components::chip::ChipTone::Plain,
                false,
                false,
                false,
                {
                    let entity = entity.clone();
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.apply_preset(&name_bg, cx);
                        });
                    }
                },
            ));
        }
        chips = chips.child(crate::components::chip::chip_add(
            &t,
            "rd-preset-save",
            "Save current",
            false,
            {
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        if this.base.is_none() {
                            this.preset_problem = Some("Save the stack before saving a preset".to_string());
                        } else {
                            this.preset_problem = None;
                            this.preset_name = Some(String::new());
                        }
                        cx.notify();
                    });
                }
            },
        ));
        // The chip's text is "+ Save current"; the shared chip takes the
        // label as its text, so the text goes in the label here.
        let mut body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(chips);
        if let Some(problem) = self.preset_problem.clone() {
            body = body.child(
                div()
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.down))
                    .child(problem),
            );
        }
        body = body.child(
            div()
                .text_size(px(14.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.mute))
                .child("Custom values"),
            );
        panel_with_padding(&t, "rd-presets", "Presets", div(), body, Some(12.0)).into_any_element()
    }

    // -- the layer edits ---------------------------------------------------------

    /// A layer's value changed.
    pub fn set_value(&mut self, key: &str, value: canvas_proto::props::PropValue, cx: &mut Context<Self>) {
        let Some(picked) = self.picked.clone() else {
            return;
        };
        let mut layers = self.draft.layers.clone();
        for layer in layers.iter_mut() {
            if layer.id == picked {
                layer.props.insert(key.to_string(), value.clone());
            }
        }
        self.draft.layers = layers;
        self.send_preview(cx);
        cx.notify();
    }

    /// A preset's values go into the layers.
    pub fn apply_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(preset) = self
            .presets
            .iter()
            .chain(self.user_presets.iter())
            .find(|p| p.name == name)
        else {
            return;
        };
        let merged = stack_draft::merged_props(&self.draft.layers, &self.schemas);
        let mut next = merged.clone();
        for (key, value) in &preset.props {
            next.insert(key.clone(), value.clone());
        }
        self.draft.layers = stack_draft::apply_merged_props(&self.draft.layers, &next, &self.schemas);
        self.send_preview(cx);
        cx.notify();
    }

    /// A layer is added: the first effect of the library.
    pub fn add_layer(&mut self, cx: &mut Context<Self>) {
        let Some(id) = stack_draft::next_layer_id(&self.draft.layers) else {
            return;
        };
        let layer = StackLayer {
            id,
            name: "Layer".to_string(),
            effect: "solid".to_string(),
            props: Default::default(),
            area: LayerArea::Canvas,
            blend: BlendMode::Normal,
            opacity: 100,
            visible: true,
        };
        self.draft.layers = stack_draft::add_layer(&self.draft.layers, layer);
        self.picked = self.draft.layers.last().map(|l| l.id.clone());
        self.send_preview(cx);
        cx.notify();
    }

    /// The picked layer is duplicated.
    pub fn duplicate(&mut self, cx: &mut Context<Self>) {
        let Some(picked) = self.picked.clone() else {
            return;
        };
        let (layers, id) = stack_draft::duplicate_layer(&self.draft.layers, &picked);
        if id.is_none() {
            self.save_error = Some(format!(
                "A stack holds {} HTML layers",
                stack_draft::MAX_HTML_LAYERS
            ));
            cx.notify();
            return;
        }
        self.draft.layers = layers;
        self.picked = id;
        self.send_preview(cx);
        cx.notify();
    }

    /// The picked layer is removed.
    pub fn remove_layer(&mut self, cx: &mut Context<Self>) {
        let Some(picked) = self.picked.clone() else {
            return;
        };
        let index = self
            .draft
            .layers
            .iter()
            .position(|l| l.id == picked)
            .unwrap_or(0);
        let next = stack_draft::remove_layer(&self.draft.layers, &picked);
        self.picked = next
            .get(index.min(next.len().saturating_sub(1)))
            .map(|l| l.id.clone());
        self.draft.layers = next;
        self.sheet_open = false;
        self.send_preview(cx);
        cx.notify();
    }

    /// The area picker opens for the picked layer.
    pub fn open_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picked) = self.picked_layer().cloned() else {
            return;
        };
        self.picker = Some(PickerState::for_layer(&picked, None));
        cx.notify();
    }

    /// The picker's mode changed.
    pub fn set_mode(&mut self, key: &str, cx: &mut Context<Self>) {
        if let (Some(picker), Some(mode)) = (self.picker.as_mut(), PickerMode::from_key(key)) {
            picker.mode = mode;
        }
        cx.notify();
    }

    /// The picker's device changed.
    pub fn set_device(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(picker) = self.picker.as_mut() {
            if !id.is_empty() {
                picker.device = id.to_string();
            }
        }
        cx.notify();
    }

    /// The picker's zone changed.
    pub fn set_zone(&mut self, index: &str, cx: &mut Context<Self>) {
        if let Some(picker) = self.picker.as_mut() {
            if let Ok(at) = index.parse::<u32>() {
                picker.zone_index = at;
            }
        }
        cx.notify();
    }

    /// A press on the desk.
    pub fn desk_press(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        if picker.mode != PickerMode::Rect {
            return;
        }
        picker.drag_from = Some(at);
        picker.rect = Some(stack_draft::rect_from_drag(at, at));
        cx.notify();
    }

    /// The area the picker chose goes to the picked layer.
    pub fn use_area(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.clone() else {
            return;
        };
        let Some(area) = picker.chosen() else {
            return;
        };
        self.picker = None;
        self.set_area(area, cx);
    }

    /// The picker closes with no change.
    pub fn close_picker(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        cx.notify();
    }

    /// Revert (`revert` in React): the saved stack comes back.
    pub fn revert(&mut self, cx: &mut Context<Self>) {
        match &self.base {
            Some(base) => {
                self.draft = base.clone();
                self.picked = base.layers.last().map(|l| l.id.clone());
            }
            None => {
                self.draft = Stack { id: "stack:".to_string(), layers: vec![] };
                self.name_draft.clear();
                self.picked = None;
            }
        }
        self.sheet_open = false;
        self.save_error = None;
        self.clash = None;
        self.send_preview(cx);
        cx.notify();
    }

    /// The builder closes: the page shows the library again.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.page.clone() {
            page.update(cx, |this, cx| this.close_stack(cx));
        }
    }
}

/// The name of a stack, from its id.
pub fn name_of(id: &str) -> String {
    id.strip_prefix("stack:").unwrap_or(id).to_string()
}

/// The strip's title and context for the builder: the stack's name and
/// "Stack · N layers".
pub fn strip_title(builder: &StackBuilder) -> (String, Option<String>) {
    let count = builder.count();
    (
        builder.name(),
        Some(format!("Stack · {count} {}", if count == 1 { "layer" } else { "layers" })),
    )
}

/// The strip's actions for the builder: Revert and Save.
pub fn strip_actions(builder: &StackBuilder, entity: gpui::Entity<StackBuilder>, narrow: bool) -> gpui::AnyElement {
    let t = builder.theme;
    let mut row = div().flex().items_center().gap_2();
    if !narrow && builder.dirty() {
        row = row.child(
            div()
                .text_size(px(15.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.mute))
                .child("Unsaved"),
        );
    }
    if !narrow {
        let entity = entity.clone();
        row = row.child(button_icon(
            &t,
            "rd-strip-revert",
            "Revert",
            Variant::Ghost,
            !builder.dirty() || builder.saving,
            true,
            "ArrowCounterClockwise",
            "",
            14.0,
            move |_, _, cx| {
                entity.update(cx, |this, cx| this.revert(cx));
            },
        ));
    }
    let entity = entity.clone();
    row = row.child(button_icon(
        &t,
        "rd-strip-save",
        // On a phone the save key keeps its icon and drops its label
        // (`.rd-btn-text` under `@media (max-width: 640px)`).
        if narrow { "" } else if builder.saving { "Saving…" } else { "Save" },
        Variant::Primary,
        builder.count() == 0 || builder.saving,
        true,
        "FloppyDisk",
        "",
        14.0,
        move |_, _, cx| {
            entity.update(cx, |this, cx| this.save(None, cx));
        },
    ));
    row.into_any_element()
}

/// The strip's leading key for the builder: Back to the effects.
pub fn strip_leading(builder: &StackBuilder, entity: gpui::Entity<StackBuilder>) -> gpui::AnyElement {
    let t = builder.theme;
    icon_button(&t, "rd-strip-back", "ArrowLeft", "", true, false, false, move |_, _, cx| {
        entity.update(cx, |this, cx| this.close(cx));
    })
    .into_any_element()
}

/// A path segment, encoded (`encodeURIComponent`).
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')') {
            out.push(c);
        } else if c == ' ' {
            out.push_str("%20");
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The route the builder's page lives at.
pub fn route_url(route: &StackRoute) -> String {
    route.url()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stack_name_is_its_id_without_the_prefix() {
        assert_eq!(name_of("stack:Sunset glow"), "Sunset glow");
        assert_eq!(name_of("Sunset glow"), "Sunset glow");
    }

    #[test]
    fn a_name_is_encoded_for_the_route() {
        assert_eq!(encode("Sunset glow"), "Sunset%20glow");
        assert_eq!(encode("a/b"), "a%2Fb");
    }

    #[test]
    fn the_builder_is_dirty_when_its_layers_differ() {
        let layer = StackLayer {
            id: "l1".into(),
            name: "A".into(),
            effect: "solid".into(),
            props: Default::default(),
            area: LayerArea::Canvas,
            blend: BlendMode::Normal,
            opacity: 100,
            visible: true,
        };
        let base = Stack { id: "stack:S".into(), layers: vec![layer.clone()] };
        let mut builder = StackBuilder {
            theme: Theme::dark(),
            width: 1440.0,
            height: 805.0,
            initial_name: "S".into(),
            base: Some(base),
            draft: Stack { id: "stack:S".into(), layers: vec![layer] },
            name_draft: String::new(),
            picked: None,
            statuses: HashMap::new(),
            schemas: Schemas::new(),
            presets: vec![],
            user_presets: vec![],
            show_areas: true,
            picker: None,
            sheet_open: false,
            saving: false,
            save_error: None,
            clash: None,
            naming: false,
            naming_error: None,
            leaving: false,
            preview_error: None,
            feed_problem: None,
            draft_state: super::super::editor::DraftState::Off,
            draft_error: None,
            draft_note: None,
            expires_in: 0,
            frame: None,
            feed_rx: None,
            feed_handle: None,
            page: None,
            session: StackSession::new("abc".into()),
            other_names: vec![],
            drag: None,
            blend_open: false,
            slider_origin: crate::components::slider::Origin::new(),
            thumbs_pending: std::collections::HashSet::new(),
            effects: Vec::new(),
            effect_open: false,
            preset_problem: None,
            preset_name: None,
        };
        assert!(!builder.dirty());
        builder.draft.layers[0].opacity = 50;
        assert!(builder.dirty());
    }
}
