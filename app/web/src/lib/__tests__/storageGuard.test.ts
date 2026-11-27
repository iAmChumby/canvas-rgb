// @vitest-environment node
// Guard for #579 (R3): a preview writes no browser storage, so every write goes through
// `lib/storage.ts`, which does nothing in preview. This test reads every shipped `.ts`/`.tsx`
// under `src` as text, like `toastGuard.test.ts`, and fails on a write made anywhere else.
//
// `STORAGE_GUARD_ROOT` points it at another copy of `src`, which is how the failure on the tree
// before the helper is shown.

import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = process.env.STORAGE_GUARD_ROOT ?? resolve(process.cwd(), "src");

/** The one file allowed to write. */
const HELPER = "lib/storage.ts";

/**
 * A write: `setItem` or `removeItem` on anything (an alias of the store is still the store), or
 * `clear()` and a keyed assignment on one of the two stores by name.
 */
const WRITE =
  /\.\s*(?:setItem|removeItem)\b|\[\s*["'`](?:setItem|removeItem)["'`]\s*\]|(?:local|session)Storage\s*(?:\.\s*clear\s*\(|\[[^\]]*\]\s*=(?!=))/;

function sources(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "__tests__" || entry.name === "proto" || entry.name === "test") continue;
      out.push(...sources(path));
    } else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
      out.push(path);
    }
  }
  return out.sort();
}

describe("every storage write goes through lib/storage.ts (#579)", () => {
  it("finds the source tree", () => {
    expect(sources(ROOT).length).toBeGreaterThan(80);
  });

  it("no other file writes localStorage or sessionStorage", () => {
    const offenders: string[] = [];
    for (const file of sources(ROOT)) {
      const rel = relative(ROOT, file).replace(/\\/g, "/");
      if (rel === HELPER) continue;
      readFileSync(file, "utf8")
        .split("\n")
        .forEach((line, i) => {
          if (WRITE.test(line)) offenders.push(`${rel}:${i + 1}: ${line.trim()}`);
        });
    }
    expect(offenders, "Write through storeLocal, removeLocal or storeSession").toEqual([]);
  });

  it("the matcher catches the shapes of a direct write", () => {
    for (const bad of [
      "window.localStorage.setItem(THEME_KEY, theme);",
      "localStorage.removeItem(CHAINS_LATER_KEY);",
      'window.sessionStorage.setItem(SHOWN_KEY, "1");',
      "store?.setItem(key, value);",
      'localStorage["canvas-rgb.theme"] = "dark";',
      "window.localStorage.clear();",
    ]) {
      expect(WRITE.test(bad), bad).toBe(true);
    }
    for (const fine of [
      "window.localStorage.getItem(KEY)",
      "storeLocal(KEY, value);",
      "removeLocal(KEY);",
      'if (localStorage["x"] === "1") return;',
    ]) {
      expect(WRITE.test(fine), fine).toBe(false);
    }
  });
});
