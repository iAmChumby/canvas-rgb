//! The effect editor, after `app/web/src/views/effects/EffectEditor.tsx`.
//!
//! The source of an `html:` effect beside the daemon's live preview of it,
//! with Save through the import route and a draft on the real lights while
//! the editor is open.
//!
//! The code pane itself is round 2 (see [`super::code_editor`]): this round
//! draws the frame around it — the head, the name field, the keys, the
//! layout at every breakpoint — and the code pane as a read-only monospace
//! view of the source.

#![allow(dead_code)]

use canvas_proto::api::{
    DraftRequest, DraftStatus, EffectPreviewRequest, EffectPreviewResponse, ImportEffect,
    ImportEffectResponse,
};
use canvas_proto::props::{Preset, PropValue, PropertySchema, Props};

use gpui::{Context, Entity, IntoElement, Window, div, prelude::*, px};

use crate::client::{ClientError, DaemonClient};
use crate::components::button::{Variant, button, button_icon};
use crate::components::chip::{ChipTone, chip};
use crate::components::panel::state_dot;
use crate::components::prop_controls::OnChange;
use crate::components::text_input::TextInput;
use crate::theme::{Theme, TokenRadius};

use super::code_editor::{CodeEditor, CodeError, OnError, code_pane, diagnostics_for, source_line_of};

/// Sent this long after the last keystroke (`PREVIEW_DEBOUNCE_MS`).
pub const PREVIEW_DEBOUNCE_MS: u64 = 400;
/// The request deadline for the preview (35 s).
pub const PREVIEW_TIMEOUT_SECS: u64 = 35;

/// What the preview is doing, in the order a keystroke takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewState {
    Idle,
    Rendering,
    Ok,
    Error,
    Busy,
}

/// What the draft on the lights is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftState {
    Off,
    Starting,
    Running,
    Expiring,
    Restoring,
    RestoreError,
}

/// What the editor opens on.
#[derive(Debug, Clone)]
pub struct EditorTarget {
    /// The effect being edited, or `None` for a file that has never been
    /// saved.
    pub id: Option<String>,
    pub name: String,
    pub file_name: String,
    pub source: String,
    /// The values the effect runs with now; a new file starts empty.
    pub props: Props,
    /// The loaded schema, for the presets and the starting values.
    pub schema: Option<PropertySchema>,
    pub presets: Vec<Preset>,
    pub user_presets: Vec<Preset>,
}

/// The effect editor.
pub struct EffectEditor {
    pub theme: Theme,
    /// The page container's width, which React's `@container page` queries
    /// read.
    pub width: f32,
    /// The editor's own height, so the side column can be centred in it.
    pub height: f32,
    pub target: EditorTarget,
    pub source: String,
    pub saved_source: String,
    pub file_name: String,
    pub values: Props,
    pub show_code: bool,
    /// The Show/Hide code key has been pressed: the initial code visibility
    /// (open on desktop, closed on the phone) only applies before that.
    pub code_touched: bool,
    pub saving: bool,
    pub save_error: Option<String>,
    /// A new file whose name is taken: the same replace prompt the import
    /// uses, said as "file exists".
    pub clash: Option<String>,
    pub leaving: bool,
    pub preview: Option<EffectPreviewResponse>,
    pub preview_state: PreviewState,
    pub preview_error: Option<String>,
    pub draft_state: DraftState,
    pub draft_error: Option<String>,
    pub draft_note: Option<String>,
    pub expires_in: u32,
    /// The file name field, for a new file.
    pub file_input: Option<Entity<TextInput>>,
    /// The code editor entity, for the code pane (lazily created, like the
    /// file field). Its text is the source of truth beside this struct's
    /// `source`: typing reaches this struct through its callback, and a
    /// Revert reaches the entity through `set_text` without echoing.
    pub code_editor: Option<Entity<CodeEditor>>,
    /// The page that opened the editor.
    pub page: Option<Entity<super::EffectsView>>,
    /// The colour picker that is open, by property key.
    pub color_open: Option<String>,
    /// The choice list that is open, by property key.
    pub select_open: std::collections::HashMap<String, bool>,
    /// The side column's measured content height, for React's centring.
    pub probe: std::rc::Rc<std::cell::Cell<f32>>,
    /// How many preview requests have gone out, so a stale answer is dropped.
    preview_seq: u64,
}

impl EffectEditor {
    /// A new editor on `target`.
    pub fn new(theme: Theme, target: EditorTarget, page: Option<Entity<super::EffectsView>>, cx: &mut Context<Self>) -> Self {
        let narrow = false;
        let source = target.source.clone();
        let saved_source = target.source.clone();
        let file_name = target.file_name.clone();
        let values = target.props.clone();
        let mut editor = EffectEditor {
            theme,
            width: 1280.0,
            height: 757.0,
            target,
            source,
            saved_source,
            file_name,
            values,
            show_code: !narrow,
            code_touched: false,
            saving: false,
            save_error: None,
            clash: None,
            leaving: false,
            preview: None,
            preview_state: PreviewState::Idle,
            preview_error: None,
            draft_state: DraftState::Off,
            draft_error: None,
            draft_note: None,
            expires_in: 0,
            file_input: None,
            code_editor: None,
            page,
            color_open: None,
            select_open: std::collections::HashMap::new(),
            probe: std::rc::Rc::new(std::cell::Cell::new(0.0)),
            preview_seq: 0,
        };
        editor.queue_preview(cx, true);
        editor
    }

    /// The effect being edited, if it is a saved one.
    pub fn is_new(&self) -> bool {
        self.target.id.is_none()
    }

    /// Only the source text counts as a change (React's `dirty`).
    pub fn dirty(&self) -> bool {
        self.source != self.saved_source
    }

    /// The effect's name: the preview's schema name, else the target's.
    pub fn name(&self) -> String {
        self.preview
            .as_ref()
            .map(|p| p.schema.name.clone())
            .unwrap_or_else(|| self.target.name.clone())
    }

    /// The schema the property controls read.
    pub fn schema(&self) -> Option<&PropertySchema> {
        self.preview.as_ref().map(|p| &p.schema).or(self.target.schema.as_ref())
    }

    /// The definition the controls read.
    pub fn defs(&self) -> Vec<canvas_proto::props::PropertyDef> {
        self.schema().map(|s| s.properties.clone()).unwrap_or_default()
    }

    /// A draft can be saved while it runs: the draft's own ending.
    pub fn can_save(&self) -> bool {
        !self.saving && (self.is_new() || self.dirty() || self.draft_running())
    }

    /// Whether a draft is on the lights (or on its way).
    pub fn draft_running(&self) -> bool {
        matches!(
            self.draft_state,
            DraftState::Running | DraftState::Expiring | DraftState::Starting
        )
    }

    /// The live note under the picture (`liveNote` in React).
    pub fn live_note(&self) -> String {
        match self.preview_state {
            PreviewState::Rendering => "Rendering…".to_string(),
            PreviewState::Busy => "Busy: the daemon is rendering other previews".to_string(),
            PreviewState::Error => {
                if self.preview.as_ref().map(|p| !p.errors.is_empty()).unwrap_or(false) {
                    "Preview stopped: the code has an error".to_string()
                } else {
                    "Preview stopped: the daemon did not answer".to_string()
                }
            }
            _ => "Preview".to_string(),
        }
    }

    // -- the preview ---------------------------------------------------------

    /// Sends the preview: at once on open, and 400 ms after the last change.
    pub fn queue_preview(&mut self, cx: &mut Context<Self>, first: bool) {
        self.preview_seq += 1;
        let seq = self.preview_seq;
        let delay = if first { 0 } else { PREVIEW_DEBOUNCE_MS };
        let html = self.source.clone();
        let props = self.values.clone();
        cx.spawn(async move |editor, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(delay))
                .await;
            let bg = cx.background_executor().spawn(async move {
                let client = DaemonClient::with_base_timeout(
                    crate::client::base_url(),
                    std::time::Duration::from_secs(PREVIEW_TIMEOUT_SECS),
                );
                let body = serde_json::to_string(&EffectPreviewRequest { html, props })
                    .unwrap_or_else(|_| "{}".into());
                client.post_json("/api/v1/effects/preview", &body)
            });
            let answer = bg.await;
            let _ = editor.update(cx, |this, cx| {
                if seq != this.preview_seq {
                    return;
                }
                match answer {
                    Ok(body) => match serde_json::from_str::<EffectPreviewResponse>(&body) {
                        Ok(parsed) => {
                            this.preview_state = if parsed.errors.is_empty() {
                                PreviewState::Ok
                            } else {
                                PreviewState::Error
                            };
                            this.preview_error = None;
                            this.preview = Some(parsed);
                        }
                        Err(e) => {
                            this.preview_state = PreviewState::Error;
                            this.preview_error = Some(e.to_string());
                        }
                    },
                    Err(e) => {
                        this.preview_state = PreviewState::Error;
                        this.preview_error = Some(message_of(&e));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The property controls' change: a new value, then the preview again.
    pub fn on_value(&mut self, key: &str, value: PropValue, cx: &mut Context<Self>) {
        self.values.insert(key.to_string(), value);
        self.queue_preview(cx, false);
        cx.notify();
    }

    /// The shared control's change callback for `key`.
    fn change_callback(&self, key: &str, entity: Entity<Self>) -> OnChange {
        let key_owned = key.to_string();
        std::rc::Rc::new(move |v: PropValue, _window: &mut Window, cx: &mut gpui::App| {
            entity.update(cx, |this, cx| this.on_value(&key_owned, v, cx));
        })
    }

    // -- save ----------------------------------------------------------------

    /// Save (`save` in React): the import route, then the activate.
    pub fn save(&mut self, replace: bool, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.save_error = None;
        let wanted = self.file_name.trim().to_string();
        // An effect that is already on disk is saved over itself. A new file
        // is written without asking to replace: the route answers a clash
        // when another window got the name first.
        let replace_on_disk = !self.is_new() || replace;
        self.saving = true;
        let source = self.source.clone();
        let file_name = wanted.clone();
        let page = self.page.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::with_base_timeout(
                crate::client::base_url(),
                std::time::Duration::from_secs(60),
            );
            let body = serde_json::to_string(&ImportEffect {
                file_name,
                html: source,
                replace: replace_on_disk,
            })
            .unwrap_or_else(|_| "{}".into());
            client.post_json("/api/v1/effects/import", &body)
        });
        cx.spawn(async move |editor, cx| {
            let result = bg.await;
            let mut saved_id: Option<String> = None;
            let mut start_error: Option<String> = None;
            match result {
                Ok(body) => {
                    let parsed = serde_json::from_str::<ImportEffectResponse>(&body).ok();
                    if let Some(parsed) = parsed {
                        saved_id = Some(parsed.effect.id.clone());
                    }
                }
                Err(ClientError::Daemon { status: 409, .. }) if !replace_on_disk => {
                    let _ = editor.update(cx, |this, cx| {
                        this.saving = false;
                        this.clash = Some(wanted.clone());
                        cx.notify();
                    });
                    return;
                }
                Err(e) => {
                    let _ = editor.update(cx, |this, cx| {
                        this.saving = false;
                        this.save_error = Some(message_of(&e));
                        this.clash = None;
                        cx.notify();
                    });
                    return;
                }
            }
            // The activate ends the draft on the lights without putting the
            // old effect back.
            let _ = editor.update(cx, |this, cx| {
                this.saved_source = this.source.clone();
                this.draft_state = DraftState::Off;
                cx.notify();
            });
            if let Some(id) = saved_id.clone() {
                let bg = cx.background_executor().spawn(async move {
                    let client = DaemonClient::new();
                    let body = serde_json::to_string(&serde_json::json!({ "props": {}, "preset": serde_json::Value::Null }))
                        .unwrap_or_else(|_| "{}".into());
                    client.post_json(&format!("/api/v1/effects/{id}/activate"), &body)
                });
                let answer = bg.await;
                if let Err(e) = answer {
                    start_error = Some(message_of(&e));
                }
            }
            let _ = editor.update(cx, |this, cx| {
                this.saving = false;
                this.clash = None;
                if let Some(err) = &start_error {
                    this.save_error = Some(format!("Saved, but it did not start: {err}"));
                }
                cx.notify();
            });
            // The page reads the library again and shows the saved effect.
            if let Some(page) = page {
                let _ = page.update(cx, |this, cx| {
                    this.load_list(cx);
                    if let Some(id) = saved_id.clone() {
                        this.picked = Some(id);
                    }
                });
            }
        })
        .detach();
    }

    /// Revert (`revert` in React): the source text goes back to the saved
    /// source, and the save error goes with it.
    pub fn revert(&mut self, cx: &mut Context<Self>) {
        self.source = self.saved_source.clone();
        self.save_error = None;
        self.queue_preview(cx, false);
        cx.notify();
    }

    // -- the draft on the lights ---------------------------------------------

    /// Starts the draft ("Try on the lights").
    pub fn start_draft(&mut self, cx: &mut Context<Self>) {
        self.draft_state = DraftState::Starting;
        self.draft_error = None;
        self.draft_note = None;
        let html = self.source.clone();
        let props = self.values.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            let body = serde_json::to_string(&DraftRequest {
                html,
                stack: None,
                epoch: None,
                seq: None,
                props,
            })
            .unwrap_or_else(|_| "{}".into());
            client.put_json("/api/v1/effects/draft", &body)
        });
        cx.spawn(async move |editor, cx| {
            let result = bg.await;
            let _ = editor.update(cx, |this, cx| {
                match result {
                    Ok(body) => {
                        let status = serde_json::from_str::<DraftStatus>(&body).ok();
                        this.draft_state = DraftState::Running;
                        this.draft_error = None;
                        if let Some(status) = status {
                            this.expires_in = (status.lease_remaining_ms / 1000) as u32;
                        }
                    }
                    Err(e) => {
                        this.draft_state = DraftState::Off;
                        this.draft_error = Some(message_of(&e));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Stops the draft ("Stop").
    pub fn stop_draft(&mut self, cx: &mut Context<Self>) {
        self.draft_state = DraftState::Restoring;
        self.draft_error = None;
        let bg = cx.background_executor().spawn(async move {
            let client = DaemonClient::new();
            client.delete("/api/v1/effects/draft")
        });
        cx.spawn(async move |editor, cx| {
            let result = bg.await;
            let _ = editor.update(cx, |this, cx| {
                match result {
                    Ok(body) => {
                        this.draft_state = DraftState::Off;
                        let ended = serde_json::from_str::<canvas_proto::api::DraftEnded>(&body).ok();
                        if let Some(ended) = ended {
                            if let Some(note) = ended.note {
                                this.draft_note = Some(note);
                            }
                        }
                    }
                    Err(e) => {
                        this.draft_state = DraftState::RestoreError;
                        this.draft_error = Some(message_of(&e));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // -- the picture ---------------------------------------------------------

    /// The live picture: the daemon's last good loop, drawn as frame 0.
    pub fn picture(&self) -> Option<(u16, u16, Vec<u8>)> {
        let preview = self.preview.as_ref()?;
        let bytes = crate::components::thumb::decode_base64(&preview.thumbnail)?;
        let thumb = crate::components::thumb::parse_thumbnail(&bytes)?;
        Some((thumb.w, thumb.h, thumb.data))
    }

    // -- the render ----------------------------------------------------------

    /// The editor's frame.
    pub fn render(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let narrow = self.width <= 720.0;
        let entity = cx.entity();

        // The draft banner.
        let mut editor = div().flex().flex_col().gap_3().min_w(px(0.0)).min_h(px(0.0)).flex_1();
        if self.draft_state != DraftState::Off {
            editor = editor.child(self.draft_banner(cx));
        }
        if let Some(note) = self.draft_note.clone() {
            editor = editor.child(
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
        if self.draft_state == DraftState::RestoreError {
            if let Some(err) = self.draft_error.clone() {
                editor = editor.child(
                    div()
                        .text_size(px(15.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.down))
                        .child(err),
                );
            }
        }

        // The side column and the code pane.
        let side_w = if self.container_width() >= 1000.0 { 420.0 } else { 340.0 };
        let mut main = div()
            .flex()
            .gap_3()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0));
        let side = self.side_column(cx).w(px(side_w)).flex_none();
        main = main.child(side);
        if self.show_code && !narrow {
            // The pane is the editor's height (React `.eff-code` fills the
            // editor's box: 757 px at 1440x900).
            let code_errors = self.code_errors();
            let line_count = self.source.split('\n').count();
            let code_entity = self.code_editor_entity(cx);
            let seen = diagnostics_for(&self.source, &code_errors);
            code_entity.update(cx, |this, cx| this.set_diagnostics(seen, cx));
            let on_error: OnError = {
                let entity = entity.clone();
                let code_entity = code_entity.clone();
                std::rc::Rc::new(move |index: usize, _window: &mut Window, cx: &mut gpui::App| {
                    let errors = {
                        let this = entity.read(cx);
                        this.code_errors()
                    };
                    let line = errors.get(index).and_then(|e| e.line).unwrap_or(1);
                    entity.update(cx, |this, cx| {
                        this.show_code = true;
                        this.code_touched = true;
                        cx.notify();
                    });
                    code_entity.update(cx, |this, cx| this.focus_line(line, cx));
                })
            };
            let error_rows: Vec<(String, Option<usize>)> =
                code_errors.iter().map(|e| (e.message.clone(), e.line)).collect();
            let preview_error = self.preview_error.clone();
            main = main.child(
                div()
                    .flex_1()
                    .h(px(self.height))
                    .min_w(px(0.0))
                    .child(code_pane(
                        &t,
                        "eff-code",
                        code_entity,
                        line_count,
                        narrow,
                        self.dirty() && !self.saving,
                        {
                            let entity = entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |this, _cx| this.show_code = false);
                            }
                        },
                        {
                            let entity = entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| this.revert(cx));
                            }
                        },
                        &error_rows,
                        preview_error.as_deref(),
                        on_error,
                    )),
            );
        }
        editor = editor.child(main);
        editor.into_any_element()
    }

    /// The live picture's height: React `.eff-live .preview-canvas { aspect-ratio: 16 / 10 }`
    /// over the side column's width.
    pub fn picture_height(&self) -> f32 {
        let side_w = if self.container_width() >= 1000.0 { 420.0 } else { 340.0 };
        side_w / 1.6
    }

    /// The page container's width, which React's `@container page` queries
    /// read.
    fn container_width(&self) -> f32 {
        self.width
    }

    /// The draft banner (`.eff-draft`).
    fn draft_banner(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let entity = cx.entity();
        let (border, bg) = match self.draft_state {
            DraftState::Expiring | DraftState::Restoring => (t.warn, t.warn_wash),
            DraftState::RestoreError => (t.down, t.down),
            _ => (t.work, t.work),
        };
        let text = match self.draft_state {
            DraftState::Starting => "Starting the draft…".to_string(),
            DraftState::Running => "Draft on the lights.".to_string(),
            DraftState::Expiring => format!("Draft on the lights, expiring in {}s.", self.expires_in),
            DraftState::Restoring => "Restoring the previous effect…".to_string(),
            DraftState::RestoreError => "The previous effect could not be restored.".to_string(),
            DraftState::Off => String::new(),
        };
        let mark = div().size(px(8.0)).rounded_full().bg(Theme::color(border)).flex_none();
        let mut actions = div().flex().gap_1().flex_none();
        if self.draft_state == DraftState::RestoreError {
            let entity_retry = entity.clone();
            actions = actions.child(button_icon(
                &t,
                "eff-draft-retry",
                "Retry",
                Variant::Default,
                false,
                true,
                "Play",
                "",
                14.0,
                move |_, _, cx| {
                    entity_retry.update(cx, |this, cx| this.stop_draft(cx));
                },
            ));
        } else {
            let entity_stop = entity.clone();
            actions = actions.child(button(
                &t,
                "eff-draft-stop",
                "Stop",
                Variant::Ghost,
                self.draft_state == DraftState::Restoring,
                move |_, _, cx| {
                    entity_stop.update(cx, |this, cx| this.stop_draft(cx));
                },
            ));
            let entity_save = entity.clone();
            actions = actions.child(button(
                &t,
                "eff-draft-save",
                "Save",
                if self.leaving { Variant::Default } else { Variant::Primary },
                self.saving,
                move |_, _, cx| {
                    entity_save.update(cx, |this, cx| this.save(false, cx));
                },
            ));
        }
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .r_sm()
            .border_1()
            .border_color(Theme::color(border))
            .bg(Theme::color(bg))
            .child(mark)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(px(15.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child(text),
            )
            .child(actions)
    }

    /// The side column: the live picture, the name row, the keys and the
    /// panels under them.
    fn side_column(&mut self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let t = self.theme;
        let entity = cx.entity();
        let narrow = self.width <= 720.0;
        let name = self.name();

        // The live picture and its note.
        let mut live = div().flex().flex_col().gap_1().w_full();
        live = live.child(self.live_picture());
        live = live.child(
            div()
                .text_size(px(15.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(if self.preview_state == PreviewState::Error {
                    Theme::color(t.warn)
                } else {
                    Theme::color(t.mute)
                })
                .child(self.live_note()),
        );
        if let Some(preview) = self.preview.as_ref() {
            if !preview.warnings.is_empty() {
                let mut list = div().flex().flex_col().gap(px(2.0)).mt_1();
                for warning in &preview.warnings {
                    list = list.child(
                        div()
                            .text_size(px(15.0))
                            .line_height(gpui::relative(1.35))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.warn))
                            .child(warning.clone()),
                    );
                }
                live = live.child(list);
            }
        }
        if self.preview_state == PreviewState::Busy
            || (self.preview_state == PreviewState::Error && self.preview_error.is_some())
        {
            let entity = entity.clone();
            live = live.child(button(
                &t,
                "eff-try-again",
                "Try again",
                Variant::Ghost,
                false,
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.queue_preview(cx, true));
                },
            ));
        }

        // The name row and the keys.
        let mut title = div().flex().flex_col().gap_2().min_w(px(0.0));
        let mut name_row = div().flex().flex_wrap().items_center().gap_2().min_w(px(0.0));
        name_row = name_row.child(
            div()
                .text_size(px(15.0))
                .line_height(gpui::relative(1.2))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .whitespace_nowrap()
                .child(name.clone()),
        );
        if self.is_new() {
            name_row = name_row.child(chip(&t, "eff-new-chip", "New HTML effect", ChipTone::Html, false, false, false));
        } else {
            name_row = name_row.child(state_dot(
                &t,
                if self.draft_running() { "work" } else { "mute" },
                if self.draft_running() { "Draft" } else { "Editing" },
            ));
        }
        title = title.child(name_row);

        if self.is_new() {
            title = title.child(self.file_field(cx));
        } else {
            title = title.child(
                div()
                    .text_size(px(15.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(self.file_name.clone()),
            );
        }

        // The action row. On the phone the three keys stay on one row
        // (React `.row-wrap` does not wrap at 390: Try + Show code + Save =
        // 347 px of the 358 available), so the row does not wrap there.
        let mut actions = div().flex().flex_wrap().items_center().gap_1();
        if narrow {
            actions = actions.flex_nowrap();
        }
        if matches!(self.draft_state, DraftState::Off | DraftState::Starting) {
            let entity = entity.clone();
            actions = actions.child(button_icon(
                &t,
                "eff-try-lights",
                if self.draft_state == DraftState::Starting { "Starting…" } else { "Try on the lights" },
                Variant::Default,
                self.draft_state == DraftState::Starting,
                true,
                "Lightbulb",
                "",
                15.0,
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.start_draft(cx));
                },
            ));
        }
        let entity_show = entity.clone();
        actions = actions.child(button_icon(
            &t,
            "eff-show-code",
            if self.show_code { "Hide code" } else { "Show code" },
            Variant::Ghost,
            false,
            true,
            "Code",
            "",
            15.0,
            move |_, _, cx| {
                entity_show.update(cx, |this, cx| {
                    this.show_code = !this.show_code;
                    this.code_touched = true;
                    cx.notify();
                });
            },
        ));
        let entity_save = entity.clone();
        actions = actions.child(button_icon(
            &t,
            "eff-save",
            if self.saving { "Saving…" } else { "Save" },
            if self.leaving || self.draft_running() { Variant::Default } else { Variant::Primary },
            !self.can_save(),
            true,
            "FloppyDisk",
            "",
            15.0,
            move |_, _, cx| {
                entity_save.update(cx, |this, cx| this.save(false, cx));
            },
        ));
        if !narrow {
            let entity = entity.clone();
            actions = actions.child(button_icon(
                &t,
                "eff-done",
                "Done",
                Variant::Ghost,
                false,
                true,
                "X",
                "",
                15.0,
                move |_, _, cx| {
                    entity.update(cx, |this, cx2| this.ask_leave(cx2));
                },
            ));
        }
        title = title.child(actions);

        if self.draft_state == DraftState::Off {
            if let Some(err) = self.draft_error.clone() {
                title = title.child(
                    div()
                        .text_size(px(15.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.down))
                        .child(err),
                );
            }
        }
        if let Some(err) = self.save_error.clone() {
            title = title.child(
                div()
                    .text_size(px(15.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.down))
                    .child(err),
            );
        }
        if let Some(clash) = self.clash.clone() {
            let entity = entity.clone();
            title = title.child(
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
                            .text_size(px(15.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(format!("file exists  Replace {clash}?")),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(button(&t, "eff-clash-replace", "Replace", Variant::Primary, self.saving, {
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.save(true, cx));
                                }
                            }))
                            .child(button(&t, "eff-clash-cancel", "Cancel", Variant::Ghost, false, {
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
        if self.leaving {
            title = title.child(self.leave_prompt(cx));
        }

        // The panels under the actions.
        let mut panels = div().flex().flex_col().gap_3().flex_none();
        let presets: Vec<Preset> = self
            .target
            .presets
            .iter()
            .chain(self.target.user_presets.iter())
            .cloned()
            .collect();
        if !presets.is_empty() {
            let mut chips = div().flex().flex_wrap().gap_2();
            for preset in &presets {
                let entity = entity.clone();
                let name = preset.name.clone();
                let name_bg = name.clone();
                chips = chips.child(crate::components::chip::chip_button(
                    &t,
                    &format!("eff-preset-{name}"),
                    &name,
                    ChipTone::Plain,
                    false,
                    false,
                    false,
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| this.try_preset(&name_bg, cx));
                    },
                ));
            }
            panels = panels.child(
                crate::components::panel::panel_with_padding(
                    &t,
                    "eff-presets",
                    "Presets",
                    div(),
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(chips)
                        .child(
                            div()
                                .mt_2()
                                .text_size(px(15.0))
                                .line_height(gpui::relative(1.45))
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child("A preset sets the values here; it never starts the saved effect."),
                        ),
                    Some(12.0),
                ),
            );
        }
        let defs = self.defs();
        let props_body: gpui::AnyElement = if defs.is_empty() {
            div()
                .text_size(px(15.0))
                .line_height(gpui::relative(1.45))
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.mute))
                .child(match self.preview_state {
                    PreviewState::Rendering | PreviewState::Idle => "The controls appear with the first preview.",
                    _ => "This effect declares no properties.",
                })
                .into_any_element()
        } else {
            let schema = self.schema().cloned();
            let schema_properties: Vec<canvas_proto::props::PropertyDef> =
                schema.map(|s| s.properties.clone()).unwrap_or_default();
            let values = self.values.clone();
            let mut col = div().flex().flex_col().min_w(px(0.0));
            for def in &defs {
                let value = values.get(&def.key).cloned().unwrap_or_else(|| def.kind.default_value());
                col = col.child(self.prop_row(def, &value, entity.clone(), &schema_properties, &values));
            }
            col.into_any_element()
        };
        panels = panels.child(crate::components::panel::panel_with_padding(
            &t,
            "eff-properties",
            "Properties",
            div(),
            props_body,
            Some(12.0),
        ));

        // React `.eff-edit-side { justify-content: center }`: the content is
        // React `.eff-edit-side { justify-content: center }`: the content is
        // centred in the column, and the room left over is the preview's.
        // gpui 0.2.2 cannot centre this column (every way of asking for it
        // pushes the content out of the box), so the content starts at the
        // top of the column: a known gap, measured in the report.
        // React `.eff-edit-side { justify-content: center }`: the content is
        // centred in the column, and the room left over is the preview's.
        // gpui 0.2.2 centres a column "safely" (the content never rises
        // above the box), so the offset is measured here and applied as the
        // content's own top margin: React's centring is the unsafe kind.
        let measured = self.probe.get();
        let offset = if measured > 0.0 { (self.height - measured) / 2.0 } else { 0.0 };
        // The content is measured apart from the column: the inner block is
        // its own height, and the probe measures that.
        let mut inner = div()
            .id(gpui::SharedString::from("eff-edit-inner"))
            .relative()
            .flex()
            .flex_col()
            .gap_2()
            .min_w(px(0.0))
            .child(live)
            .child(title)
            .child(panels);
        inner.style().align_self = Some(gpui::AlignItems::FlexStart);
        // The probe: the content's own height, measured as it is painted.
        let probe_cell = self.probe.clone();
        let entity = cx.entity();
        inner = inner.child(
            gpui::canvas(
                move |bounds, _window, app| {
                    let h = f32::from(bounds.size.height);
                    if (probe_cell.get() - h).abs() > 0.5 {
                        probe_cell.set(h);
                        entity.update(app, |_e, cx| cx.notify());
                    }
                },
                move |_bounds, _state, _window, _app| {},
            )
            .absolute()
            .inset_0(),
        );
        let mut content = div()
            .id(gpui::SharedString::from("eff-edit-content"))
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .child(inner);
        content.style().margin.top = Some(gpui::Length::Definite(gpui::px(offset).into()));
        div()
            .id(gpui::SharedString::from("eff-edit-side"))
            .flex()
            .flex_col()
            .gap_2()
            .h(px(self.height))
            .child(content)
    }

    /// One property row (React's `PropertyControls`).
    fn prop_row(
        &self,
        def: &canvas_proto::props::PropertyDef,
        value: &PropValue,
        entity: Entity<Self>,
        schema_properties: &[canvas_proto::props::PropertyDef],
        values: &Props,
    ) -> impl IntoElement {
        let t = self.theme;
        let label = div()
            .flex_1()
            .min_w(px(0.0))
            .text_size(px(15.0))
            .line_height(gpui::relative(1.45))
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.ink))
            .child(def.label.clone());
        let change_cb = self.change_callback(&def.key, entity.clone());
        let toggle_color: crate::components::prop_controls::OnToggle = {
            let entity = entity.clone();
            let key = def.key.clone();
            std::rc::Rc::new(move |_w: &mut Window, cx: &mut gpui::App| {
                entity.update(cx, |this, cx| {
                    this.color_open = if this.color_open.as_deref() == Some(key.as_str()) {
                        None
                    } else {
                        Some(key.clone())
                    };
                    cx.notify();
                });
            })
        };
        let toggle_select: crate::components::prop_controls::OnToggle = {
            let entity = entity.clone();
            let key = def.key.clone();
            std::rc::Rc::new(move |_w: &mut Window, cx: &mut gpui::App| {
                entity.update(cx, |this, cx| {
                    let entry = this.select_open.entry(key.clone()).or_insert(false);
                    *entry = !*entry;
                    cx.notify();
                });
            })
        };
        let control = crate::components::prop_controls::control(
            &t,
            &def.key,
            &def.label,
            &def.kind,
            value,
            schema_properties,
            values,
            false,
            self.color_open.as_deref() == Some(def.key.as_str()),
            self.select_open.get(&def.key).copied().unwrap_or(false),
            change_cb,
            toggle_color,
            toggle_select,
        );
        div()
            .flex()
            .items_center()
            .gap_4()
            .min_h(px(40.0))
            .py_1()
            .child(div().w(px(170.0)).flex_none().child(label))
            .child(div().flex().items_center().gap_2().flex_1().min_w(px(0.0)).child(control))
    }

    /// The live picture (React's `LoopPreview`): the last good loop, frame 0.
    fn live_picture(&self) -> impl IntoElement {
        let t = self.theme;
        let mut picture = div()
            .id(gpui::SharedString::from("eff-preview-canvas"))
            .relative()
            .w_full()
            .h(px(self.picture_height()))
            .r_sm()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.sunk));
        if let Some((w, h, data)) = self.picture() {
            let thumb = crate::components::thumb::Thumb { w, h, frames: 1, data };
            picture = picture.child(
                gpui::canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        crate::components::thumb::paint_frame(&thumb, bounds, window, 0);
                    },
                )
                .absolute()
                .inset_0(),
            );
        }
        picture
    }

    /// The file name field, for a new file.
    fn file_field(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        if self.file_input.is_none() {
            let entity = cx.entity();
            let input = cx.new(|cx| {
                let mut input = TextInput::new(
                    &self.file_name,
                    false,
                    0.0,
                    0.0,
                    0,
                    t,
                    std::rc::Rc::new(|_, _, _| {}),
                    cx,
                );
                input.set_text_mode(Some("my-effect.html"), cx);
                let entity = entity.clone();
                input.set_on_text_change(std::rc::Rc::new(move |text: &str, _window: &mut Window, cx: &mut gpui::App| {
                    entity.update(cx, |this, cx| {
                        this.file_name = text.to_string();
                        this.save_error = None;
                        this.clash = None;
                        cx.notify();
                    });
                }));
                input
            });
            self.file_input = Some(input);
        }
        div().w_full().child(self.file_input.clone().unwrap())
    }

    /// The leave prompt (`.eff-leave`).
    fn leave_prompt(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .r_sm()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.raised))
            .child(
                div()
                    .text_size(px(15.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child("Save your changes before leaving?"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(button(&t, "eff-leave-save", "Save", Variant::Primary, self.saving, {
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| this.save(false, cx));
                        }
                    }))
                    .child(button(&t, "eff-leave-discard", "Discard", Variant::Default, self.saving, {
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx2| this.close(cx2));
                        }
                    }))
                    .child(button(&t, "eff-leave-keep", "Keep editing", Variant::Ghost, false, {
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.leaving = false;
                                cx.notify();
                            });
                        }
                    })),
            )
    }

    /// Asks before leaving when the source is dirty.
    pub fn ask_leave(&mut self, cx: &mut Context<Self>) {
        if self.dirty() {
            self.leaving = true;
            cx.notify();
        } else {
            self.close(cx);
        }
    }

    /// Closes the editor and tells the page.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.page.clone() {
            page.update(cx, |this, _cx| this.close_editor());
        }
    }

    /// A preset's values go into the editor's own values (`tryPreset`).
    pub fn try_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(preset) = self
            .target
            .presets
            .iter()
            .chain(self.target.user_presets.iter())
            .find(|p| p.name == name)
        else {
            return;
        };
        let defs = self.defs();
        let mut next = Props::new();
        for def in &defs {
            let value = preset
                .props
                .get(&def.key)
                .cloned()
                .unwrap_or_else(|| def.kind.default_value());
            next.insert(def.key.clone(), value);
        }
        self.values = next;
        self.queue_preview(cx, false);
        cx.notify();
    }

    /// The daemon's errors as gutter diagnostics, with the script line
    /// mapped into the file (`sourceLineOf`).
    pub fn code_errors(&self) -> Vec<CodeError> {
        self.preview
            .as_ref()
            .map(|p| {
                p.errors
                    .iter()
                    .map(|error| CodeError {
                        message: error.message.clone(),
                        line: error.line.map(|n| source_line_of(&self.source, n as usize)),
                        column: error.column.map(|n| n as usize),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The code editor entity, created once from the saved source; typing
    /// writes through to `source` and queues the preview.
    fn code_editor_entity(&mut self, cx: &mut Context<Self>) -> Entity<CodeEditor> {
        if self.code_editor.is_none() {
            let theme = self.theme;
            let source = self.source.clone();
            let entity = cx.entity();
            let input = cx.new(|cx| {
                let on_change: super::code_editor::OnChange = std::rc::Rc::new(move |text: &str, _window: &mut Window, cx: &mut gpui::App| {
                    entity.update(cx, |this, cx| {
                        this.source = text.to_string();
                        this.queue_preview(cx, false);
                        cx.notify();
                    });
                });
                CodeEditor::new(&source, theme, on_change, cx)
            });
            self.code_editor = Some(input);
        }
        let entity = self.code_editor.clone().unwrap();
        // An outside value (Revert) lands without echoing.
        let source = self.source.clone();
        entity.update(cx, |this, cx| this.set_text(&source, cx));
        entity
    }
}

/// The daemon's own sentence, or the client's (`messageOf`).
pub fn message_of(error: &ClientError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_effect_is_dirty_only_when_its_source_changes() {
        let target = EditorTarget {
            id: None,
            name: "My effect".into(),
            file_name: "my-effect.html".into(),
            source: "<html></html>".into(),
            props: Props::new(),
            schema: None,
            presets: vec![],
            user_presets: vec![],
        };
        let editor = EffectEditor {
            theme: Theme::dark(),
            width: 1440.0,
            height: 757.0,
            target,
            source: "<html></html>".into(),
            saved_source: "<html></html>".into(),
            file_name: "my-effect.html".into(),
            values: Props::new(),
            show_code: true,
            code_touched: false,
            saving: false,
            save_error: None,
            clash: None,
            leaving: false,
            preview: None,
            preview_state: PreviewState::Idle,
            preview_error: None,
            draft_state: DraftState::Off,
            draft_error: None,
            draft_note: None,
            expires_in: 0,
            file_input: None,
            code_editor: None,
            page: None,
            color_open: None,
            select_open: std::collections::HashMap::new(),
            probe: std::rc::Rc::new(std::cell::Cell::new(0.0)),
            preview_seq: 0,
        };
        assert!(!editor.dirty());
        assert!(editor.is_new());
        assert!(editor.can_save(), "a new file always has something to save");
    }

    #[test]
    fn the_live_note_says_what_the_preview_is_doing() {
        let mut editor = EffectEditor {
            theme: Theme::dark(),
            width: 1440.0,
            height: 757.0,
            target: EditorTarget {
                id: None,
                name: "X".into(),
                file_name: "x.html".into(),
                source: String::new(),
                props: Props::new(),
                schema: None,
                presets: vec![],
                user_presets: vec![],
            },
            source: String::new(),
            saved_source: String::new(),
            file_name: "x.html".into(),
            values: Props::new(),
            show_code: true,
            code_touched: false,
            saving: false,
            save_error: None,
            clash: None,
            leaving: false,
            preview: None,
            preview_state: PreviewState::Rendering,
            preview_error: None,
            draft_state: DraftState::Off,
            draft_error: None,
            draft_note: None,
            expires_in: 0,
            file_input: None,
            code_editor: None,
            page: None,
            color_open: None,
            select_open: std::collections::HashMap::new(),
            probe: std::rc::Rc::new(std::cell::Cell::new(0.0)),
            preview_seq: 0,
        };
        assert_eq!(editor.live_note(), "Rendering…");
        editor.preview_state = PreviewState::Busy;
        assert_eq!(editor.live_note(), "Busy: the daemon is rendering other previews");
        editor.preview_state = PreviewState::Error;
        assert_eq!(editor.live_note(), "Preview stopped: the daemon did not answer");
        editor.preview_state = PreviewState::Ok;
        assert_eq!(editor.live_note(), "Preview");
    }

    #[test]
    fn the_preview_debounce_is_the_css_one() {
        assert_eq!(PREVIEW_DEBOUNCE_MS, 400);
    }
}
