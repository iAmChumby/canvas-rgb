// The opener of the host bridge (#365 fix pass): the notes come from a release feed, so an
// address of any other shape is never started. The rule is checked here, on the real function,
// because the notes' links are only one caller of it.

import { afterEach, describe, expect, it, vi } from "vitest";
import { openExternal } from "@/lib/host";

afterEach(() => {
  delete window.__TAURI__;
  vi.restoreAllMocks();
});

describe("opening an address outside the app", () => {
  it("never asks the tray's opener for an address that is not https", async () => {
    const invoke = vi.fn(async () => null);
    window.__TAURI__ = { core: { invoke }, event: { listen: vi.fn(async () => () => undefined) } };
    await openExternal("javascript:alert(1)");
    await openExternal("http://example.com");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("opens no window for an address that is not https in a plain browser", async () => {
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    await openExternal("javascript:alert(1)");
    await openExternal("http://example.com");
    expect(open).not.toHaveBeenCalled();
  });

  it("asks the tray's opener for an https address", async () => {
    const invoke = vi.fn(async () => null);
    window.__TAURI__ = { core: { invoke }, event: { listen: vi.fn(async () => () => undefined) } };
    await openExternal("https://github.com/iAmChumby/canvas-rgb/issues/365");
    expect(invoke).toHaveBeenCalledWith("plugin:opener|open_url", {
      url: "https://github.com/iAmChumby/canvas-rgb/issues/365",
    });
  });
});
