import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * The class fix for #258: a plain "Loading…", "Reading…" or "Waiting…" sentence is the wait the
 * editor used to draw in a dozen different ways. A wait is now drawn by the loading kit (the busy
 * Button, InlineSkeleton, Select `loading`, the canvas placeholders, LoadFrame). This test reads
 * every .ts and .tsx under src and fails on a new hard-coded wait sentence, so the next one is
 * written with the kit and not with a line of text.
 *
 * What it matches: a wait word (`Loading`, `Reading`, ...) and then an ellipsis (`…`, `&hellip;`,
 * `…`, `...`) on the same line, with anything between them (quotes, braces, a template
 * `${...}`), and the same pair inside one JSX text run that is split across lines (the run is
 * joined before it is matched). Comments are stripped first, so prose about a wait is free.
 *
 * What it allows: a stage or screen-reader label that carries no ellipsis ("Loading the effects",
 * "Waking the mouse"), which the pattern does not match, and the explicit `ALLOWED` list below:
 * one entry per file and sentence, each with the reason it is a stage label and not a wait drawn as
 * text. There is no whole-file exemption: `components/Loading.tsx` is scanned like every other file.
 *
 * `PLAIN_WAITS_ROOT` points the scan at another copy of `src` (used to show the test failing on
 * the tree before the kit).
 */

const ROOT = process.env.PLAIN_WAITS_ROOT ?? resolve(process.cwd(), "src");

const WAIT_WORDS = [
  "Loading",
  "Reading",
  "Waiting",
  "Waking",
  "Connecting",
  "Fetching",
  "Preparing",
  "Updating",
  "Downloading",
  "Checking",
  "Saving",
  "Sending",
  "Applying",
  "Please wait",
  // Wait words the first version of this test already knew.
  "Looking",
  "Rendering",
  "Retrying",
  "Starting",
  "Restoring",
  "Searching",
  "Scanning",
];
const ELLIPSIS = "(?:…|&hellip;|\\\\u2026|\\.\\.\\.)";
/** A wait word, then within 80 characters (any kind, quotes and braces included) an ellipsis. */
const PLAIN_WAIT = new RegExp(`\\b(?:${WAIT_WORDS.join("|")})\\b[^\\n]{0,80}?${ELLIPSIS}`);

/** Sentences that are allowed, by file (relative to src) and the matched text. Empty today. */
const ALLOWED: { file: string; text: string; why: string }[] = [];

function walk(dir: string): string[] {
  return (readdirSync(dir, { withFileTypes: true }) as { name: string; isDirectory(): boolean }[]).flatMap((e) => {
    const path = `${dir}/${e.name}`;
    if (e.isDirectory()) return e.name === "__tests__" || e.name === "proto" ? [] : walk(path);
    return /\.(ts|tsx)$/.test(e.name) && !/\.test\.tsx?$/.test(e.name) ? [path] : [];
  });
}

/**
 * The source with its comments blanked (same line numbers). Block comments go first; a `//` starts
 * a line comment unless it follows a colon (a URL) or sits inside a quote on its line.
 */
export function stripComments(source: string): string {
  const noBlock = source.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, " "));
  return noBlock
    .split("\n")
    .map((line) => {
      let quote: string | null = null;
      for (let i = 0; i < line.length; i++) {
        const c = line[i];
        if (quote) {
          if (c === "\\") i++;
          else if (c === quote) quote = null;
        } else if (c === '"' || c === "'" || c === "`") quote = c;
        else if (c === "/" && line[i + 1] === "/" && line[i - 1] !== ":") return line.slice(0, i);
      }
      return line;
    })
    .join("\n");
}

/** Every wait sentence in one source: same-line matches, and matches inside a joined JSX text run. */
export function plainWaits(source: string): { line: number; text: string }[] {
  const clean = stripComments(source);
  const hits: { line: number; text: string }[] = [];
  clean.split("\n").forEach((line, i) => {
    const m = PLAIN_WAIT.exec(line);
    if (m) hits.push({ line: i + 1, text: m[0] });
  });
  // A JSX text run: the characters between `>` and the next `<`, with no braces. Joined across
  // lines, so a sentence broken by the formatter is still one sentence.
  for (const run of clean.matchAll(/>([^<>{}]*\n[^<>{}]*)</g)) {
    const text = run[1]!.replace(/\s+/g, " ").trim();
    const m = PLAIN_WAIT.exec(text);
    if (m) hits.push({ line: clean.slice(0, run.index).split("\n").length, text: m[0] });
  }
  return hits;
}

describe("no hard-coded wait text outside the loading kit", () => {
  it("finds the source tree", () => {
    expect(walk(ROOT).length).toBeGreaterThan(80);
  });

  it("no source file draws a wait as a plain sentence", () => {
    const hits: string[] = [];
    for (const file of walk(ROOT)) {
      const rel = file.slice(ROOT.length + 1).replace(/\\/g, "/");
      for (const hit of plainWaits(readFileSync(file, "utf8"))) {
        if (ALLOWED.some((a) => a.file === rel && a.text === hit.text)) continue;
        hits.push(`${rel}:${hit.line}  ${hit.text}`);
      }
    }
    expect(hits, `A wait is drawn by the loading kit, not by a sentence:\n${hits.join("\n")}`).toEqual([]);
  });

  describe("the matcher", () => {
    it("catches the sentences it exists for", () => {
      for (const bad of [
        'const x = "Loading…";',
        "<p>Reading…</p>",
        "<p>Waiting for the device…</p>",
        "<p>Loading the editor&hellip;</p>",
        'label: "Saving..."',
        'title="Waking up your lights…"',
        "`Checking ${what}…`",
        '{ text: "Please wait..." }',
        'busy ? "Fetching the list…" : "Fetch"',
        "'Preparing the update\\u2026'",
      ]) {
        expect(plainWaits(bad).length, bad).toBeGreaterThan(0);
      }
    });

    it("catches a JSX text run split across lines", () => {
      const source = "<p className=\"dim\">\n  Loading the\n  effect's properties&hellip;\n</p>";
      expect(plainWaits(source).length).toBeGreaterThan(0);
    });

    it("leaves a stage or label with no ellipsis, and comments, alone", () => {
      for (const fine of [
        '<LoadFrame label="Loading the effects" />',
        'stage: { label: "Waking the mouse" }',
        '"Reading settings · 3 of 5"',
        'const url = "http://example.test/loading";',
        "// Loading… is what this used to say",
        "/* Reading… too */ const a = 1;",
        "/**\n * Saving… in a doc comment\n */\nconst a = 1;",
        "const next = { ...state, loading: true };",
      ]) {
        expect(plainWaits(fine), fine).toEqual([]);
      }
    });
  });
});
