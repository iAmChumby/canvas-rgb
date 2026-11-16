// The editor's copy of the scene's HTML limit is the daemon's, in number and in words (#370).
//
// The limit is one constant in the spine and one sentence in the daemon, and the editor keeps a
// copy of both so a picker row can say why before the owner asks. A copy is a drift risk, so this
// test reads both Rust files and fails when the two no longer agree. jsdom cannot see Rust; only
// these reads can.

import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { htmlLimitSentence, SCENE_HTML_MAX } from "@/lib/htmlLimit";

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
const SPINE = read("crates/canvas-proto/src/scene.rs");
const DAEMON = read("crates/canvas-daemon/src/scenes.rs");

/** `SCENE_HTML_MAX` as the spine sets it. */
function spineLimit(): number {
  const found = /pub const SCENE_HTML_MAX: usize = (\d+);/.exec(SPINE);
  if (!found) throw new Error("SCENE_HTML_MAX is not in crates/canvas-proto/src/scene.rs");
  return Number(found[1]);
}

/** The daemon's `html_limit_sentence`, with its format's slots filled in as Rust fills them. */
function daemonSentence(count: number): string {
  const found = /"A scene runs \{SCENE_HTML_MAX\} HTML effect\{\} at a time\. This one has \{count\}\. Move one to another scene\."/.exec(
    DAEMON,
  );
  if (!found) throw new Error("html_limit_sentence's words moved in crates/canvas-daemon/src/scenes.rs");
  const max = spineLimit();
  return found[0]
    .slice(1, -1)
    .replace("{SCENE_HTML_MAX}", String(max))
    .replace("{}", max === 1 ? "" : "s")
    .replace("{count}", String(count));
}

describe("the scene's HTML limit copy (#370)", () => {
  it("the editor's limit is the spine's", () => {
    expect(SCENE_HTML_MAX).toBe(spineLimit());
  });

  it("a scene runs two, and the picker's sentence is the daemon's", () => {
    expect(SCENE_HTML_MAX).toBe(2);
    for (const count of [1, 2, 3, 5]) {
      expect(htmlLimitSentence(count)).toBe(daemonSentence(count));
    }
  });
});
