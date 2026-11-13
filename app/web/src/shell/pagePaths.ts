/**
 * The pages the tray can ask the editor to open, by the name the tray sends, and where each one
 * lives. There is no `layout` page any more (#310): Layout is gone, and the tray sends `scenes`.
 */
export const PAGE_PATHS: Record<string, string> = {
  scenes: "/",
  effects: "/effects/",
  devices: "/devices/",
  fans: "/fans/",
  settings: "/settings/",
  updates: "/updates/",
};
