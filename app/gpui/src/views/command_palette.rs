//! The command palette (Ctrl+K), after `app/web/src/components/CommandPalette.tsx`
//! and the palette wiring in `app/web/src/shell/Shell.tsx` (`:134-232`), styled
//! from the palette block in `app/web/src/styles/motion.css` (`:888-1025`).
//!
//! The backdrop, panel, search field with magnifier, group headings, command
//! and status rows, hints, the empty line and the list max height of 46vh.
//! The list building, filtering and highlight movement are plain functions
//! ([`build_commands`], [`build_short`], [`filtered_commands`],
//! [`move_active`]) so the tests below never need a window.
//!
//! React quirks kept on purpose (decisions, not fixes):
//! - the short list shows at every width although its comments say it is for
//!   phones (`docs/gpui-parity/command-palette.md` Breakpoints);
//! - there are no combobox/listbox/option roles, no focus trap and no focus
//!   return; Enter runs the highlighted row, not the focused one;
//! - power and effect commands close the palette before their request
//!   finishes (the shell runs the action, then closes);
//! - status rows match the filter but the arrow keys skip them, so Enter can
//!   never run one;
//! - the panel has no enter animation in GPUI (React `canvas-enter` 180 ms /
//!   `sheet-up` on a phone): the overlay appears in the same frame either way,
//!   with no visible difference once it has settled.

use std::rc::Rc;

use canvas_proto::api::{EffectsResponse, LinkState, Status};
use gpui::{Context, Focusable, ScrollHandle, Window, div, prelude::*, px};

use crate::components::panel::tracked_text;
use crate::components::text_input::TextInput;
use crate::shell::{ellipsize, text_width};
use crate::theme::{Theme, TokenRadius};

/// What a command does when run. The shell executes it
/// (`run_palette_action`); status rows carry [`PaletteAction::None`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteAction {
    /// A status line: it says what is true and it is not an action.
    None,
    /// Open the status details popover (the shell's `details_open`).
    OpenDetails,
    /// Go to a page by its GPUI page name (`layout`, `effects`, ...).
    Navigate(String),
    /// Switch the theme (settings Appearance radio).
    Theme,
    /// Toggle interface motion (settings Appearance radio).
    Motion,
    /// Pause or resume the lights (existing output PATCH).
    Power,
    /// Run the effect with this id (existing activate POST).
    RunEffect(String),
}

/// One palette row, after React's `Command` (`CommandPalette.tsx:6-17`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteCommand {
    pub id: String,
    pub group: &'static str,
    pub label: String,
    pub hint: Option<String>,
    pub status_row: bool,
    pub action: PaletteAction,
}

/// The pages the palette offers, in order, with the hrefs React puts in the
/// hints (`Rail.tsx` `NAV`, `Shell.tsx:163-171`).
pub const PALETTE_PAGES: &[(&str, &str)] = &[
    ("Layout", "/"),
    ("Effects", "/effects/"),
    ("Devices", "/devices/"),
    ("Displays", "/displays/"),
    ("Peripherals", "/peripherals/"),
    ("Fans", "/fans/"),
    ("Settings", "/settings/"),
];

/// The group heading's font size (React's `.cmdk-group`). React asks for
/// `var(--t12)`, and `tokens.css` defines no `--t12`: the declaration is
/// invalid at computed-value time, so the heading inherits the body's
/// 15 px (`app.css:23`). Measured in the browser:
/// `getComputedStyle(.cmdk-group)` reads font-size 15px, line-height
/// 21.75px, letter-spacing 0.9px, weight 600, colour 133,139,149.
pub const GROUP_HEADING_FONT_PX: f32 = 15.0;

/// The search field's line height: React's `font: inherit` gives the native
/// input the body's 1.45, so its line box is 23.2 px at `--t16`.
pub const FIELD_LINE_HEIGHT: f32 = 1.45;

/// The field's vertical padding: the browser's own `padding: 1px 2px`
/// (`getComputedStyle(input).padding` = "1px 2px").
pub const FIELD_V_PAD_PX: f32 = 1.0;

/// The field's left padding: the same browser padding, 2 px, so the text
/// starts 2 px into the input (x 479 at 1440x900).
pub const FIELD_LEFT_PAD_PX: f32 = 2.0;

/// How far right gpui's mono hint text is moved to sit where Chrome's does.
pub const HINT_SHIFT_PX: f32 = 1.0;

/// The panel shadow's gpui blur radius (React: `0 18px 48px`, a gaussian of
/// sigma 24; gpui takes the sigma directly).
pub const PALETTE_SHADOW_BLUR_PX: f32 = 24.0;

/// The placeholder's colour: the browser's default `darkgray`, #757575,
/// the same in both themes (React's app CSS has no `::placeholder` rule).
pub const PLACEHOLDER_GREY: u32 = 0x757575;

fn link_state_word(state: LinkState) -> &'static str {
    match state {
        LinkState::Connected => "connected",
        LinkState::Connecting => "connecting",
        LinkState::Down => "down",
    }
}

/// The status lines' words (`lib/status.ts` `links`): the daemon is `online`,
/// `offline` or `connecting`; the two links carry the daemon's own words.
pub fn status_link_hints(status: Option<&Status>, offline: bool) -> Vec<(&'static str, String)> {
    vec![
        (
            "The daemon",
            if status.is_some() {
                "online"
            } else if offline {
                "offline"
            } else {
                "connecting"
            }
            .to_string(),
        ),
        (
            "OpenRGB",
            status
                .map(|s| link_state_word(s.openrgb).to_string())
                .unwrap_or_else(|| "connecting".to_string()),
        ),
        (
            "Fan service",
            status
                .map(|s| link_state_word(s.service).to_string())
                .unwrap_or_else(|| "connecting".to_string()),
        ),
    ]
}

/// The playing line as one sentence (`lib/status.ts` `playingLine`).
pub fn playing_line(status: Option<&Status>, effects: Option<&EffectsResponse>) -> String {
    let (name, detail, _) = crate::shell::playing(status, effects);
    if detail.is_empty() {
        name
    } else {
        format!("{name} {detail}")
    }
}

/// The full command list, in build order (`Shell.tsx:134-208`): Status (the
/// now-playing line, the three link lines, then the details action), Pages,
/// Appearance, Lights, Effects. `effects` of `None` adds no Run rows.
pub fn build_commands(
    status: Option<&Status>,
    offline: bool,
    effects: Option<&EffectsResponse>,
    theme_hint: &str,
    motion_hint: &str,
) -> Vec<PaletteCommand> {
    let mut list: Vec<PaletteCommand> = Vec::new();
    list.push(PaletteCommand {
        id: "status-now".to_string(),
        group: "Status",
        label: "Now playing".to_string(),
        hint: Some(playing_line(status, effects)),
        status_row: true,
        action: PaletteAction::None,
    });
    for (name, state) in status_link_hints(status, offline) {
        list.push(PaletteCommand {
            id: format!("status-link-{name}"),
            group: "Status",
            label: name.to_string(),
            hint: Some(state),
            status_row: true,
            action: PaletteAction::None,
        });
    }
    list.push(PaletteCommand {
        id: "status-details".to_string(),
        group: "Status",
        label: "Open the status details".to_string(),
        hint: None,
        status_row: false,
        action: PaletteAction::OpenDetails,
    });
    for (label, href) in PALETTE_PAGES {
        let page = match *href {
            "/" => "layout",
            "/effects/" => "effects",
            "/devices/" => "devices",
            "/displays/" => "displays",
            "/peripherals/" => "peripherals",
            "/fans/" => "fans",
            _ => "settings",
        };
        list.push(PaletteCommand {
            id: format!("page-{href}"),
            group: "Pages",
            label: format!("Go to {label}"),
            hint: Some(href.to_string()),
            status_row: false,
            action: PaletteAction::Navigate(page.to_string()),
        });
    }
    list.push(PaletteCommand {
        id: "theme".to_string(),
        group: "Appearance",
        label: "Switch the theme".to_string(),
        hint: Some(theme_hint.to_string()),
        status_row: false,
        action: PaletteAction::Theme,
    });
    list.push(PaletteCommand {
        id: "motion".to_string(),
        group: "Appearance",
        label: "Toggle interface motion".to_string(),
        hint: Some(motion_hint.to_string()),
        status_row: false,
        action: PaletteAction::Motion,
    });
    let paused = status.is_some_and(|s| s.power == canvas_proto::api::Power::Paused);
    list.push(PaletteCommand {
        id: "power".to_string(),
        group: "Lights",
        label: if paused {
            "Resume the lights".to_string()
        } else {
            "Pause the lights".to_string()
        },
        hint: None,
        status_row: false,
        action: PaletteAction::Power,
    });
    if let Some(effects) = effects {
        for e in &effects.effects {
            list.push(PaletteCommand {
                id: format!("run-{}", e.id),
                group: "Effects",
                label: format!("Run {}", e.name),
                hint: None,
                status_row: false,
                action: PaletteAction::RunEffect(e.id.clone()),
            });
        }
    }
    list
}

/// The short list for the empty query (`Shell.tsx:212-232`): the Status rows
/// first, then Pages and Lights, then the running effect and each favourite,
/// de-duplicated by id. Both Appearance rows stay out.
pub fn build_short(
    commands: &[PaletteCommand],
    effects: Option<&EffectsResponse>,
    status: Option<&Status>,
) -> Vec<PaletteCommand> {
    let mut list: Vec<PaletteCommand> = commands
        .iter()
        .filter(|c| c.group == "Pages" || c.group == "Lights")
        .cloned()
        .collect();
    if let Some(effects) = effects {
        let running = status
            .and_then(|s| s.active_effect.clone())
            .and_then(|id| effects.effects.iter().find(|e| e.id == id).cloned());
        let mut wanted: Vec<canvas_proto::api::EffectSummary> = Vec::new();
        if let Some(running) = running {
            wanted.push(running);
        }
        wanted.extend(effects.effects.iter().filter(|e| e.favorite).cloned());
        let mut seen = std::collections::HashSet::new();
        for e in wanted {
            if !seen.insert(e.id.clone()) {
                continue;
            }
            list.push(PaletteCommand {
                id: format!("run-{}", e.id),
                group: "Effects",
                label: format!("Run {}", e.name),
                hint: None,
                status_row: false,
                action: PaletteAction::RunEffect(e.id.clone()),
            });
        }
    }
    let mut status_rows: Vec<PaletteCommand> = commands
        .iter()
        .filter(|c| c.group == "Status")
        .cloned()
        .collect();
    status_rows.extend(list);
    status_rows
}

/// The rows the palette shows (`CommandPalette.tsx:39-44`): an empty (or
/// whitespace-only) query shows the short list; a typed query filters the
/// full list on `"<group> <label> <hint>"` containing the trimmed,
/// lower-cased query.
pub fn filtered_commands(
    commands: &[PaletteCommand],
    short: &[PaletteCommand],
    query: &str,
) -> Vec<PaletteCommand> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return short.to_vec();
    }
    commands
        .iter()
        .filter(|c| {
            format!("{} {} {}", c.group, c.label, c.hint.as_deref().unwrap_or(""))
                .to_lowercase()
                .contains(&q)
        })
        .cloned()
        .collect()
}

/// The rows that can be picked (`CommandPalette.tsx:47`): everything but the
/// status lines.
pub fn actionable_count(filtered: &[PaletteCommand]) -> usize {
    filtered.iter().filter(|c| !c.status_row).count()
}

/// Which action index a filtered row is (`CommandPalette.tsx:50-60`), or
/// `None` for a status row.
pub fn action_index_at(filtered: &[PaletteCommand], row: usize) -> Option<usize> {
    if row >= filtered.len() || filtered[row].status_row {
        return None;
    }
    Some(filtered[..row].iter().filter(|c| !c.status_row).count())
}

/// The highlight after an arrow key (`CommandPalette.tsx:75-84`): one step
/// over the action rows, wrapping; with no action rows the highlight is 0.
pub fn move_active(current: usize, action_count: usize, delta: i32) -> usize {
    if action_count == 0 {
        return 0;
    }
    let n = action_count as i32;
    (current as i32 + delta).rem_euclid(n) as usize
}

/// Group headings: the name of the group each run of rows starts with, drawn
/// once above its first row (`CommandPalette.tsx:105-112`).
pub fn group_headings(filtered: &[PaletteCommand]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for (i, c) in filtered.iter().enumerate() {
        let before = i.checked_sub(1).map(|j| &filtered[j]);
        if before.is_none_or(|b| b.group != c.group) {
            out.push((c.group.to_string(), i));
        }
    }
    out
}

/// The child index of a row in the list container, for scrolling the
/// highlight into view: group headings are children too, so each row's index
/// is its position plus the headings at or before it.
pub fn scroll_child_index(filtered: &[PaletteCommand], row: usize) -> usize {
    let headings_before = group_headings(filtered)
        .iter()
        .filter(|(_, first)| *first <= row)
        .count();
    row + headings_before
}

/// Ctrl+K / Meta+K (`Shell.tsx:97-107`): `e.key` "k" with ctrl or meta, case
/// insensitive, Shift unchecked (so Ctrl+Shift+K matches too).
pub fn is_palette_toggle_key(key: &str, control: bool, platform: bool) -> bool {
    (control || platform) && key.eq_ignore_ascii_case("k")
}

/// The search field's state: the query and the highlighted action row.
/// Every change to the query resets the highlight to row 0
/// (`CommandPalette.tsx:130-133`).
#[derive(Debug, Clone, Default)]
pub struct PaletteState {
    pub query: String,
    pub active: usize,
}

impl PaletteState {
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.active = 0;
    }
}

/// Told when a row runs (the shell executes the action).
pub type OnRun = Rc<dyn Fn(&PaletteAction, &mut Window, &mut gpui::App)>;
/// Told when the palette closes.
pub type OnClose = Rc<dyn Fn(&mut Window, &mut gpui::App)>;

/// The palette overlay: the backdrop, the panel and the search field. The
/// shell owns whether it is open and executes what runs.
pub struct CommandPaletteView {
    pub theme: Theme,
    pub input: gpui::Entity<TextInput>,
    pub state: PaletteState,
    /// The full and short lists, kept fresh by the shell each render.
    pub commands: Vec<PaletteCommand>,
    pub short: Vec<PaletteCommand>,
    scroll: ScrollHandle,
    /// The scroll key the list last scrolled to: `(active, filtered len,
    /// first row id)`.
    last_scroll_key: Option<(usize, usize, Option<String>)>,
    /// Focus the input the next time the palette renders (the capture flag's
    /// `--palette` opens it at start, with no window to schedule the 20 ms
    /// delay in).
    pub focus_once: bool,
    on_run: Option<OnRun>,
    on_close: Option<OnClose>,
}

impl CommandPaletteView {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextInput::new(
                "",
                false,
                0.0,
                0.0,
                0,
                theme,
                Rc::new(|_, _, _| {}),
                cx,
            );
            // React `.cmdk-input input`: bare, left-aligned, `inherit` at
            // `--t16`, no outline (the panel carries the focus state).
            input.set_text_mode(Some("Search pages, effects and actions"), cx);
            input.set_bare(cx);
            // The field's text is `--t16` (React's `font-size: var(--t16)`).
            input.set_font_size_px(16.0, cx);
            // React's field is a native input: `font: inherit` gives it the
            // body's 1.45 line height (a 23.2 px line box at `--t16`), and
            // the browser's own `padding: 1px 2px` adds 2 px, so the input
            // box is 25.1875 px and the row 50.1875 px (measured on the
            // React shot). Without the 2 px the row is 2 px short and every
            // row under it sits 2 px high.
            input.set_line_height(FIELD_LINE_HEIGHT, cx);
            input.set_vertical_pad(FIELD_V_PAD_PX, cx);
            // The same 2 px on the sides: React's text starts 2 px into the
            // input (x 479 on the 1440 shot), not at its edge.
            input.set_left_pad(FIELD_LEFT_PAD_PX, cx);
            // React's app CSS has no `::placeholder` rule, so the browser's
            // own default grey paints the placeholder: `darkgray`, #757575,
            // the same in both themes (measured on the React shots).
            input.set_placeholder_color(gpui::rgb(PLACEHOLDER_GREY), cx);
            input
        });
        CommandPaletteView {
            theme,
            input,
            state: PaletteState::default(),
            commands: Vec::new(),
            short: Vec::new(),
            scroll: ScrollHandle::new(),
            last_scroll_key: None,
            focus_once: false,
            on_run: None,
            on_close: None,
        }
    }

    /// Connect the search field's change feed and the shell's callbacks. The
    /// shell calls this once, after both entities exist.
    pub fn connect(&mut self, cx: &mut Context<Self>) {
        let entity = cx.entity();
        let on_change = Rc::new(move |text: &str, _window: &mut Window, cx: &mut gpui::App| {
            entity.update(cx, |this, cx| {
                this.state.set_query(text);
                cx.notify();
            });
        });
        self.input
            .update(cx, |input, _cx| input.set_on_text_change(on_change));
    }

    pub fn set_on_run(&mut self, on_run: OnRun) {
        self.on_run = Some(on_run);
    }

    pub fn set_on_close(&mut self, on_close: OnClose) {
        self.on_close = Some(on_close);
    }

    /// Keep the lists fresh (the shell calls this every render while open, so
    /// rows appear as the effects answer lands).
    pub fn set_data(&mut self, commands: Vec<PaletteCommand>, short: Vec<PaletteCommand>) {
        self.commands = commands;
        self.short = short;
    }

    /// Open with `initial_query` (empty on a normal open; `--palette-query`
    /// for captures), focus the input after React's 20 ms, and reset the
    /// highlight to row 0 (`CommandPalette.tsx:63-66`).
    pub fn open(&mut self, initial_query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.state.set_query(initial_query);
        self.last_scroll_key = None;
        self.input
            .update(cx, |input, cx| input.set_value(initial_query, cx));
        let input = self.input.clone();
        cx.spawn_in(window, async move |_view, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(20))
                .await;
            let _ = cx.update(|window, cx| {
                let handle = input.read(cx).focus_handle(cx);
                handle.focus(window);
            });
        })
        .detach();
        cx.notify();
    }

    /// Open with `query` typed and focus the input on the first frame (the
    /// capture flag `--palette-query`; no window exists yet to schedule
    /// React's 20 ms focus delay).
    pub fn open_initial(&mut self, query: &str, cx: &mut Context<Self>) {
        self.state.set_query(query);
        self.last_scroll_key = None;
        self.input
            .update(cx, |input, cx| input.set_value(query, cx));
        self.focus_once = true;
        cx.notify();
    }

    fn filtered(&self) -> Vec<PaletteCommand> {
        filtered_commands(&self.commands, &self.short, &self.state.query)
    }

    fn close(&self, window: &mut Window, cx: &mut gpui::App) {
        if let Some(on_close) = &self.on_close {
            on_close(window, cx);
        }
    }

    fn run(&self, action: &PaletteAction, window: &mut Window, cx: &mut gpui::App) {
        // React runs the row, then closes (`CommandPalette.tsx:156-159`).
        if let Some(on_run) = &self.on_run {
            on_run(action, window, cx);
        }
        self.close(window, cx);
    }

    fn on_palette_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let filtered = self.filtered();
        let count = actionable_count(&filtered);
        match key {
            "escape" => self.close(window, cx),
            "down" => {
                self.state.active = move_active(self.state.active, count, 1);
                cx.notify();
            }
            "up" => {
                self.state.active = move_active(self.state.active, count, -1);
                cx.notify();
            }
            "enter" => {
                let cmd = filtered
                    .iter()
                    .filter(|c| !c.status_row)
                    .nth(self.state.active);
                if let Some(cmd) = cmd {
                    let action = cmd.action.clone();
                    self.run(&action, window, cx);
                }
            }
            _ => {}
        }
    }

    fn on_backdrop_mouse_down(&mut self, _event: &gpui::MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    fn row(
        &self,
        window: &mut Window,
        cmd: &PaletteCommand,
        _row_index: usize,
        action_index: Option<usize>,
        panel_w: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let t = &self.theme;
        let active = action_index.is_some_and(|a| a == self.state.active);
        let hint_w = match &cmd.hint {
            Some(hint) => text_width(window, hint, 14.0, gpui::FontWeight::NORMAL, crate::fonts::MONO_FAMILY),
            None => 0.0,
        };
        // The label's budget: the panel minus its 1 px border (2), the list
        // padding (s1, 4 each side), the row padding (s3, 12 each side), the
        // gap (s3) and the hint. React truncates with an ellipsis on one
        // line (`.cmdk-item-label`, `motion.css:991-997`).
        let budget = panel_w - 2.0 - 8.0 - 24.0 - if cmd.hint.is_some() { 12.0 } else { 0.0 } - hint_w;
        let label = ellipsize(
            window,
            &cmd.label,
            budget,
            15.0,
            gpui::FontWeight::NORMAL,
            crate::fonts::UI_FAMILY,
        );
        let mut row = div()
            .id(gpui::SharedString::from(format!("palette-row-{}", cmd.id)))
            .flex()
            .items_center()
            .gap(px(12.0))
            .w_full()
            .py(px(8.0))
            .px(px(12.0))
            .r_sm()
            .text_size(px(15.0))
            .line_height(gpui::relative(1.45))
            .font_family(crate::fonts::UI_FAMILY)
            .text_color(Theme::color(t.dim))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(label),
            );
        if let Some(hint) = &cmd.hint {
            row = row.child(
                div()
                    .flex_none()
                    // Measured on the 1440x900 shots: gpui paints the mono
                    // hint 0.6 px left of Chrome's (centroids of seven hints
                    // sit -0.43 to -0.71 px, mean -0.6; labels sit +0.1 px),
                    // so the box moves right by that.
                    .mr(px(-HINT_SHIFT_PX))
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::MONO_FAMILY)
                    .text_color(Theme::color(if cmd.status_row { t.ink } else { t.mute }))
                    .child(hint.clone()),
            );
        }
        if cmd.status_row {
            // A status line: dim, no hover background, never highlighted, no
            // click handler (`motion.css:948-960`).
            return row.into_any_element();
        }
        if active {
            row = row.bg(t.live_soft()).text_color(Theme::color(t.ink));
        }
        let action = cmd.action.clone();
        let action_index = action_index.unwrap_or(0);
        row.cursor_pointer()
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                // Hover moves the highlight (`CommandPalette.tsx:155`).
                if *hovered {
                    this.state.active = action_index;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                // Row click runs the row and then closes
                // (`CommandPalette.tsx:156-159`).
                this.run(&action, window, cx);
            }))
            .into_any_element()
    }
}

impl Render for CommandPaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        if self.focus_once {
            self.focus_once = false;
            let handle = self.input.read(cx).focus_handle(cx);
            handle.focus(window);
        }
        let bounds = window.window_bounds().get_bounds();
        let win_w: f32 = bounds.size.width.into();
        let win_h: f32 = bounds.size.height.into();
        let phone = win_w <= 720.0;
        let panel_w = if phone { win_w } else { 560.0_f32.min(win_w - 32.0) };

        // The search field row (`.cmdk-input`): magnifier, gap s2, padding s3,
        // border-bottom `line`.
        let input_row = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .p(px(12.0))
            .border_b_1()
            .border_color(Theme::color(t.line))
            .child(
                gpui::svg()
                    .path("icons/MagnifyingGlass.svg")
                    .size(px(16.0))
                    .flex_none()
                    // React's Phosphor icon inherits the body's colour: the
                    // `.cmdk-input` row sets none, so the icon is `--ink`.
                    .text_color(Theme::color(t.ink)),
            )
            .child(self.input.clone());

        let filtered = self.filtered();
        // Keep the highlighted row in view (`scrollIntoView`, block nearest).
        let scroll_key = (
            self.state.active,
            filtered.len(),
            filtered.first().map(|c| c.id.clone()),
        );
        let scroll_stale = self.last_scroll_key != Some(scroll_key.clone());
        if scroll_stale {
            self.last_scroll_key = Some(scroll_key);
            // The row whose action index is the active one.
            let mut row = None;
            for i in 0..filtered.len() {
                if action_index_at(&filtered, i) == Some(self.state.active) {
                    row = Some(i);
                    break;
                }
            }
            if let Some(row) = row {
                self.scroll.scroll_to_item(scroll_child_index(&filtered, row));
            }
        }

        let headings = group_headings(&filtered);
        let mut list = div()
            .id("palette-list")
            .flex()
            .flex_col()
            .max_h(px(win_h * 0.46))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p(px(4.0));
        if filtered.is_empty() {
            // The empty line (`.cmdk-empty`): padding s4, mute, t14, centred.
            list = list.child(
                div()
                    .p(px(16.0))
                    .text_size(px(14.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.mute))
                    .text_center()
                    .child("Nothing matches that."),
            );
        }
        for (i, cmd) in filtered.iter().enumerate() {
            if let Some((name, _)) = headings.iter().find(|(_, first)| *first == i) {
                list = list.child(
                    div()
                        .pt(px(12.0))
                        .px(px(12.0))
                        .pb(px(4.0))
                        // React's `.cmdk-group` asks for `var(--t12)`, and
                        // `tokens.css` has no `--t12`: the declaration is
                        // invalid at computed-value time, so the heading
                        // inherits the body's 15 px (`app.css:23`). Measured
                        // on the React shot: the heading's line box is
                        // 21.75 px (15 x 1.45), not the 17.4 px a 12 px font
                        // would give, which put every row under it 4.35 px
                        // high.
                        .child(tracked_text(
                            &name.to_uppercase(),
                            GROUP_HEADING_FONT_PX,
                            0.06,
                            gpui::FontWeight::SEMIBOLD,
                            crate::fonts::UI_FAMILY,
                            Theme::color(t.mute),
                        )),
                );
            }
            list = list.child(self.row(window, cmd, i, action_index_at(&filtered, i), panel_w, cx));
        }

        let mut panel = div()
            .flex()
            .flex_col()
            .w(px(panel_w))
            .overflow_hidden()
            .r_lg()
            .border_1()
            .border_color(Theme::color(t.line_2))
            .bg(Theme::color(t.panel))
            .shadow(vec![gpui::BoxShadow {
                color: gpui::Hsla::from(gpui::Rgba {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 0.4,
                }),
                offset: gpui::point(px(0.0), px(18.0)),
                // CSS `blur 48px` is a gaussian with sigma 24; gpui's
                // `blur_radius` is the sigma itself (measured: 48 here
                // spread the shadow to sigma ~47 and darkened the scrim
                // 60 px out), so 24 draws React's falloff.
                blur_radius: px(PALETTE_SHADOW_BLUR_PX),
                spread_radius: px(0.0),
            }])
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(input_row)
            .child(list);
        if phone {
            // The phone sheet: width 100%, max height 86vh, top radius only
            // (`motion.css:1014-1025`): the bottom corners are square.
            panel = panel.max_h(px(win_h * 0.86));
            let top = gpui::AbsoluteLength::Pixels(px(16.0));
            let none = gpui::AbsoluteLength::Pixels(px(0.0));
            panel.style().corner_radii.top_left = Some(top);
            panel.style().corner_radii.top_right = Some(top);
            panel.style().corner_radii.bottom_left = Some(none);
            panel.style().corner_radii.bottom_right = Some(none);
        }

        let mut backdrop = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .bg(Theme::color(t.scrim))
            .capture_key_down(cx.listener(Self::on_palette_key))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(Self::on_backdrop_mouse_down),
            )
            .child(panel);
        if phone {
            backdrop = backdrop.justify_end();
        } else {
            backdrop = backdrop.pt(px(win_h * 0.12));
        }
        backdrop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_proto::api::{EffectSummary, Power};

    fn effect(id: &str, name: &str, favorite: bool) -> EffectSummary {
        EffectSummary {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            needs: Default::default(),
            favorite,
        }
    }

    fn effects(list: Vec<EffectSummary>) -> EffectsResponse {
        EffectsResponse {
            effects: list,
            active: None,
            failed: Vec::new(),
        }
    }

    fn status(power: Power, active_effect: Option<&str>) -> Status {
        Status {
            version: "0.8.4".into(),
            power,
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
            active_effect: active_effect.map(str::to_string),
            openrgb: LinkState::Connected,
            service: LinkState::Connected,
            service_down_ms: None,
            conflicts: Vec::new(),
            show_zone_panel: false,
            onboarding_complete: true,
        }
    }

    #[test]
    fn empty_query_shows_the_short_list_and_whitespace_query_too() {
        let commands = build_commands(None, false, None, "dark", "full");
        let short = build_short(&commands, None, None);
        assert_eq!(filtered_commands(&commands, &short, ""), short);
        assert_eq!(filtered_commands(&commands, &short, "   "), short);
    }

    #[test]
    fn a_typed_query_searches_the_full_list() {
        let commands = build_commands(None, false, None, "dark", "full");
        let short = build_short(&commands, None, None);
        let hits = filtered_commands(&commands, &short, "motion");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, "Toggle interface motion");
    }

    #[test]
    fn the_filter_matches_status_rows_but_they_stay_unpickable() {
        let commands = build_commands(None, false, None, "dark", "full");
        let short = build_short(&commands, None, None);
        let hits = filtered_commands(&commands, &short, "daemon");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, "The daemon");
        assert!(hits[0].status_row);
        assert_eq!(actionable_count(&hits), 0);
        assert_eq!(action_index_at(&hits, 0), None);
    }

    #[test]
    fn the_filter_covers_group_label_and_hint() {
        let commands = build_commands(None, false, None, "dark", "full");
        let short = build_short(&commands, None, None);
        // The hint: every page row's href is searchable.
        let hits = filtered_commands(&commands, &short, "/fans/");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].label, "Go to Fans");
        // The group: "appearance" finds both appearance rows.
        let hits = filtered_commands(&commands, &short, "appearance");
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn short_list_order_is_status_pages_lights_effects_without_appearance() {
        let effs = effects(vec![
            effect("fav", "Favourite", true),
            effect("plain", "Plain", false),
            effect("run", "Running", false),
        ]);
        let st = status(Power::Running, Some("run"));
        let commands = build_commands(Some(&st), false, Some(&effs), "dark", "full");
        let short = build_short(&commands, Some(&effs), Some(&st));
        let groups: Vec<&str> = short.iter().map(|c| c.group).collect();
        assert_eq!(
            groups,
            [
                "Status", "Status", "Status", "Status", "Status", // now + 3 links + details = 5
                "Pages", "Pages", "Pages", "Pages", "Pages", "Pages", "Pages", "Lights", "Effects",
                "Effects"
            ]
        );
        assert!(!groups.contains(&"Appearance"));
        // The running effect first, then the favourite; "plain" is full-list only.
        let fx: Vec<&str> = short
            .iter()
            .filter(|c| c.group == "Effects")
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(fx, ["Run Running", "Run Favourite"]);
    }

    #[test]
    fn short_list_dedups_a_running_favourite() {
        let effs = effects(vec![effect("both", "Both", true)]);
        let st = status(Power::Running, Some("both"));
        let commands = build_commands(Some(&st), false, Some(&effs), "dark", "full");
        let short = build_short(&commands, Some(&effs), Some(&st));
        let runs = short.iter().filter(|c| c.group == "Effects").count();
        assert_eq!(runs, 1);
    }

    #[test]
    fn short_list_without_effects_has_no_run_rows() {
        let commands = build_commands(None, false, None, "dark", "full");
        let short = build_short(&commands, None, None);
        assert!(short.iter().all(|c| c.group != "Effects"));
        // And without effects the full list has no Effects group either.
        assert!(commands.iter().all(|c| c.group != "Effects"));
    }

    #[test]
    fn full_list_build_order_is_status_pages_appearance_lights_effects() {
        let effs = effects(vec![effect("a", "Alpha", false)]);
        let st = status(Power::Running, None);
        let commands = build_commands(Some(&st), false, Some(&effs), "dark", "full");
        let mut order: Vec<&str> = Vec::new();
        for c in &commands {
            if order.last() != Some(&c.group) {
                order.push(c.group);
            }
        }
        assert_eq!(order, ["Status", "Pages", "Appearance", "Lights", "Effects"]);
    }

    #[test]
    fn group_heading_appears_once_per_group_run() {
        let commands = build_commands(None, false, None, "dark", "full");
        let headings = group_headings(&commands);
        assert_eq!(
            headings,
            [
                ("Status".to_string(), 0),
                ("Pages".to_string(), 5),
                ("Appearance".to_string(), 12),
                ("Lights".to_string(), 14),
            ]
            .into_iter()
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn arrows_wrap_over_action_rows_only() {
        // 3 action rows, wrapping both ways.
        assert_eq!(move_active(2, 3, 1), 0);
        assert_eq!(move_active(0, 3, -1), 2);
        assert_eq!(move_active(0, 3, 1), 1);
        // With no action rows the highlight is 0 and Enter does nothing.
        assert_eq!(move_active(0, 0, 1), 0);
        assert_eq!(move_active(5, 0, -1), 0);
    }

    #[test]
    fn status_rows_are_skipped_by_the_action_index() {
        // Status, status, action, status, action: rows 2 and 4 are actions 0 and 1.
        let commands = vec![
            PaletteCommand { id: "a".into(), group: "Status", label: "A".into(), hint: None, status_row: true, action: PaletteAction::None },
            PaletteCommand { id: "b".into(), group: "Status", label: "B".into(), hint: None, status_row: true, action: PaletteAction::None },
            PaletteCommand { id: "c".into(), group: "Pages", label: "C".into(), hint: None, status_row: false, action: PaletteAction::Theme },
            PaletteCommand { id: "d".into(), group: "Status", label: "D".into(), hint: None, status_row: true, action: PaletteAction::None },
            PaletteCommand { id: "e".into(), group: "Lights", label: "E".into(), hint: None, status_row: false, action: PaletteAction::Power },
        ];
        assert_eq!(action_index_at(&commands, 0), None);
        assert_eq!(action_index_at(&commands, 2), Some(0));
        assert_eq!(action_index_at(&commands, 3), None);
        assert_eq!(action_index_at(&commands, 4), Some(1));
        assert_eq!(actionable_count(&commands), 2);
        // The scroll child of row 4: 4 rows + 3 headings (Status, Pages, Lights... here
        // Status, Pages, Status, Lights = 4 headings at rows 0,2,3,4).
        assert_eq!(scroll_child_index(&commands, 4), 8);
    }

    #[test]
    fn a_query_change_resets_the_highlight() {
        let mut state = PaletteState { query: String::new(), active: 3 };
        state.set_query("fan");
        assert_eq!(state.query, "fan");
        assert_eq!(state.active, 0);
    }

    #[test]
    fn status_hints_cover_the_null_offline_and_online_states() {
        // Status not yet answered, no error: links say "connecting".
        let hints = status_link_hints(None, false);
        assert_eq!(hints[0].1, "connecting");
        assert_eq!(hints[1].1, "connecting");
        assert_eq!(hints[2].1, "connecting");
        assert_eq!(playing_line(None, None), "—");
        // Daemon down.
        let hints = status_link_hints(None, true);
        assert_eq!(hints[0].1, "offline");
        // Online, with the links connected.
        let st = status(Power::Running, None);
        let hints = status_link_hints(Some(&st), false);
        assert_eq!(hints[0].1, "online");
        assert_eq!(hints[1].1, "connected");
        assert_eq!(hints[2].1, "connected");
    }

    #[test]
    fn the_power_row_reads_resume_only_when_paused() {
        let running = status(Power::Running, None);
        let paused = status(Power::Paused, None);
        let label = |s: Option<&Status>| {
            build_commands(s, false, None, "dark", "full")
                .iter()
                .find(|c| c.id == "power")
                .map(|c| c.label.clone())
                .unwrap_or_default()
        };
        assert_eq!(label(Some(&running)), "Pause the lights");
        assert_eq!(label(Some(&paused)), "Resume the lights");
        assert_eq!(label(None), "Pause the lights");
    }

    #[test]
    fn ctrl_k_toggles_with_ctrl_or_meta_and_shift_unchecked() {
        assert!(is_palette_toggle_key("k", true, false));
        assert!(is_palette_toggle_key("k", false, true));
        assert!(is_palette_toggle_key("K", true, true));
        assert!(is_palette_toggle_key("k", true, false));
        assert!(!is_palette_toggle_key("k", false, false));
        assert!(!is_palette_toggle_key("j", true, false));
    }

    /// The palette's own metrics are the ones the browser reports for React's
    /// (measured with `.build/probe-palette.mjs`, `getComputedStyle`):
    /// the heading inherits the body's 15 px (React asks for `var(--t12)`,
    /// which `tokens.css` does not define), the native input keeps the
    /// browser's 1 px vertical and 2 px horizontal padding, and the
    /// placeholder wears the browser's default grey.
    #[test]
    fn the_palette_metrics_are_the_ones_the_browser_reports_for_react() {
        // The heading: 15 px at the body's 1.45 is a 21.75 px line box.
        assert_eq!(GROUP_HEADING_FONT_PX, 15.0);
        assert!((GROUP_HEADING_FONT_PX * 1.45 - 21.75).abs() < 0.001);
        // The field: 16 px at 1.45 is a 23.2 px line box; the browser's own
        // 1 px above and below make the input 25.1875 px tall.
        assert_eq!(FIELD_LINE_HEIGHT, 1.45);
        let field_h = 16.0 * FIELD_LINE_HEIGHT + 2.0 * FIELD_V_PAD_PX;
        assert!((field_h - 25.1875).abs() < 0.05, "the field is {field_h} px");
        assert_eq!(FIELD_LEFT_PAD_PX, 2.0);
        assert_eq!(PLACEHOLDER_GREY, 0x757575);
    }
}
