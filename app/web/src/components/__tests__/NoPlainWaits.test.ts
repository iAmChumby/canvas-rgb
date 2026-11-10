import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * The class fix for #258: a plain "Loading…", "Reading…" or "Waiting…" sentence is the wait the
 * editor used to draw in a dozen different ways. A wait is now drawn by the loading kit (the busy
 * Button, InlineSkeleton, Select `loading`, the canvas placeholders, LoadFrame). This test reads
 * every .tsx under src and fails on a new hard-coded wait sentence, so the next one is written
 * with the kit and not with a line of text.
 *
 * Allowed: `components/Loading.tsx` (the kit's home), and a screen-reader label or stage string
 * that carries no ellipsis (`label="Loading the effects"`, "Waking the mouse"), which this pattern
 * does not match. `PLAIN_WAITS_ROOT` points the scan at another copy of `src` (used to show the
 * test failing on the tree before the kit).
 */

const ROOT = process.env.PLAIN_WAITS_ROOT ?? resolve(process.cwd(), "src");

/** Words that name a wait, followed within one phrase by an ellipsis: "Loading the editor…". */
const WAIT_WORDS =
  "Loading|Reading|Waiting|Looking|Rendering|Connecting|Saving|Sending|Applying|Checking|Retrying|Starting|Restoring|Searching|Scanning";
const PLAIN_WAIT = new RegExp(`\\b(${WAIT_WORDS})\\b[^\\n"'\`<>{}]{0,40}(…|&hellip;|\\\\u2026|\\.\\.\\.)`);

/** Files that may hold the words: the kit's own home. */
const ALLOWED = ["components/Loading.tsx"];

function walk(dir: string): string[] {
  return (readdirSync(dir, { withFileTypes: true }) as { name: string; isDirectory(): boolean }[]).flatMap((e) => {
    const path = `${dir}/${e.name}`;
    if (e.isDirectory()) return e.name === "__tests__" || e.name === "proto" ? [] : walk(path);
    return e.name.endsWith(".tsx") ? [path] : [];
  });
}

/** A line that is only a comment: the words there describe code and are never drawn. */
const isComment = (line: string) => /^\s*(\*|\/\/|\/\*|\{\/\*)/.test(line);

describe("no hard-coded wait text outside the loading kit", () => {
  it("finds the source tree", () => {
    expect(walk(ROOT).length).toBeGreaterThan(40);
  });

  it("no .tsx draws a wait as a plain sentence", () => {
    const hits: string[] = [];
    for (const file of walk(ROOT)) {
      const rel = file.slice(ROOT.length + 1).replace(/\\/g, "/");
      if (ALLOWED.includes(rel)) continue;
      readFileSync(file, "utf8")
        .split("\n")
        .forEach((line, i) => {
          if (isComment(line)) return;
          const m = PLAIN_WAIT.exec(line);
          if (m) hits.push(`${rel}:${i + 1}  ${m[0]}`);
        });
    }
    expect(hits, `A wait is drawn by the loading kit, not by a sentence:\n${hits.join("\n")}`).toEqual([]);
  });

  it("the pattern catches the sentences it exists for", () => {
    for (const bad of ["Loading…", "Reading…", "Waiting for the device…", "Loading the editor&hellip;", "Saving...", "Rendering…"]) {
      expect(PLAIN_WAIT.test(bad), bad).toBe(true);
    }
    // A screen-reader label or a stage line has no ellipsis.
    for (const fine of ["Loading the effects", "Waking the mouse", "Reading settings · 3 of 5", "Waiting for the device"]) {
      expect(PLAIN_WAIT.test(fine), fine).toBe(false);
    }
  });
});
