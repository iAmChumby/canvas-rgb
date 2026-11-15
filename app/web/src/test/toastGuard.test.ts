// Class guard for #366 (the fourth time Luke asked for one toast look): a message that appears
// and goes away is drawn by `components/Notify.tsx`, which is the only file that talks to sonner.
// Everything else either says it through `notify()` or is a standing banner that waits for the
// person. This test reads every `.ts`/`.tsx` under `src` (tests and the generated `proto/`
// excluded) and fails on the four ways a second kind of toast can be made by hand:
//
//   1. another file importing `sonner`, so a second policy (and a second look) exists;
//   2. another file drawing the lane's own body (`notify-…` class names);
//   3. another file drawing a `<Banner>`: the two that do are standing notices, listed below;
//   4. a component giving a message its own dismissal clock (a `setTimeout` of two seconds or
//      more that clears a state to `null`/`false`). How long a message stays is the lane's
//      (`NOTE_MS`, `lifeMs`), never a component's timer. #366 found the last one: the preset
//      strip's "Deleted X / Undo" line, cleared by its own 8 s timer beside the same toast.
//
// The scan is written like `editorGuard.test.ts` and `NoPlainWaits.test.ts`: read the source as
// text, so a copy of the old pattern cannot land. `TOAST_GUARD_ROOT` points it at another copy
// of `src`, which is how the failure on the tree before this fix is shown.

import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = process.env.TOAST_GUARD_ROOT ?? resolve(process.cwd(), "src");

/** The one file allowed to know the library, and the only one that draws a toast body. */
const WRAPPER = "components/Notify.tsx";

/**
 * The files allowed to draw a standing `<Banner>`: the shell's notices (the SignalRGB conflict,
 * no scene yet, the header panel, a release offered, an install that failed, the fan service
 * away) and the scene list's "this light is in no group" notice. Each waits for a key or for its
 * condition to end; none of them says something and then takes it away on a clock.
 */
const BANNER_FILES = new Set(["shell/GlobalBanners.tsx", "views/scenes/NewLightNotice.tsx"]);

/** A notice's own clock is at least this long: anything shorter is an animation flag. */
const NOTICE_MS = 2000;

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

/** The name a file is known by in this test: its path under `src`, with forward slashes. */
function nameOf(file: string): string {
  return relative(ROOT, file).replace(/\\/g, "/");
}

/**
 * The arguments of every `setTimeout(...)` in one source, as text: the body and the delay,
 * together. The parenthesis walk keeps a callback's own braces and calls out of the count, so a
 * call wrapped by the formatter is still one call.
 */
export function timeouts(source: string): string[] {
  const found: string[] = [];
  const needle = "setTimeout(";
  for (let i = source.indexOf(needle); i >= 0; i = source.indexOf(needle, i + needle.length)) {
    let depth = 0;
    for (let j = i + needle.length - 1; j < source.length; j++) {
      const c = source[j];
      if (c === "(") depth++;
      else if (c === ")") {
        depth--;
        if (depth === 0) {
          found.push(source.slice(i + needle.length, j));
          break;
        }
      }
    }
  }
  return found;
}

/** The delay of one call's arguments: the number after the last top-level comma. */
function delayOf(args: string): number | null {
  let depth = 0;
  let quote: string | null = null;
  let lastComma = -1;
  for (let i = 0; i < args.length; i++) {
    const c = args[i];
    if (quote) {
      if (c === "\\") i++;
      else if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'" || c === "`") quote = c;
    else if (c === "(" || c === "[" || c === "{") depth++;
    else if (c === ")" || c === "]" || c === "}") depth--;
    else if (c === "," && depth === 0) lastComma = i;
  }
  if (lastComma < 0) return null;
  const digits = args.slice(lastComma + 1).replace(/[\s_]/g, "");
  return /^\d+$/.test(digits) ? Number(digits) : null;
}

/** A timer that clears a message: `setSomething(null)` or `setSomething(false)` in its body. */
const CLEARS = /set[A-Za-z0-9_]*\(\s*(?:null|false)\s*\)/;

/** Every hand-written dismissal clock in one source, as the call's text. */
export function noticeClocks(source: string): string[] {
  return timeouts(source).filter((args) => {
    const delay = delayOf(args);
    return delay !== null && delay >= NOTICE_MS && CLEARS.test(args);
  });
}

describe("every toast is the one toast (#366)", () => {
  it("finds the source tree", () => {
    expect(sources(ROOT).length).toBeGreaterThan(80);
  });

  it("only the wrapper imports sonner", () => {
    const offenders: string[] = [];
    for (const file of sources(ROOT)) {
      const rel = nameOf(file);
      if (rel === WRAPPER) continue;
      const text = readFileSync(file, "utf8");
      if (/from\s+["']sonner["']/.test(text)) offenders.push(rel);
    }
    expect(offenders, "A toast goes through notify() in components/Notify.tsx, never sonner directly").toEqual([]);
  });

  it("only the wrapper draws a toast body", () => {
    const offenders: string[] = [];
    for (const file of sources(ROOT)) {
      const rel = nameOf(file);
      if (rel === WRAPPER) continue;
      const text = readFileSync(file, "utf8");
      // The lane's own class names: notify-body, notify-title, notify-close, ...
      if (/["'`][^"'`]*\bnotify-[a-z]/.test(text)) offenders.push(rel);
    }
    expect(offenders, "notify-* is the wrapper's card; a second one is a second look").toEqual([]);
  });

  it("a banner is a standing notice, and only the shell and the scene list draw one", () => {
    const offenders: string[] = [];
    for (const file of sources(ROOT)) {
      const rel = nameOf(file);
      if (BANNER_FILES.has(rel)) continue;
      if (/<Banner\b/.test(readFileSync(file, "utf8"))) offenders.push(rel);
    }
    expect(offenders, "A message that comes and goes is a toast (notify); a Banner waits for the person").toEqual([]);
  });

  it("no component gives a message its own dismissal clock", () => {
    const offenders: string[] = [];
    for (const file of sources(ROOT)) {
      const rel = nameOf(file);
      if (rel === WRAPPER) continue;
      for (const clock of noticeClocks(readFileSync(file, "utf8"))) {
        offenders.push(`${rel}: ${clock.trim().replace(/\s+/g, " ")}`);
      }
    }
    expect(offenders, "How long a message stays is the lane's lifeMs, never a component's timer").toEqual([]);
  });

  describe("the matcher", () => {
    it("catches the shape #366 removed", () => {
      const old = [
        "if (next) undoTimer.current = setTimeout(() => setUndoState(null), 8000);",
        "setTimeout(() => setNotice(null), 5_000);",
        "window.setTimeout(() => setToast(false), 3000);",
      ];
      for (const line of old) expect(noticeClocks(line), line).toHaveLength(1);
    });

    it("leaves an animation flag and a wait alone", () => {
      for (const fine of [
        "setTimeout(() => setFavPop(null), 260);",
        "setTimeout(() => setSettled(true), 240);",
        "const timer = setTimeout(() => setSilent(true), TRAY_TIMEOUT_MS);",
        "setTimeout(() => void ask(), 1_000);",
      ]) {
        expect(noticeClocks(fine), fine).toEqual([]);
      }
    });
  });
});
