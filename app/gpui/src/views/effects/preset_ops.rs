//! The preset writes, after `EffectsView.tsx`'s preset handlers (apply,
//! save, update, rename, duplicate, delete, undo) and the strip's attempt
//! bookkeeping (`working`, the refusal line, the held key).

use canvas_proto::api::PresetsResponse;
use canvas_proto::props::Props;
use gpui::{AppContext, Context, Window};

use crate::client::{ClientError, DaemonClient};
use crate::components::text_input::TextInput;
use crate::views::effects::preset_match;
use crate::views::effects::preset_strip::{self, Doing, Naming};
use crate::views::effects::EffectsView;

fn client() -> DaemonClient {
    DaemonClient::new()
}

/// A preset another window removed or changed: say so and read the list again.
pub fn notify_gone(view: &mut EffectsView, cx: &mut Context<EffectsView>, message: &str) {
    view.set_toast(message, cx);
    view.reload_tick += 1;
    if let Some(id) = view.selected_id() {
        view.load_detail(&id, cx);
    }
}

/// One preset write with the strip's bookkeeping: `working` while it flies,
/// the refusal line when it fails, the held key for a save.
#[allow(clippy::too_many_arguments)]
fn attempt(
    view: &mut EffectsView,
    cx: &mut Context<EffectsView>,
    doing: Doing,
    held: Option<String>,
    method: fn(&str, &str) -> Result<String, ClientError>,
    path: String,
    body: String,
    done_on_ok: impl FnOnce(&mut EffectsView, &str, &mut Context<EffectsView>) + 'static,
) {
    view.preset.working = Some(doing);
    view.preset.error = None;
    if let Some(name) = &held {
        view.preset.saving = Some(name.clone());
    }
    let path_bg = path;
    let body_bg = body;
    let bg = cx.background_executor().spawn(async move {
        method(&path_bg, &body_bg)
    });
    cx.spawn(async move |view, cx| {
        let result = bg.await;
        let _ = view.update(cx, |this, cx| {
            this.preset.working = None;
            this.preset.saving = None;
            match result {
                Ok(body) => {
                    done_on_ok(this, &body, cx);
                }
                Err(ClientError::Daemon { status, .. }) if matches!(status, 404) && matches!(doing, Doing::Apply | Doing::Update | Doing::Rename | Doing::Delete) => {
                    notify_gone(this, cx, "That preset was removed in another window");
                }
                Err(ClientError::Daemon { status: 409, .. }) if doing == Doing::Update => {
                    notify_gone(this, cx, "Changed in another window");
                }
                Err(e) => {
                    let (message, status) = match &e {
                        ClientError::Daemon { status, message } => (message.clone(), Some(*status)),
                        other => (other.to_string(), None),
                    };
                    this.preset.error = Some(preset_strip::explain(&message, status, doing));
                }
            }
            cx.notify();
        });
    })
    .detach();
    cx.notify();
}

impl EffectsView {
    /// The effect the strip writes to, if its detail is on screen.
    fn preset_target(&self) -> Option<(String, Vec<canvas_proto::props::Preset>, Vec<canvas_proto::props::Preset>)> {
        let detail = self.loaded_detail.as_ref()?;
        if self.loaded_id.as_ref() != self.selected_id().as_ref() {
            return None;
        }
        Some((detail.schema.id.clone(), detail.schema.presets.clone(), detail.user_presets.clone()))
    }

    fn set_user_presets(&mut self, id: &str, user_presets: Vec<canvas_proto::props::Preset>) {
        if let Some(detail) = &mut self.loaded_detail {
            if detail.schema.id == id {
                detail.user_presets = user_presets;
            }
        }
    }

    /// Apply a preset (`applyPreset`): activate with the preset, then read
    /// the list and the values again.
    pub fn apply_preset(&mut self, name: &str, cx: &mut Context<EffectsView>) {
        let Some((id, _, _)) = self.preset_target() else {
            return;
        };
        self.preset.error = None;
        let path = format!("/api/v1/effects/{id}/activate");
        let body = serde_json::to_string(&preset_strip::ActivateBody { props: Props::new(), preset: Some(name.to_string()) })
            .unwrap_or_else(|_| "{}".into());
        let applied_id = id.clone();
        let applied_name = name.to_string();
        attempt(
            self,
            cx,
            Doing::Apply,
            None,
            |p, b| client().post_json(p, b),
            path,
            body,
            move |this, _body, cx| {
                this.applied.insert(applied_id.clone(), applied_name);
                this.load_list(cx);
                this.reload_tick += 1;
                if let Some(sel) = this.selected_id() {
                    this.load_detail(&sel, cx);
                }
            },
        );
    }

    pub fn do_reset(&mut self, name: &str, cx: &mut Context<EffectsView>) {
        self.apply_preset(name, cx);
    }

    pub fn do_update(&mut self, name: &str, cx: &mut Context<EffectsView>) {
        let Some((id, _, user)) = self.preset_target() else {
            return;
        };
        let Some(preset) = user.iter().find(|p| p.name == name) else {
            return;
        };
        let Some(detail) = self.loaded_detail.clone() else {
            return;
        };
        let body = serde_json::to_string(&serde_json::json!({
            "rename_to": null,
            "props": preset_match::fill_defaults(&detail.schema.properties, &self.live_values()),
            "expected": preset.props,
        }))
        .unwrap_or_else(|_| "{}".into());
        let path = format!("/api/v1/effects/{id}/presets/{}", encode(name));
        let applied_id = id.clone();
        let applied_name = name.to_string();
        attempt(
            self,
            cx,
            Doing::Update,
            None,
            |p, b| client().put_json(p, b),
            path,
            body,
            move |this, body, _cx| {
                if let Ok(r) = serde_json::from_str::<PresetsResponse>(body) {
                    this.set_user_presets(&applied_id, r.user_presets);
                }
                this.applied.insert(applied_id.clone(), applied_name);
            },
        );
    }

    pub fn duplicate_preset(&mut self, name: &str, cx: &mut Context<EffectsView>) {
        let Some((id, built_in, user)) = self.preset_target() else {
            return;
        };
        let Some(preset) = user.iter().find(|p| p.name == name) else {
            return;
        };
        let lists: Vec<&[canvas_proto::props::Preset]> = vec![&built_in, &user];
        let copy = preset_match::next_free_name(name, &lists);
        // `POST /effects/{id}/presets` takes `{ name, props }`.
        let body = serde_json::to_string(&serde_json::json!({ "name": copy, "props": preset.props }))
            .unwrap_or_else(|_| "{}".into());
        let path = format!("/api/v1/effects/{id}/presets");
        attempt(
            self,
            cx,
            Doing::Duplicate,
            None,
            |p, b| client().post_json(p, b),
            path,
            body,
            move |this, body, _cx| {
                if let Ok(r) = serde_json::from_str::<PresetsResponse>(body) {
                    this.set_user_presets(&id, r.user_presets);
                }
            },
        );
    }

    pub fn delete_preset(&mut self, name: &str, cx: &mut Context<EffectsView>) {
        let Some((id, _, user)) = self.preset_target() else {
            return;
        };
        let Some(preset) = user.iter().find(|p| p.name == name) else {
            return;
        };
        let was_applied = self.applied.get(&id).is_some_and(|a| a == name);
        let preset_props = preset.props.clone();
        let path = format!("/api/v1/effects/{id}/presets/{}", encode(name));
        let delete_name = name.to_string();
        attempt(
            self,
            cx,
            Doing::Delete,
            None,
            |p, _b| client().delete(p),
            path,
            String::new(),
            move |this, body, cx| {
                this.close_menu();
                if let Ok(r) = serde_json::from_str::<PresetsResponse>(body) {
                    this.set_user_presets(&id, r.user_presets);
                }
                // The live values stay as they are; they just no longer
                // belong to a preset.
                if was_applied {
                    this.applied.remove(&id);
                }
                this.undo = Some(preset_strip::Undo {
                    name: delete_name.clone(),
                    effect_id: id.clone(),
                    props: preset_props,
                    was_applied,
                });
                this.undo_generation += 1;
                let generation = this.undo_generation;
                cx.spawn(async move |view, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(crate::views::effects::UNDO_WINDOW_MS)).await;
                    let _ = view.update(cx, |this, cx| {
                        if generation == this.undo_generation {
                            this.undo = None;
                        }
                        cx.notify();
                    });
                })
                .detach();
                this.set_toast(&format!("Deleted {delete_name}"), cx);
            },
        );
    }

    pub fn do_undo(&mut self, cx: &mut Context<EffectsView>) {
        let Some(undo) = self.undo.clone() else {
            return;
        };
        let body = serde_json::to_string(&serde_json::json!({ "name": undo.name, "props": undo.props }))
            .unwrap_or_else(|_| "{}".into());
        let path = format!("/api/v1/effects/{}/presets", undo.effect_id);
        let undo_id = undo.effect_id.clone();
        let undo_name = undo.name.clone();
        let undo_applied = undo.was_applied;
        attempt(
            self,
            cx,
            Doing::Undo,
            None,
            |p, b| client().post_json(p, b),
            path,
            body,
            move |this, body, _cx| {
                this.undo = None;
                if let Ok(r) = serde_json::from_str::<PresetsResponse>(body) {
                    this.set_user_presets(&undo_id, r.user_presets);
                }
                if undo_applied {
                    this.applied.insert(undo_id.clone(), undo_name);
                }
            },
        );
    }

    // -- naming -----------------------------------------------------------

    pub fn open_naming(&mut self, naming: Naming, cx: &mut Context<EffectsView>) {
        self.preset.error = None;
        self.preset.menu = None;
        self.preset.naming = Some(naming.clone());
        let initial = match &naming {
            Naming::Rename { name } => name.clone(),
            _ => String::new(),
        };
        self.naming_text = initial.clone();
        let input = match &self.name_input {
            Some(existing) => {
                let existing = existing.clone();
                existing.update(cx, |input, cx| input.set_value(&initial, cx));
                existing
            }
            None => {
                let entity_text = cx.entity();
                let entity_submit = entity_text.clone();
                let entity_escape = entity_text.clone();
                let input = cx.new(|cx| {
                    let mut input = TextInput::new(&initial, false, 0.0, 0.0, 0, self.theme, std::rc::Rc::new(|_, _, _| {}), cx);
                    input.set_text_mode(Some(if matches!(naming, Naming::Rename { .. }) { "New name" } else { "Name these values" }), cx);
                    input.set_on_text_change(std::rc::Rc::new(move |text: &str, _w: &mut Window, cx: &mut gpui::App| {
                        entity_text.update(cx, |this, cx| {
                            this.naming_text = text.to_string();
                            this.preset.error = None;
                            cx.notify();
                        });
                    }));
                    input.set_on_submit_escape(
                        std::rc::Rc::new(move |_w: &mut Window, cx: &mut gpui::App| {
                            entity_submit.update(cx, |this, cx| this.submit_naming(cx));
                        }),
                        std::rc::Rc::new(move |_w: &mut Window, cx: &mut gpui::App| {
                            entity_escape.update(cx, |this, _cx| this.close_naming());
                        }),
                    );
                    input
                });
                self.name_input = Some(input.clone());
                input
            }
        };
        let _ = input;
    }

    pub fn close_naming(&mut self) {
        self.preset.naming = None;
        self.naming_text = String::new();
        self.preset.error = None;
    }

    pub fn toggle_save_naming(&mut self, cx: &mut Context<EffectsView>) {
        if self.preset.naming.as_ref() == Some(&Naming::Save) {
            self.close_naming();
        } else {
            self.open_naming(Naming::Save, cx);
        }
    }

    fn naming_value(&self) -> String {
        self.naming_text.clone()
    }

    pub fn submit_naming(&mut self, cx: &mut Context<EffectsView>) {
        let Some(naming) = self.preset.naming.clone() else {
            return;
        };
        if self.preset.working.is_some() {
            return;
        }
        let name = self.naming_value().trim().to_string();
        if name.is_empty() {
            self.preset.error = Some("Give the preset a name.".to_string());
            return;
        }
        let except = if let Naming::Rename { name: own } = &naming {
            Some(own.clone())
        } else {
            None
        };
        if let Naming::Rename { name: own } = &naming {
            if &name == own {
                self.close_naming();
                return;
            }
        }
        let Some((id, built_in, user)) = self.preset_target() else {
            return;
        };
        let lists: Vec<&[canvas_proto::props::Preset]> = vec![&built_in, &user];
        if preset_match::name_taken(&name, &lists, except.as_deref()) {
            self.preset.error = Some(format!("{name} is already a preset of this effect."));
            return;
        }
        match naming {
            Naming::Rename { name: old } => {
                let Some(detail) = self.loaded_detail.clone() else {
                    return;
                };
                let Some(preset) = detail.user_presets.iter().find(|p| p.name == old) else {
                    return;
                };
                let body = serde_json::to_string(&serde_json::json!({
                    "rename_to": name,
                    "props": preset.props,
                    "expected": preset.props,
                }))
                .unwrap_or_else(|_| "{}".into());
                let path = format!("/api/v1/effects/{id}/presets/{}", encode(&old));
                let applied_id = id.clone();
                let old_name = old.clone();
                let new_name = name.clone();
                attempt(
                    self,
                    cx,
                    Doing::Rename,
                    None,
                    |p, b| client().put_json(p, b),
                    path,
                    body,
                    move |this, body, _cx| {
                        if let Ok(r) = serde_json::from_str::<PresetsResponse>(body) {
                            this.set_user_presets(&applied_id, r.user_presets);
                        }
                        if this.applied.get(&applied_id).is_some_and(|a| a == &old_name) {
                            this.applied.insert(applied_id.clone(), new_name);
                        }
                        this.close_naming();
                    },
                );
            }
            Naming::Save | Naming::SaveAs => {
                let Some(detail) = self.loaded_detail.clone() else {
                    return;
                };
                let body = serde_json::to_string(&serde_json::json!({
                    "name": name,
                    "props": preset_match::fill_defaults(&detail.schema.properties, &self.live_values()),
                }))
                .unwrap_or_else(|_| "{}".into());
                let path = format!("/api/v1/effects/{id}/presets");
                let applied_id = id.clone();
                let saved_name = name.clone();
                let held = name.clone();
                attempt(
                    self,
                    cx,
                    Doing::Save,
                    Some(held),
                    |p, b| client().post_json(p, b),
                    path,
                    body,
                    move |this, body, _cx| {
                        if let Ok(r) = serde_json::from_str::<PresetsResponse>(body) {
                            this.set_user_presets(&applied_id, r.user_presets);
                        }
                        this.applied.insert(applied_id.clone(), saved_name);
                        this.close_naming();
                    },
                );
            }
        }
    }

    // -- menu --------------------------------------------------------------

    pub fn open_menu(&mut self, name: &str, confirm: bool) {
        self.preset.menu = Some((name.to_string(), confirm));
        self.preset.error = None;
    }

    pub fn close_menu(&mut self) {
        self.preset.menu = None;
    }
}

fn encode(name: &str) -> String {
    // Preset names are used raw in React's path; the daemon matches them
    // decoded. Names with a space or slash need encoding to reach the
    // right route.
    let mut out = String::new();
    for byte in name.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
