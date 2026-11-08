//! The code pane, after `app/web/src/views/effects/EffectCode.tsx` and
//! `app/web/src/components/CodeEditor.tsx` (CodeMirror in React).
//!
//! A [`CodeEditor`] entity owns the draft, draws it with a line-number
//! gutter, token colours, wrapping, a caret and a selection, and takes the
//! Tab, Enter (auto-indent) and undo/redo keys. The rules that need no window
//! are free functions, so they are unit-tested: [`tokenize_line`] is the
//! tokenizer behind the highlight theme, and the `line_*` / `word_*`
//! helpers are the key handling's place maths.
//!
//! The highlight colours are React `CodeEditor.tsx`'s `highlight` set:
//! tag names `--a1`, attribute names and property names `--a2`, values and
//! strings `--a5`, keywords and function names `--a3`, numbers and bools
//! `--a4`, comments italic `--mute`, operators and punctuation `--dim`.

#![allow(dead_code)]

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Context, CursorStyle, Entity, EntityInputHandler, FocusHandle,
    Focusable, FontStyle, FontWeight, Hsla, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Window, div, prelude::*, px,
};

use crate::components::button::{Variant, button_icon};
use crate::theme::{Theme, TokenRadius};

/// The code line's height (`--code-line` in `app.css`).
pub const CODE_LINE: f32 = 20.8;
/// The code's own face size (CodeMirror's theme in `CodeEditor.tsx`).
pub const CODE_FONT: f32 = 13.0;
/// The line-number gutter's width. React measures 36 px at 198 lines (the
/// widest element plus `padding: 0 8px 0 4px`), so this is a box width that
/// covers three-digit numbers: `minWidth: 28px` in `CodeEditor.tsx` is a
/// floor, not the width. The code starts at 36 (numbers) + 18 (lint) + 1
/// (the shared border) after the pane edge (probed: the pane at 769, the
/// code at 824).
pub const GUTTER_W: f32 = 36.0;
/// The lint gutter's width (the extra column the round-1 comment already
/// named: 18 px beside the numbers).
pub const LINT_GUTTER_W: f32 = 18.0;
/// The tab size the editor declares (`EditorState.tabSize.of(2)`).
pub const TAB_SIZE: usize = 2;

/// One token's kind, named for the CodeMirror tag it stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Comment,
    TagName,
    AngleBracket,
    AttributeName,
    AttributeValue,
    Str,
    Keyword,
    Number,
    BoolNull,
    PropertyName,
    TypeName,
    FunctionName,
    VariableName,
    Operator,
    Punctuation,
    Invalid,
    Plain,
}

/// One token: a kind and its byte range in the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '-'
}

fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "const" | "let" | "var" | "function" | "return" | "if" | "else" | "for" | "while" | "do"
            | "switch" | "case" | "default" | "break" | "continue" | "new" | "class" | "extends"
            | "super" | "typeof" | "instanceof" | "in" | "of" | "try" | "catch" | "finally"
            | "throw" | "yield" | "async" | "await" | "import" | "export" | "from" | "this"
            | "void" | "delete" | "static" | "get" | "set"
    )
}

/// Tokenizes one logical line the way the effect source reads: HTML tags and
/// attributes outside a script, the JavaScript inside one. Pure, so it is
/// pinned by tests below against the highlight colours.
pub fn tokenize_line(line: &str) -> Vec<Token> {
    tokenize_line_in(line, false, false)
}

fn tokenize_line_in(line: &str, in_html: bool, in_script: bool) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut in_script = in_script;
    let mut in_tag = false;
    // A line can open a script that runs past its end; the next line is
    // tokenized inside a script until `</script>`. The flag is carried by
    // the caller across lines via tokenize_document.
    while i < bytes.len() {
        // HTML comment runs to the end of the line at least.
        if line[i..].starts_with("<!--") {
            let end = line[i..].find("-->").map(|at| i + at + 3).unwrap_or(line.len());
            out.push(Token { kind: TokenKind::Comment, start: i, end });
            i = end;
            continue;
        }
        // Script detection without a full parser: `<script` opens one,
        // `</script` closes it. The `<`, the name and the `>` all take their
        // own tokens, in order, so the run covers the line without a gap.
        if !in_script && line[i..].starts_with("<script") {
            let mut j = i + 1;
            while j < line.len() {
                let c = line[j..].chars().next().unwrap();
                if c == '>' {
                    break;
                }
                j += c.len_utf8();
            }
            out.push(Token { kind: TokenKind::AngleBracket, start: i, end: i + 1 });
            // The tag name itself.
            out.push(Token { kind: TokenKind::TagName, start: i + 1, end: i + 7 });
            if j < line.len() {
                out.push(Token { kind: TokenKind::AngleBracket, start: j, end: j + 1 });
                i = j + 1;
            } else {
                i = j;
            }
            in_script = true;
            in_tag = false;
            continue;
        }
        if line[i..].starts_with("</script>") {
            out.push(Token { kind: TokenKind::AngleBracket, start: i, end: i + 1 });
            out.push(Token { kind: TokenKind::AngleBracket, start: i + 1, end: i + 2 });
            out.push(Token { kind: TokenKind::TagName, start: i + 2, end: i + 8 });
            out.push(Token { kind: TokenKind::AngleBracket, start: i + 8, end: i + 9 });
            i += 9;
            in_script = false;
            continue;
        }
        let ch = line[i..].chars().next().unwrap();
        // Block and line comments inside a script.
        if in_script && (line[i..].starts_with("/*") || line[i..].starts_with("//")) {
            let end = if line[i..].starts_with("/*") {
                line[i..].find("*/").map(|at| i + at + 2).unwrap_or(line.len())
            } else {
                line.len()
            };
            out.push(Token { kind: TokenKind::Comment, start: i, end });
            i = end;
            continue;
        }
        // Strings: double, single, backtick.
        if ch == '"' || ch == '\'' || ch == '`' {
            let quote = ch;
            let mut j = i + 1;
            while j < line.len() {
                let c = line[j..].chars().next().unwrap();
                if c == '\\' {
                    j += 1;
                    continue;
                }
                if c == quote {
                    j += c.len_utf8();
                    break;
                }
                j += c.len_utf8();
            }
            let kind = if in_tag && out.last().map(|t| t.kind == TokenKind::Operator).unwrap_or(false) {
                TokenKind::AttributeValue
            } else {
                TokenKind::Str
            };
            out.push(Token { kind, start: i, end: j.min(line.len()) });
            i = j.min(line.len());
            continue;
        }
        // A tag open: `<name` or `</name`.
        if ch == '<' {
            let mut j = i + 1;
            if j < line.len() && line[j..].starts_with('/') {
                j += 1;
            }
            if j < line.len() {
                let c = line[j..].chars().next().unwrap();
                if c.is_ascii_alphabetic() || c == '!' || c == '?' {
                    let start = j;
                    while j < line.len() {
                        let c = line[j..].chars().next().unwrap();
                        if is_ident_char(c) && c != '-' {
                            j += c.len_utf8();
                        } else {
                            break;
                        }
                    }
                    out.push(Token { kind: TokenKind::AngleBracket, start: i, end: i + 1 });
                    out.push(Token { kind: TokenKind::TagName, start, end: j });
                    i = j;
                    in_tag = true;
                    continue;
                }
            }
            out.push(Token { kind: TokenKind::Operator, start: i, end: i + 1 });
            i += 1;
            continue;
        }
        if ch == '>' {
            out.push(Token { kind: TokenKind::AngleBracket, start: i, end: i + 1 });
            i += 1;
            in_tag = false;
            continue;
        }
        // Attribute names inside a tag (`name=`): the name is `--a2`.
        if in_tag && (is_ident_start(ch) || ch == '-' || ch == ':' || ch == '.') {
            let start = i;
            while i < line.len() {
                let c = line[i..].chars().next().unwrap();
                if is_ident_start(c) || c.is_ascii_digit() || matches!(c, '-' | ':' | '.' | '_') {
                    i += c.len_utf8();
                } else {
                    break;
                }
            }
            out.push(Token { kind: TokenKind::AttributeName, start, end: i });
            continue;
        }
        // Numbers, including hex and exponents.
        if ch.is_ascii_digit() {
            let start = i;
            if line[i..].starts_with("0x") || line[i..].starts_with("0X") {
                i += 2;
                while i < line.len() && line[i..].chars().next().unwrap().is_ascii_hexdigit() {
                    i += 1;
                }
            } else {
                while i < line.len() {
                    let c = line[i..].chars().next().unwrap();
                    if c.is_ascii_digit() || c == '.' {
                        i += c.len_utf8();
                    } else {
                        break;
                    }
                }
                if i < line.len() && (line[i..].starts_with('e') || line[i..].starts_with('E')) {
                    let mut j = i + 1;
                    if j < line.len() && (line[j..].starts_with('+') || line[j..].starts_with('-')) {
                        j += 1;
                    }
                    if j < line.len() && line[j..].chars().next().unwrap().is_ascii_digit() {
                        i = j;
                        while i < line.len() && line[i..].chars().next().unwrap().is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
            }
            out.push(Token { kind: if in_script { TokenKind::Number } else { TokenKind::Plain }, start, end: i });
            continue;
        }
        // Identifiers: keyword, function, property, type or variable.
        // Outside a script they are plain text, apart from the tag and
        // attribute cases handled above (React's `tags.variableName` is the
        // ink for HTML text too).
        if is_ident_start(ch) {
            let start = i;
            while i < line.len() {
                let c = line[i..].chars().next().unwrap();
                if is_ident_start(c) || c.is_ascii_digit() {
                    i += c.len_utf8();
                } else {
                    break;
                }
            }
            let word = &line[start..i];
            // What follows decides a function or a property name.
            let mut next = i;
            while next < line.len() && line[next..].starts_with(' ') {
                next += 1;
            }
            let prev_non_space = {
                let mut prev = start;
                while prev > 0 && line[..prev].ends_with(' ') {
                    prev -= 1;
                }
                if prev == 0 {
                    None
                } else {
                    Some(line[prev - 1..prev].chars().next().unwrap())
                }
            };
            let kind = if !in_script {
                TokenKind::Plain
            } else if matches!(word, "true" | "false" | "null") {
                TokenKind::BoolNull
            } else if is_keyword(word) {
                TokenKind::Keyword
            } else if next < line.len() && line[next..].starts_with('(') {
                TokenKind::FunctionName
            } else if prev_non_space == Some('.') || (next < line.len() && line[next..].starts_with(':')) {
                TokenKind::PropertyName
            } else if word.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false) {
                TokenKind::TypeName
            } else {
                TokenKind::VariableName
            };
            out.push(Token { kind, start, end: i });
            continue;
        }
        // Operators and punctuation both wear `--dim`.
        if matches!(ch, '+' | '-' | '*' | '/' | '%' | '^' | '&' | '|' | '?' | ':' | '!' | '~' | '=' | '<' | '>' | ';' | ',' | '.' | '{' | '}' | '[' | ']' | '(' | ')' | '#') {
            out.push(Token { kind: TokenKind::Punctuation, start: i, end: i + ch.len_utf8() });
            i += ch.len_utf8();
            continue;
        }
        // Anything else (spaces, tabs) is plain ink.
        out.push(Token { kind: TokenKind::Plain, start: i, end: i + ch.len_utf8() });
        i += ch.len_utf8();
    }
    let _ = (in_html, in_tag);
    out
}

/// Tokenizes a whole document, keeping the script state across lines so a
/// `//` line inside `<script>` reads as a comment and HTML text does not.
/// Returns one token vector per logical line.
pub fn tokenize_document(source: &str) -> Vec<Vec<Token>> {
    let mut out = Vec::new();
    let mut in_script = false;
    for line in source.split('\n') {
        // The flags are recomputed from this line for the next one: a line
        // that opens `<script` without closing it puts the next line inside.
        let mut next_in_script = in_script;
        if !in_script && line.contains("<script") && !line.contains("</script>") {
            next_in_script = true;
        }
        if in_script && line.contains("</script>") {
            next_in_script = false;
        }
        out.push(tokenize_line_in(line, false, in_script));
        in_script = next_in_script;
    }
    out
}

/// The colour React's highlight theme gives a token kind (`CodeEditor.tsx`).
pub fn colour_for(kind: TokenKind, t: &Theme) -> gpui::Rgba {
    match kind {
        TokenKind::Comment => Theme::color(t.mute),
        TokenKind::TagName => Theme::color(t.a1),
        TokenKind::AngleBracket => Theme::color(t.mute),
        TokenKind::AttributeName => Theme::color(t.a2),
        TokenKind::AttributeValue => Theme::color(t.a5),
        TokenKind::Str => Theme::color(t.a5),
        TokenKind::Keyword => Theme::color(t.a3),
        TokenKind::Number => Theme::color(t.a4),
        TokenKind::BoolNull => Theme::color(t.a4),
        TokenKind::PropertyName => Theme::color(t.a2),
        TokenKind::TypeName => Theme::color(t.a2),
        TokenKind::FunctionName => Theme::color(t.a3),
        TokenKind::VariableName => Theme::color(t.ink),
        TokenKind::Operator | TokenKind::Punctuation => Theme::color(t.dim),
        TokenKind::Invalid => Theme::color(t.down),
        TokenKind::Plain => Theme::color(t.ink),
    }
}

/// One daemon error, with the file line it marks (`CodeEditor.tsx`
/// `CodeError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

/// The file line (1-based) a runtime's script-relative line falls on
/// (`sourceLineOf` in `effectSource.ts`): the first inline `<script>`'s
/// open tag line is added; a source with no script keeps the line.
pub fn source_line_of(source: &str, script_line: usize) -> usize {
    let lower = source.to_lowercase();
    let Some(at) = lower.find("<script") else { return script_line };
    let Some(open) = source[at..].find('>').map(|rel| at + rel) else { return script_line };
    let Some(close) = lower[open..].find("</script").map(|rel| open + rel) else { return script_line };
    let _ = close;
    let line_start = source[..open + 1].split('\n').count() - 1;
    line_start + script_line
}

/// One diagnostic: the document offsets it covers and its line (`setDiagnostics`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub from: usize,
    pub to: usize,
    pub line: usize,
    pub message: String,
}

/// The diagnostics for `errors` over `source`, ported from
/// `diagnosticsFor`: the line clamps into the document, the columns clamp
/// onto the line.
pub fn diagnostics_for(source: &str, errors: &[CodeError]) -> Vec<Diagnostic> {
    let lines: Vec<&str> = source.split('\n').collect();
    let line_count = lines.len().max(1);
    let mut offsets: Vec<usize> = Vec::with_capacity(lines.len());
    let mut at = 0usize;
    for line in &lines {
        offsets.push(at);
        at += line.len() + 1;
    }
    errors
        .iter()
        .map(|error| {
            let line_number = error.line.unwrap_or(1).clamp(1, line_count);
            let line = lines[line_number - 1];
            let column = error.column.unwrap_or(1).max(1);
            let from = (offsets[line_number - 1] + (column - 1).min(line.len())).min(offsets[line_number - 1] + line.len());
            let to = (offsets[line_number - 1] + line.len()).min(from + line.len().max(1));
            let to = to.max(from);
            Diagnostic { from, to, line: line_number, message: error.message.clone() }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The place maths the keys share (pure, so they are pinned by tests).

/// The start of the line holding `offset`.
pub fn line_start(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())].rfind('\n').map(|at| at + 1).unwrap_or(0)
}

/// The end of the line holding `offset` (the newline itself is outside).
pub fn line_end(source: &str, offset: usize) -> usize {
    let at = offset.min(source.len());
    source[at..].find('\n').map(|rel| at + rel).unwrap_or(source.len())
}

/// The 1-based line holding `offset`.
pub fn line_of(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())].matches('\n').count() + 1
}

/// The 0-based offset of the first byte of `line` (1-based).
fn line_offset(source: &str, line: usize) -> usize {
    let mut current = 1usize;
    let mut at = 0usize;
    for (i, ch) in source.char_indices() {
        if current == line {
            return at;
        }
        if ch == '\n' {
            current += 1;
            at = i + 1;
        }
    }
    if current == line { at } else { source.len() }
}

/// The previous word boundary before `offset` (Ctrl-Backspace).
pub fn prev_word(source: &str, offset: usize) -> usize {
    let mut at = offset.min(source.len());
    let mut stripped = false;
    while at > 0 && source[..at].ends_with(|c: char| c.is_whitespace()) {
        at -= source[..at].chars().last().unwrap().len_utf8();
        stripped = true;
    }
    // After a gap, the boundary is the end of the previous word.
    if stripped {
        return at;
    }
    while at > 0 {
        let c = source[..at].chars().last().unwrap();
        if c.is_whitespace() {
            break;
        }
        at -= c.len_utf8();
    }
    at
}

/// The next word boundary after `offset` (Ctrl-Delete).
pub fn next_word(source: &str, offset: usize) -> usize {
    let mut at = offset.min(source.len());
    let rest = &source[at..];
    let mut chars = rest.char_indices().peekable();
    // Skip the remainder of the current word, then the gap after it.
    while let Some((_, c)) = chars.peek() {
        if c.is_whitespace() {
            break;
        }
        at += c.len_utf8();
        chars.next();
    }
    while let Some((_, c)) = chars.peek() {
        if !c.is_whitespace() {
            break;
        }
        at += c.len_utf8();
        chars.next();
    }
    at
}

/// The leading spaces and tabs of `line`.
pub fn indent_of(line: &str) -> &str {
    let end = line.find(|c: char| c != ' ' && c != '\t').unwrap_or(line.len());
    &line[..end]
}

/// Tab indenting `selection` in `source`: a plain Tab is two spaces (`indentWithTab`
/// with `tabSize 2`); a multi-line selection indents every touched line.
pub fn apply_tab(source: &str, selection: &Range<usize>) -> (String, Range<usize>) {
    if !source[selection.clone()].contains('\n') {
        let mut text = source.to_string();
        text.replace_range(selection.clone(), "  ");
        let caret = selection.start + 2;
        return (text, caret..caret);
    }
    let start = line_start(source, selection.start);
    let end = selection.end;
    let mut text = String::with_capacity(source.len() + 32);
    text.push_str(&source[..start]);
    let mut at = start;
    let mut first = true;
    let mut shift = 0usize;
    while at <= end && at < source.len() {
        text.push_str("  ");
        if first {
            shift = 2;
            first = false;
        }
        let next = source[at..].find('\n').map(|rel| at + rel + 1).unwrap_or(source.len());
        text.push_str(&source[at..next]);
        at = next;
    }
    text.push_str(&source[at.max(end)..]);
    let new_start = selection.start + shift;
    let new_end = selection.end + 2 * (text[..(new_start)].matches('\n').count() + 1);
    (text, new_start..new_end.max(new_start))
}

/// Shift-Tab outdenting: two spaces (or one tab) off every touched line.
pub fn apply_outdent(source: &str, selection: &Range<usize>) -> (String, Range<usize>) {
    let start = line_start(source, selection.start);
    let end = selection.end;
    let mut text = String::with_capacity(source.len());
    text.push_str(&source[..start]);
    let mut at = start;
    let mut removed_before_start = 0usize;
    let mut removed_before_end = 0usize;
    while at <= end && at < source.len() {
        let line_end_at = line_end(source, at);
        let line = &source[at..line_end_at];
        let remove = if line.starts_with("  ") { 2 } else if line.starts_with('\t') { 1 } else { 0 };
        if at <= selection.start { removed_before_start += remove; }
        if at <= selection.end { removed_before_end += remove; }
        text.push_str(&line[remove..]);
        if source[line_end_at..].starts_with('\n') {
            text.push('\n');
            at = line_end_at + 1;
        } else {
            at = line_end_at;
        }
    }
    text.push_str(&source[at.max(end)..]);
    let new_start = selection.start.saturating_sub(removed_before_start);
    let new_end = selection.end.saturating_sub(removed_before_end).max(new_start);
    (text, new_start..new_end)
}

/// Enter: a newline with the current line's indent carried over
/// (`insertNewlineAndIndent`).
pub fn apply_enter(source: &str, selection: &Range<usize>) -> (String, Range<usize>) {
    let start = line_start(source, selection.start);
    let indent = indent_of(&source[start..line_end(source, selection.start)]).to_string();
    let mut text = source.to_string();
    let insertion = format!("\n{indent}");
    text.replace_range(selection.clone(), &insertion);
    let caret = selection.start + insertion.len();
    (text, caret..caret)
}

// ---------------------------------------------------------------------------
// The editor entity.

/// Told the editor's text on every change.
pub type OnChange = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// The code editor: a focusable multiline field with a gutter, token
/// colours, wrapping, a caret and a selection, after CodeMirror 6.
pub struct CodeEditor {
    focus_handle: FocusHandle,
    text: String,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    is_selecting: bool,
    theme: Theme,
    on_change: OnChange,
    diagnostics: Vec<Diagnostic>,
    focus: bool,
    undo_stack: Vec<(String, Range<usize>)>,
    redo_stack: Vec<(String, Range<usize>)>,
}

impl CodeEditor {
    pub fn new(initial: &str, theme: Theme, on_change: OnChange, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            text: initial.to_string(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            is_selecting: false,
            theme,
            on_change,
            diagnostics: Vec::new(),
            focus: false,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// The full source.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The line count (`source.split('\n').length`).
    pub fn line_count(&self) -> usize {
        self.text.split('\n').count()
    }

    /// An outside value (Revert, first load): written in without echoing.
    pub fn set_text(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.text == value {
            return;
        }
        self.text = value.to_string();
        self.clamp_selection();
        cx.notify();
    }

    /// The daemon's diagnostics (gutter markers and squiggles).
    pub fn set_diagnostics(&mut self, diagnostics: Vec<Diagnostic>, cx: &mut Context<Self>) {
        if self.diagnostics != diagnostics {
            self.diagnostics = diagnostics;
            cx.notify();
        }
    }

    /// Puts the caret at the start of `line` (1-based) and shows it.
    pub fn focus_line(&mut self, line: usize, cx: &mut Context<Self>) {
        let lines = self.text.split('\n').count();
        let wanted = line.clamp(1, lines.max(1));
        let at = line_offset(&self.text, wanted);
        self.selected_range = at..at;
        self.selection_reversed = false;
        cx.notify();
    }

    /// Slides within CharBoundary.
    pub fn focus_line_clamped(&mut self, line: usize, cx: &mut Context<Self>) {
        self.focus_line(line, cx);
    }

    fn clamp_selection(&mut self) {
        let len = self.text.len();
        if !self.text.is_char_boundary(self.selected_range.start.min(len)) {
            self.selected_range.start = self.selected_range.start.min(len);
        }
        if !self.text.is_char_boundary(self.selected_range.end.min(len)) {
            self.selected_range.end = self.selected_range.end.min(len);
        }
        self.selected_range.start = self.selected_range.start.min(len);
        self.selected_range.end = self.selected_range.end.min(len);
        if self.selected_range.end < self.selected_range.start {
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed { self.selected_range.start } else { self.selected_range.end }
    }

    fn push_history(&mut self) {
        self.undo_stack.push((self.text.clone(), self.selected_range.clone()));
        if self.undo_stack.len() > 200 {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((text, selection)) = self.undo_stack.pop() {
            self.redo_stack.push((std::mem::replace(&mut self.text, text), std::mem::replace(&mut self.selected_range, selection)));
            self.clamp_selection();
            let text = self.text.clone();
            (self.on_change)(&text, window, cx);
            cx.notify();
        }
    }

    fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((text, selection)) = self.redo_stack.pop() {
            self.undo_stack.push((std::mem::replace(&mut self.text, text), std::mem::replace(&mut self.selected_range, selection)));
            self.clamp_selection();
            let text = self.text.clone();
            (self.on_change)(&text, window, cx);
            cx.notify();
        }
    }

    fn commit_edit(&mut self, next: String, window: &mut Window, cx: &mut Context<Self>) {
        if next == self.text {
            return;
        }
        self.push_history();
        self.text = next;
        self.clamp_selection();
        let text = self.text.clone();
        (self.on_change)(&text, window, cx);
        cx.notify();
    }

    fn replace(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let range = range.or_else(|| self.marked_range.clone()).unwrap_or_else(|| self.selected_range.clone());
        let mut next = self.text.clone();
        next.replace_range(range.clone(), text);
        self.selected_range = range.start..range.start + text.len();
        self.marked_range = None;
        self.commit_edit(next, window, cx);
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
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

    fn previous_boundary(&self, offset: usize) -> usize {
        self.text.char_indices().rev().find_map(|(i, _)| (i < offset).then_some(i)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.text.char_indices().find_map(|(i, _)| (i > offset).then_some(i)).unwrap_or(self.text.len())
    }

    /// The keys the editor answers to: Tab indents, Enter carries the indent,
    /// Ctrl-Z/Y undo and redo. The rest move or delete.
    fn on_key_down(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        let control = mods.control || mods.platform && !mods.shift;
        // On Windows the redo key is Ctrl-Y; Ctrl-Shift-Z redoes only on Linux
        // in CodeMirror, so it does nothing here.
        if control && (key == "z" || key == "Z") {
            self.undo(window, cx);
            return;
        }
        if control && (key == "y" || key == "Y") {
            self.redo(window, cx);
            return;
        }
        if control && (key == "a" || key == "A") {
            self.selected_range = 0..self.text.len();
            self.selection_reversed = false;
            cx.notify();
            return;
        }
        if control && (key == "/" || key == "slash") {
            self.toggle_line_comment(window, cx);
            return;
        }
        match key {
            "escape" => {
                // CodeMirror `simplifySelection`: collapse a range selection.
                if !self.selected_range.is_empty() {
                    let at = self.cursor_offset();
                    self.selected_range = at..at;
                    cx.notify();
                }
            }
            "tab" => {
                let (next, sel) = if mods.shift {
                    apply_outdent(&self.text, &self.selected_range)
                } else {
                    apply_tab(&self.text, &self.selected_range)
                };
                if next != self.text {
                    self.selected_range = sel.clone();
                }
                self.commit_edit(next, window, cx);
            }
            "enter" => {
                let (next, sel) = apply_enter(&self.text, &self.selected_range);
                self.selected_range = sel;
                self.commit_edit(next, window, cx);
            }
            "backspace" => {
                if self.selected_range.is_empty() {
                    let cursor = self.cursor_offset();
                    let at = if control { prev_word(&self.text, cursor) } else { self.previous_boundary(cursor) };
                    if at < cursor {
                        self.selected_range = at..cursor;
                    }
                }
                self.replace(None, "", window, cx);
            }
            "delete" => {
                if self.selected_range.is_empty() {
                    let cursor = self.cursor_offset();
                    let at = if control { next_word(&self.text, cursor) } else { self.next_boundary(self.selected_range.end) };
                    if at > self.selected_range.end {
                        self.selected_range = self.selected_range.end..at;
                    }
                }
                self.replace(None, "", window, cx);
            }
            "left" => {
                if control {
                    self.move_to(prev_word(&self.text, self.cursor_offset()), cx);
                } else if self.selected_range.is_empty() {
                    let at = self.previous_boundary(self.cursor_offset());
                    if mods.shift { self.select_to(at, cx) } else { self.move_to(at, cx) }
                } else {
                    let at = self.selected_range.start;
                    self.move_to(at, cx);
                }
            }
            "right" => {
                if control {
                    self.move_to(next_word(&self.text, self.cursor_offset()), cx);
                } else if self.selected_range.is_empty() {
                    let at = self.next_boundary(self.selected_range.end);
                    if mods.shift { self.select_to(at, cx) } else { self.move_to(at, cx) }
                } else {
                    let at = self.selected_range.end;
                    self.move_to(at, cx);
                }
            }
            "up" => {
                if mods.alt {
                    self.move_line(-1, window, cx);
                } else {
                    self.move_vertical(-1, mods.shift, cx);
                }
            }
            "down" => {
                if mods.alt {
                    self.move_line(1, window, cx);
                } else {
                    self.move_vertical(1, mods.shift, cx);
                }
            }
            "home" => {
                let at = line_start(&self.text, self.cursor_offset());
                if mods.shift { self.select_to(at, cx) } else { self.move_to(at, cx) }
            }
            "end" => {
                let at = line_end(&self.text, self.cursor_offset());
                if mods.shift { self.select_to(at, cx) } else { self.move_to(at, cx) }
            }
            _ => {}
        }
    }

    fn move_vertical(&mut self, delta: i64, extend: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let line = line_of(&self.text, cursor);
        let lines = self.text.split('\n').count() as i64;
        let target = (line as i64 + delta).clamp(1, lines.max(1)) as usize;
        let column = cursor - line_start(&self.text, cursor);
        let start = line_offset(&self.text, target);
        let end = line_end(&self.text, start);
        let at = (start + column).min(end);
        if extend { self.select_to(at, cx) } else { self.move_to(at, cx) }
    }

    fn move_line(&mut self, delta: i64, window: &mut Window, cx: &mut Context<Self>) {
        let start = line_start(&self.text, self.cursor_offset());
        let end = line_end(&self.text, self.cursor_offset());
        let line_text = self.text[start..end].to_string();
        let lines: Vec<&str> = self.text.split('\n').collect();
        let line_no = (line_of(&self.text, self.cursor_offset()) as i64 - 1) as usize;
        let other = (line_no as i64 + delta) as usize;
        if other >= lines.len() || (delta < 0 && line_no == 0) || (delta > 0 && line_no + 1 >= lines.len()) {
            return;
        }
        let column = self.cursor_offset() - start;
        let mut next = String::new();
        for (i, l) in lines.iter().enumerate() {
            if i == line_no {
                next.push_str(lines[other]);
            } else if i == other {
                next.push_str(&line_text);
            } else {
                next.push_str(l);
            }
            if i + 1 < lines.len() {
                next.push('\n');
            }
        }
        let new_start = line_offset(&next, other + 1);
        self.selected_range = (new_start + column)..(new_start + column);
        self.commit_edit(next, window, cx);
    }

    fn toggle_line_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = line_start(&self.text, self.selected_range.start);
        let end = line_end(&self.text, self.selected_range.end);
        let line = self.text[start..end].to_string();
        let trimmed = line.trim_start();
        let next = if let Some(stripped) = trimmed.strip_prefix("// ") {
            let indent_len = line.len() - trimmed.len();
            let mut l = line.clone();
            l.replace_range(indent_len..indent_len + 3, "");
            let _ = stripped;
            l
        } else if trimmed.starts_with("//") {
            let indent_len = line.len() - trimmed.len();
            let mut l = line.clone();
            l.replace_range(indent_len..indent_len + 2, "");
            l
        } else {
            let indent = indent_of(&line).to_string();
            format!("{indent}// {trimmed}")
        };
        let mut text = self.text.clone();
        text.replace_range(start..end, &next);
        self.commit_edit(text, window, cx);
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        // The row estimate is refined by the row that owns the press.
        if event.modifiers.shift {
            self.select_to(self.selected_range.end, cx);
        }
    }

    fn click_line(&mut self, line: usize, column: usize, extend: bool, cx: &mut Context<Self>) {
        let lines = self.text.split('\n').count();
        let target = line.clamp(1, lines.max(1));
        let start = line_offset(&self.text, target);
        let end = line_end(&self.text, start);
        let at = (start + column).min(end);
        if extend { self.select_to(at, cx) } else { self.move_to(at, cx) }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, _: &MouseMoveEvent, _: &mut Window, _: &mut Context<Self>) {}
}

impl EntityInputHandler for CodeEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = range_utf16;
        actual_range.replace(range.clone());
        Some(self.text.get(range)?.to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::UTF16Selection> {
        Some(gpui::UTF16Selection { range: self.selected_range.clone(), reversed: self.selection_reversed })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
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
        let range = range.or_else(|| self.marked_range.clone()).unwrap_or_else(|| self.selected_range.clone());
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
        _range: Range<usize>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        // The editor draws itself per line; the OS input window's own caret
        // positioning follows the mouse estimate instead.
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<gpui::Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Focusable for CodeEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The editor's run font. React's theme sets `fontFamily: "var(--mono)"`
/// on `.cm-scroller`, but `--mono` is not defined, so the scroller inherits
/// the page's Schibsted: the code draws proportional in the UI family, not
/// in JetBrains Mono. Ported deliberately like the checklist's other React
/// gaps; the measured computed family is "Schibsted Grotesk Variable".
fn code_font(italic: bool) -> gpui::Font {
    gpui::Font {
        family: crate::fonts::UI_FAMILY.to_string().into(),
        weight: FontWeight::NORMAL,
        style: if italic { FontStyle::Italic } else { FontStyle::Normal },
        // `kern`, like the text the shell draws: Chromium's CodeMirror kerns
        // (the browser's `font-kerning` is `auto`), and DirectWrite only
        // kerns when a typography names the feature. Without it every word
        // in the pane sat a fraction of a pixel right of the browser's.
        features: crate::fonts::kerning(),
        fallbacks: None,
    }
}

fn hsla(color: gpui::Rgba) -> Hsla {
    Hsla::from(color)
}

/// One logical line's display: the line with tabs expanded to two spaces
/// (CodeMirror `tabSize 2`), its token colours and its runs.
fn line_runs(
    line: &str,
    tokens: &[Token],
    t: &Theme,
    selection_in_line: Option<(usize, usize)>,
    caret_in_line: Option<usize>,
    underline_ranges: &[(usize, usize)],
    focused: bool,
) -> (String, Vec<gpui::TextRun>) {
    // Expand tabs: one tab is two spaces. The display is walked over the
    // whole line, never per-token, so a tokenizer gap (the `/` in
    // `</head>`) stays plain text instead of vanishing.
    let mut display = String::with_capacity(line.len() + 8);
    let mut tab_prefix = vec![0usize; line.len() + 1];
    {
        let mut seen = 0usize;
        for (at, c) in line.char_indices() {
            tab_prefix[at] = seen;
            if c == '\t' {
                seen += 1;
            }
        }
        tab_prefix[line.len()] = seen;
    }
    let disp = |at: usize| at + tab_prefix[at.min(line.len())];
    for c in line.chars() {
        if c == '\t' {
            display.push_str("  ");
        } else {
            display.push(c);
        }
    }
    let expanded_tokens: Vec<Token> = tokens
        .iter()
        .map(|t| Token { kind: t.kind, start: disp(t.start), end: disp(t.end) })
        .collect();
    // Breakpoints: token edges, selection edges, caret, underline edges.
    let mut cuts: Vec<usize> = vec![0, display.len()];
    for token in &expanded_tokens {
        cuts.push(token.start);
        cuts.push(token.end);
    }
    if let Some((s, e)) = selection_in_line {
        cuts.push(s);
        cuts.push(e);
    }
    if let Some(c) = caret_in_line {
        cuts.push(c);
        if c < display.len() {
            cuts.push(c + display[c..].chars().next().unwrap().len_utf8());
        }
    }
    for &(s, e) in underline_ranges {
        cuts.push(s);
        cuts.push(e);
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts.retain(|&c| c <= display.len() && display.is_char_boundary(c));
    // A caret at the end of (or on) an empty line needs a visible cell.
    let caret_at_end = caret_in_line.map(|c| c >= display.len()).unwrap_or(false);
    if caret_at_end && display.is_empty() {
        display = " ".to_string();
        cuts = vec![0, 1];
    } else if caret_at_end {
        // Append a space so the caret run has a cell; the runs below cut
        // at display.len() before the append, so this draws one space past
        // the text, the same width a bar would take.
        display.push(' ');
        cuts.push(display.len());
        cuts.sort_unstable();
        cuts.dedup();
    }
    let selection_bg = {
        let mut c = hsla(Theme::color(t.a1));
        c.a = 0.3;
        c
    };
    let caret_bg = hsla(Theme::color(t.a1));
    let down = Theme::color(t.down);
    let mut runs: Vec<gpui::TextRun> = Vec::new();
    for pair in cuts.windows(2) {
        let (s, e) = (pair[0], pair[1]);
        if s == e {
            continue;
        }
        // The token holding `s`.
        let kind = expanded_tokens.iter().find(|tok| s >= tok.start && s < tok.end).map(|tok| tok.kind).unwrap_or(TokenKind::Plain);
        let mut run = gpui::TextRun {
            len: e - s,
            font: code_font(kind == TokenKind::Comment),
            color: hsla(colour_for(kind, t)),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        if let Some((sel_s, sel_e)) = selection_in_line {
            if s >= sel_s && e <= sel_e {
                run.background_color = Some(selection_bg);
            }
        }
        if let Some(caret) = caret_in_line {
            // The caret sits on its own cell: solid `--a1`.
            if focused && (s == caret || (caret_at_end && s == display.len() - 1)) {
                run.background_color = Some(caret_bg);
            }
        }
        for &(u_s, u_e) in underline_ranges {
            if s >= u_s && e <= u_e {
                run.underline = Some(gpui::UnderlineStyle { thickness: px(1.0), color: Some(hsla(down)), wavy: true });
                break;
            }
        }
        runs.push(run);
    }
    // Runs must cover the whole display text.
    let covered: usize = runs.iter().map(|r| r.len).sum();
    if covered < display.len() {
        runs.push(gpui::TextRun {
            len: display.len() - covered,
            font: code_font(false),
            color: hsla(Theme::color(t.ink)),
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    (display, runs)
}

impl Render for CodeEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let focused = self.focus_handle.is_focused(window);
        self.focus = focused;
        let source = self.text.clone();
        let tokens_per_line = tokenize_document(&source);
        let lines: Vec<&str> = source.split('\n').collect();
        let cursor = self.cursor_offset();
        let active_line = line_of(&source, cursor);
        // Selection and diagnostics mapped into each line.
        let sel = self.selected_range.clone();
        let mut offsets: Vec<usize> = Vec::with_capacity(lines.len());
        let mut at = 0usize;
        for line in &lines {
            offsets.push(at);
            at += line.len() + 1;
        }
        // `.cm-activeLine`: `color-mix(in srgb, var(--ink) 4%, transparent)`.
        let mut active_bg = gpui::Hsla::from(Theme::color(t.ink));
        active_bg.a = 0.04;
        let mut col = div().flex().flex_col().flex_1().min_w(px(0.0)).overflow_hidden();
        for (i, line) in lines.iter().enumerate() {
            let row_line = i + 1;
            let start = offsets[i];
            let end = start + line.len();
            // Selection clipped to this line (byte offsets inside the line).
            let selection_in_line = if sel.end > start && sel.start < end {
                Some((sel.start.max(start) - start, sel.end.min(end) - start))
            } else {
                None
            };
            let caret_in_line = if cursor >= start && cursor <= end { Some(cursor - start) } else { None };
            // Diagnostics clipped to this line.
            let mut underlines: Vec<(usize, usize)> = Vec::new();
            for diag in &self.diagnostics {
                let d_s = (diag.from.max(start)) - start;
                let d_e = (diag.to.min(end)) - start;
                if diag.from >= start && diag.from <= end && d_e > d_s {
                    underlines.push((d_s, d_e));
                }
            }
            let (display, runs) = line_runs(line, &tokens_per_line[i], &t, selection_in_line, caret_in_line, &underlines, focused);
            let is_active = row_line == active_line;
            let digit = (i + 1).to_string();
            let row_has_error = self.diagnostics.iter().any(|d| d.line == row_line);
            // The row: the number gutter, the lint gutter, then the code.
            // `.cm-gutters` draws one shared right border over both gutters.
            // The numbers sit at the top of a wrapped line's first visual row
            // (`.cm-gutterElement` spans the line's height with the number in
            // its first row), so the lint dot is top-rowed the same way.
            let gutter = div()
                .flex()
                .flex_none()
                .child(
                    div()
                        .w(px(GUTTER_W))
                        .flex_none()
                        .pr(px(8.0))
                        .pl(px(4.0))
                        .text_right()
                        .text_size(px(CODE_FONT))
                        .line_height(px(CODE_LINE))
                        .font_family(crate::fonts::UI_FAMILY)
                        .text_color(Theme::color(if is_active { t.dim } else { t.mute }))
                        .child(digit),
                )
                .child(
                    div()
                        .flex_none()
                        // The widths are border-box: 18 of lint then the 1 px
                        // `.cm-gutters` right border past it, so the code
                        // starts 36 + 18 + 1 px from the pane edge like
                        // React's (measured: the pane at 769, the code at
                        // 824).
                        .w(px(LINT_GUTTER_W + 1.0))
                        .border_r_1()
                        .border_color(Theme::color(t.line))
                        .child(
                            div()
                                .h(px(CODE_LINE))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(if row_has_error {
                                    div().size(px(7.0)).rounded_full().bg(Theme::color(t.down))
                                } else {
                                    div().size(px(7.0))
                                }),
                        ),
                );
            let mut code = div().flex_1().min_w(px(0.0)).pl(px(8.0)).pr(px(12.0));
            if is_active {
                code = code.bg(active_bg);
            }
            let text = if display.is_empty() {
                // An empty line still takes its full height.
                div().h(px(CODE_LINE))
            } else {
                div()
                    .text_size(px(CODE_FONT))
                    .line_height(px(CODE_LINE))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.ink))
                    .child(gpui::StyledText::new(display).with_runs(runs))
            };
            code = code.child(text);
            let entity = cx.entity();
            let row = div()
                .flex()
                .flex_none()
                .min_w(px(0.0))
                .child(gutter)
                .child(code)
                .on_mouse_down(
                    MouseButton::Left,
                    {
                        let entity = entity.clone();
                        move |event: &MouseDownEvent, _, cx| {
                            entity.update(cx, |this, cx| {
                                // Approximate the column from the press.
                                // The code starts after both gutters and the
                                // line's 8 px left padding; the figure is a
                                // proportional advance, so it stays approximate.
                                let col = ((f32::from(event.position.x) - GUTTER_W - LINT_GUTTER_W - 8.0) / 7.0).max(0.0) as usize;
                                this.click_line(row_line, col.min(200), event.modifiers.shift, cx);
                            });
                        }
                    },
                );
            col = col.child(row);
        }
        div()
            .id(gpui::SharedString::from("code-editor"))
            .flex_1()
            .min_h(px(180.0))
            .overflow_hidden()
            .cursor(CursorStyle::IBeam)
            .track_focus(&self.focus_handle(cx))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_key_down(cx.listener(Self::on_key_down))
            .child(col)
    }
}

/// The placeholder while the engine loads (`EffectCode.tsx` `.cm-loading`):
/// "Loading the editor…" centred in the pane's height so the box does not
/// jump when the editor arrives.
pub fn loading_box(t: &Theme) -> impl IntoElement {
    div()
        .flex_1()
        .min_h(px(180.0))
        .items_center()
        .justify_center()
        .text_size(px(14.0))
        .line_height(gpui::relative(1.45))
        .font_family(crate::fonts::UI_FAMILY)
        .text_color(Theme::color(t.mute))
        .child("Loading the editor…")
}

/// Told which error row was pressed, by index into `errors` (the row puts
/// the caret on its line and shows the pane).
pub type OnError = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// The code pane: its head (the icon, the title, the line count and the
/// Revert key) over the editor, with the error list and the preview error
/// under it (`ul.eff-errors` and `p.eff-preview-error`).
///
/// `narrow` is the phone layout, where the head carries a Back key and the
/// Save key instead of the icon. `editor` is the code editor entity the
/// caller owns, so typing writes through to the owner's `source`.
#[allow(clippy::too_many_arguments)]
pub fn code_pane(
    t: &Theme,
    id: &str,
    editor: Entity<CodeEditor>,
    line_count: usize,
    narrow: bool,
    can_revert: bool,
    on_back: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    on_revert: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    errors: &[(String, Option<usize>)],
    preview_error: Option<&str>,
    on_error: OnError,
) -> impl IntoElement {
    let mut head = div()
        .flex()
        .flex_nowrap()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(Theme::color(t.line))
        .min_w(px(0.0));
    if narrow {
        head = head.child(crate::components::button::icon_button(
            t,
            &format!("{id}-back"),
            "CaretLeft",
            "",
            false,
            false,
            false,
            on_back,
        ));
    } else {
        head = head.child(
            gpui::svg()
                .path("icons/Code.svg")
                .size(px(15.0))
                .flex_none()
                .text_color(Theme::color(t.dim)),
        );
    }
    head = head
        .child(
            div()
                .text_size(px(15.0))
                .line_height(gpui::relative(1.45))
                .font_weight(FontWeight::SEMIBOLD)
                .font_family(crate::fonts::UI_FAMILY)
                .text_color(Theme::color(t.ink))
                .child("Code"),
        )
        // React `.eff-code-count { margin-left: auto }`: the count sits at
        // the head's right, before the actions.
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .justify_end()
                .child(
                    div()
                        .text_size(px(15.0))
                        .line_height(gpui::relative(1.45))
                        .font_family(crate::fonts::MONO_FAMILY)
                        .text_color(Theme::color(t.mute))
                        .child(format!("{line_count} lines")),
                ),
        )
        .child(
            div()
                .flex()
                .gap_1()
                .child(button_icon(
                    t,
                    &format!("{id}-revert"),
                    "Revert",
                    Variant::Ghost,
                    true,
                    !can_revert,
                    "ArrowCounterClockwise",
                    "",
                    14.0,
                    on_revert,
                )),
        );

    let body = div()
        .id(gpui::SharedString::from(format!("{id}-body")))
        .flex_1()
        .min_h(px(180.0))
        .overflow_hidden()
        .child(editor);

    // The error list under the code (`ul.eff-errors`), then the preview
    // error line (`p.eff-preview-error`), inside the pane's box.
    let mut pane = div()
        .id(gpui::SharedString::from(id.to_string()))
        .flex()
        .flex_col()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .bg(Theme::color(t.sunk))
        .border_1()
        .border_color(Theme::color(t.line))
        .r_sm()
        .overflow_hidden()
        .child(head)
        .child(body);
    if !errors.is_empty() {
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px_2()
            .py_1()
            .min_w(px(0.0));
        list = list.border_t_1().border_color(Theme::color(t.line));
        for (index, (message, line)) in errors.iter().enumerate() {
            let line_label = match line {
                Some(n) => format!("line {n}"),
                None => "error".to_string(),
            };
            let on_error = on_error.clone();
            list = list.child(
                div()
                    .id(gpui::SharedString::from(format!("{id}-error-{index}")))
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .p_1()
                    .cursor_pointer()
                    .text_color(Theme::color(t.down))
                    .on_click(move |_, window, cx| on_error(index, window, cx))
                    .child(
                        gpui::svg()
                            .path("icons/Warning.svg")
                            .size(px(14.0))
                            .flex_none()
                            .text_color(Theme::color(t.down)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(15.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::MONO_FAMILY)
                            .text_color(Theme::color(t.warn))
                            .child(line_label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(15.0))
                            .line_height(gpui::relative(1.45))
                            .font_family(crate::fonts::UI_FAMILY)
                            .text_color(Theme::color(t.dim))
                            .child(message.clone()),
                    ),
            );
        }
        pane = pane.child(list);
    }
    if let Some(text) = preview_error {
        if !text.is_empty() {
            pane = pane.child(
                div()
                    .border_t_1()
                    .border_color(Theme::color(t.line))
                    .px_2()
                    .py_1()
                    .text_size(px(15.0))
                    .line_height(gpui::relative(1.45))
                    .font_family(crate::fonts::UI_FAMILY)
                    .text_color(Theme::color(t.down))
                    .child(text.to_string()),
            );
        }
    }
    pane
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_line_height_is_the_css_one() {
        assert_eq!(CODE_LINE, 20.8);
        assert_eq!(CODE_FONT, 13.0);
        assert_eq!(GUTTER_W, 36.0);
        assert_eq!(LINT_GUTTER_W, 18.0);
    }

    fn kinds(line: &str) -> Vec<TokenKind> {
        tokenize_line(line).into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn a_tag_and_its_brackets_take_the_tag_colours() {
        let tokens = tokenize_line("<div>");
        assert_eq!(tokens[0].kind, TokenKind::AngleBracket);
        assert_eq!(tokens[1].kind, TokenKind::TagName);
        assert_eq!(tokens[2].kind, TokenKind::AngleBracket);
    }

    #[test]
    fn an_attribute_is_a2_and_its_value_a5() {
        let tokens = tokenize_line(r#"<div class="x">"#);
        // class -> AttributeName, "x" -> Str (a5).
        assert!(tokens.iter().any(|t| t.kind == TokenKind::AttributeName));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Str || t.kind == TokenKind::AttributeValue));
    }

    #[test]
    fn keywords_numbers_and_calls_take_their_colours() {
        let doc = tokenize_document("<script>\nconst x = 42; f(x);");
        let tokens = &doc[1];
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Keyword));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::Number));
        assert!(tokens.iter().any(|t| t.kind == TokenKind::FunctionName));
    }

    #[test]
    fn html_text_outside_a_script_is_plain() {
        let tokens = tokenize_line("Ink Drop");
        assert!(tokens.iter().all(|t| t.kind == TokenKind::Plain));
    }

    #[test]
    fn a_closing_tag_keeps_its_slash_in_the_display() {
        let tokens = tokenize_line("</head>");
        let (display, runs) = line_runs("</head>", &tokens, &Theme::dark(), None, None, &[], false);
        assert_eq!(display, "</head>");
        assert_eq!(runs.iter().map(|r| r.len).sum::<usize>(), display.len());
    }

    #[test]
    fn a_tab_expands_to_two_spaces_in_the_display() {
        let (display, _) = line_runs("a\tb", &[], &Theme::dark(), None, None, &[], false);
        assert_eq!(display, "a  b");
    }

    #[test]
    fn comments_take_the_mute_italic_colour() {
        let _tokens = tokenize_line("// hi");
        // A `//` outside a script is a comment only inside one; the line
        // hook treats it as punctuation there, and inside a script as a
        // comment. The document tokenizer carries the script flag.
        let doc = tokenize_document("<script>\n// hi");
        assert_eq!(doc[1][0].kind, TokenKind::Comment);
        let _ = kinds("<div>");
    }

    #[test]
    fn diagnostics_clamp_onto_the_document() {
        let source = "ab\ncde\nfg";
        let diags = diagnostics_for(source, &[
            CodeError { message: "m".into(), line: Some(2), column: Some(2) },
            CodeError { message: "n".into(), line: Some(99), column: Some(99) },
        ]);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].from, 4);
        assert_eq!(diags[0].to, 6);
        assert_eq!(diags[1].line, 3);
    }

    #[test]
    fn tab_is_two_spaces() {
        let (next, sel) = apply_tab("ab", &(2..2));
        assert_eq!(next, "ab  ");
        assert_eq!(sel, 4..4);
    }

    #[test]
    fn enter_carries_the_indent() {
        let (next, sel) = apply_enter("  ab", &(4..4));
        assert_eq!(next, "  ab\n  ");
        assert_eq!(sel, 7..7);
    }

    #[test]
    fn outdent_removes_two_spaces() {
        let (next, _) = apply_outdent("  ab", &(4..4));
        assert_eq!(next, "ab");
    }

    #[test]
    fn word_boundaries_skip_gaps() {
        let source = "ab  cde";
        assert_eq!(prev_word(source, 4), 2);
        assert_eq!(prev_word(source, 7), 4);
        assert_eq!(next_word(source, 0), 4);
    }

    #[test]
    fn lines_are_counted_from_newlines() {
        let source = "a\nbc\nd";
        assert_eq!(line_of(source, 0), 1);
        assert_eq!(line_of(source, 2), 2);
        assert_eq!(line_start(source, 2), 2);
        assert_eq!(line_end(source, 0), 1);
        assert_eq!(indent_of("  x"), "  ");
    }

    #[test]
    fn a_selection_over_lines_indents_each_touched_line() {
        let (next, _) = apply_tab("a\nb", &(0..3));
        assert_eq!(next, "  a\n  b");
    }
}
