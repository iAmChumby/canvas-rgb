// Task 11: each light's row on Devices renames it, says whether it is a strip or a ring (where
// the hardware says linear) and turns it around, all through the lights route.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { patchSnap, refreshAll } from "@/lib/store";
import { DevicesView } from "@/views/DevicesView";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  globalThis.IntersectionObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  } as unknown as typeof IntersectionObserver;
  Element.prototype.scrollTo = (() => {}) as typeof Element.prototype.scrollTo;
  Element.prototype.scrollIntoView = (() => {}) as typeof Element.prototype.scrollIntoView;
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  patchSnap({ devices: null, devicesTotal: null, devicesError: null });
});

describe("a light's row on Devices", () => {
  it("says a light with no LEDs is not sized, not zero LEDs", async () => {
    window.history.replaceState({}, "", "/devices/?state=complete");
    await act(async () => {
      await refreshAll();
    });
    render(<DevicesView />);
    await screen.findAllByTestId("device-card");
    expect(screen.getAllByText("Not sized").length).toBeGreaterThan(0);
    expect(document.body.textContent).not.toMatch(/\b0 LEDs\b/);
    expect(document.body.textContent).not.toMatch(/\b0–240\b/);
  });

  it("sets ring and reversed through the lights route", async () => {
    window.history.replaceState({}, "", "/devices/");
    await act(async () => {
      await refreshAll();
    });
    const patch = vi.spyOn(api.lights, "patch").mockImplementation(async (id, body) => ({
      light: {
        id,
        device_id: "openrgb:strip-1",
        zone_index: 0,
        arrangement: body.arrangement ?? "strip",
        reversed: body.reversed ?? false,
      },
      led_count: 60,
      connected: true,
      label: "Arctic Liquid Freezer III · Strip",
    }));
    render(<DevicesView />);
    // The freezer's one zone is linear, so its light may be a strip or a ring.
    const card = (await screen.findAllByTestId("device-card")).find(
      (c) => c.getAttribute("aria-label") === "Arctic Liquid Freezer III",
    );
    expect(card).toBeTruthy();
    const row = within(card as HTMLElement);
    await act(async () => {
      fireEvent.click(row.getByRole("radio", { name: "Ring" }));
    });
    await waitFor(() => expect(patch).toHaveBeenCalledWith("openrgb:strip-1#z0", { arrangement: "ring" }));
    await act(async () => {
      fireEvent.click(row.getByRole("switch", { name: "Runs the other way" }));
    });
    await waitFor(() => expect(patch).toHaveBeenCalledWith("openrgb:strip-1#z0", { reversed: true }));
  });
});
