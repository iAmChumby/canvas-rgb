// @vitest-environment node
// Class guard for #171 and #481 (the same mistake twice): a `mounted`/`alive` ref that the unmount
// cleanup sets to false and nothing ever sets back to true. React's StrictMode mounts, runs the
// cleanup and mounts again on the same instance, so a guard that starts true and is only ever
// cleared stays false for the component's whole life: every answer or draft it guards is dropped
// (#171: the effect preview never showed; #481: the pattern editor never put a draft on the lights).
// `next.config.ts` sets `reactStrictMode: true`, so `next dev` and any test under `<StrictMode>` hit it.
//
// The rule, read from the source like `toastGuard.test.ts` and `editorGuard.test.ts`: a file that
// writes `<flag>.current = false` for a lifecycle flag (`mounted`, `alive`, `isMounted`) must also
// write `<flag>.current = true`, which is the effect body's job (`EffectEditor.tsx`, `FanBulk.tsx`).
// The behaviour itself is tested under StrictMode in `EffectEditorStrictMode.test.tsx` and
// `views/patterns/__tests__/PatternGesture491.test.tsx`.
//
// `MOUNT_GUARD_ROOT` points the scan at another copy of `src`, which is how the failure on the
// tree before the fix is shown.

import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

// `src`, found from this file so the scan does not depend on the directory vitest was started in.
const ROOT = process.env.MOUNT_GUARD_ROOT ?? fileURLToPath(new URL("..", import.meta.url));

const FLAGS = ["mounted", "alive", "isMounted"];

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
  return out;
}

/** The flags a source clears (`flag.current = false`) without ever setting them (`flag.current = true`). */
export function clearedNeverSet(text: string): string[] {
  return FLAGS.filter((flag) => {
    const clears = new RegExp(`\\b${flag}\\.current\\s*=\\s*false\\b`).test(text);
    const sets = new RegExp(`\\b${flag}\\.current\\s*=\\s*true\\b`).test(text);
    return clears && !sets;
  });
}

describe("lifecycle flags survive StrictMode's mount, cleanup, mount (#171, #481)", () => {
  it("a cleanup-only flag is caught", () => {
    const bad = "useEffect(() => () => { mounted.current = false; }, []);";
    const good = "useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);";
    expect(clearedNeverSet(bad)).toEqual(["mounted"]);
    expect(clearedNeverSet(good)).toEqual([]);
  });

  it("no file clears a mounted/alive ref that it never sets back", () => {
    const offenders = sources(ROOT)
      .map((path) => ({ path: relative(ROOT, path).replaceAll("\\", "/"), flags: clearedNeverSet(readFileSync(path, "utf8")) }))
      .filter((file) => file.flags.length > 0)
      .map((file) => `${file.path}: ${file.flags.join(", ")}`);
    expect(offenders).toEqual([]);
  });
});
