// The editor's copy of the spine's names for the group preview (#378): the route and the two
// query keys.
//
// The spine owns them (`crates/canvas-proto`), and the editor keeps a copy, so a copy is a
// drift risk. Nothing generates these files: ts-rs writes the types, not the routes and the
// constants. So this test reads the Rust files and fails when the two no longer agree. jsdom
// cannot see Rust; only these reads can.

import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { GROUP_PREVIEW_QUERY, PREVIEW_TOKEN_QUERY } from "@/proto/consts";
import { routes } from "@/proto/routes";

/** The repository's root: the first folder above this one that holds the spine. */
function repoRoot(): string {
  let dir = process.cwd();
  for (let up = 0; up < 6; up += 1) {
    if (existsSync(resolve(dir, "crates/canvas-proto/src/scene.rs"))) return dir;
    dir = resolve(dir, "..");
  }
  throw new Error(`cannot find the repository from ${process.cwd()}`);
}
const REPO = repoRoot();
const read = (path: string) => readFileSync(resolve(REPO, path), "utf8").replace(/\r\n/g, "\n");
const API = read("crates/canvas-proto/src/api.rs");
const CONSTS = read("crates/canvas-proto/src/consts.rs");

/** The route as the spine sets it: `pub const PREVIEW_GROUP: &str = "...";`. */
function spineRoute(): string {
  const found = /pub const PREVIEW_GROUP: &str = "([^"]+)";/.exec(API);
  if (!found) throw new Error("PREVIEW_GROUP is not in crates/canvas-proto/src/api.rs");
  return found[1];
}

/** A query key as the spine sets it: `pub const NAME: &str = "...";`. */
function spineQueryKey(name: string): string {
  const found = new RegExp(`pub const ${name}: &str = "([^"]+)";`).exec(CONSTS);
  if (!found) throw new Error(`${name} is not in crates/canvas-proto/src/consts.rs`);
  return found[1];
}

describe("the spine's names, as the editor keeps them (#378)", () => {
  it("the group preview's route is the spine's", () => {
    expect(routes.previewGroup).toBe(spineRoute());
  });

  it("the preview's two query keys are the spine's", () => {
    expect(PREVIEW_TOKEN_QUERY).toBe(spineQueryKey("PREVIEW_TOKEN_QUERY"));
    expect(GROUP_PREVIEW_QUERY).toBe(spineQueryKey("GROUP_PREVIEW_QUERY"));
  });
});
