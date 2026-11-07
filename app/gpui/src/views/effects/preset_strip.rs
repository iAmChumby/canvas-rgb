//! The preset strip, after `app/web/src/views/PresetStrip.tsx`: the keys
//! (built-ins, then the owner's, then "Save current"), the line under them
//! ("Matches X", "Changed from X" with Update/Save-as/Reset, the naming
//! field, or "Deleted X" with Undo) and the note under that (the change in
//! numbers, or why something was refused).
//!
//! The strip owns only what it shows (the name field, the open menu). Every
//! change goes out through a method on the Effects view that returns when
//! the daemon has answered; a refusal prints under the line and the field
//! stays open to be fixed.

use canvas_proto::props::{Preset, Props, PropertyDef};
use gpui::{IntoElement, div, prelude::*, px};
use serde::Serialize;

use crate::components::button::{Variant, button, icon_button};
use crate::components::chip::{ChipTone, chip, chip_add, chip_button};
use crate::components::panel::panel_with_padding;
use crate::theme::{Theme, TokenRadius};
use crate::views::effects::preset_match::{RunningPreset, diff_props};
use crate::views::effects::EffectsView;

/// What the name field is for: a new preset from the live values, or a rename.
#[derive(Debug, Clone, PartialEq)]
pub enum Naming {
    Save,
    SaveAs,
    Rename { name: String },
}

/// What the strip is asking the daemon to do, in the words of its error
/// ("Could not save ...").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Doing {
    Apply,
    Save,
    Update,
    Rename,
    Duplicate,
    Delete,
    Undo,
}

/// The words of a refusal: the daemon's own sentence for a clash or a bad
/// name, else what failed and why (`explain`).
pub fn explain(message: &str, status: Option<u16>, doing: Doing) -> String {
    if let Some(status) = status {
        if status == 409 || status == 422 {
            return message.to_string();
        }
    }
    // "could not save the effects: access is denied" -> "Access is denied":
    // the verb is said once, up front.
    let bare = message
        .split_once(": ")
        .filter(|(head, _)| head.to_ascii_lowercase().starts_with("could not "))
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_else(|| message.to_string());
    let reason = match bare.chars().next() {
        Some(first) => first.to_uppercase().to_string() + &bare[first.len_utf8()..],
        None => String::new(),
    };
    let failed = match doing {
        Doing::Apply => "Not applied",
        Doing::Save => "Not saved",
        Doing::Update => "Not updated",
        Doing::Rename => "Not renamed",
        Doing::Duplicate => "Not duplicated",
        Doing::Delete => "Not deleted",
        Doing::Undo => "Not put back",
    };
    format!("{failed}: {reason}")
}

/// The preset deleted last, while it can still be put back (8 s).
#[derive(Debug, Clone)]
pub struct Undo {
    pub name: String,
    pub effect_id: String,
    pub props: Props,
    pub was_applied: bool,
}

/// The strip's own state.
#[derive(Debug, Default)]
pub struct PresetStripState {
    pub naming: Option<Naming>,
    pub working: Option<Doing>,
    pub error: Option<String>,
    /// The name being saved, shown as a held key at the end of the owner's.
    pub saving: Option<String>,
    pub menu: Option<(String, bool)>,
}

/// The body of `POST /api/v1/effects/{id}/activate` (spine
/// `ActivateEffect`).
#[derive(Clone, Serialize)]
pub struct ActivateBody {
    pub props: Props,
    pub preset: Option<String>,
}

/// How many changes the diff line spells out before it says "+N more".
pub const DIFF_SHOWN: usize = 2;

type Entity = gpui::Entity<EffectsView>;

/// The strip: the keys, the line, the note, and the preset menu.
#[allow(clippy::too_many_arguments)]
pub fn render(
    t: &Theme,
    entity: Entity,
    state: &PresetStripState,
    name_input: Option<gpui::Entity<crate::components::text_input::TextInput>>,
    built_in: &[Preset],
    user: &[Preset],
    defs: &[PropertyDef],
    live: &Props,
    running: Option<&RunningPreset>,
    disabled: bool,
    undo: Option<&Undo>,
    narrow: bool,
) -> impl IntoElement {
    let running_user = running.and_then(|r| user.iter().find(|p| p.name == r.name));
    let running_any = running.and_then(|r| {
        built_in
            .iter()
            .chain(user.iter())
            .find(|p| p.name == r.name)
    });
    let changes = match running {
        Some(r) if r.modified => match running_any {
            Some(any) => diff_props(defs, &any.props, live),
            None => Vec::new(),
        },
        _ => Vec::new(),
    };
    let change_text = {
        let shown = changes
            .iter()
            .take(DIFF_SHOWN)
            .map(|c| format!("{} {} to {}", c.label, c.from, c.to))
            .collect::<Vec<_>>()
            .join(", ");
        if changes.len() > DIFF_SHOWN {
            format!("{} +{} more", shown, changes.len() - DIFF_SHOWN)
        } else {
            shown
        }
    };

    // The keys: built-ins, a rule, the owner's, the held key, "Save current"
    // (first on a phone).
    let mut chips = div()
        .id(gpui::SharedString::from("preset-chips"))
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2();
    let make_save_current = |entity: Entity| {
        chip_add(t, "preset-save-current", "Save current", state.naming.as_ref() == Some(&Naming::Save), move |_, _, cx| {
            entity.update(cx, |this, cx| this.toggle_save_naming(cx));
        })
    };
    if narrow {
        chips = chips.child(make_save_current(entity.clone()));
    }
    for p in built_in {
        chips = chips.child(preset_key(t, entity.clone(), p, false, running, built_in));
    }
    if !built_in.is_empty() && (!user.is_empty() || state.saving.is_some()) {
        // React `.preset-rule`: 1 px, margin 4 px on every side (20 px of
        // rule in a 28 px row, 8 px of space each side beyond the gap).
        chips = chips.child(
            div()
                .w(px(1.0))
                .h(px(20.0))
                .flex_none()
                .mx(px(4.0))
                .my(px(4.0))
                .bg(Theme::color(t.line_2)),
        );
    }
    for p in user {
        chips = chips.child(preset_key(t, entity.clone(), p, true, running, built_in));
    }
    if let Some(saving) = &state.saving {
        if !user.iter().any(|p| &p.name == saving) {
            chips = chips.child(chip(
                t,
                &format!("preset-saving-{saving}"),
                saving,
                ChipTone::Plain,
                true,
                false,
                true,
            ));
        }
    }
    if !narrow {
        chips = chips.child(make_save_current(entity.clone()));
    }

    // The line under the keys: always 36 px, so no state moves the page.
    let save_word = matches!(state.working, Some(Doing::Save | Doing::Update | Doing::Rename));
    let line: gpui::AnyElement = if let Some(naming) = &state.naming {
        let verb = if matches!(naming, Naming::Rename { .. }) { "Rename" } else { "Save" };
        if save_word {
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child("Saving…".to_string()),
                )
                .into_any_element()
        } else {
            let mut row = div().flex().items_center().gap_2().flex_1().min_w(px(0.0));
            if let Some(input) = name_input {
                row = row.child(div().flex_1().min_w(px(0.0)).child(input));
            }
            let entity = entity.clone();
            let entity_cancel = entity.clone();
            if narrow {
                row = row
                    .child(icon_button(t, "preset-name-submit", "Check", "bold", true, false, false, move |_, _, cx| {
                        entity.update(cx, |this, cx| this.submit_naming(cx));
                    }))
                    .child(icon_button(t, "preset-name-cancel", "X", "", true, false, false, move |_, _, cx| {
                        entity_cancel.update(cx, |this, _cx| this.close_naming());
                    }));
            } else {
                row = row
                    .child(button(t, "preset-name-submit", verb, Variant::Primary, false, move |_, _, cx| {
                        entity.update(cx, |this, cx| this.submit_naming(cx));
                    }))
                    .child(button(t, "preset-name-cancel", "Cancel", Variant::Ghost, false, move |_, _, cx| {
                        entity_cancel.update(cx, |this, _cx| this.close_naming());
                    }));
            }
            row.into_any_element()
        }
    } else if running.is_some_and(|r| r.modified) {
        let r = running.expect("checked");
        let busy = state.working.is_some() || disabled;
        let mut row = div().flex().items_center().gap_2().flex_1().min_w(px(0.0));
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_1()
                .min_w(px(0.0))
                .child(div().size(px(6.0)).rounded_full().flex_none().bg(Theme::color(t.warn)))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.dim))
                        .child(
                            div()
                                .flex()
                                .child("Changed from ")
                                .child(
                                    div()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(Theme::color(t.ink))
                                        .child(r.name.clone()),
                                ),
                        ),
                ),
        );
        if running_user.is_some() {
            let entity = entity.clone();
            let name = r.name.clone();
            row = row.child(button(t, "preset-update", "Update", Variant::Primary, busy, move |_, _, cx| {
                entity.update(cx, |this, cx| this.do_update(&name, cx));
            }));
        }
        let entity_as = entity.clone();
        row = row.child(icon_button(t, "preset-save-as", "Plus", "", false, false, state.working.is_some(), move |_, _, cx| {
            entity_as.update(cx, |this, cx| this.open_naming(Naming::SaveAs, cx));
        }));
        let entity_reset = entity.clone();
        let reset_name = r.name.clone();
        row = row.child(icon_button(t, "preset-reset", "ArrowCounterClockwise", "", false, false, state.working.is_some(), move |_, _, cx| {
            entity_reset.update(cx, |this, cx| this.do_reset(&reset_name, cx));
        }));
        row.into_any_element()
    } else if let Some(undo) = undo {
        let entity = entity.clone();
        let busy = state.working.is_some();
        div()
            .flex()
            .items_center()
            .gap_2()
            .flex_1()
            .min_w(px(0.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(
                        div()
                            .flex()
                            .child("Deleted ")
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(Theme::color(t.ink))
                                    .child(undo.name.clone()),
                            ),
                    ),
            )
            .child(button(t, "preset-undo", "Undo", Variant::Default, busy, move |_, _, cx| {
                entity.update(cx, |this, cx| this.do_undo(cx));
            }))
            .into_any_element()
    } else {
        let text: gpui::AnyElement = match running {
            Some(r) => div()
                .flex()
                .child("Matches ")
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Theme::color(t.ink))
                        .child(r.name.clone()),
                )
                .into_any_element(),
            None => div().child("Custom values".to_string()).into_any_element(),
        };
        div()
            .flex()
            .items_center()
            .flex_1()
            .min_w(px(0.0))
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.dim))
            .child(text)
            .into_any_element()
    };

    let note_text = state.error.clone().unwrap_or_else(|| {
        if state.naming.is_some() {
            String::new()
        } else {
            change_text
        }
    });
    let note = div()
        .h(px(22.0))
        .mt_1()
        .whitespace_nowrap()
        .overflow_hidden()
        .text_sm()
        .font_family(if state.error.is_some() {
            crate::fonts::UI_FAMILY
        } else {
            crate::fonts::MONO_FAMILY
        })
        .text_color(if state.error.is_some() {
            Theme::color(t.down)
        } else {
            Theme::color(t.mute)
        })
        .child(note_text);

    panel_with_padding(
        t,
        "preset-strip",
        "Presets",
        div(),
        div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .child(chips)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(36.0))
                    .mt_3()
                    .min_w(px(0.0))
                    .child(line),
            )
            .child(note)
            .child(preset_menu(t, entity, state, user)),
        Some(12.0),
    )
}

fn preset_key(
    t: &Theme,
    entity: Entity,
    p: &Preset,
    mine: bool,
    running: Option<&RunningPreset>,
    built_in: &[Preset],
) -> gpui::AnyElement {
    let lit = running.is_some_and(|r| r.name == p.name);
    let marked = lit && running.is_some_and(|r| r.modified);
    // A built-in preset added under a name this owner's preset already uses
    // wins: the daemon looks up built-ins first, so the user's key cannot be
    // reached until it is renamed.
    let shadowed = mine && built_in.iter().any(|b| crate::views::effects::preset_match::ascii_fold(&b.name) == crate::views::effects::preset_match::ascii_fold(&p.name));
    let mut label = p.name.clone();
    if marked {
        label.push_str(" (modified)");
    }
    if shadowed {
        label.push_str(" (shadowed)");
    }
    let name = p.name.clone();
    let entity_apply = entity.clone();
    let name_apply = name.clone();
    let mut key = div().flex().items_center().gap(px(2.0)).flex_none().child(chip_button(
        t,
        &format!("preset-key-{}", p.name),
        &label,
        ChipTone::Plain,
        lit,
        marked,
        false,
        move |_, _, cx| {
            entity_apply.update(cx, |this, cx| this.apply_preset(&name_apply, cx));
        },
    ));
    if mine {
        let entity_menu = entity.clone();
        let name_menu = name.clone();
        key = key.child(icon_button(
            t,
            &format!("preset-actions-{}", p.name),
            "DotsThree",
            "bold",
            true,
            false,
            false,
            move |_, _, cx| {
                entity_menu.update(cx, |this, _cx| this.open_menu(&name_menu, false));
            },
        ));
    }
    key.into_any_element()
}

/// The menu of a user preset's key: Rename, Duplicate, Delete — or the
/// "Delete X?" ask.
fn preset_menu(t: &Theme, entity: Entity, state: &PresetStripState, user: &[Preset]) -> gpui::AnyElement {
    let Some((name, confirm)) = &state.menu else {
        return div().into_any_element();
    };
    if !user.iter().any(|p| &p.name == name) {
        return div().into_any_element();
    }
    let busy = state.working.is_some();
    let mut menu = div()
        .id(gpui::SharedString::from(format!("preset-menu-{name}")))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .p_1()
        .mt_2()
        .r_lg()
        .border_1()
        .border_color(Theme::color(t.line_2))
        .bg(Theme::color(t.panel));
    if *confirm {
        let entity_cancel = entity.clone();
        let entity_delete = entity.clone();
        let delete_name = name.clone();
        let cancel_name = name.clone();
        menu = menu
            .child(
                div()
                    .p_2()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child(format!("Delete {name}?")),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_1()
                    .pb_1()
                    .child(button(
                        t,
                        &format!("preset-menu-delete-{cancel_name}"),
                        "Delete",
                        Variant::Primary,
                        busy,
                        move |_, _, cx| {
                            entity_delete.update(cx, |this, cx| this.delete_preset(&delete_name, cx));
                        },
                    ))
                    .child(button(t, &format!("preset-menu-cancel-{cancel_name}"), "Cancel", Variant::Ghost, false, move |_, _, cx| {
                        entity_cancel.update(cx, |this, _cx| this.close_menu());
                    })),
            );
    } else {
        for (id, icon, word) in [
            ("rename", "PencilSimple", "Rename"),
            ("duplicate", "CopySimple", "Duplicate"),
            ("delete", "Trash", "Delete"),
        ] {
            let entity_item = entity.clone();
            let item_name = name.clone();
            let _ = icon;
            menu = menu.child(button(
                t,
                &format!("preset-menu-{id}-{item_name}"),
                word,
                Variant::Ghost,
                busy,
                move |_, _, cx| {
                    entity_item.update(cx, |this, cx| {
                        match word {
                            "Rename" => this.open_naming(Naming::Rename { name: item_name.clone() }, cx),
                            "Duplicate" => this.duplicate_preset(&item_name, cx),
                            _ => this.open_menu(&item_name, true),
                        }
                    });
                },
            ));
        }
    }
    menu.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_name_the_reason_with_a_capital_first_letter() {
        let got = explain("the name is taken", None, Doing::Save);
        assert_eq!(got, "Not saved: The name is taken");
        assert_eq!(explain("access denied", None, Doing::Delete), "Not deleted: Access denied");
    }

    #[test]
    fn a_clash_shows_the_daemons_own_sentence() {
        let got = explain("that name is taken", Some(409), Doing::Save);
        assert_eq!(got, "that name is taken");
        let got = explain("the name must be 40 characters or fewer", Some(422), Doing::Save);
        assert_eq!(got, "the name must be 40 characters or fewer");
        let got = explain("the name is taken", Some(500), Doing::Save);
        assert_eq!(got, "Not saved: The name is taken");
    }
}
