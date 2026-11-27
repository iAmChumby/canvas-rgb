// Class guard for #576/#575 (in the style of `toastGuard.test.ts`): an error the editor shows has
// one anatomy. `messageOf` is the `ErrorState` component's helper, so a view must take its text
// through `errorText` and render it with `ErrorState` or its compact `ErrorLine`. A bare dynamic
// `<p className="error-line">` is the old hand-rolled shape this change removes.
//
// The scan reads source as text, so a copy of the old pattern cannot land. `ERROR_GUARD_ROOT`
// points it at another copy of `src`, which is how the failure on the tree before this fix is
// shown.

import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = process.env.ERROR_GUARD_ROOT ?? resolve(process.cwd(), "src");

/** Only the views: that is the surface the migration covers. */
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

function views(): string[] {
  return sources(join(ROOT, "views"));
}

function nameOf(file: string): string {
  return relative(ROOT, file).split(sep).join("/");
}

/** A call to the old helper, including an import from the errors module. */
export function rawMessageOf(source: string): boolean {
  return /\bmessageOf\s*\(/.test(source);
}

/** A hand-written error paragraph with a value interpolated into it. */
export function bareErrorLine(source: string): string[] {
  const found: string[] = [];
  const pattern = /<p\s+className="error-line[^"]*"[^>]*>\{[^<]*\}<\/p>/g;
  for (const match of source.matchAll(pattern)) found.push(match[0].replace(/\s+/g, " "));
  return found;
}

describe("every error has one anatomy (#576)", () => {
  it("finds the views", () => {
    expect(views().length).toBeGreaterThan(30);
  });

  it("keeps messageOf inside ErrorState", () => {
    const offenders: string[] = [];
    for (const file of views()) {
      if (rawMessageOf(readFileSync(file, "utf8"))) offenders.push(nameOf(file));
    }
    expect(
      offenders,
      "A view takes error text through errorText and renders it with ErrorState/ErrorLine",
    ).toEqual([]);
  });

  it("never writes a bare dynamic error paragraph", () => {
    const offenders: string[] = [];
    for (const file of views()) {
      for (const line of bareErrorLine(readFileSync(file, "utf8"))) {
        offenders.push(`${nameOf(file)}: ${line}`);
      }
    }
    expect(offenders, "The compact line is ErrorLine from components/ErrorState.tsx").toEqual([]);
  });

  describe("the matcher", () => {
    it("catches the old helper call", () => {
      expect(rawMessageOf("setErr(messageOf(e));")).toBe(true);
      expect(rawMessageOf("import { ApiFailure, messageOf } from \"@/lib/errors\";")).toBe(false);
      expect(rawMessageOf("setErr(errorText(e));")).toBe(false);
    });

    it("catches a dynamic error paragraph and leaves the component alone", () => {
      expect(bareErrorLine("{err && <p className=\"error-line\">{err}</p>}")).toHaveLength(1);
      expect(
        bareErrorLine("<p className=\"error-line\" style={{ marginTop: 8 }}>{err}</p>"),
      ).toHaveLength(1);
      expect(bareErrorLine("<p className=\"error-line\">This effect is gone.</p>")).toEqual([]);
      expect(bareErrorLine("<ErrorLine>{err}</ErrorLine>")).toEqual([]);
    });
  });
});
