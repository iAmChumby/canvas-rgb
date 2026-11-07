//! `EffectsView`, after `app/web/src/views/EffectsView.tsx`,
//! `PresetStrip.tsx`, `PropertyControls.tsx` and
//! `views/effects/NewEffectChooser.tsx`
//! (`docs/gpui-parity/effects-library.md`).
//!
//! The page: the library gallery (cards with thumbnails, the filter, the
//! categories and the search), the detail head with its Activate key, the
//! preset strip and the property controls. It talks to the daemon through
//! `crate::client::DaemonClient` with calls off the UI thread, using the
//! routes React uses (`crates/canvas-proto/src/api.rs`).
//!
//! The effect editor itself is #214: opening an effect for editing shows the
//! route target as "Not ported yet", and the stack builder is the same.

pub mod preset_strip;
pub mod preset_match;
pub mod search;

mod preset_ops;
pub mod render;

use std::collections::{HashMap, HashSet};

use canvas_proto::api::{EffectDetail, EffectsResponse, Status};
use canvas_proto::props::{PropValue, Props, PropertySchema};
use gpui::{AppContext, Context, Window};

use crate::client::DaemonClient;
use crate::components::prop_controls::OnChange;
use crate::components::text_input::TextInput;
use crate::theme::Theme;
use crate::views::effects::preset_match::RunningPreset;

pub const LOAD_DELAY_MS: u64 = 150;
pub const PATCH_DEBOUNCE_MS: u64 = 60;
pub const UNDO_WINDOW_MS: u64 = 8000;
pub const TOAST_MS: u64 = 3000;

// ---------------------------------------------------------------------------
// Logic, ported from effects/*.ts.

/// A library category, in filter order (`effectCategories.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Ambient,
    Motion,
    Reactive,
    Nature,
}

impl Category {
    pub const ALL: [(Category, &'static str, &'static str); 4] = [
        (Category::Ambient, "Solid & ambient", "One colour, held or pulsing"),
        (Category::Motion, "Motion", "Patterns that move on their own"),
        (Category::Reactive, "Reactive", "Effects that read the screen or the audio"),
        (Category::Nature, "Nature", "Weather, fire and the northern lights"),
    ];
}

/// Which category an effect id sits in, or None (a user effect has none).
pub fn category_of(id: &str) -> Option<Category> {
    match id {
        "solid" | "gradient" | "breathing" => Some(Category::Ambient),
        "rainbow_wave" | "spiral" | "color_cycle" => Some(Category::Motion),
        "screen_ambience" | "audio_spectrum" => Some(Category::Reactive),
        "aurora" | "rain" | "fire" => Some(Category::Nature),
        _ => None,
    }
}

/// The layer names a stack's schema carries (`stackLayers.ts`): the daemon
/// labels each property `{layer} · {property}`, so a layer's name is
/// everything before the first separator.
pub fn stack_layer_names(schema: &PropertySchema) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for def in &schema.properties {
        let name = match def.label.find(" · ") {
            Some(at) => &def.label[..at],
            None => def.label.as_str(),
        };
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    if names.is_empty() {
        if let Some(after) = schema.description.split(": ").nth(1) {
            return after.split(", ").map(String::from).collect();
        }
    }
    names
}

/// One gallery row: a library entry with its search match.
#[derive(Debug, Clone)]
pub struct Row {
    pub entry: canvas_proto::api::EffectSummary,
    pub ranges: Vec<(usize, usize)>,
    pub excerpt: Option<(String, Vec<(usize, usize)>)>,
    pub score: f64,
}

/// The gallery list (`EffectsView.tsx` `list`): searched, filtered by the
/// Favorites filter and the category, then sorted by match score, highest
/// first.
pub fn gallery_list(
    effects: &[canvas_proto::api::EffectSummary],
    query: &str,
    favorites_only: bool,
    category: Option<Category>,
) -> Vec<Row> {
    let q = query.trim();
    let mut rows: Vec<Row> = Vec::new();
    for (i, entry) in effects.iter().enumerate() {
        if favorites_only && !entry.favorite {
            continue;
        }
        if let Some(want) = category {
            if category_of(&entry.id) != Some(want) {
                continue;
            }
        }
        if let Some(m) = search::match_effect(q, &entry.name, &entry.description, i) {
            rows.push(Row {
                entry: entry.clone(),
                ranges: m.ranges,
                excerpt: m.excerpt,
                score: m.score,
            });
        }
    }
    rows.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    rows
}

/// The file name without its extension (`fileStem`).
pub fn file_stem(file_name: &str) -> &str {
    file_name.strip_suffix(".html").or_else(|| file_name.strip_suffix(".HTML")).unwrap_or(file_name)
}

/// The effect id a file name would have.
pub fn id_of_file(file_name: &str) -> String {
    format!("html:{}", file_stem(file_name))
}

/// A file name that is not in the library yet: `ink-drop copy.html`, `ink-drop copy 2.html`.
pub fn copy_file_name(from: &str, taken_ids: &[String]) -> String {
    let stem = file_stem(from);
    let taken: HashSet<String> = taken_ids.iter().map(|id| id.to_lowercase()).collect();
    let mut candidate = format!("{stem} copy.html");
    let mut n = 2;
    while taken.contains(&id_of_file(&candidate).to_lowercase()) {
        candidate = format!("{stem} copy {n}.html");
        n += 1;
    }
    candidate
}

/// A blank file's name that is not in the library yet: `my-effect.html`, `my-effect 2.html`.
pub fn blank_file_name(taken_ids: &[String]) -> String {
    let taken: HashSet<String> = taken_ids.iter().map(|id| id.to_lowercase()).collect();
    let mut candidate = "my-effect.html".to_string();
    let mut n = 2;
    while taken.contains(&id_of_file(&candidate).to_lowercase()) {
        candidate = format!("my-effect {n}.html");
        n += 1;
    }
    candidate
}

/// The title a source declares, or "" when it has none.
pub fn title_of(source: &str) -> String {
    let lowered = source.to_lowercase();
    let Some(open) = lowered.find("<title") else {
        return String::new();
    };
    let Some(close_of_open) = lowered[open..].find('>') else {
        return String::new();
    };
    let content_start = open + close_of_open + 1;
    let Some(close) = lowered[content_start..].find("</title>") else {
        return String::new();
    };
    source[content_start..content_start + close].trim().to_string()
}

/// The minimal file `New effect > Blank` opens (`BLANK_EFFECT`): #214's
/// editor reads this; the placeholder pane does not.
#[allow(dead_code)]
pub const BLANK_EFFECT: &str = "<!doctype html>\n<head>\n  <title>My effect</title>\n  <meta description=\"One line about what the effect does.\" />\n  <meta property=\"ink\" label=\"Ink\" type=\"color\" default=\"#8fb4e8\" />\n  <meta property=\"speed\" label=\"Speed\" type=\"number\" min=\"0\" max=\"100\" default=\"30\" />\n</head>\n<body style=\"margin: 0; padding: 0\">\n  <canvas id=\"stage\" width=\"320\" height=\"200\"></canvas>\n</body>\n<script>\n  // The property values arrive as globals. Read them inside the draw loop, not once at load.\n  var ink = \"#8fb4e8\";\n  var speed = 30;\n\n  var stage = document.getElementById(\"stage\");\n  var ctx = stage.getContext(\"2d\");\n  var t = 0;\n\n  function draw() {\n    ctx.fillStyle = \"#07080b\";\n    ctx.fillRect(0, 0, 320, 200);\n    ctx.fillStyle = ink;\n    var x = 160 + 120 * Math.sin(t / (140 - speed));\n    ctx.beginPath();\n    ctx.arc(x, 100, 26, 0, Math.PI * 2);\n    ctx.fill();\n    t += 1;\n    window.requestAnimationFrame(draw);\n  }\n  window.requestAnimationFrame(draw);\n</script>\n";

// ---------------------------------------------------------------------------
// The view.

#[derive(Debug, Clone)]
pub struct EditorTarget {
    pub name: String,
    pub file_name: String,
}

pub enum EditorState {
    Closed,
    Loading,
    Error { id: String, message: String },
    Open(EditorTarget),
}

pub struct EffectsView {
    pub theme: Theme,
    pub width: f32,
    // Gallery list (`GET /api/v1/effects`).
    pub effects: Option<EffectsResponse>,
    pub list_error: Option<String>,
    pub list_busy: bool,
    pub list_loading_shown: bool,
    // Filters.
    pub favorites_only: bool,
    pub category: Option<Category>,
    pub query: String,
    pub search_input: gpui::Entity<TextInput>,
    // Selection and detail.
    pub picked: Option<String>,
    pub loaded_id: Option<String>,
    pub loaded_detail: Option<EffectDetail>,
    pub read_error: Option<String>,
    pub detail_request: u64,
    pub detail_loading_shown: bool,
    // Live values and the write path.
    pub values: Props,
    pub saved: Props,
    pub pending: Props,
    pub pending_for: Option<String>,
    pub pending_generation: u64,
    pub write_error: Option<(Option<String>, String)>,
    // Page actions.
    pub importing: bool,
    pub import_error: Option<String>,
    pub replace_with: Option<(String, String)>,
    pub deleting: Option<String>,
    pub show_failed: bool,
    pub more_open: bool,
    // The editor pane (#214 territory, shown as "Not ported yet").
    pub editor: EditorState,
    pub new_open: bool,
    pub new_busy: Option<String>,
    pub new_error: Option<String>,
    // Phone sheet.
    pub pushed: bool,
    // Daemon status (for `noLayout`).
    pub status: Option<Status>,
    // Thumbnails.
    pub thumbs_pending: HashSet<String>,
    // Presets.
    pub applied: HashMap<String, String>,
    pub preset: preset_strip::PresetStripState,
    pub name_input: Option<gpui::Entity<TextInput>>,
    pub naming_text: String,
    pub color_open: Option<String>,
    pub select_open: HashMap<String, bool>,
    pub undo: Option<preset_strip::Undo>,
    pub undo_generation: u64,
    pub toast: Option<String>,
    pub toast_generation: u64,
    // Stale detail.
    pub reload_tick: u32,
}

impl EffectsView {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| {
            TextInput::new("", false, 0.0, 0.0, 0, theme, std::rc::Rc::new(|_, _, _| {}), cx)
        });
        let mut view = EffectsView {
            theme,
            width: 1280.0,
            effects: None,
            list_error: None,
            list_busy: false,
            list_loading_shown: false,
            favorites_only: false,
            category: None,
            query: String::new(),
            search_input,
            picked: None,
            loaded_id: None,
            loaded_detail: None,
            read_error: None,
            detail_request: 0,
            detail_loading_shown: false,
            values: Props::new(),
            saved: Props::new(),
            pending: Props::new(),
            pending_for: None,
            pending_generation: 0,
            write_error: None,
            importing: false,
            import_error: None,
            replace_with: None,
            deleting: None,
            show_failed: false,
            more_open: false,
            editor: EditorState::Closed,
            new_open: false,
            new_busy: None,
            new_error: None,
            pushed: false,
            status: None,
            thumbs_pending: HashSet::new(),
            applied: HashMap::new(),
            preset: preset_strip::PresetStripState::default(),
            undo: None,
            undo_generation: 0,
            toast: None,
            toast_generation: 0,
            reload_tick: 0,
            name_input: None,
            naming_text: String::new(),
            color_open: None,
            select_open: HashMap::new(),
        };
        // The search box's change feed needs the search entity; the shell
        view.load_list(cx);
        view.load_status(cx);
        view.start_load_delay(cx);
        view
    }

    fn start_load_delay(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(LOAD_DELAY_MS)).await;
            let _ = view.update(cx, |this, cx| {
                this.list_loading_shown = this.effects.is_none();
                this.detail_loading_shown = this.loaded_detail.is_none() && this.read_error.is_none();
                cx.notify();
            });
        })
        .detach();
    }

    fn client() -> DaemonClient {
        DaemonClient::new()
    }

    /// The selected effect id: the pick, else the running one, else the
    /// first in the library.
    pub fn selected_id(&self) -> Option<String> {
        if let Some(picked) = &self.picked {
            return Some(picked.clone());
        }
        if let Some(active) = self.effects.as_ref().and_then(|e| e.active.clone()) {
            return Some(active);
        }
        self.effects.as_ref().and_then(|e| e.effects.first()).map(|e| e.id.clone())
    }

    /// The detail the inspector shows: the last one loaded while the next is
    /// on its way (`stale` says the two disagree).
    fn detail(&self) -> Option<EffectDetail> {
        self.loaded_detail.clone()
    }

    fn stale(&self) -> bool {
        self.loaded_id.is_some() && self.loaded_id != self.selected_id()
    }

    fn running(&self) -> bool {
        let Some(id) = self.selected_id() else {
            return false;
        };
        self.effects.as_ref().and_then(|e| e.active.as_ref()) == Some(&id)
            || self.status.as_ref().and_then(|s| s.active_effect.as_ref()) == Some(&id)
    }

    fn editing(&self) -> bool {
        !matches!(self.editor, EditorState::Closed) || self.new_open
    }

    fn no_layout(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.active_layout.is_none())
    }

    fn list_stale(&self) -> bool {
        self.effects.is_some() && self.list_error.is_some()
    }

    /// The values the controls show: the detail's, with live edits on top —
    /// no: on load `values` becomes the detail's own props, and edits insert
    /// into it, so it is the live map already. (`values` starts empty and
    /// is replaced when a detail lands.)
    fn live_values(&self) -> Props {
        let mut out = self.detail().map(|d| d.props.clone()).unwrap_or_default();
        // `values` only holds edits after the last load; when it equals the
        // loaded props it is the same map. Edits insert into `values`
        // directly, so prefer it when it is non-empty.
        if !self.values.is_empty() {
            out = self.values.clone();
        }
        out
    }

    fn running_preset(&self) -> Option<RunningPreset> {
        let detail = self.detail()?;
        if self.stale() {
            return None;
        }
        preset_match::running_of(
            &detail.schema.properties,
            &detail.schema.presets,
            &detail.user_presets,
            &self.live_values(),
            self.applied.get(&detail.schema.id).map(String::as_str),
        )
    }

    // -- reads -------------------------------------------------------------

    /// `GET /api/v1/effects` (`loadList`).
    fn load_list(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.get("/api/v1/effects")
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.list_busy = false;
                match result {
                    Ok(body) => match serde_json::from_str::<EffectsResponse>(&body) {
                        Ok(parsed) => {
                            this.effects = Some(parsed);
                            this.list_error = None;
                            // The selected effect may have appeared.
                            if this.loaded_detail.is_none() {
                                if let Some(id) = this.selected_id() {
                                    this.load_detail(&id, cx);
                                }
                            }
                        }
                        Err(e) => this.list_error = Some(e.to_string()),
                    },
                    Err(e) => this.list_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Try again" (`retryList`).
    pub fn retry_list(&mut self, cx: &mut Context<Self>) {
        if self.list_busy {
            return;
        }
        self.list_error = None;
        self.list_busy = true;
        self.load_list(cx);
        cx.notify();
    }

    fn load_status(&mut self, cx: &mut Context<Self>) {
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.get("/api/v1/status")
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                if let Ok(body) = result {
                    if let Ok(parsed) = serde_json::from_str::<Status>(&body) {
                        this.status = Some(parsed);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `GET /api/v1/effects/{id}` for the selection.
    fn load_detail(&mut self, id: &str, cx: &mut Context<Self>) {
        self.detail_request += 1;
        let n = self.detail_request;
        let id_owned = id.to_string();
        let id_bg = id_owned.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.get(&format!("/api/v1/effects/{id_bg}"))
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                if n != this.detail_request {
                    return;
                }
                match result {
                    Ok(body) => match serde_json::from_str::<EffectDetail>(&body) {
                        Ok(detail) => {
                            this.values = detail.props.clone();
                            this.saved = detail.props.clone();
                            this.loaded_id = Some(id_owned.clone());
                            this.loaded_detail = Some(detail);
                            this.read_error = None;
                        }
                        Err(e) => this.read_error = Some(e.to_string()),
                    },
                    Err(e) => this.read_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Try again" on the detail error panel: clears the read error and
    /// reads the effect again.
    pub fn retry_detail(&mut self, cx: &mut Context<Self>) {
        self.read_error = None;
        self.reload_tick += 1;
        if let Some(id) = self.selected_id() {
            self.load_detail(&id, cx);
        }
    }

    // -- edits -------------------------------------------------------------

    /// Sends the waiting values to the effect they were edited on, and to no
    /// other (`flush`).
    fn flush(&mut self, cx: &mut Context<Self>) {
        self.pending_generation += 1;
        let id = self.pending_for.take();
        let batch = std::mem::take(&mut self.pending);
        let Some(id) = id else {
            return;
        };
        if batch.is_empty() {
            return;
        }
        let to_running = self.status.as_ref().and_then(|s| s.active_effect.as_ref()) == Some(&id)
            || self.effects.as_ref().and_then(|e| e.active.as_ref()) == Some(&id);
        let batch_clone = batch.clone();
        let id_bg = id.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            if to_running {
                let body = serde_json::to_string(&batch_clone).unwrap_or_else(|_| "{}".into());
                client.patch_json("/api/v1/effects/active/props", &body)
            } else {
                // The first edit to a stopped effect starts it, with the
                // edited values on top.
                let body = serde_json::to_string(&preset_strip::ActivateBody {
                    props: batch_clone.clone(),
                    preset: None,
                })
                .unwrap_or_else(|_| "{}".into());
                client.post_json(&format!("/api/v1/effects/{id_bg}/activate"), &body)
            }
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        if !to_running {
                            this.load_list(cx);
                            this.load_status(cx);
                        }
                        if this.selected_id().as_deref() == Some(&id) {
                            for (k, v) in batch.clone() {
                                this.saved.insert(k, v);
                            }
                        }
                        if this.write_error.as_ref().is_some_and(|(w, _)| w.as_deref() == Some(&id)) {
                            this.write_error = None;
                        }
                    }
                    Err(e) => {
                        this.write_error = Some((Some(id.clone()), e.to_string()));
                        // The control goes back to the value the daemon has.
                        if this.selected_id().as_deref() == Some(&id) {
                            this.values = this.saved.clone();
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// One property's change (`onChange`): the batch is sent 60 ms after
    /// the last change, and a batch waiting for another effect goes there
    /// first (it never mixes into this one's).
    fn on_change(&mut self, key: &str, value: PropValue, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id() else {
            return;
        };
        if self.pending_for.as_deref() == Some(id.as_str()) && !self.pending.is_empty() {
            // Already waiting for this effect: keep waiting together.
        } else if self.pending_for.is_some() {
            // Values waiting for another effect go to it first.
            self.flush(cx);
        }
        self.pending_for = Some(id.clone());
        // An edit replaces what a failed write asked for.
        if self.write_error.as_ref().is_some_and(|(w, _)| w.as_deref() == Some(id.as_str())) {
            self.write_error = None;
        }
        self.values.insert(key.to_string(), value.clone());
        self.pending.insert(key.to_string(), value);
        self.pending_generation += 1;
        let generation = self.pending_generation;
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(PATCH_DEBOUNCE_MS)).await;
            let _ = view.update(cx, |this, cx| {
                if generation == this.pending_generation {
                    this.flush(cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The shared control's change callback for `key`.
    fn change_callback(&self, key: &str, entity: gpui::Entity<Self>) -> OnChange {
        let key_owned = key.to_string();
        std::rc::Rc::new(move |v: PropValue, _window: &mut Window, cx: &mut gpui::App| {
            entity.update(cx, |this, cx| this.on_change(&key_owned, v, cx));
        })
    }

    // -- page writes --------------------------------------------------------

    /// Activate (`activate(null)`).
    fn activate(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_id() else {
            return;
        };
        self.write_error = None;
        let id_bg = id.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let body = serde_json::to_string(&preset_strip::ActivateBody { props: Props::new(), preset: None })
                .unwrap_or_else(|_| "{}".into());
            client.post_json(&format!("/api/v1/effects/{id_bg}/activate"), &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.load_list(cx);
                        this.load_status(cx);
                    }
                    Err(e) => this.write_error = Some((Some(id.clone()), e.to_string())),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The card star and the head chip (`toggleFavorite`).
    fn toggle_favorite(&mut self, id: &str, favorite: bool, cx: &mut Context<Self>) {
        // A page action's failure clears on the next favourite attempt.
        if self.write_error.as_ref().is_some_and(|(w, _)| w.is_none()) {
            self.write_error = None;
        }
        let id_bg = id.to_string();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let body = serde_json::to_string(&serde_json::json!({ "favorite": favorite })).unwrap_or_else(|_| "{}".into());
            client.put_json(&format!("/api/v1/effects/{id_bg}/favorite"), &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.load_list(cx),
                    Err(e) => this.write_error = Some((None, e.to_string())),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Remove a user effect (`deleteEffect`).
    fn delete_effect(&mut self, id: &str, keep_presets: bool, cx: &mut Context<Self>) {
        self.deleting = None;
        if self.write_error.as_ref().is_some_and(|(w, _)| w.is_none()) {
            self.write_error = None;
        }
        let id_bg = id.to_string();
        let query = if keep_presets { "?keep_presets=true" } else { "" };
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.delete(&format!("/api/v1/effects/{id_bg}{query}"))
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(_) => this.load_list(cx),
                    Err(e) => this.write_error = Some((None, e.to_string())),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Sends an HTML effect to the daemon (`sendImport`). Without a file
    /// picker there is no way to reach this from the page; the import note
    /// and the replace flow are kept, the button stays disabled, and the
    /// divergence is reported.
    #[allow(dead_code)]
    fn send_import(&mut self, file_name: &str, html: &str, replace: bool, cx: &mut Context<Self>) {
        self.importing = true;
        let file_name = file_name.to_string();
        let html = html.to_string();
        let file_name_bg = file_name.clone();
        let html_bg = html.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            let body = serde_json::to_string(&serde_json::json!({
                "file_name": file_name_bg,
                "html": html_bg,
                "replace": replace,
            }))
            .unwrap_or_else(|_| "{}".into());
            client.post_json("/api/v1/effects/import", &body)
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.importing = false;
                match result {
                    Ok(body) => {
                        if let Ok(parsed) = serde_json::from_str::<canvas_proto::api::ImportEffectResponse>(&body) {
                            this.picked = Some(parsed.effect.id.clone());
                            this.replace_with = None;
                            this.import_error = None;
                            // The long-load sentence is not in the library,
                            // so it is said as a toast.
                            let in_library = this
                                .effects
                                .as_ref()
                                .is_some_and(|e| e.effects.iter().any(|x| x.id == parsed.effect.id && x.description == parsed.effect.description));
                            if !in_library {
                                this.set_toast(&parsed.effect.description, cx);
                            }
                        }
                        this.load_list(cx);
                    }
                    Err(crate::client::ClientError::Daemon { status: 409, .. }) => {
                        this.replace_with = Some((file_name.clone(), html.clone()));
                        this.import_error = None;
                    }
                    Err(e) => {
                        this.replace_with = None;
                        this.import_error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_toast(&mut self, message: &str, cx: &mut Context<Self>) {
        self.toast = Some(message.to_string());
        self.toast_generation += 1;
        let generation = self.toast_generation;
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(TOAST_MS)).await;
            let _ = view.update(cx, |this, cx| {
                if generation == this.toast_generation {
                    this.toast = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    // -- pick -------------------------------------------------------------

    /// A card click (`pick`).
    fn pick(&mut self, id: &str, cx: &mut Context<Self>) {
        self.picked = Some(id.to_string());
        // A failed favourite or delete was said on the page the user has
        // moved on from.
        if self.write_error.as_ref().is_some_and(|(w, _)| w.is_none()) {
            self.write_error = None;
        }
        if self.width <= 720.0 {
            self.pushed = true;
        }
        // A batch still waiting goes to the effect it was edited on.
        if self.pending_for.is_some() {
            self.flush(cx);
        }
        self.load_detail(id, cx);
        cx.notify();
    }

    fn close_sheet(&mut self) {
        self.pushed = false;
        self.new_open = false;
    }

    // -- editor entry ------------------------------------------------------

    /// Drops a waiting batch and opens the code editor for one effect
    /// (#214: the pane itself is not ported, so this fetches the source and
    /// then shows the placeholder).
    fn start_edit(&mut self, id: &str, cx: &mut Context<Self>) {
        self.pending_generation += 1;
        self.pending.clear();
        self.pending_for = None;
        self.new_open = false;
        self.new_error = None;
        self.editor = EditorState::Loading;
        if self.width <= 720.0 {
            self.pushed = true;
        }
        let id_bg = id.to_string();
        let id_out = id_bg.clone();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.get(&format!("/api/v1/effects/{id_bg}/source"))
        });
        let name_bg = self.detail().map(|d| d.schema.name.clone()).unwrap_or_else(|| id.to_string());
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(body) => {
                        let file_name = serde_json::from_str::<canvas_proto::api::EffectSource>(&body)
                            .map(|s| s.file_name)
                            .unwrap_or_default();
                        this.editor = EditorState::Open(EditorTarget { name: name_bg.clone(), file_name });
                    }
                    Err(e) => {
                        this.editor = EditorState::Error { id: id_out.clone(), message: e.to_string() };
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn retry_edit(&mut self, cx: &mut Context<Self>) {
        if let EditorState::Error { id, .. } = &self.editor {
            let id = id.clone();
            self.start_edit(&id, cx);
        }
    }

    fn close_editor(&mut self) {
        self.editor = EditorState::Closed;
        self.new_open = false;
        if self.width <= 720.0 {
            self.pushed = false;
        }
    }

    /// New from code > Blank.
    fn start_blank(&mut self) {
        let taken: Vec<String> = self.effects.as_ref().map(|e| e.effects.iter().map(|x| x.id.clone()).collect()).unwrap_or_default();
        self.new_open = false;
        self.new_error = None;
        self.editor = EditorState::Open(EditorTarget {
            name: "My effect".to_string(),
            file_name: blank_file_name(&taken),
        });
        if self.width <= 720.0 {
            self.pushed = true;
        }
    }

    /// New from code > Copy.
    fn start_copy(&mut self, id: &str, cx: &mut Context<Self>) {
        self.new_busy = Some(id.to_string());
        self.new_error = None;
        let id_bg = id.to_string();
        let bg = cx.background_executor().spawn(async move {
            let client = Self::client();
            client.get(&format!("/api/v1/effects/{id_bg}/source"))
        });
        cx.spawn(async move |view, cx| {
            let result = bg.await;
            let _ = view.update(cx, |this, cx| {
                this.new_busy = None;
                match result {
                    Ok(body) => {
                        if let Ok(source) = serde_json::from_str::<canvas_proto::api::EffectSource>(&body) {
                            let title = title_of(&source.source);
                            let name = format!("{} copy", if title.is_empty() { "Effect".to_string() } else { title });
                            let taken: Vec<String> = this.effects.as_ref().map(|e| e.effects.iter().map(|x| x.id.clone()).collect()).unwrap_or_default();
                            let file_name = copy_file_name(&source.file_name, &taken);
                            this.new_open = false;
                            this.editor = EditorState::Open(EditorTarget { name, file_name });
                            if this.width <= 720.0 {
                                this.pushed = true;
                            }
                        }
                    }
                    Err(e) => this.new_error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // -- strip --------------------------------------------------------------

    pub fn strip_title(&self) -> (String, Option<String>) {
        let count = match (&self.effects, self.list_stale()) {
            (Some(e), false) => {
                let rows = gallery_list(&e.effects, &self.query, self.favorites_only, self.category);
                Some(format!("{} effects", rows.len()))
            }
            _ => Some("– effects".to_string()),
        };
        ("Effects".to_string(), count)
    }

    /// The search box runs its own text now that its entity exists: the
    /// shell creates the view first, then connects the feed.
    pub fn connect_search(&mut self, cx: &mut Context<Self>) {
        let entity = cx.entity();
        let on_change = std::rc::Rc::new(move |text: &str, _window: &mut Window, cx: &mut gpui::App| {
            entity.update(cx, |this, cx| {
                this.query = text.to_string();
                cx.notify();
            });
        });
        self.search_input.update(cx, |input, _cx| {
            input.set_on_text_change(on_change);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_membership_is_the_static_id_map() {
        assert_eq!(category_of("solid"), Some(Category::Ambient));
        assert_eq!(category_of("gradient"), Some(Category::Ambient));
        assert_eq!(category_of("breathing"), Some(Category::Ambient));
        assert_eq!(category_of("rainbow_wave"), Some(Category::Motion));
        assert_eq!(category_of("spiral"), Some(Category::Motion));
        assert_eq!(category_of("color_cycle"), Some(Category::Motion));
        assert_eq!(category_of("screen_ambience"), Some(Category::Reactive));
        assert_eq!(category_of("audio_spectrum"), Some(Category::Reactive));
        assert_eq!(category_of("aurora"), Some(Category::Nature));
        assert_eq!(category_of("rain"), Some(Category::Nature));
        assert_eq!(category_of("fire"), Some(Category::Nature));
        assert_eq!(category_of("html:mine"), None);
        assert_eq!(category_of("stack:mine"), None);
    }

    #[test]
    fn a_category_pick_hides_the_uncategorised_effects() {
        let effects = vec![
            canvas_proto::api::EffectSummary { id: "html:mine".into(), name: "Mine".into(), description: String::new(), needs: Default::default(), favorite: false },
            canvas_proto::api::EffectSummary { id: "spiral".into(), name: "Spiral".into(), description: String::new(), needs: Default::default(), favorite: false },
        ];
        let rows = gallery_list(&effects, "", false, Some(Category::Motion));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry.id, "spiral");
    }

    #[test]
    fn the_favorites_filter_keeps_only_favourite_rows() {
        let effects = vec![
            canvas_proto::api::EffectSummary { id: "a".into(), name: "A".into(), description: String::new(), needs: Default::default(), favorite: true },
            canvas_proto::api::EffectSummary { id: "b".into(), name: "B".into(), description: String::new(), needs: Default::default(), favorite: false },
        ];
        let rows = gallery_list(&effects, "", true, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry.id, "a");
    }

    #[test]
    fn the_gallery_sorts_by_match_score_highest_first() {
        let effects = vec![
            canvas_proto::api::EffectSummary { id: "a".into(), name: "Other".into(), description: "a spiral thing".into(), needs: Default::default(), favorite: false },
            canvas_proto::api::EffectSummary { id: "b".into(), name: "Spiral".into(), description: String::new(), needs: Default::default(), favorite: false },
        ];
        let rows = gallery_list(&effects, "spir", false, None);
        assert_eq!(rows[0].entry.id, "b");
    }

    #[test]
    fn copy_and_blank_file_names_avoid_the_library() {
        let taken = vec!["html:ink-drop".to_string()];
        assert_eq!(copy_file_name("ink-drop.html", &taken), "ink-drop copy.html");
        assert_eq!(blank_file_name(&[]), "my-effect.html");
        assert_eq!(blank_file_name(&["html:my-effect".to_string()]), "my-effect.html".replace("my-effect.html", "my-effect 2.html"));
    }

    #[test]
    fn the_blank_file_is_the_documented_minimal_one() {
        assert!(BLANK_EFFECT.contains("<title>My effect</title>"));
        assert!(BLANK_EFFECT.contains("requestAnimationFrame"));
        assert_eq!(title_of(BLANK_EFFECT), "My effect");
        assert_eq!(title_of("<html></html>"), "");
    }

    #[test]
    fn stack_layers_read_off_the_property_labels() {
        let schema = PropertySchema {
            id: "stack:s".into(),
            name: "S".into(),
            description: "two layers".into(),
            needs: Default::default(),
            properties: vec![
                canvas_proto::props::PropertyDef { key: "a".into(), label: "Rain · Speed".into(), kind: canvas_proto::props::PropKind::Number { min: 0.0, max: 100.0, step: 1.0, default: 1.0 } },
                canvas_proto::props::PropertyDef { key: "b".into(), label: "Fire · Ink".into(), kind: canvas_proto::props::PropKind::Color { default: "#ffffff".parse().unwrap() } },
            ],
            presets: vec![],
            thumbnail: None,
        };
        assert_eq!(stack_layer_names(&schema), vec!["Rain".to_string(), "Fire".to_string()]);
    }
}
