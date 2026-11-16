// Mirrors crates/canvas-proto/src/consts.rs. Spine file: edit only through the spine owner.
// The names the API and the editor share, so neither side writes them out by hand.

/** The preview WebSocket cannot carry headers from a browser; it takes `?token=`. */
export const PREVIEW_TOKEN_QUERY = "token";

/** The query key that names the group on the per-group preview socket (#378). Its value is a
 *  group id. */
export const GROUP_PREVIEW_QUERY = "group";
