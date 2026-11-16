// The numbers the chain setup states and the daemon enforces: the TypeScript copy of `COPIES_MAX`
// must equal the Rust one, or the screen would accept a count the daemon then refuses.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import { COPIES_MAX, shownTimes, shownWords } from "@/views/chain/limits";

describe("COPIES_MAX", () => {
  it("equals the Rust constant in the spine", () => {
    // The tests run from app/web, two folders below the repository root.
    const spine = resolve(process.cwd(), "../../crates/canvas-proto/src/scene.rs");
    const rust = readFileSync(spine, "utf8");
    const found = /pub const COPIES_MAX: u32 = (\d+);/.exec(rust);
    expect(found, "scene.rs declares COPIES_MAX").not.toBeNull();
    expect(Number(found?.[1])).toBe(COPIES_MAX);
  });
});

describe("how many times a light is shown", () => {
  const light = (copies?: number | null) => ({ light: { copies } });

  it("is once unless the light says 2 or more", () => {
    expect(shownTimes(light())).toBe(1);
    expect(shownTimes(light(null))).toBe(1);
    expect(shownTimes(light(1))).toBe(1);
    expect(shownTimes(light(8))).toBe(8);
  });

  it("is worded only for a mirrored light", () => {
    expect(shownWords(light())).toBe("");
    expect(shownWords(light(1))).toBe("");
    expect(shownWords(light(8))).toBe("shown 8 times");
  });
});
