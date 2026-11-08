//! The Effects page's rendering: the toolbar, the gallery, the inspector
//! (head, preset strip, properties), the editor placeholder (#214) and the
//! new-effect chooser.

use gpui::{Context, IntoElement, StatefulInteractiveElement, Window, div, prelude::*, px};

use crate::components::button::{Variant, button, button_icon, button_icon_wide, icon_button};
use crate::components::chip::{ChipTone, chip, chip_button, chip_icon, chip_icon_button};
use crate::components::empty_state::empty_state;
use crate::components::panel::{panel_with_padding, state_dot};
use crate::components::prop_controls::{OnChange, control};
use crate::components::segmented::segmented;
use crate::components::thumb::{cached, fetch as fetch_thumb, thumb};
use crate::shell::text_width;
use crate::theme::{Theme, TokenRadius};
use crate::views::effects::preset_strip::{self};
use crate::views::effects::{Category, EditorState, EffectsView, Row, gallery_list, stack_layer_names};

type Entity = gpui::Entity<EffectsView>;

/// The strip's page actions (React `PageActions`): New effect, More, Import.
pub fn strip_actions(view: &EffectsView, cx: &mut Context<EffectsView>) -> gpui::AnyElement {
    // Build mode: the builder's own keys (Revert, Save).
    if let Some(builder) = view.builder.clone() {
        let narrow = view.width <= 720.0;
        let for_actions = builder.clone();
        return builder.update(cx, move |b, _cx| {
            crate::views::effects::stack_builder::strip_actions(b, for_actions.clone(), narrow)
        });
    }
    view.actions_row(cx).into_any_element()
}

/// The strip's leading key (React `PageTitle`'s `leading`): the builder's
/// Back to the effects.
pub fn strip_leading(view: &EffectsView, cx: &mut Context<EffectsView>) -> Option<gpui::AnyElement> {
    let builder = view.builder.clone()?;
    let for_leading = builder.clone();
    Some(builder.update(cx, move |b, _cx| {
        crate::views::effects::stack_builder::strip_leading(b, for_leading.clone())
    }))
}

impl EffectsView {
    /// The page actions, entity-bound (called from [`strip_actions`]).
    pub fn actions_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let entity = cx.entity();
        let editing = self.editing();
        let phone = self.width <= 640.0;
        // The strip row's gap is `--s1` (4) on a phone, `--s2` (8) above.
        let mut row = div()
            .flex()
            .items_center()
            .gap(if phone { px(4.0) } else { px(8.0) });
        let entity_new = entity.clone();
        row = row.child(button_icon(
            t,
            "eff-new-effect",
            if phone { "" } else { "New effect" },
            Variant::Primary,
            editing,
            true,
            "Plus",
            "",
            15.0,
            move |_, _, cx| {
                entity_new.update(cx, |this, cx| this.open_stack_placeholder("new", cx));
            },
        ));
        let entity_more = entity.clone();
        let more = div()
            .relative()
            .child(icon_button(
                t,
                "eff-more",
                "DotsThreeOutline",
                "",
                false,
                false,
                false,
                move |_, _, cx| {
                    entity_more.update(cx, |this, cx| {
                        this.more_open = !this.more_open;
                        cx.notify();
                    });
                },
            ));
        let more = if self.more_open {
            let entity_code = entity.clone();
            more.child(
                div()
                    .id(gpui::SharedString::from("eff-more-menu"))
                    .absolute()
                    .top(px(38.0))
                    .right(px(0.0))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w(px(260.0))
                    .p_3()
                    .r_lg()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .bg(Theme::color(t.panel))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child("Advanced".to_string()),
                    )
                    .child(button_icon(
                        t,
                        "eff-new-from-code",
                        "New from code",
                        Variant::Default,
                        true,
                        editing,
                        "Code",
                        "",
                        15.0,
                        move |_, _, cx| {
                            entity_code.update(cx, |this, _cx| {
                                this.more_open = false;
                                this.new_open = true;
                                this.new_error = None;
                                if this.width <= 720.0 {
                                    this.pushed = true;
                                }
                            });
                        },
                    ))
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child("An HTML effect you write here: copy one of the library's, or start blank.".to_string()),
                    ),
            )
        } else {
            more
        };
        row = row.child(more);
        // Import: React's FileButton is a hidden <input type=file>, which
        // needs no tray; GPUI's native path prompt is the same door
        // (Settings' config import uses it). Disabled while importing or
        // editing, as React's is. It is the default (32 px) key, not sm:
        // measured 42x32 at 390 against GPUI's 38x28 (#244).
        let entity_import = entity.clone();
        row = row.child(button_icon(
            t,
            "eff-import",
            if phone { "" } else { "Import" },
            Variant::Default,
            self.importing || editing,
            false,
            "UploadSimple",
            "",
            16.0,
            move |_, _, cx| {
                entity_import.update(cx, |this, cx| this.pick_import_file(cx));
            },
        ));
        row
    }

    /// The stack builder's route target (#214).
    pub fn open_stack_placeholder(&mut self, name: &str, cx: &mut Context<Self>) {
        self.more_open = false;
        self.open_stack(name, cx);
    }

    fn thumbs_needed(&mut self, rows: &[Row], cx: &mut Context<Self>) {
        for row in rows {
            let id = row.entry.id.clone();
            if cached(&id).is_none() && self.thumbs_pending.insert(id.clone()) {
                fetch_thumb(id, cx.entity(), cx, |_this, cx, _id| {
                    cx.notify();
                });
            }
        }
    }

    fn skeletons(&self, count: usize) -> impl IntoElement {
        let t = &self.theme;
        let mut grid = div().flex().flex_wrap().gap_2();
        for _ in 0..count {
            grid = grid.child(
                div()
                    .w(px(168.0))
                    .flex_none()
                    .r_lg()
                    .border_1()
                    .border_color(Theme::color(t.line))
                    .bg(Theme::color(t.panel))
                    .overflow_hidden()
                    .child(div().w_full().h(px(105.0)).bg(Theme::color(t.raised)))
                    .child(
                        div()
                            .p_2()
                            .child(div().w(px(80.0)).h(px(12.0)).r_sm().bg(Theme::color(t.raised))),
                    ),
            );
        }
        grid
    }

    fn card(&self, _window: &Window, _cx: &mut Context<Self>, row: &Row, card_w: f32, entity: Entity) -> impl IntoElement {
        let t = &self.theme;
        let e = &row.entry;
        let html = e.id.starts_with("html:");
        let stack = e.id.starts_with("stack:");
        let selected = Some(&e.id) == self.selected_id().as_ref();
        let running = self.effects.as_ref().and_then(|x| x.active.as_ref()) == Some(&e.id);
        let editing = self.editing();
        // React `.effect-card-preview` is aspect 16/10 of the card's
        // content width (inside the 1 px border): (card_w - 2) * 10/16,
        // not the outer width. Off the outer width the card ran 1 px
        // taller and every row drifted 2 px further from React's.
        let preview_h = (card_w - 2.0) * 10.0 / 16.0;
        let mut card = div()
            .id(gpui::SharedString::from(format!("eff-card-{}", e.id)))
            .relative()
            .flex_none()
            .w(px(card_w))
            .r_lg()
            .border_1()
            .border_color(if selected { t.live() } else { Theme::color(t.line) })
            .bg(Theme::color(t.panel))
            .overflow_hidden();
        // React's card clips the preview at its r-lg top corners
        // (`.effect-card{overflow:hidden}`); the thumb keeps its own r-sm.
        let preview_wrap = div()
            .relative()
            .w_full()
            .h(px(preview_h))
            .bg(Theme::color(t.sunk))
            .overflow_hidden()
            .child(thumb(t, &e.id, card_w - 2.0, preview_h, cached(&e.id), false));
        card = card.child(preview_wrap);
        if !editing {
            let mut name_box = div().flex().flex_col().gap(px(2.0)).flex_1().min_w(px(0.0));
            name_box = name_box.child(marked_name(t, &e.name, &row.ranges));
            if let Some((excerpt, _)) = &row.excerpt {
                name_box = name_box.child(
                    div()
                        .text_sm()
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(excerpt.clone()),
                );
            }
            // React `.rd-effects .effect-card-meta` padding s2 (8 px all
            // sides): the scoped rule outranks motion.css's s2 s3.
            let mut meta = div().flex().items_center().gap_2().p_2().child(name_box);
            if html {
                meta = meta.child(chip(t, "eff-chip-html", "HTML", ChipTone::Html, false, false, false));
            }
            if stack {
                meta = meta.child(chip(t, "eff-chip-stack", "Stack", ChipTone::Stack, false, false, false));
            }
            if running {
                meta = meta.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .flex_none()
                        .text_sm()
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(t.ok))
                        .child(div().size(px(7.0)).rounded_full().bg(Theme::color(t.ok)))
                        .child("running".to_string()),
                );
            }
            card = card.child(meta);
            let entity_fav = entity.clone();
            let fav_id = e.id.clone();
            let favorite = e.favorite;
            // React `.effect-card-fav` (motion.css): an `IconButton` over the
            // panel face with a line-2 border, no skirt (`app.css .ibtn` is
            // transparent; the fav rule gives it `--panel` and `--line-2`).
            let star = div()
                .id(gpui::SharedString::from(format!("eff-fav-{}", e.id)))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.0))
                .r_md()
                .border_1()
                .border_color(Theme::color(t.line_2))
                .bg(Theme::color(t.panel))
                .text_color(Theme::color(t.dim))
                .child(
                    gpui::svg()
                        .path(if favorite { "icons/Star-fill.svg" } else { "icons/Star.svg" })
                        .size(px(16.0))
                        .text_color(Theme::color(t.dim)),
                )
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    entity_fav.update(cx, |this, cx| this.toggle_favorite(&fav_id, !favorite, cx));
                });
            card = card.child(div().absolute().top(px(8.0)).right(px(8.0)).child(star));
        }
        let entity_open = entity.clone();
        let open_id = e.id.clone();
        let is_stack = stack;
        card.cursor_pointer().on_click(move |_, _, cx| {
            entity_open.update(cx, |this, cx| {
                if this.editing() {
                    return;
                }
                if is_stack {
                    this.open_stack_placeholder(&open_id, cx);
                } else {
                    this.pick(&open_id, cx);
                }
            });
        })
    }

    /// The gallery: loading skeleton, error, empty, or the cards.
    fn gallery(&mut self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let entity = cx.entity();
        match (&self.effects, &self.list_error, self.list_busy) {
            (None, Some(_), false) => {
                let err = self.list_error.clone().unwrap_or_default();
                let entity_retry = entity.clone();
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .p_4()
                    .r_lg()
                    .border_1()
                    .border_color(Theme::color(self.theme.line))
                    .bg(Theme::color(self.theme.panel))
                    .child(div().mb_3().child("The effects could not be read".to_string()))
                    .child(div().child("canvas-rgb could not read the effect list. Nothing was changed.".to_string()))
                    .child(div().child(err))
                    .child(div().flex().mt_3().child(button(&self.theme, "eff-try-again", "Try again", Variant::Default, self.list_busy, move |_, _, cx| {
                        entity_retry.update(cx, |this, cx| this.retry_list(cx));
                    })))
                    .into_any_element()
            }
            (None, _, _) => {
                if self.list_loading_shown {
                    self.skeletons(12).into_any_element()
                } else {
                    div().into_any_element()
                }
            }
            (Some(effects), _, _) => {
                let rows = gallery_list(&effects.effects, &self.query, self.favorites_only, self.category);
                if rows.is_empty() {
                    let library_empty = effects.effects.is_empty();
                    let (icon, title, body) = if library_empty {
                        ("Waveform", "No effects yet", "This machine has no effect to run. Import an HTML effect to add one.")
                    } else {
                        ("MagnifyingGlass", "No effect matches that", "Try fewer words, or clear the filter and the category.")
                    };
                    return empty_state(&self.theme, "eff-empty", icon, title, body, None::<gpui::Div>).into_any_element();
                }
                let win_w = f32::from(window.window_bounds().get_bounds().size.width);
                let phone = win_w <= 720.0;
                let content_w = win_w
                    - if phone { 0.0 } else if win_w < 1280.0 { 64.0 } else { 200.0 }
                    - if phone { 32.0 } else { 48.0 };
                let inspector_w = if content_w < 640.0 {
                    content_w
                } else if content_w >= 1500.0 {
                    400.0
                } else {
                    360.0
                };
                // React `.rd-effects .effect-gallery` gap s2 (8 px): the
                // scoped rule outranks motion.css's s3. The column count
                // below divides by the same gap, as the CSS grid does.
                let gap = 8.0;
                let gallery_w = if phone { content_w } else { content_w - inspector_w - 16.0 };
                let min_tile = if content_w <= 999.0 { 140.0 } else { 168.0 };
                let mut cols = ((gallery_w + gap) / (min_tile + gap)).floor() as usize;
                if phone {
                    cols = 2;
                } else if win_w <= 960.0 && content_w > 999.0 {
                    cols = 4;
                } else if win_w <= 860.0 && content_w > 999.0 {
                    cols = 3;
                }
                cols = cols.max(1);
                let editing = self.editing();
                let card_w = if editing {
                    96.0
                } else {
                    ((gallery_w - gap * (cols as f32 - 1.0)) / cols as f32).max(96.0)
                };
                self.thumbs_needed(&rows, cx);
                let mut grid = div()
                    .id(gpui::SharedString::from("eff-gallery"))
                    .flex()
                    .flex_wrap()
                    // React `.effect-gallery` has `align-content: start`:
                    // without it GPUI's default stretch fills the scroll
                    // pane's height with the lines, and the cards (also
                    // stretched) grew 55 px of empty meta at 1920.
                    .content_start()
                    .gap_2()
                    .min_w(px(0.0));
                for row in &rows {
                    grid = grid.child(self.card(window, cx, row, card_w, entity.clone()));
                }
                grid.into_any_element()
            }
        }
    }

    /// The toolbar: filter, categories, search, the failed-files key.
    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let entity = cx.entity();
        let filter = if self.favorites_only { "favorites" } else { "all" };
        let seg = segmented(
            t,
            "eff-filter",
            &[("all", "All"), ("favorites", "Favorites")],
            filter,
            {
                let entity = entity.clone();
                move |v: String, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.favorites_only = v == "favorites";
                        cx.notify();
                    });
                }
            },
        );
        let mut cats = div().flex().flex_wrap().items_center().gap(px(4.0));
        if self.width <= 720.0 {
            // Below 640 px the container rule gives the categories their own
            // full row (`flex: 1 1 100%`): they wrap inside it.
            cats = cats.w_full();
        } else {
            cats = cats.flex_none();
        }
        for (category, label, hint) in Category::ALL {
            let _ = hint;
            let pressed = self.category == Some(category);
            let entity = entity.clone();
            cats = cats.child(chip_button(
                t,
                &format!("eff-cat-{label}"),
                label,
                ChipTone::Plain,
                pressed,
                false,
                false,
                move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.category = if this.category == Some(category) { None } else { Some(category) };
                        cx.notify();
                    });
                },
            ));
        }
        // React `.effect-search`: the icon is absolute inside the field
        // (left 8, centred on the 32 px height) and the input's text
        // starts 30 px in. The icon used to sit outside the input box,
        // pushing the whole field 22 px right.
        let search: gpui::AnyElement = div()
            .relative()
            .flex_1()
            .min_w(px(160.0))
            .child(self.search_input.clone())
            .child(
                gpui::svg()
                    .path("icons/MagnifyingGlass.svg")
                    .size(px(14.0))
                    .absolute()
                    .left(px(8.0))
                    .top(px(9.0))
                    .text_color(Theme::color(t.mute)),
            )
            .into_any_element();
        let mut row = div().flex().flex_wrap().items_center().gap(px(8.0)).flex_none().min_w(px(0.0)).child(seg).child(cats).child(search);
        if let Some(effects) = &self.effects {
            if !effects.failed.is_empty() {
                let entity = entity.clone();
                let label = format!("{} file{} did not load", effects.failed.len(), if effects.failed.len() == 1 { "" } else { "s" });
                row = row.child(chip_icon_button(
                    t,
                    "eff-failed",
                    &label,
                    Some(("Warning", "", 14.0)),
                    ChipTone::Plain,
                    false,
                    false,
                    false,
                    move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.show_failed = !this.show_failed;
                            cx.notify();
                        });
                    },
                ));
            }
        }
        row
    }

    fn stale_note(&self) -> impl IntoElement {
        let t = &self.theme;
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
            .text_sm()
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.warn))
            .child("The effect list may be out of date.".to_string())
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .child(self.list_error.clone().unwrap_or_default()),
            )
    }

    fn import_note(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let entity = cx.entity();
        if let Some((file_name, html)) = &self.replace_with {
            let entity_replace = entity.clone();
            let entity_cancel = entity.clone();
            let replace_file = file_name.clone();
            let replace_html = html.clone();
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .p_2()
                .r_md()
                .border_1()
                .border_color(Theme::color(t.warn))
                .bg(Theme::color(t.warn_wash))
                .child(format!("Replace {file_name}?"))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(button(t, "eff-replace", "Replace", Variant::Primary, self.importing, move |_, _, cx| {
                            let file = replace_file.clone();
                            let html = replace_html.clone();
                            entity_replace.update(cx, |this, cx| this.send_import(&file, &html, true, cx));
                        }))
                        .child(button(t, "eff-replace-cancel", "Cancel", Variant::Ghost, false, move |_, _, cx| {
                            entity_cancel.update(cx, |this, cx| {
                                this.replace_with = None;
                                cx.notify();
                            });
                        })),
                )
        } else if let Some(err) = &self.import_error {
            div()
                .p_2()
                .r_md()
                .border_1()
                .border_color(Theme::color(t.down))
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.down))
                .child(err.clone())
        } else {
            div()
        }
    }

    fn detail_pane(&self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        // The detail pane: head, presets, properties (or layers).
        if self.editing() {
            let win_h = f32::from(window.window_bounds().get_bounds().size.height);
            return self.editor_pane(win_h, cx);
        }
        if self.no_layout() && self.loaded_detail.is_none() {
            return empty_state(
                &self.theme,
                "eff-no-layout",
                "SquaresFour",
                "No layout to run on yet",
                "The effects are still here, ready for one.",
                Some(
                    div()
                        .id(gpui::SharedString::from("eff-open-layout"))
                        .flex()
                        .items_center()
                        .h(px(28.0))
                        .px(px(10.0))
                        .r_md()
                        .bg(Theme::color(self.theme.live))
                        .text_color(Theme::color(self.theme.on_accent))
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .font_family(crate::fonts::UI_FAMILY)
                        .child("Open the Layout".to_string()),
                ),
            )
            .into_any_element();
        }
        let Some(detail) = self.detail() else {
            if let Some(err) = self.read_error.clone() {
                let entity = cx.entity();
                return div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .p_4()
                    .r_lg()
                    .border_1()
                    .border_color(Theme::color(self.theme.line))
                    .bg(Theme::color(self.theme.panel))
                    .child(div().mb_3().child("The effect could not be read".to_string()))
                    .child(div().child("canvas-rgb could not read that effect. The others are still here.".to_string()))
                    .child(div().child(err))
                    .child(div().flex().mt_3().child(button(&self.theme, "eff-detail-retry", "Try again", Variant::Default, false, move |_, _, cx| {
                        entity.update(cx, |this, cx| this.retry_detail(cx));
                    })))
                    .into_any_element();
            }
            if self.detail_loading_shown {
                return div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .min_w(px(0.0))
                    .child(div().w_full().h(px(220.0)).r_lg().bg(Theme::color(self.theme.raised)))
                    .child(div().w_full().h(px(120.0)).r_lg().bg(Theme::color(self.theme.raised)))
                    .into_any_element();
            }
            return div().into_any_element();
        };
        let t = &self.theme;
        let selected_id = self.selected_id().unwrap_or_default();
        let is_stack = selected_id.starts_with("stack:");
        let needs_input = detail.schema.needs.screen || detail.schema.needs.audio;
        let running = self.running();
        let stale = self.stale();
        let head = self.head_panel(window, cx, &detail, needs_input, running, stale);
        let running_preset = self.running_preset();
        let undo = self.undo.as_ref().filter(|u| u.effect_id == detail.schema.id);
        let entity = cx.entity();
        let strip = preset_strip::render(
            t,
            entity.clone(),
            &self.preset,
            self.name_input.clone(),
            &detail.schema.presets,
            &detail.user_presets,
            &detail.schema.properties,
            &self.live_values(),
            running_preset.as_ref(),
            stale,
            undo,
            self.width <= 720.0,
            self.tight,
        );
        let body: gpui::AnyElement = if is_stack {
            let mut col = div().flex().flex_col().gap_3();
            col = col.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child(detail.schema.description.clone()),
            );
            let mut layers = div().flex().flex_wrap().gap_2();
            for name in stack_layer_names(&detail.schema) {
                layers = layers.child(chip(t, "eff-layer-chip", &name, ChipTone::Plain, false, false, false));
            }
            col = col.child(layers);
            col = col.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child("A stack keeps its layers, blends and areas in its own builder; the values have no controls here.".to_string()),
            );
            let entity_stack = entity.clone();
            let stack_id = detail.schema.id.clone();
            let entity_activate = entity.clone();
            col = col.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(button_icon(
                        t,
                        "eff-edit-stack",
                        "Edit stack",
                        Variant::Primary,
                        true,
                        stale,
                        "PencilSimple",
                        "",
                        14.0,
                        move |_, _, cx| {
                            entity_stack.update(cx, |this, cx| this.open_stack_placeholder(&stack_id, cx));
                        },
                    ))
                    .child(button_icon(
                        t,
                        "eff-layers-activate",
                        if running { "Running" } else { "Activate" },
                        if running { Variant::Default } else { Variant::Primary },
                        true,
                        running || stale,
                        "Play",
                        "fill",
                        14.0,
                        move |_, _, cx| entity_activate.update(cx, |this, cx| this.activate(cx)),
                    )),
            );
            col.into_any_element()
        } else {
            let mut col = div().flex().flex_col().min_w(px(0.0));
            let schema = detail.schema.properties.clone();
            let vals = self.values.clone();
            let last_at = schema.len().saturating_sub(1);
            for (at, def) in schema.iter().enumerate() {
                let value = vals.get(&def.key).cloned().unwrap_or_else(|| def.kind.default_value());
                col = col.child(self.prop_row(def, &value, entity.clone(), at == last_at));
            }
            col.into_any_element()
        };
        let error_line = self.write_error.as_ref().and_then(|(id, msg)| {
            if id.is_none() || id.as_deref() == Some(selected_id.as_str()) {
                Some(msg.clone())
            } else {
                None
            }
        });
        let mut props_panel = panel_with_padding(
            t,
            "eff-properties",
            if is_stack { "Layers" } else { "Properties" },
            div(),
            body,
            Some(if self.tight { 8.0 } else { 12.0 }),
        );
        if let Some(err) = error_line {
            props_panel = props_panel.child(
                div()
                    .mt_2()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.down))
                    .child(err),
            );
        }
        div()
            .flex()
            .flex_col()
            // React the detail is a `.stack`: gap s4 (16 px), not the
            // inspector column's s3. At 12 the presets sat 8 px high.
            .gap_4()
            .min_w(px(0.0))
            .child(head)
            .child(strip)
            .child(props_panel)
            .into_any_element()
    }

    fn head_panel(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
        detail: &canvas_proto::api::EffectDetail,
        needs_input: bool,
        running: bool,
        stale: bool,
    ) -> impl IntoElement {
        let t = &self.theme;
        let entity = cx.entity();
        let selected_id = self.selected_id().unwrap_or_default();
        let is_stack = selected_id.starts_with("stack:");
        let html = selected_id.starts_with("html:");
        let note_word = if needs_input { "Live frame" } else { "Preview" };
        // React `.effect-live` is always the bordered well (border line-2,
        // radius r-md), whichever picture sits in it.
        let preview = thumb(t, &detail.schema.id, 128.0, 80.0, cached(&detail.schema.id), true);
        let mut name_row = div().flex().flex_wrap().items_center().gap_2().min_w(px(0.0));
        name_row = name_row.child(
            div()
                .text_size(px(28.0))
                // React `h2` line-height 1.25 (a 35 px box at 28 px); the
                // GPUI default 1.618 grew the head by 10 px.
                .line_height(gpui::relative(1.25))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child(detail.schema.name.clone()),
        );
        if detail.schema.needs.screen {
            name_row = name_row.child(chip_icon(
                t,
                "eff-need-screen",
                "Screen capture",
                Some(("MonitorPlay", "", 14.0)),
                ChipTone::Plain,
                false,
                false,
                false,
            ));
        }
        if detail.schema.needs.audio {
            name_row = name_row.child(chip_icon(
                t,
                "eff-need-audio",
                "Audio input",
                Some(("Waveform", "", 14.0)),
                ChipTone::Plain,
                false,
                false,
                false,
            ));
        }
        if html {
            name_row = name_row.child(chip(t, "eff-head-html", "HTML effect", ChipTone::Html, false, false, false));
        }
        if is_stack {
            name_row = name_row.child(chip(t, "eff-head-stack", "Stack", ChipTone::Stack, false, false, false));
        }
        name_row = name_row.child(state_dot(t, if running { "ok" } else { "mute" }, if running { "Running" } else { "Idle" }));
        let widest = text_width(window, "Activate", 14.0, gpui::FontWeight::MEDIUM, crate::fonts::UI_FAMILY);
        let entity_activate = entity.clone();
        let activate = button_icon_wide(
            t,
            "eff-activate",
            if running { "Running" } else { "Activate" },
            widest.max(0.0),
            if running { Variant::Default } else { Variant::Primary },
            false,
            running || stale,
            "Play",
            "fill",
            14.0,
            move |_, _, cx| entity_activate.update(cx, |this, cx| this.activate(cx)),
        );
        let head_text = div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_w(px(0.0))
            .child(name_row)
            .child(div().flex().child(activate))
            // React's head grid has a second ("key") row with nothing in
            // it, so the row-gap (s2, 8 px) still lands under the Activate
            // key and the pane is 3 px taller (83 px, not 80). The spacer
            // is that empty row: zero height, the flex gap is the row-gap.
            .child(div().h(px(0.0)));
        // The head favourite is React's `Chip` with the star as its only
        // child (the name lives in its aria-label, which GPUI does not draw).
        let entity_fav = entity.clone();
        let fav_id = detail.schema.id.clone();
        let favorite = detail.favorite;
        let fav = chip_icon_button(
            t,
            "eff-head-fav",
            "",
            Some(("Star", if favorite { "fill" } else { "" }, 16.0)),
            ChipTone::Plain,
            favorite,
            false,
            false,
            move |_, _, cx| entity_fav.update(cx, |this, cx| this.toggle_favorite(&fav_id, !favorite, cx)),
        );
        let head_row = div().flex().items_start().gap_3().child(preview).child(head_text).child(fav);
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            // The short-window rule (`@container page (max-height: 700px)`):
            // the inspector's panels pad s2, not s3.
            .p(if self.tight { px(8.0) } else { px(12.0) })
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line))
            .bg(Theme::color(t.panel))
            .child(head_row)
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_3()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            // React `.rd-eff-desc` inherits the body 15 px
                            // at 1.45 and ellipsizes ("One flat color across
                            // the whole canv…"); at 14 px the whole sentence
                            // fitted and "Preview" sat at the edge. The cut
                            // is measured (`ellipsize`), since GPUI's
                            // truncate() hard-cuts without the ellipsis.
                            .text_size(px(15.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.dim))
                            .child(ellipsized_desc(window, self, &detail.schema.description, note_word)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_sm()
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.mute))
                            .child(note_word.to_string()),
                    ),
            );
        // The delete block (html: effects only).
        if html {
            let deleting = self.deleting.as_deref() == Some(selected_id.as_str());
            if deleting {
                let presets = detail.user_presets.len();
                let text = {
                    let mut s = String::new();
                    if running {
                        s.push_str(&format!("{} is playing. ", detail.schema.name));
                    }
                    if presets > 0 {
                        s.push_str(&format!("Delete it? It has {presets} preset{} of yours.", if presets == 1 { "" } else { "s" }));
                    } else {
                        s.push_str("Delete it anyway?");
                    }
                    s
                };
                let entity_delete = entity.clone();
                let entity_keep = entity.clone();
                let entity_cancel = entity.clone();
                let delete_id = selected_id.clone();
                let cancel_id = selected_id.clone();
                let mut row = div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(Theme::color(t.line))
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.dim))
                            .child(text),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(button(
                                t,
                                "eff-delete-confirm",
                                if presets > 0 { "Delete with presets" } else { "Delete" },
                                Variant::Primary,
                                false,
                                move |_, _, cx| {
                                    entity_delete.update(cx, |this, cx| this.delete_effect(&delete_id, false, cx));
                                },
                            )),
                    );
                if presets > 0 {
                    row = row.child(button(t, "eff-keep-presets", "Keep presets", Variant::Default, false, move |_, _, cx| {
                        entity_keep.update(cx, |this, cx| this.delete_effect(&cancel_id, true, cx));
                    }));
                }
                row = row.child(button(t, "eff-delete-cancel", "Cancel", Variant::Ghost, false, move |_, _, cx| {
                    entity_cancel.update(cx, |this, cx| {
                        this.deleting = None;
                        cx.notify();
                    });
                }));
                panel = panel.child(row);
            } else {
                let entity_edit = entity.clone();
                let entity_delete = entity.clone();
                let edit_id = selected_id.clone();
                let delete_id = selected_id.clone();
                panel = panel.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(Theme::color(t.line))
                        .child(button_icon(
                            t,
                            "eff-edit-code",
                            "Edit code",
                            Variant::Ghost,
                            true,
                            stale,
                            "PencilSimple",
                            "",
                            16.0,
                            move |_, _, cx| {
                                entity_edit.update(cx, |this, cx| this.start_edit(&edit_id, cx));
                            },
                        ))
                        .child(button_icon(
                            t,
                            "eff-delete",
                            "Delete effect",
                            Variant::Ghost,
                            true,
                            false,
                            "Trash",
                            "",
                            16.0,
                            move |_, _, cx| {
                                entity_delete.update(cx, |this, cx| {
                                    this.deleting = Some(delete_id.clone());
                                    cx.notify();
                                });
                            },
                        )),
                );
            }
        }
        panel
    }

    fn prop_row(&self, def: &canvas_proto::props::PropertyDef, value: &canvas_proto::props::PropValue, entity: Entity, last: bool) -> impl IntoElement {
        let t = &self.theme;
        let narrow = self.width <= 720.0;
        let kind_name = match &def.kind {
            canvas_proto::props::PropKind::Number { .. } => "number",
            canvas_proto::props::PropKind::Color { .. } => "color",
            canvas_proto::props::PropKind::Bool { .. } => "bool",
            canvas_proto::props::PropKind::Choice { .. } => "choice",
            canvas_proto::props::PropKind::Hue { .. } => "hue",
            canvas_proto::props::PropKind::Angle { .. } => "angle",
        };
        let label_el = div()
            .flex_1()
            .min_w(px(0.0))
            .text_size(px(15.0))
            .line_height(gpui::relative(1.45))
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.ink))
            .child(def.label.clone());
        let change_cb: OnChange = self.change_callback(&def.key, entity.clone());
        let toggle_color = {
            let entity = entity.clone();
            let key = def.key.clone();
            std::rc::Rc::new(move |_w: &mut Window, cx: &mut gpui::App| {
                entity.update(cx, |this, cx| {
                    this.color_open = if this.color_open_for(&key) { None } else { Some(key.clone()) };
                    cx.notify();
                });
            }) as crate::components::prop_controls::OnToggle
        };
        let toggle_select = {
            let entity = entity.clone();
            let key = def.key.clone();
            std::rc::Rc::new(move |_w: &mut Window, cx: &mut gpui::App| {
                entity.update(cx, |this, cx| {
                    let entry = this.select_open.entry(key.clone()).or_insert(false);
                    *entry = !*entry;
                    cx.notify();
                });
            }) as crate::components::prop_controls::OnToggle
        };
        let schema: Vec<canvas_proto::props::PropertyDef> = self.loaded_detail.as_ref().map(|d| d.schema.properties.clone()).unwrap_or_default();
        let values = self.values.clone();
        let control_el = control(
            t,
            &def.key,
            &def.label,
            &def.kind,
            value,
            &schema,
            &values,
            false,
            self.color_open_for(&def.key),
            self.select_open.get(&def.key).copied().unwrap_or(false),
            change_cb,
            toggle_color,
            toggle_select,
        );
        // React `.rd-eff-inspector .prop-item`: min-height 40, padding s1
        // (4 px) 0, and no divider under the last row (`.prop-item:last-child`).
        let border = if last {
            div()
        } else {
            div().border_b_1().border_color(Theme::color(t.line))
        };
        let _ = border;
        if narrow && kind_name != "number" {
            let mut row = div()
                .flex()
                .flex_col()
                .gap_2()
                .min_h(px(44.0))
                .py_1();
            if !last {
                row = row.border_b_1().border_color(Theme::color(t.line));
            }
            row.child(label_el).child(control_el)
        } else if kind_name == "number" {
            // React number: minmax(64,96) label, 1fr slider, 64 num, gap 12.
            // The short-window rule drops the inspector row minimum to 36.
            let mut row = div()
                .flex()
                .items_center()
                .gap_3()
                .min_h(px(if self.tight { 36.0 } else { 40.0 }))
                .py_1();
            if !last {
                row = row.border_b_1().border_color(Theme::color(t.line));
            }
            row.child(div().w(px(96.0)).flex_none().child(label_el))
                .child(div().flex().items_center().gap_3().flex_1().min_w(px(0.0)).child(control_el))
        } else if kind_name == "hue" || kind_name == "angle" {
            // React hue/angle: minmax(64,96) label, 1fr control, gap 16.
            // The short-window rule drops the inspector row minimum to 36.
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(16.0))
                .min_h(px(if self.tight { 36.0 } else { 40.0 }))
                .py_1();
            if !last {
                row = row.border_b_1().border_color(Theme::color(t.line));
            }
            row.child(div().w(px(96.0)).flex_none().child(label_el))
                .child(div().flex().items_center().gap_3().flex_1().min_w(px(0.0)).child(control_el))
        } else {
            // React `.prop-item`: minmax(120,170) label column, 16 px column
            // gap, and the control at the start of its column (`.prop-control`
            // has no justify-end: the colour swatch sat at the panel's right
            // edge here, 49 px right of React's).
            let mut row = div()
                .flex()
                .items_center()
                .gap(px(16.0))
                .min_h(px(if self.tight { 36.0 } else { 40.0 }))
                .py_1();
            if !last {
                row = row.border_b_1().border_color(Theme::color(t.line));
            }
            row.child(div().w(px(170.0)).flex_none().child(label_el))
                .child(div().flex().items_center().gap_2().flex_1().min_w(px(0.0)).child(control_el))
        }
    }

    fn color_open_for(&self, key: &str) -> bool {
        self.color_open.as_deref() == Some(key)
    }

    fn editor_pane(&self, win_h: f32, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &self.theme;
        let entity = cx.entity();
        let narrow = self.width <= 720.0;
        match &self.editor {
            EditorState::Open(target) => {
                // The editor itself (#214).
                if let Some(editor) = self.editor_view.clone() {
                    // The editor's own box: the content box's width (React's
                    // `@container page` reads it) and the height the side
                    // column centres its content in.
                    let editor_w = self.content_width();
                    let editor_h = if narrow {
                        win_h - 95.0
                    } else {
                        win_h - 143.0
                    };
                    return editor.update(cx, |e, cx| {
                        e.width = editor_w;
                        e.height = editor_h;
                        // The code starts open on desktop and closed on the
                        // phone (React's `useState(!narrow)`): until the
                        // Show/Hide key is pressed, a narrow editor renders
                        // with the code hidden.
                        if !e.code_touched {
                            e.show_code = editor_w > 720.0;
                        }
                        e.render(cx)
                    });
                }
                let mut col = div().flex().flex_col().gap_3().min_w(px(0.0));
                col = col.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_h(px(36.0))
                        .child(
                            div()
                                .text_size(px(22.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child("New from code".to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_sm()
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child(format!("{} · {}", target.name, target.file_name)),
                        ),
                );
                if !narrow {
                    let entity_cancel = entity.clone();
                    col = col.child(
                        div()
                            .flex()
                            .justify_end()
                            .child(button(t, "eff-editor-cancel", "Cancel", Variant::Ghost, false, move |_, _, cx| {
                                entity_cancel.update(cx, |this, _cx| this.close_editor());
                            })),
                    );
                }
                col.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .p_8()
                        .r_lg()
                        .border_1()
                        .border_color(Theme::color(t.line))
                        .bg(Theme::color(t.panel))
                        .child(
                            div()
                                .text_base()
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.dim))
                                .child("Not ported yet"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.mute))
                                .child("The effect editor is issue #214; this pane is its route target.".to_string()),
                        ),
                )
                .into_any_element()
            }
            EditorState::Error { message, .. } => {
                let entity_retry = entity.clone();
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .p_4()
                    .r_lg()
                    .border_1()
                    .border_color(Theme::color(t.line))
                    .bg(Theme::color(t.panel))
                    .child(div().mb_3().child("The code could not be read".to_string()))
                    .child(div().child("canvas-rgb could not read that effect's file. Nothing was changed.".to_string()))
                    .child(div().child(message.clone()))
                    .child(div().flex().mt_3().child(button(t, "eff-code-retry", "Try again", Variant::Default, false, move |_, _, cx| {
                        entity_retry.update(cx, |this, cx| this.retry_edit(cx));
                    })))
                    .into_any_element()
            }
            EditorState::Loading => div()
                .flex()
                .flex_col()
                .gap_3()
                .min_w(px(0.0))
                .child(div().w_full().h(px(220.0)).r_lg().bg(Theme::color(t.raised)))
                .child(div().w_full().h(px(120.0)).r_lg().bg(Theme::color(t.raised)))
                .into_any_element(),
            EditorState::Closed => {
                if self.new_open {
                    self.chooser(entity)
                } else {
                    div().into_any_element()
                }
            }
        }
    }

    fn chooser(&self, entity: Entity) -> gpui::AnyElement {
        let t = &self.theme;
        let narrow = self.width <= 720.0;
        let list: Vec<canvas_proto::api::EffectSummary> = self
            .effects
            .as_ref()
            .map(|e| e.effects.iter().filter(|x| x.id.starts_with("html:")).cloned().collect())
            .unwrap_or_default();
        let mut col = div().flex().flex_col().gap_3().min_w(px(0.0));
        let entity_cancel = entity.clone();
        let mut head = div().flex().items_center().gap_2().min_h(px(36.0));
        if narrow {
            let entity_back = entity.clone();
            head = head.child(icon_button(
                t,
                "eff-choose-back",
                "CaretLeft",
                "",
                false,
                false,
                false,
                move |_, _, cx| {
                    entity_back.update(cx, |this, _cx| {
                        this.new_open = false;
                        if this.width <= 720.0 {
                            this.pushed = false;
                        }
                    });
                },
            ));
        }
        head = head
            .child(
                div()
                    .text_size(px(22.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child("New from code".to_string()),
            )
            .child(div().flex_1());
        if !narrow {
            head = head.child(button(t, "eff-choose-cancel", "Cancel", Variant::Ghost, false, move |_, _, cx| {
                entity_cancel.update(cx, |this, _cx| {
                    this.new_open = false;
                });
            }));
        }
        col = col.child(head);
        // Copy an effect.
        let mut copy_body = div().flex().flex_col().gap_2();
        copy_body = copy_body.child(
            div()
                .text_sm()
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.dim))
                .child("A user or stock HTML effect. Its author and licence line stay at the top of the file.".to_string()),
        );
        if list.is_empty() {
            copy_body = copy_body.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child("There is no HTML effect to copy yet. Start blank instead.".to_string()),
            );
        } else {
            for effect in &list {
                let entity_copy = entity.clone();
                let copy_id = effect.id.clone();
                let busy = self.new_busy.is_some();
                copy_body = copy_body.child(
                    div()
                        .id(gpui::SharedString::from(format!("eff-choose-{}", effect.id)))
                        .flex()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .p(px(4.0))
                        .r_sm()
                        .bg(Theme::color(t.raised))
                        .child(thumb(t, &effect.id, 64.0, 40.0, cached(&effect.id), false))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .text_sm()
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.ink))
                                .child(effect.name.clone()),
                        )
                        .child(chip(t, "eff-choose-html", "HTML", ChipTone::Html, false, false, false))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_sm()
                                .font_family(crate::fonts::UI_FAMILY)
                                .text_color(Theme::color(t.dim))
                                .child(if self.new_busy.as_deref() == Some(effect.id.as_str()) {
                                    "Reading…".to_string()
                                } else {
                                    "Copy".to_string()
                                }),
                        )
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            if busy {
                                return;
                            }
                            entity_copy.update(cx, |this, cx| this.start_copy(&copy_id, cx));
                        }),
                );
            }
        }
        if let Some(err) = &self.new_error {
            copy_body = copy_body.child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.down))
                    .child(err.clone()),
            );
        }
        col = col.child(panel_with_padding(t, "eff-choose-copy", "Copy an effect", div(), copy_body, Some(12.0)));
        // Start blank.
        let entity_blank = entity.clone();
        let blank_body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.dim))
                    .child("The minimal file from the docs: a canvas, a colour and a draw loop that moves.".to_string()),
            )
            .child(div().flex().child(button_icon(
                t,
                "eff-blank",
                "Blank effect",
                Variant::Primary,
                true,
                self.new_busy.is_some(),
                "FilePlus",
                "",
                15.0,
                move |_, _, cx| {
                    entity_blank.update(cx, |this, _cx| this.start_blank());
                },
            )));
        col = col.child(panel_with_padding(t, "eff-choose-blank", "Start blank", div(), blank_body, Some(12.0)));
        col.into_any_element()
    }
}

/// The detail description cut to the width React's one-line ellipsis
/// leaves it: the inspector's inner width less the note ("Preview" /
/// "Live frame", 14 px) and the 12 px gap. The inspector width follows
/// `render`'s own formula (360 px, 400 px from a 1500 px container up,
/// full width on a phone).
fn ellipsized_desc(window: &Window, view: &EffectsView, description: &str, note_word: &str) -> String {
    let width = view.width;
    let phone = width <= 720.0;
    let content_w = width - if phone { 0.0 } else if width < 1280.0 { 64.0 } else { 200.0 } - if phone { 32.0 } else { 48.0 };
    let inspector_w = if content_w < 640.0 {
        content_w
    } else if content_w >= 1500.0 {
        400.0
    } else {
        360.0
    };
    // The panel's padding: s3 (24 px of padding plus the 2 px of border)
    // normally, s2 in the short-window density (18 px total).
    let inner_w = inspector_w - if view.tight { 18.0 } else { 26.0 };
    let note_w = text_width(window, note_word, 14.0, gpui::FontWeight::NORMAL, crate::fonts::UI_FAMILY);
    let max_w = (inner_w - note_w - 12.0).max(0.0);
    crate::shell::ellipsize(window, description, max_w, 15.0, gpui::FontWeight::NORMAL, crate::fonts::UI_FAMILY)
}

/// A card name with the matched letters marked (React `Highlighted`): the
/// mark is the live accent at 20%, so `live_soft` (16%) is the nearest
/// token value the theme parser serves.
fn marked_name(t: &Theme, name: &str, ranges: &[(usize, usize)]) -> impl IntoElement {
    let mut row = div()
        .flex()
        .items_center()
        .whitespace_nowrap()
        .overflow_hidden()
        // React `.effect-card-name` inherits the body 15 px at 1.45,
        // semibold; 14 px at the GPUI default 1.618 sat 1-4 px off.
        .text_size(px(15.0))
        .line_height(gpui::relative(1.45))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .font_family(crate::fonts::UI_FAMILY)
        .text_color(Theme::color(t.ink));
    if ranges.is_empty() {
        return row.child(name.to_string()).into_any_element();
    }
    let mut at = 0usize;
    for (start, end) in ranges {
        if *start > at {
            row = row.child(div().flex_none().child(name[at..*start].to_string()));
        }
        row = row.child(
            div()
                .flex_none()
                .r_sm()
                .bg(t.live_soft())
                .text_color(Theme::color(t.ink))
                .child(name[*start..*end].to_string()),
        );
        at = *end;
    }
    if at < name.len() {
        row = row.child(div().flex_none().child(name[at..].to_string()));
    }
    row.into_any_element()
}

impl Render for EffectsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.width = f32::from(window.window_bounds().get_bounds().size.width);
        // React `@container page (max-height: 700px)`: the page box is the
        // window less the 48 px strip and the content padding, so it hits
        // 700 at a 795 px window (measured: tight at 794, normal at 796).
        // The density touches only the inspector: panel padding s2 and the
        // property rows' 36 px minimum.
        let win_h = f32::from(window.window_bounds().get_bounds().size.height);
        self.tight = self.width > 720.0 && win_h - 95.0 <= 700.0;
        let t = self.theme;
        let phone = self.width <= 720.0;
        let content_w = self.width
            - if phone { 0.0 } else if self.width < 1280.0 { 64.0 } else { 200.0 }
            - if phone { 32.0 } else { 48.0 };
        let inspector_w = if content_w < 640.0 {
            content_w
        } else if content_w >= 1500.0 {
            400.0
        } else {
            360.0
        };
        let entity = cx.entity();
        // Build mode: the builder takes the whole page.
        if let Some(builder) = self.builder.clone() {
            // The builder's own box: the window minus the strip and the
            // content padding (React's `rd-page` is 805 px at 1440x900).
            let win_h = f32::from(window.window_bounds().get_bounds().size.height);
            return builder.update(cx, |b, cx| {
                b.width = self.width;
                b.height = if phone {
                    win_h - 184.0
                } else {
                    win_h - 95.0
                };
                b.render(cx)
            });
        }
        let editing = self.editing();
        // Edit mode: the gallery becomes a strip of tiles and the inspector
        // grows over it (React `.rd-eff-body[data-editing="true"]`).
        let (gallery_w, inspector_w) = if editing {
            (96.0, content_w - 96.0 - 16.0)
        } else {
            (content_w - inspector_w - 16.0, inspector_w)
        };
        let gallery = self.gallery(window, cx);
        let detail = self.detail_pane(window, cx);
        let toolbar = self.toolbar(cx);
        let stale_note = if self.list_stale() {
            self.stale_note().into_any_element()
        } else {
            div().into_any_element()
        };
        let import_note = self.import_note(cx).into_any_element();
        let body: gpui::AnyElement = if phone {
            if self.pushed {
                let entity_back = entity.clone();
                // There is a gap of one s3 under the back row (React's
                // `.eff-sheet { gap: var(--s3) }`): measured 108 to the
                // editor's top at 390.
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .flex_none()
                            .child(icon_button(
                                &t,
                                "eff-back",
                                "CaretLeft",
                                "",
                                false,
                                false,
                                false,
                                move |_, _, cx| {
                                    entity_back.update(cx, |this, _cx| this.close_sheet());
                                },
                            ))
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .font_family(crate::fonts::UI_FAMILY)
                                    .text_color(Theme::color(t.ink))
                                    .child("Effects".to_string()),
                            ),
                    )
                    .child(
                        div()
                            .id(gpui::SharedString::from("eff-sheet-scroll"))
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .child(detail),
                    )
                    .into_any_element()
            } else {
                div()
                    .id(gpui::SharedString::from("eff-gallery-scroll"))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .child(gallery)
                    .into_any_element()
            }
        } else {
            let gallery_col = div()
                .id(gpui::SharedString::from("eff-gallery-scroll"))
                .w(px(gallery_w))
                .flex_none()
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(gallery);
            let inspector_col = div()
                .id(gpui::SharedString::from("eff-inspector-scroll"))
                .w(px(inspector_w))
                .flex_none()
                .flex()
                .flex_col()
                .gap_3()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .overflow_y_scroll()
                .child(detail);
            div()
                .flex()
                .gap_4()
                .flex_1()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .child(gallery_col)
                .child(inspector_col)
                .into_any_element()
        };
        let mut page = div()
            .flex()
            .flex_col()
            .gap_4()
            .flex_1()
            .min_h(px(0.0))
            .min_w(px(0.0));
        // The phone editor takes the whole page (React routes to the
        // editor): the library toolbar and gallery leave with it.
        if !(phone && editing) {
            page = page.child(toolbar);
        }
        if self.replace_with.is_some() || self.import_error.is_some() {
            page = page.child(import_note);
        }
        if self.list_stale() {
            page = page.child(stale_note);
        }
        page = page.child(body);
        if let Some(toast) = self.toast.clone() {
            let entity_undo = entity.clone();
            page = page.child(
                div()
                    .absolute()
                    .bottom(px(16.0))
                    .left(px(16.0))
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_2()
                    .r_md()
                    .border_1()
                    .border_color(Theme::color(t.line_2))
                    .bg(Theme::color(t.panel))
                    .child(
                        div()
                            .text_sm()
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.ink))
                            .child(toast),
                    )
                    .child(button(&t, "eff-toast-undo", "Undo", Variant::Default, false, move |_, _, cx| {
                        entity_undo.update(cx, |this, cx| {
                            this.toast = None;
                            this.do_undo(cx);
                        });
                    })),
            );
        }
        page.into_any_element()
    }
}
