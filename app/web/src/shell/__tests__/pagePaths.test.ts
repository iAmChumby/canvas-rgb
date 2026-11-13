import { describe, expect, it } from "vitest";
import { PAGE_PATHS } from "@/shell/pagePaths";

// The tray still sends "layout" (app/src-tauri/src/tray.rs); until it stops, the editor must open
// the Scenes page for it, not ignore it.
describe("the pages the tray can open", () => {
  it("keeps `layout` as an alias of the Scenes page", () => {
    expect(PAGE_PATHS.layout).toBe("/");
    expect(PAGE_PATHS.layout).toBe(PAGE_PATHS.scenes);
  });

  it("opens every other page where the rail has it", () => {
    expect(PAGE_PATHS.effects).toBe("/effects/");
    expect(PAGE_PATHS.devices).toBe("/devices/");
    expect(PAGE_PATHS.fans).toBe("/fans/");
    expect(PAGE_PATHS.settings).toBe("/settings/");
  });
});
