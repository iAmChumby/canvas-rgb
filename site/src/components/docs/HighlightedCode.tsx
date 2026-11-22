/*
 * The small code colouring the docs use: comments, quoted strings, tags, a few keywords and
 * command flags. Each colour is one of the editor's token classes, so light and dark both hold.
 */

import { Fragment, type ReactNode } from "react";

const TOKEN =
  /("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|<\/?[A-Za-z][^>]*>|\b(?:function|const|var|let|return|await|new)\b|\b(?:curl|canvas-rgb|fetch)\b|--?[\w-]+|\/\/.*$|#.*$)/g;

/** A flag and its argument, including a bracketed `[--width <n>]`, are one token. */
const FLAG_GROUP = /\[[^\]\n]*--[^\]\n]*\]|--[\w-]+(?:\s+<[^>]+>)?/g;

function cls(token: string): string | undefined {
  if (/^(\/\/|#)/.test(token)) return "c-com";
  if (/^["']/.test(token)) return "c-str";
  if (/^<\/?[A-Za-z]/.test(token)) return "syn-tag";
  if (/^(function|const|var|let|return|await|new)$/.test(token)) return "c-fn";
  if (/^(curl|canvas-rgb|fetch)$/.test(token)) return "c-flag";
  if (/^--?[\w-]+$/.test(token)) return "c-flag";
  return undefined;
}

/**
 * A code line breaks only at a boundary the reader can see in the text: after a path or namespace
 * separator, or between the words of a PascalCase name. Every other character stays with its
 * token, so no word is cut in the middle. The break is an explicit <wbr>, never a CSS fallback.
 */
export function breakable(text: string, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  let buffer = "";
  for (let i = 0; i < text.length; i++) {
    const c = text.charAt(i);
    const prev = i > 0 ? text.charAt(i - 1) : "";
    buffer += c;
    const separator = "/?:-_.,>{}[];".includes(c);
    const camel = /[A-Z]/.test(c) && /[a-z0-9]/.test(prev);
    if (separator || camel) {
      out.push(
        <Fragment key={`${key}-${i}`}>
          {buffer}
          <wbr />
        </Fragment>,
      );
      buffer = "";
    }
  }
  if (buffer) out.push(<Fragment key={`${key}-end`}>{buffer}</Fragment>);
  return out;
}

/** Applies the editor's token colours, then the break opportunities, to one piece of text. */
function highlight(text: string, key: string): ReactNode[] {
  const parts: Array<{ text: string; className?: string }> = [];
  let cursor = 0;
  TOKEN.lastIndex = 0;
  for (const match of text.matchAll(TOKEN)) {
    const at = match.index ?? 0;
    if (at > cursor) parts.push({ text: text.slice(cursor, at) });
    parts.push({ text: match[0], className: cls(match[0]) });
    cursor = at + match[0].length;
  }
  if (cursor < text.length) parts.push({ text: text.slice(cursor) });
  return parts.map((part, at) => (
    <span className={part.className} key={`${key}-${at}`}>
      {breakable(part.text, `${key}-${at}`)}
    </span>
  ));
}

/** A code block, split into lines so a comment line keeps its own muted ink. */
export function HighlightedCode({ code }: { code: string }) {
  return (
    <>
      {code.split("\n").map((line, lineAt) => {
        const nodes: ReactNode[] = [];
        let cursor = 0;
        FLAG_GROUP.lastIndex = 0;
        for (const match of line.matchAll(FLAG_GROUP)) {
          const at = match.index ?? 0;
          if (at > cursor) nodes.push(<Fragment key={`t${cursor}`}>{highlight(line.slice(cursor, at), `t${cursor}`)}</Fragment>);
          nodes.push(
            <span className="token-nowrap" key={`f${at}`}>
              {highlight(match[0], `f${at}`)}
            </span>,
          );
          cursor = at + match[0].length;
        }
        if (cursor < line.length) nodes.push(<Fragment key={`t${cursor}`}>{highlight(line.slice(cursor), `t${cursor}`)}</Fragment>);
        return (
          <span key={lineAt}>
            {lineAt === 0 ? "" : "\n"}
            {nodes}
          </span>
        );
      })}
    </>
  );
}
