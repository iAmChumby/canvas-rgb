// Where the runtime's line numbers fall in an effect file.
//
// The daemon runs each `<script>` body on its own, so the line and column it reports are
// relative to that body, not to the file (`eval_script:<line>:<col>`,
// `crates/canvas-daemon/src/user_effects.rs`). The `<script>` body starts right after the
// tag's `>`, so the body's line 1 is the rest of the tag's own line
// (`crates/canvas-html/src/parse.rs`, `read_text_until`). The editor marks the gutter in the
// file, so it adds the tag's line to the runtime's.
//
// An effect with several script blocks is read by the runtime one block at a time, and the API
// carries no block index: the first inline block is the one mapped. Every effect in the
// library has one.

/** The first inline `<script>`'s body, and the 0-based line of the tag that opens it. */
export function scriptBody(source: string): { body: string; lineStart: number } | null {
  const lower = source.toLowerCase();
  const at = lower.indexOf("<script");
  if (at < 0) return null;
  const open = source.indexOf(">", at);
  if (open < 0) return null;
  const close = lower.indexOf("</script", open);
  if (close < 0) return null;
  return {
    body: source.slice(open + 1, close),
    lineStart: source.slice(0, open + 1).split("\n").length - 1,
  };
}

/**
 * The file line (1-based) a runtime's script-relative line falls on. A source with no script
 * block keeps the line it was given.
 */
export function sourceLineOf(source: string, scriptLine: number): number {
  const script = scriptBody(source);
  return script ? script.lineStart + scriptLine : scriptLine;
}
