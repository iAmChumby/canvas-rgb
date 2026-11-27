import { Fragment, type ReactNode } from "react";

/** Break opportunities after visible separators and between PascalCase words. */
function breakAfterSeparators(text: string, key: string): ReactNode[] {
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

/** A bracketed flag carries its argument, so `[--width <n>]` is one whitespace token. */
function chunks(text: string): string[] {
  const out: string[] = [];
  const re = /\[[^\]]*\]|\S+/g;
  let cursor = 0;
  for (const match of text.matchAll(re)) {
    const at = match.index ?? 0;
    if (at > cursor) out.push(text.slice(cursor, at));
    out.push(match[0]);
    cursor = at + match[0].length;
  }
  if (cursor < text.length) out.push(text.slice(cursor));
  return out;
}

/**
 * Wraps only where a reader expects a break: at spaces, after a path separator, or between
 * whole tokens. A flag, its argument and a hyphenated command name stay on one line.
 */
export function NoBreakText({ value, command = false }: { value: string; command?: boolean }) {
  return (
    <>
      {chunks(String(value ?? "")).map((chunk, index) => {
        if (/^\s+$/.test(chunk)) return <Fragment key={index}>{chunk}</Fragment>;
        const flag = /--[\w-]/.test(chunk);
        const commandWord = command && chunk.includes("-") && /^[a-z][\w-]*$/.test(chunk);
        if (flag || commandWord) {
          return (
            <span className="token-nowrap" key={index}>
              {chunk}
            </span>
          );
        }
        return <Fragment key={index}>{breakAfterSeparators(chunk, `p${index}`)}</Fragment>;
      })}
    </>
  );
}

/** Type names break at separators and between PascalCase words, never in the middle of one. */
export function BreakableTypes({ value }: { value: string }) {
  const text = String(value ?? "");
  const out: ReactNode[] = [];
  let buffer = "";
  for (let i = 0; i < text.length; i++) {
    const c = text.charAt(i);
    const prev = text.charAt(i - 1);
    if ("/?:-_.,>{}[];".includes(c) && buffer) {
      out.push(
        <Fragment key={`s${i}`}>
          {buffer}
          <wbr />
        </Fragment>,
      );
      buffer = "";
    } else if (/[A-Z]/.test(c) && prev && /[a-z0-9]/.test(prev) && buffer) {
      out.push(
        <Fragment key={`c${i}`}>
          {buffer}
          <wbr />
        </Fragment>,
      );
      buffer = "";
    }
    buffer += c;
  }
  if (buffer) out.push(<Fragment key="end">{buffer}</Fragment>);
  return <>{out}</>;
}
