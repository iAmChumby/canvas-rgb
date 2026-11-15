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
//      more that clears a message: a state set to `null`, `false`, `undefined` or `""`, or a
//      call or handle named `clear*`, `dismiss*` or `hide*`). How long a message stays is the
//      lane's (`NOTE_MS`, `lifeMs`), never a component's timer. #366 found the last one: the
//      preset strip's "Deleted X / Undo" line, cleared by its own 8 s timer beside the same
//      toast. #389 found the two ways around that check: a delay named by a `const`, and a
//      clear that is not a `set…(null|false)`.
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

/** The delay of one call's arguments: the text after the last top-level comma. */
function delayTextOf(args: string): string | null {
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
  return args.slice(lastComma + 1).trim();
}

/**
 * The delays a file names for itself: `const UNDO_MS = 8000;`, and `= 8_000` too. A timer may
 * write its delay as a number or as one of these; anything else is a delay this test cannot read.
 */
const NAMED_DELAY = /\bconst\s+([A-Za-z_$][\w$]*)\s*(?::\s*number\s*)?=\s*(\d[\d_]*)\s*;/g;

/**
 * The names a file gives a motion time rather than a clock: a `const` whose value comes from the
 * stylesheet, like `const ms = motionOffNow() ? 0 : readCssMs("--dur-exit", 90)`. How long a panel
 * takes to leave is a motion time, never how long a message stays (#389: the exit of the panel in
 * `components/Popover.tsx` and `components/Select.tsx`, which the first pass flagged).
 */
const MOTION_DELAY = /\bconst\s+([A-Za-z_$][\w$]*)\s*=\s*[^;\n]*readCssMs\([^;\n]*;/g;

interface Delays {
  /** Named numeric delays, in ms. */
  numbers: Map<string, number>;
  /** Names that hold a motion time read from the stylesheet. */
  motion: Set<string>;
}

export function delaysIn(source: string): Delays {
  const numbers = new Map<string, number>();
  for (const m of source.matchAll(NAMED_DELAY)) {
    const name = m[1];
    const digits = m[2];
    if (name && digits) numbers.set(name, Number(digits.replace(/_/g, "")));
  }
  const motion = new Set<string>();
  for (const m of source.matchAll(MOTION_DELAY)) if (m[1]) motion.add(m[1]);
  return { numbers, motion };
}

/**
 * How long one timer waits, in ms: its literal, or the number a `const` in the same file gives
 * it. `"motion"` when the file reads the delay out of the stylesheet. Null when the text is a
 * name or an expression this file does not define (#389: the named delay is how
 * `setTimeout(() => setX(null), UNDO_MS)` used to pass the check).
 */
function delayMsOf(text: string, delays: Delays): number | "motion" | null {
  const bare = text.replace(/[\s_]/g, "");
  if (/^\d+$/.test(bare)) return Number(bare);
  const name = text.trim();
  if (/^[A-Za-z_$][\w$]*$/.test(name)) {
    if (delays.motion.has(name)) return "motion";
    const ms = delays.numbers.get(name);
    if (ms !== undefined) return ms;
  }
  return null;
}

/**
 * The ways a timer's body takes a message away: a state set to nothing (`null`, `false`,
 * `undefined`, the empty string), or a call or handle named for going away (`clear()`,
 * `dismissNotice()`, `setTimeout(hide, 8000)`). `clearTimeout` and `clearInterval` are the
 * timer's own APIs, not a message, so they are not one of these (#389).
 */
const CLEARS =
  /set[A-Za-z0-9_$]*\(\s*(?:null|false|undefined|""|''|``)\s*\)|(?<![\w$.-])(?:clear|dismiss|hide)(?!Timeout|Interval)[A-Za-z0-9_$]*/;

/**
 * Every hand-written dismissal clock in one source, as the call's text. A clear on a delay the
 * file gives as a number is one only at two seconds or more; a clear on a delay this file cannot
 * read is one, because an unreadable delay is exactly how a clock hides (#389).
 */
export function noticeClocks(source: string): string[] {
  const delays = delaysIn(source);
  return timeouts(source).filter((args) => {
    if (!CLEARS.test(args)) return false;
    const text = delayTextOf(args);
    // No delay at all: the call fires on the next turn (a flush, a focus), not on a clock.
    if (text === null || text === "") return false;
    const ms = delayMsOf(text, delays);
    if (ms === "motion") return false;
    return ms === null || ms >= NOTICE_MS;
  });
}

/**
 * Rule 1: a file that names the library, whether through `from`, `import()` or `require()`
 * (#389: the last two went unseen).
 */
export function importsSonner(text: string): boolean {
  return /["']sonner["']/.test(text);
}

/**
 * Rule 2: the lane's own class names, `notify-body`, `notify-title`, ... A word that happens to
 * end in `notify` before the hyphen (`re-notify-x`) is another name, not this one (#389).
 */
export function drawsToastClass(text: string): boolean {
  return /(?<![\w-])notify-[a-z]/.test(text);
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
      if (importsSonner(text)) offenders.push(rel);
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
      if (drawsToastClass(text)) offenders.push(rel);
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

    it("catches a delay named by a const in the same file (#389)", () => {
      const named = [
        "const UNDO_MS = 8000;",
        "const clearIt = () => setTimeout(() => setUndo(null), UNDO_MS);",
      ].join("\n");
      expect(noticeClocks(named)).toHaveLength(1);
      // Under two seconds it is an animation flag, named or not.
      expect(noticeClocks("const POP_MS = 260;\nsetTimeout(() => setFavPop(null), POP_MS);")).toEqual([]);
    });

    it("catches a clear on a delay this file cannot read (#389)", () => {
      for (const line of [
        "setTimeout(() => setNotice(null), holdMs);",
        "setTimeout(() => setNotice(null), OTHER_MS);",
        "setTimeout(() => setNotice(null), life.left + 10);",
      ]) {
        expect(noticeClocks(line), line).toHaveLength(1);
      }
    });

    it("catches the other ways a timer takes a message away (#389)", () => {
      for (const line of [
        "setTimeout(() => setNotice(undefined), 8000);",
        'setTimeout(() => setNotice(""), 8000);',
        "setTimeout(clearNotice, 8000);",
        "setTimeout(() => dismissNotice(), 8000);",
        "setTimeout(() => hide(), 3000);",
        "setTimeout(hideToast, 8000);",
      ]) {
        expect(noticeClocks(line), line).toHaveLength(1);
      }
    });

    it("leaves an animation flag, a debounce and a wait alone", () => {
      for (const fine of [
        "setTimeout(() => setFavPop(null), 260);",
        "setTimeout(() => setSettled(true), 240);",
        "const timer = setTimeout(() => setSilent(true), TRAY_TIMEOUT_MS);",
        "setTimeout(() => void ask(), 1_000);",
        // A debounce that flushes: no message is cleared, whatever its delay.
        "setTimeout(() => void flush(), PATCH_DEBOUNCE_MS);",
        "const PATCH_DEBOUNCE_MS = 350;\ntimer.current = setTimeout(() => void flush(), PATCH_DEBOUNCE_MS);",
        // A long animation flag: the state it sets is not a message going away.
        "const UNDO_MS = 8000;\nsetTimeout(() => setSettled(true), UNDO_MS);",
        // A panel's exit: the delay is a motion time read out of the stylesheet.
        'const ms = motionOffNow() ? 0 : readCssMs("--dur-exit", 90);\nsetTimeout(() => { setRendered(false); setClosing(false); }, ms);',
        // The timers' own API is not a message.
        "setTimeout(() => clearTimeout(timer), 3000);",
      ]) {
        expect(noticeClocks(fine), fine).toEqual([]);
      }
    });

    it("catches a side door into the library, and leaves a plain mention alone (#389)", () => {
      for (const bad of [
        'import { toast } from "sonner";',
        'const { toast } = await import("sonner");',
        'const { toast } = require("sonner");',
        "import type { ToasterProps } from 'sonner';",
      ]) {
        expect(importsSonner(bad), bad).toBe(true);
      }
      // A comment that names the library without quoting it is prose, not a second policy.
      expect(importsSonner("// sonner draws the lane, behind components/Notify.tsx")).toBe(false);
    });

    it("catches a second toast body, and leaves a name that ends in notify alone (#389)", () => {
      for (const bad of ['className="notify-body"', "`notify-title`", "a notify-close b"]) {
        expect(drawsToastClass(bad), bad).toBe(true);
      }
      for (const fine of ['className="re-notify-x"', 'className="not-notify-body"', 'className="unnotify-me"']) {
        expect(drawsToastClass(fine), fine).toBe(false);
      }
    });
  });
});
