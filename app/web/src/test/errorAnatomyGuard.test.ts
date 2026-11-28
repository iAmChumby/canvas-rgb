// Class guard for #576/#575 (P9 and F8, in the style of toastGuard.test.ts): error text reaches a
// page only through `ErrorState` in components/ErrorState.tsx, which alone draws `ErrorLine`.
// The scan reads source as text, so a copy of the old pattern cannot land. `ERROR_GUARD_ROOT`
// points it at another copy of `src`, which is how the failures on the tree before this fix are shown.

import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = process.env.ERROR_GUARD_ROOT ?? resolve(process.cwd(), "src");
const ALLOWED = "components/ErrorState.tsx";

function sources(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "__tests__" || entry.name === "proto") continue;
      out.push(...sources(path));
    } else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
      out.push(path);
    }
  }
  return out.sort();
}

/** P9 widens the guard from views to components as well. */
function guarded(): string[] {
  return [...sources(join(ROOT, "views")), ...sources(join(ROOT, "components"))];
}

function nameOf(file: string): string {
  return relative(ROOT, file).split(sep).join("/");
}

/** A call of the text helpers: views and components hand the caught value to `ErrorState`. */
const TEXT_CALL = /\b(?:messageOf|errorText)\s*\(/g;

/**
 * The three ways a failure's words may leave the anatomy, each pinned to one use: a toast, a
 * `title` attribute and the code editor's gutter tooltip. Any other call is a violation, so none
 * of them can become another name for `messageOf`.
 */
const SINKS: { call: RegExp; allowed: (before: string, file: string) => boolean }[] = [
  { call: /\btoastText\s*\(/g, allowed: (before) => /\bnotify\(\s*$/.test(before) },
  { call: /\btitleText\s*\(/g, allowed: (before) => /\btitle=\{[^{}\n]*$/.test(before) },
  { call: /\blintText\s*\(/g, allowed: (_before, file) => file === "components/CodeEditor.tsx" },
];

/**
 * Every way error text reaches a page by hand: the text helpers, a bare `ErrorLine`, a
 * hand-written error element, a raw `.message` read, `String(e)`, or a bare JSX error.
 */
export function violations(source: string, file = "views/Planted.tsx"): string[] {
  const found: string[] = [];
  for (const match of source.matchAll(TEXT_CALL)) found.push(match[0]);
  for (const sink of SINKS) {
    for (const match of source.matchAll(sink.call)) {
      if (!sink.allowed(source.slice(0, match.index), file)) found.push(match[0]);
    }
  }
  for (const match of source.matchAll(/<ErrorLine\b|\bimport\s*\{[^}]*\bErrorLine\b/g)) found.push(match[0]);
  for (const match of source.matchAll(/<(?:p|span|div)\s+className=(?:"|\{`|\{s\.)[^"`}]*[^"`}>]*?(?<![\w-])(?:error-line|errorText|presetError|recovery-refused)/g)) {
    found.push(match[0].replace(/\s+/g, " "));
  }
  // The raw words behind a disclosure: the stale-read notices did this with a store error.
  for (const match of source.matchAll(/<Details>\s*\{\w*(?:e|E)rr(?:or)?\}/g)) found.push(match[0]);
  for (const match of source.matchAll(/\b(?:e|err|error)\s*\.\s*message\b/g)) found.push(match[0]);
  for (const match of source.matchAll(/\bString\s*\(\s*(?:e|err|error)\s*\)/g)) found.push(match[0]);
  for (const match of source.matchAll(/\?\.\s*message\b/g)) found.push(match[0]);
  for (const match of source.matchAll(/\(\s*(?:e|err|error)\s+as\s+Error\s*\)\s*\.\s*message\b/g)) found.push(match[0]);
  // A daemon's `.error` field rendered straight into the page (F8).
  for (const match of source.matchAll(/(?<![\w]=|\$)\{(?:f|failure|issue)\.(?:error|message)\}/g)) found.push(match[0]);
  // An error value rendered as an element's whole content.
  for (const match of source.matchAll(
    /<(p|span|div)\b[^<>]*>\s*\{(?:e|err|error|\w+(?:Error|Err|Problem))(?:\.message)?\}\s*<\/\1>/g,
  )) {
    found.push(match[0].replace(/\s+/g, " "));
  }
  return found;
}

describe("every error has one anatomy (#576, F8/F9)", () => {
  it("finds the views and components", () => {
    expect(guarded().length).toBeGreaterThan(80);
  });

  it("keeps error text inside ErrorState", () => {
    const offenders: string[] = [];
    for (const file of guarded()) {
      const name = nameOf(file);
      if (name === ALLOWED) continue;
      for (const hit of violations(readFileSync(file, "utf8"), name)) offenders.push(`${name}: ${hit}`);
    }
    expect(offenders, "Error text reaches the page only through ErrorState").toEqual([]);
  });

  describe("the matcher", () => {
    it.each([
      ["messageOf", "setErr(messageOf(e));"],
      ["errorText", "setErr(errorText(e));"],
      ["a bare ErrorLine", "<ErrorLine>{err}</ErrorLine>"],
      ["an ErrorLine import", 'import { ErrorLine } from "@/components/ErrorState";'],
      ["a toast helper outside notify", "setErr(toastText(e));"],
      ["a title helper outside title", "setErr(titleText(e));"],
      ["a lint helper outside the code editor", "setErr(lintText(e));"],
      ["a hand-written error line", '<p className="error-line">{words}</p>'],
      ["a hand-written module class", "<p className={s.errorText}>{setup.error}</p>"],
      ["raw words behind a disclosure", "<Details>{devicesError}</Details>"],
      [".message", "return { state: 'refused', message: e.message };"],
      ["String(error)", "const text = String(error);"],
      ["?.message", "setErr(e?.message);"],
      ["a cast .message", "setErr((e as Error).message);"],
      ["a daemon .error field", "<span>{f.error}</span>"],
      ["a bare error element", '<div className="load-note">{error}</div>'],
      ["a named error element", "<span>{saveError}</span>"],
    ])("catches %s", (_name, planted) => {
      expect(violations(planted)).not.toEqual([]);
    });

    it("leaves the anatomy and its callers alone", () => {
      expect(violations("setErr(caught(e));")).toEqual([]);
      expect(violations('notify(toastText(e), "down");')).toEqual([]);
      expect(violations("title={refused ? titleText(refused) : undefined}")).toEqual([]);
      expect(violations("message: lintText(error),", "components/CodeEditor.tsx")).toEqual([]);
      expect(violations("<ErrorState compact error={err} />")).toEqual([]);
      expect(violations("<ErrorState error={e} onRetry={load} />")).toEqual([]);
      expect(violations("patternRefusal.message")).toEqual([]);
      expect(violations('<span className="fig eff-error-line">')).toEqual([]);
      expect(violations("key={`${issue.message}-${i}`}")).toEqual([]);
    });
  });
});
