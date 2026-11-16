import { describe, expect, it } from "vitest";
import { PAGE_PATHS } from "@/shell/pagePaths";

// The tray names a page the rail has (app/src-tauri/src/tray.rs). There is no Layout page any
// more (#310): the tray sends `scenes`, and nothing answers to `layout`.
describe("the pages the tray can open", () => {
  it("has no layout page", () => {
    expect(PAGE_PATHS.layout).toBeUndefined();
    expect(PAGE_PATHS.scenes).toBe("/");
  });

  it("opens every other page where the rail has it", () => {
    expect(PAGE_PATHS.effects).toBe("/effects/");
    expect(PAGE_PATHS.devices).toBe("/devices/");
    expect(PAGE_PATHS.fans).toBe("/fans/");
    expect(PAGE_PATHS.settings).toBe("/settings/");
  });
});
