//! A text input, after gpui 0.2.2's own `examples/input.rs`.
//!
//! React's `NumberField` (`app/web/src/components/NumberField.tsx`) is a
//! plain text input: the reader types a number, the value is clamped to the
//! field's range when it is committed (Enter or blur), and the arrow keys
//! step it. GPUI 0.2.2 has no text input of its own, so this is one: an
//! entity that owns the draft, draws it with a caret and a selection, takes
//! the keys, and hands the committed number back.
//!
//! The rules that do not need a window are free functions, so they can be
//! tested: [`filter_numeric`] keeps only the characters a number may hold and
//! [`clamp_value`] is the clamp a commit applies.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Rgba, ShapedLine,
    SharedString, Style, Subscription, UTF16Selection, Window, div, fill, point, px, prelude::*,
    relative, rgba, size,
};

use crate::theme::{Theme, TokenRadius};

/// Keep only the characters a number may hold: the digits, one dot, and a
/// minus at the front. Everything else is dropped, as a number field that
/// only takes numbers does.
pub fn filter_numeric(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut dot = false;
    for (i, ch) in text.chars().enumerate() {
        if ch.is_ascii_digit() {
            out.push(ch);
        } else if ch == '.' && !dot {
            dot = true;
            out.push(ch);
        } else if ch == '-' && i == 0 && out.is_empty() {
            out.push(ch);
        }
    }
    out
}

/// The clamp a commit applies: the value may not leave the field's range.
/// A value that is not a number at all is the range's floor.
pub fn clamp_value(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() {
        return min;
    }
    value.clamp(min, max)
}

/// The text a value is shown as: whole numbers without a dot, and the rest
/// with the field's own number of decimals.
pub fn format_value(value: f32, decimals: usize) -> String {
    if decimals == 0 {
        format!("{}", value.round() as i64)
    } else {
        format!("{:.*}", decimals, value)
    }
}

/// Told the committed number.
pub type OnCommit = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// Told the field's text whenever it changes (the search box filters as the
/// reader types; a number field leaves this out).
pub type OnTextChange = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// Told on Enter in text mode (the preset name field submits).
pub type OnSubmit = Rc<dyn Fn(&mut Window, &mut App)>;

/// A text input: the field's own entity.
///
/// The field owns the draft (what is typed) and the value (what the model
/// says). It commits the draft on Enter and when it loses the focus, and it
/// tells its owner the committed number.
#[allow(clippy::too_many_arguments)]
pub struct TextInput {
    focus_handle: FocusHandle,
    /// What is typed and not yet committed.
    content: SharedString,
    /// What the model says; the field shows this when nothing is typed.
    value: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    /// A number field keeps only the characters a number may hold.
    numeric: bool,
    /// A disabled field shows its value and takes nothing.
    disabled: bool,
    min: f32,
    max: f32,
    decimals: usize,
    theme: Theme,
    /// Told the committed number.
    on_commit: OnCommit,
    /// Told the text on every change (the search box); None for number fields.
    on_text_change: Option<OnTextChange>,
    /// Told on Enter in text mode (the preset name submits); None for numbers.
    on_submit: Option<OnSubmit>,
    /// Told on Escape in text mode (the preset name cancels).
    on_escape: Option<OnSubmit>,
    /// Left-aligned in the UI face (the search box); number fields keep the
    /// mono, right-aligned form React's `.num` draws.
    ui_left: bool,
    /// Bare (the command palette's search field): transparent, no border, no
    /// fixed height, like React's `.cmdk-input input`.
    bare: bool,
    /// The text size in px (15 for number fields; 16 for the palette).
    font_px: f32,
    /// The field's own line height (CSS `line-height`, a number), when it is
    /// set. `None` leaves GPUI's default (1.618); React's body is 1.45 and
    /// the palette's search field asks for it, so its text sits in a 23.2 px
    /// line box instead of a 25.9 px one.
    line_height: Option<f32>,
    /// The field's vertical padding (CSS `padding-top`/`padding-bottom`).
    /// React's native input keeps the browser's own 1 px, so the palette's
    /// field is 2 px taller than its line box.
    v_pad: f32,
    /// The search box wears React's small radius (`.effect-search input`
    /// `r-sm`); a number field wears the input's `r-md`.
    radius_sm: bool,
    /// The effects search's border is `--line-2` (motion.css
    /// `.effect-search input`); a number or name field's is `--line`
    /// (hardware.css's wells rule for `.num`/`.text-in`).
    border_2: bool,
    /// The field's left inset: 8 px everywhere except the effects search,
    /// whose icon sits inside it (React's `padding-left: 30px`).
    left_pad: f32,
    /// The field's right inset: 8 px everywhere except React's `.text-in`
    /// (a name field), whose padding is `var(--s3)` (12 px) on each side.
    right_pad: f32,
    /// Shown dimmed while nothing is typed (the search placeholder).
    placeholder: Option<SharedString>,
    /// The placeholder's colour. `None` uses the theme's mute.
    placeholder_color: Option<Rgba>,
    /// Keeps the blur listener alive.
    blur: Option<Subscription>,
}

impl TextInput {
    /// A field that shows `value` and commits a number in `min..=max`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        value: &str,
        numeric: bool,
        min: f32,
        max: f32,
        decimals: usize,
        theme: Theme,
        on_commit: OnCommit,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: value.to_string().into(),
            value: value.to_string().into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            numeric,
            disabled: false,
            min,
            max,
            decimals,
            theme,
            on_commit,
            on_text_change: None,
            on_submit: None,
            on_escape: None,
            ui_left: false,
            bare: false,
            font_px: 15.0,
            line_height: None,
            v_pad: 0.0,
            radius_sm: false,
            border_2: false,
            left_pad: 8.0,
            right_pad: 8.0,
            placeholder: None,
            placeholder_color: None,
            blur: None,
        }
    }

    /// The field is a plain text field (the search box, the preset's name):
    /// left-aligned in the UI face, with a dimmed placeholder while empty.
    pub fn set_text_mode(&mut self, placeholder: Option<&'static str>, cx: &mut Context<Self>) {
        self.ui_left = true;
        self.radius_sm = true;
        self.placeholder = placeholder.map(SharedString::from);
        cx.notify();
    }

    /// The effects-search border (`--line-2`).
    pub fn set_border_2(&mut self, cx: &mut Context<Self>) {
        self.border_2 = true;
        cx.notify();
    }

    /// The left inset (the effects search's icon sits inside the field at
    /// 8 px, so its text starts 30 px in, as React's `padding-left` does).
    pub fn set_left_pad(&mut self, pad: f32, cx: &mut Context<Self>) {
        self.left_pad = pad;
        cx.notify();
    }

    /// The field's right inset (React's `.text-in` has 12 px on each side).
    pub fn set_right_pad(&mut self, pad: f32, cx: &mut Context<Self>) {
        self.right_pad = pad;
        cx.notify();
    }

    /// The input's own radius (React's `.text-in` wears `r-md`; the search
    /// box wears the small one).
    pub fn set_radius_md(&mut self, cx: &mut Context<Self>) {
        self.radius_sm = false;
        cx.notify();
    }

    /// Bare (the command palette's search field): transparent, no border, no
    /// fixed height, like React's `.cmdk-input input`.
    pub fn set_bare(&mut self, cx: &mut Context<Self>) {
        self.bare = true;
        cx.notify();
    }

    /// The text size in px (the palette's field is `--t16`, 16 px).
    pub fn set_font_size_px(&mut self, px_value: f32, cx: &mut Context<Self>) {
        self.font_px = px_value;
        cx.notify();
    }

    /// The field's own line height (CSS `line-height`, a number). React's
    /// body is 1.45; GPUI lays text out at 1.618 unless it is told.
    pub fn set_line_height(&mut self, line_height: f32, cx: &mut Context<Self>) {
        self.line_height = Some(line_height);
        cx.notify();
    }

    /// The field's vertical padding (CSS `padding-top`/`padding-bottom`).
    /// React's native input keeps the browser's own 1 px.
    pub fn set_vertical_pad(&mut self, pad: f32, cx: &mut Context<Self>) {
        self.v_pad = pad;
        cx.notify();
    }

    /// The placeholder's colour. React's app CSS has no `::placeholder`
    /// rule, so every placeholder in the React editor wears the browser's
    /// own default grey (`darkgray`, `#757575`).
    pub fn set_placeholder_color(&mut self, color: Rgba, cx: &mut Context<Self>) {
        self.placeholder_color = Some(color);
        cx.notify();
    }

    /// Told the text whenever it changes.
    pub fn set_on_text_change(&mut self, on: OnTextChange) {
        self.on_text_change = Some(on);
    }

    /// Told on Enter (the preset's name submits) and on Escape (cancels).
    pub fn set_on_submit_escape(&mut self, submit: OnSubmit, escape: OnSubmit) {
        self.on_submit = Some(submit);
        self.on_escape = Some(escape);
    }

    /// The field is disabled (React's `disabled`): it shows its value and
    /// takes nothing.
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled == disabled {
            return;
        }
        self.disabled = disabled;
        cx.notify();
    }

    /// The model's value changed: the field shows it again and drops the
    /// draft.
    pub fn set_value(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.value.as_ref() == value {
            return;
        }
        self.value = value.to_string().into();
        self.content = value.to_string().into();
        self.selected_range = 0..0;
        self.marked_range = None;
        cx.notify();
    }

    /// Listen for the focus leaving the field, so the draft is committed then
    /// too (React commits on blur).
    pub fn watch_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blur.is_some() {
            return;
        }
        let handle = self.focus_handle.clone();
        self.blur = Some(cx.on_blur(&handle, window, |input, window, cx| {
            input.commit(window, cx);
        }));
    }

    /// Commit the draft: the number in it, clamped to the field's range. In
    /// text mode Enter hands the draft to the submit callback (the name
    /// field) instead of parsing a number.
    pub fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.numeric {
            if let Some(submit) = self.on_submit.clone() {
                submit(window, cx);
            }
            return;
        }
        let draft = self.content.to_string();
        let parsed = draft.trim().parse::<f32>().ok();
        let Some(parsed) = parsed else {
            // React drops a draft that is not a number and shows the value again.
            let value = self.value.clone();
            self.content = value.clone();
            self.selected_range = 0..value.len();
            cx.notify();
            return;
        };
        let clamped = clamp_value(parsed, self.min, self.max);
        let shown = format_value(clamped, self.decimals);
        self.content = shown.clone().into();
        self.value = shown.into();
        self.selected_range = 0..self.content.len();
        (self.on_commit)(clamped, window, cx);
        cx.notify();
    }

    /// The caret's place in the text.
    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    /// The place just before the caret, on a character boundary.
    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }

    /// The place just after the caret, on a character boundary.
    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .find_map(|(i, _)| (i > offset).then_some(i))
            .unwrap_or(self.content.len())
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(position.x - bounds.left())
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    /// The keys the field answers to. The letters themselves arrive through
    /// the input handler; these are the ones that move or delete.
    fn on_key_down(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => self.commit(window, cx),
            "escape" => {
                if let Some(escape) = self.on_escape.clone() {
                    escape(window, cx);
                }
            }
            "backspace" => {
                if self.selected_range.is_empty() {
                    let at = self.previous_boundary(self.cursor_offset());
                    self.selected_range = at..self.cursor_offset();
                }
                self.replace(None, "", window, cx);
            }
            "delete" => {
                if self.selected_range.is_empty() {
                    let at = self.next_boundary(self.selected_range.end);
                    self.selected_range = self.selected_range.end..at;
                }
                self.replace(None, "", window, cx);
            }
            "left" => {
                if self.selected_range.is_empty() {
                    let at = self.previous_boundary(self.cursor_offset());
                    self.move_to(at, cx);
                } else {
                    self.move_to(self.selected_range.start, cx);
                }
            }
            "right" => {
                if self.selected_range.is_empty() {
                    let at = self.next_boundary(self.selected_range.end);
                    self.move_to(at, cx);
                } else {
                    self.move_to(self.selected_range.end, cx);
                }
            }
            "home" => self.move_to(0, cx),
            "end" => self.move_to(self.content.len(), cx),
            _ => {}
        }
    }

    /// Put `text` where the selection is.
    fn replace(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .or_else(|| self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let mut content = self.content.to_string();
        content.replace_range(range.clone(), text);
        if self.numeric {
            content = filter_numeric(&content);
        }
        self.content = content.into();
        self.selected_range = range.start..range.start + text.len();
        self.marked_range = None;
        cx.notify();
        if let Some(change) = self.on_text_change.clone() {
            let typed = self.content.to_string();
            change(&typed, window, cx);
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = range_utf16;
        actual_range.replace(range.clone());
        Some(self.content.get(range)?.to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.selected_range.clone(),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range.clone()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(range, text, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range
            .or_else(|| self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        self.replace(Some(range.clone()), new_text, window, cx);
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        if let Some(selected) = new_selected_range {
            self.selected_range = range.start + selected.start..range.start + selected.end;
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        Some(Bounds::from_corners(
            point(
                bounds.left() + last_layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                bounds.left() + last_layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;
        last_layout.index_for_x(point.x - line_point.x)
    }
}

/// The element that draws the text, the caret and the selection.
struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    origin: Point<Pixels>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let shown_empty = input.content.is_empty() && input.placeholder.is_some();
        let content = if shown_empty {
            input.placeholder.clone().unwrap()
        } else {
            input.content.clone()
        };
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let focused = input.focus_handle.is_focused(window);
        let caret = Theme::color(input.theme.live);
        let style = window.text_style();

        // The run comes from the text style the field's own box carries, so
        // the field's family, size and weight are the ones it was given.
        let mut run = style.to_run(content.len());
        run.background_color = None;
        run.underline = None;
        run.strikethrough = None;
        if shown_empty {
            let color = input
                .placeholder_color
                .unwrap_or_else(|| Theme::color(input.theme.mute));
            run.color = color.into();
        }
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(content.clone(), font_size, &[run], None);

        // The field's text is right-aligned, as React's `.num` is; a text
        // field (the search box) is left-aligned.
        let text_width = line.x_for_index(content.len());
        let slack = bounds.size.width - text_width;
        let origin = point(bounds.left() + if input.ui_left || slack < px(0.0) { px(0.0) } else { slack }, bounds.top());

        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(origin.x + line.x_for_index(cursor), bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    caret,
                )),
            )
        } else if !focused {
            // A field that is not being typed into shows no selection.
            (None, None)
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            origin.x + line.x_for_index(selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            origin.x + line.x_for_index(selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    rgba(0x3311ff30),
                )),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            cursor,
            selection,
            origin,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection)
        }
        let origin = prepaint.origin;
        let line = prepaint.line.take().unwrap();
        line.paint(origin, window.line_height(), window, cx)
            .unwrap();

        if focus_handle.is_focused(window) && window.is_window_active() {
            // The caret belongs to a window that has the focus: a browser
            // hides it when its window is not the active one, and React's
            // capture never shows one (measured: the input was focused and
            // `document.hasFocus()` was true, yet 14 frames over 1.4 s of the
            // search field were pixel-identical).
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let disabled = self.disabled;
        // The field commits when it loses the focus, so it listens for it.
        self.watch_blur(window, cx);
        let bare = self.bare;
        let mut field = div()
            .id(gpui::SharedString::from("text-input"))
            .relative()
            .flex()
            .items_center()
            .justify_end()
            .w_full()
            .pl(px(self.left_pad))
            .pr(if bare { px(0.0) } else { px(self.right_pad) });
        if !bare {
            field = field.h(px(32.0));
        }
        if let Some(line_height) = self.line_height {
            field = field.line_height(gpui::relative(line_height));
        }
        if self.v_pad > 0.0 {
            field = field.py(px(self.v_pad));
        }
        field = if self.radius_sm { field.r_sm() } else { field.r_md() };
        field = field.text_color(if disabled {
            Theme::color(t.mute)
        } else {
            Theme::color(t.ink)
        })
        .text_size(px(self.font_px))
        .font_family(if self.ui_left { crate::fonts::UI_FAMILY } else { crate::fonts::MONO_FAMILY });
        field = if bare {
            field.bg(gpui::transparent_black())
        } else {
            field
                .border_1()
                .border_color(if self.border_2 { Theme::color(t.line_2) } else { Theme::color(t.line) })
                .bg(Theme::color(t.sunk))
        };
        if disabled {
            return field.child(TextElement { input: cx.entity() });
        }
        field = field
            .cursor(CursorStyle::IBeam)
            .track_focus(&self.focus_handle(cx))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_key_down(cx.listener(Self::on_key_down));
        field.child(TextElement { input: cx.entity() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_field_keeps_only_the_characters_a_number_may_hold() {
        assert_eq!(filter_numeric("30"), "30");
        assert_eq!(filter_numeric("3a0"), "30");
        assert_eq!(filter_numeric("30.5"), "30.5");
        assert_eq!(filter_numeric("30.5.2"), "30.52");
        assert_eq!(filter_numeric("-12"), "-12");
        assert_eq!(filter_numeric("12-"), "12");
        assert_eq!(filter_numeric("°C"), "");
        assert_eq!(filter_numeric(""), "");
    }

    #[test]
    fn a_commit_clamps_to_the_field_s_range() {
        assert_eq!(clamp_value(50.0, 0.0, 100.0), 50.0);
        assert_eq!(clamp_value(-5.0, 0.0, 100.0), 0.0);
        assert_eq!(clamp_value(150.0, 0.0, 100.0), 100.0);
        assert_eq!(clamp_value(f32::NAN, 20.0, 100.0), 20.0);
    }

    #[test]
    fn a_value_is_shown_with_the_field_s_own_decimals() {
        assert_eq!(format_value(30.0, 0), "30");
        assert_eq!(format_value(30.4, 0), "30");
        assert_eq!(format_value(42.5, 1), "42.5");
        assert_eq!(format_value(42.0, 1), "42.0");
    }
}
