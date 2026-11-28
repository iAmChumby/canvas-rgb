import { act } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

// The control for `PreviewWalk.test.tsx` (#579): the same world without `?preview=1`. Its
// counters see the daemon, the sockets, the tray and the stores, so the walk's zeros mean
// something.

const world = await vi.hoisted(async () => {
  const { makeWorld } = await import("./previewWorld");
  return makeWorld("/");
});

import { DAEMON } from "./previewWorld";
import { api } from "@/lib/endpoints";
import { isDemo } from "@/lib/host";
import { startPreview, stopPreview } from "@/lib/preview";
import { applyTheme } from "@/shell/prefs";
import { putOffSetup } from "@/views/editor/flow";

describe("the same world without a preview", () => {
  it("counts requests, sockets, tray commands and writes", async () => {
    expect(isDemo()).toBe(false);
    await api.status().catch(() => undefined);
    await act(async () => {
      startPreview();
      await new Promise<void>((r) => setTimeout(r, 50));
    });
    stopPreview();
    applyTheme("light");
    putOffSetup();

    expect(world.fetched.filter((u) => u.includes(DAEMON)).length).toBeGreaterThan(0);
    expect(world.sockets.length).toBeGreaterThan(0);
    expect(world.invoked).toContain("get_api_config");
    expect(world.writes.length).toBeGreaterThan(0);
  });
});
