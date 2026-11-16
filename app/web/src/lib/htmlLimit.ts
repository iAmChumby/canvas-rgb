// The scene's HTML limit, in the editor's own words (#370).
//
// Two HTML effects may play at a time. Each one is its own script runtime (the slowest group
// there is), and a stack counts one for each of its HTML layers and masks. The number is the
// spine's (`SCENE_HTML_MAX` in `crates/canvas-proto/src/scene.rs`) and the sentence is the
// daemon's (`html_limit_sentence` in `crates/canvas-daemon/src/scenes.rs`), so a row that says
// why and a refusal that says why say the same thing; `htmlLimitCopy.test.ts` reads both files
// and fails when this copy drifts from them.
//
// The daemon is the one that refuses: this copy is what the picker shows before the owner asks.
//
// The spine does not export `SCENE_HTML_MAX` to the web, so this literal is the copy: the number
// is accepted as it is, and `htmlLimitCopy.test.ts` is the guard that keeps it the spine's.

/** How many HTML effects one scene may play at a time (`SCENE_HTML_MAX` in the spine). The type
 *  is `number`, not the literal 2, because the sentence's plural is read from the value. */
export const SCENE_HTML_MAX: number = 2;

/**
 * The daemon's sentence for a scene that would start more HTML runtimes than it may: the limit,
 * the scene's own count, and the fix.
 */
export function htmlLimitSentence(count: number): string {
  const effects = SCENE_HTML_MAX === 1 ? "effect" : "effects";
  return `A scene runs ${SCENE_HTML_MAX} HTML ${effects} at a time. This one has ${count}. Move one to another scene.`;
}
