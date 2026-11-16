// The group preview's own facts (#378): the size of the picture a group sends, and the close
// code the daemon sends when a newer group preview takes the place. The daemon's own numbers
// are in crates/canvas-proto (`PREVIEW_MAX_FPS`, `routes::PREVIEW_GROUP`).

/** The width of a group's own canvas, as the daemon sends it. */
export const GROUP_PREVIEW_W = 80;

/** The height of a group's own canvas, as the daemon sends it. */
export const GROUP_PREVIEW_H = 50;

/** The close code the daemon sends when a newer group preview takes the place. The socket
 *  that gets it is not tried again by itself: the stage says the preview stopped and the
 *  owner starts it again by hand. */
export const GROUP_TAKEN_CODE = 1013;
