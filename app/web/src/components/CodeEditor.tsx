"use client";

// The code editor: CodeMirror 6, HTML with the JavaScript inside it highlighted. This module is
// reached through a dynamic import (see EffectCode), so CodeMirror is its own chunk and the
// Effects page does not pay for it until an effect is edited.
//
// The errors come from the daemon, already structured (line and column, no text parsing); they
// are turned into CodeMirror diagnostics here, which draw the gutter markers and the squiggles.

import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { HighlightStyle, bracketMatching, syntaxHighlighting } from "@codemirror/language";
import { html } from "@codemirror/lang-html";
import { lintGutter, setDiagnostics, type Diagnostic } from "@codemirror/lint";
import { tags } from "@lezer/highlight";

/** One error, with the file line it marks (the caller adds the script's offset). */
export interface CodeError {
  message: string;
  line?: number;
  column?: number;
}

/** The editor's colours: every one a token, so both themes apply without a second style set. */
const theme = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--ink)",
    backgroundColor: "var(--sunk)",
    fontSize: "13px",
  },
  ".cm-scroller": {
    fontFamily: "var(--mono)",
    lineHeight: "1.6",
    overflow: "auto",
  },
  ".cm-content": {
    caretColor: "var(--a1)",
    padding: "var(--s2) 0",
  },
  ".cm-line": { padding: "0 var(--s3) 0 var(--s2)" },
  ".cm-gutters": {
    backgroundColor: "var(--sunk)",
    color: "var(--mute)",
    border: "none",
    borderRight: "1px solid var(--line)",
  },
  ".cm-lineNumbers .cm-gutterElement": { minWidth: "28px", padding: "0 var(--s2) 0 var(--s1)" },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--ink) 4%, transparent)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--dim)" },
  "&.cm-focused": { outline: "none" },
  "&.cm-focused .cm-cursor": { borderLeftColor: "var(--a1)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection": {
    backgroundColor: "color-mix(in srgb, var(--a1) 30%, transparent)",
  },
  ".cm-lintRange-error": {
    backgroundImage: "none",
    textDecoration: "underline wavy var(--down)",
    textUnderlineOffset: "3px",
  },
  ".cm-tooltip": {
    backgroundColor: "var(--raised)",
    border: "1px solid var(--line-2)",
    borderRadius: "var(--r-sm)",
    color: "var(--ink)",
    padding: "var(--s1) var(--s2)",
    fontFamily: "var(--sans)",
    maxWidth: "320px",
  },
  ".cm-diagnostic-error": { borderLeft: "2px solid var(--down)", paddingLeft: "var(--s2)" },
});

const highlight = HighlightStyle.define([
  { tag: tags.tagName, color: "var(--a1)" },
  { tag: tags.angleBracket, color: "var(--mute)" },
  { tag: tags.attributeName, color: "var(--a2)" },
  { tag: tags.attributeValue, color: "var(--a5)" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--a5)" },
  { tag: [tags.keyword, tags.operatorKeyword, tags.controlKeyword], color: "var(--a3)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--a4)" },
  { tag: [tags.comment, tags.blockComment], color: "var(--mute)", fontStyle: "italic" },
  { tag: [tags.function(tags.variableName), tags.definition(tags.variableName)], color: "var(--a3)" },
  { tag: [tags.propertyName, tags.definition(tags.propertyName)], color: "var(--a2)" },
  { tag: [tags.typeName, tags.className], color: "var(--a2)" },
  { tag: tags.variableName, color: "var(--ink)" },
  { tag: tags.operator, color: "var(--dim)" },
  { tag: tags.punctuation, color: "var(--dim)" },
  { tag: tags.invalid, color: "var(--down)" },
]);

/** The line of the document an error marks, and the columns it covers, as pure positions. */
function diagnosticsFor(state: EditorState, errors: CodeError[]): Diagnostic[] {
  return errors.map((error) => {
    const lineNumber = Math.max(1, Math.min(Math.round(error.line ?? 1), state.doc.lines));
    const line = state.doc.line(lineNumber);
    const column = Math.max(1, Math.floor(error.column ?? 1));
    const from = Math.min(line.from + column - 1, line.to);
    const to = Math.min(line.to, Math.max(from + 1, line.to));
    return {
      from,
      to: Math.max(from, to),
      severity: "error" as const,
      message: error.message,
      source: "effect",
    };
  });
}

export default function CodeEditor({
  value,
  onChange,
  errors,
  focusLine,
  focusNonce,
  revealLine,
  revealNonce,
  label,
}: {
  value: string;
  onChange: (next: string) => void;
  errors: CodeError[];
  /** A file line to put the cursor on, when `focusNonce` changes. */
  focusLine?: number | null;
  focusNonce?: number;
  /** A file line to bring into view (an error just reported), without moving the cursor. */
  revealLine?: number | null;
  revealNonce?: number;
  label: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  const applying = useRef(false);

  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        lintGutter(),
        history(),
        drawSelection(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        bracketMatching(),
        EditorState.tabSize.of(2),
        html(),
        syntaxHighlighting(highlight),
        theme,
        keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
        EditorView.lineWrapping,
        EditorView.contentAttributes.of({ "aria-label": label, spellcheck: "false" }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && !applying.current) onChangeRef.current(update.state.doc.toString());
        }),
      ],
    });
    const editor = new EditorView({ state, parent });
    view.current = editor;
    return () => {
      editor.destroy();
      view.current = null;
    };
    // The view is created once; value, errors and focus arrive through the effects below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [label]);

  // The source from outside (a revert, or the first load): put it in the view when it differs.
  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const now = editor.state.doc.toString();
    if (value === now) return;
    applying.current = true;
    editor.dispatch({ changes: { from: 0, to: now.length, insert: value } });
    applying.current = false;
  }, [value]);

  // The daemon's errors, as gutter markers and squiggles.
  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    editor.dispatch(setDiagnostics(editor.state, diagnosticsFor(editor.state, errors)));
  }, [errors]);

  // A click on the error list puts the cursor on that line and shows it.
  useEffect(() => {
    const editor = view.current;
    if (!editor || !focusLine) return;
    const line = editor.state.doc.line(Math.max(1, Math.min(focusLine, editor.state.doc.lines)));
    editor.dispatch({
      selection: { anchor: line.from },
      effects: EditorView.scrollIntoView(line.from, { y: "center" }),
    });
    editor.focus();
  }, [focusLine, focusNonce]);

  // An error the daemon just reported: the line is brought into view if it is off screen. The
  // cursor stays where the typing left it.
  useEffect(() => {
    const editor = view.current;
    if (!editor || !revealLine) return;
    const line = editor.state.doc.line(Math.max(1, Math.min(revealLine, editor.state.doc.lines)));
    editor.dispatch({ effects: EditorView.scrollIntoView(line.from, { y: "nearest" }) });
  }, [revealLine, revealNonce]);

  return <div className="cm-host" ref={host} data-testid="effect-code" />;
}
