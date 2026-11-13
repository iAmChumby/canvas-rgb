/**
 * The pages the tray can ask the editor to open, by the name the tray sends, and where each one
 * lives. `layout` stays an alias of the Scenes page (`/`) until the tray stops sending it
 * (Task 12): a window left open on the editor would otherwise ignore the tray's "layout".
 */
export const PAGE_PATHS: Record<string, string> = {
  scenes: "/",
  layout: "/",
  effects: "/effects/",
  devices: "/devices/",
  fans: "/fans/",
  settings: "/settings/",
  updates: "/updates/",
};
