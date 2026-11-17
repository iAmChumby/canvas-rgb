// Task 11: each light's row on Devices renames it, says whether it is a strip or a ring (where
// the hardware says linear) and turns it around, all through the lights route.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { patchSnap, refreshAll } from "@/lib/store";
import type { DeviceInfo } from "@/proto/DeviceInfo";
import type { LightView } from "@/proto/LightView";
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

  it("a one-LED light offers no shape switch at all", async () => {
    // The mic, keyboard and mouse each have one LED: a strip/ring switch would let the owner
    // persist a Ring arrangement a light of one LED cannot have.
    window.history.replaceState({}, "", "/devices/");
    await act(async () => {
      await refreshAll();
    });
    render(<DevicesView />);
    const cards = await screen.findAllByTestId("device-card");
    for (const name of ["HyperX Quadcast S", "ROYUAN MonsGeek Keyboard", "Logitech G Pro X Superlight"]) {
      const card = cards.find((c) => c.getAttribute("aria-label") === name);
      expect(card).toBeTruthy();
      expect(within(card as HTMLElement).queryByRole("radio")).toBeNull();
      expect(within(card as HTMLElement).queryByText("Strip")).toBeNull();
    }
  });

  it("a grid light shows its own grid shape, not a plain strip/ring switch", async () => {
    const device: DeviceInfo = {
      id: "fake:matrix-1",
      name: "Grid Pad",
      vendor: "Fake",
      kind: "other",
      adapter: "fake",
      adapter_index: 0,
      serial: "matrix-1",
      location: "",
      connected: true,
      zones: [
        {
          index: 0,
          name: "Matrix",
          kind: { type: "matrix", width: 2, height: 2, map: [0, 1, 2, 3] },
          led_count: 4,
          min_leds: 4,
          max_leds: 4,
          resizable: false,
        },
      ],
    };
    const light: LightView = {
      light: {
        id: "fake:matrix-1#z0",
        device_id: device.id,
        zone_index: 0,
        leds: { start: 0, count: 4 },
        name: "Matrix",
        arrangement: "grid",
        reversed: false,
      },
      led_count: 4,
      connected: true,
      label: "Grid Pad · Matrix",
    };
    patchSnap({
      devices: [device],
      devicesTotal: 1,
      devicesError: null,
      lights: [light],
      chains: [],
      status: null,
      statusError: null,
    });
    render(<DevicesView />);
    const card = await screen.findByTestId("device-card");
    expect(within(card).queryByText("Strip")).toBeNull();
    expect(within(card).getByRole("radio", { name: "Grid" })).toBeTruthy();
  });

  it("identifies the zone's LED for the first LED of a light that starts past the zone's first", async () => {
    // A placed slice: the light's LED 0 is the zone's LED 12, and first_led counts inside it.
    const device: DeviceInfo = {
      id: "fake:slice-1",
      name: "Slice Strip",
      vendor: "Fake",
      kind: "other",
      adapter: "fake",
      adapter_index: 0,
      serial: "slice-1",
      location: "",
      connected: true,
      zones: [{ index: 0, name: "Strip", kind: { type: "linear" }, led_count: 24, min_leds: 1, max_leds: 24, resizable: false }],
    };
    const view: LightView = {
      light: {
        id: "fake:slice-1#z0",
        device_id: device.id,
        zone_index: 0,
        leds: { start: 12, count: 12 },
        arrangement: "strip",
        reversed: false,
        first_led: 3,
      },
      led_count: 12,
      connected: true,
      label: "Slice Strip · Strip",
    };
    patchSnap({ devices: [device], devicesTotal: 1, devicesError: null, lights: [view], chains: [], status: null, statusError: null });
    const identify = vi.spyOn(api, "identify").mockResolvedValue({ ok: true } as never);
    // A read-back still in flight from the test before must land on this light, not another.
    vi.spyOn(api.lights, "list").mockResolvedValue({ lights: [view] });
    render(<DevicesView />);
    const card = await screen.findByTestId("device-card");
    const button = within(card).getByTestId("led-order-identify");
    await act(async () => {
      fireEvent.click(button);
    });
    expect(identify).toHaveBeenCalledWith("fake:slice-1", { zone: 0, led: 15, count: 1, stop: false });
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
    // The row is busy while the patch and the read-back are in flight, and both controls are
    // disabled until they land: a click on a disabled switch is dropped, so wait for the control
    // to be usable instead of for the patch call alone (#483).
    const direction = () => row.getByRole("switch", { name: "Runs the other way" }) as HTMLButtonElement;
    await waitFor(() => expect(direction().disabled).toBe(false));
    await act(async () => {
      fireEvent.click(direction());
    });
    await waitFor(() => expect(patch).toHaveBeenCalledWith("openrgb:strip-1#z0", { reversed: true, first_led: 0 }));
  });
});
