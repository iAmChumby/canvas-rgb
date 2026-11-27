// Class guard for #576/#575 (P9, in the style of toastGuard.test.ts): error text reaches a page
// only through `ErrorState`/`ErrorLine` in components/ErrorState.tsx. The scan reads source as
// text, so a copy of the old pattern cannot land. `ERROR_GUARD_ROOT` points it at another copy of
// `src`, which is how the failures on the tree before this fix are shown.

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

/** The raw helper call: views and components take text through `errorText` instead. */
export function rawMessageOf(source: string): boolean {
  return /\bmessageOf\s*\(/.test(source);
}

/** A hand-written error element, a plain `.message` read, `String(e)`, or a bare JSX error. */
export function violations(source: string): string[] {
  const found: string[] = [];
  if (rawMessageOf(source)) found.push("messageOf(");
  for (const match of source.matchAll(/<(?:p|span)\s+className=(?:"[^"]*error-line|`[^`]*error-line)/g)) {
    found.push(match[0].replace(/\s+/g, " "));
  }
  for (const match of source.matchAll(/\b(?:e|err|error)\s*\.\s*message\b/g)) found.push(match[0]);
  for (const match of source.matchAll(/\bString\s*\(\s*(?:e|err|error)\s*\)/g)) found.push(match[0]);
  for (const match of source.matchAll(/<(?:p|span|div)[^>]*>\{(?:e|err|error)\}<\/(?:p|span|div)>/g)) {
    found.push(match[0].replace(/\s+/g, " "));
  }
  return found;
}

describe("every error has one anatomy (#576, P9)", () => {
  it("finds the views and components", () => {
    expect(guarded().length).toBeGreaterThan(80);
  });

  it("keeps messageOf and raw error text inside ErrorState", () => {
    const offenders: string[] = [];
    for (const file of guarded()) {
      const name = nameOf(file);
      if (name === ALLOWED) continue;
      for (const hit of violations(readFileSync(file, "utf8"))) offenders.push(`${name}: ${hit}`);
    }
    expect(offenders, "Error text reaches the page only through ErrorState/ErrorLine").toEqual([]);
  });

  describe("the matcher", () => {
    it("catches three planted violations", () => {
      expect(violations("setErr(messageOf(e));")).toContain("messageOf(");
      expect(violations("<p className=\"error-line\">{err}</p>").length).toBeGreaterThan(0);
      expect(violations("return { state: 'refused', message: e.message };")).toContain("e.message");
      expect(violations("const text = String(error);")).toContain("String(error)");
      expect(violations("<div className=\"load-note\">{error}</div>").length).toBeGreaterThan(0);
    });

    it("leaves the anatomy and its callers alone", () => {
      expect(violations("import { errorText } from \"@/lib/errors\";")).toEqual([]);
      expect(violations("setErr(errorText(e));")).toEqual([]);
      expect(violations("<ErrorLine>{err}</ErrorLine>")).toEqual([]);
      expect(violations("<ErrorState error={e} onRetry={load} />")).toEqual([]);
      expect(violations("patternRefusal.message")).toEqual([]);
    });
  });
});
