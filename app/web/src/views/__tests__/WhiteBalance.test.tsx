import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/endpoints";
import { demoRequest } from "@/lib/demo";
import { DEVICES, INITIAL_SETTINGS } from "@/lib/fixtures";
import { patchSnap } from "@/lib/store";
import { DevicesView } from "@/views/DevicesView";
import { SettingsView } from "@/views/SettingsView";

// #50 editor half: Settings' Warmth row commits through `api.patchOutput({ warmth })`, and a
// device's White balance section PUTs a dragged channel and DELETEs on Reset. The demo
// backend answers like the daemon.

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: () => undefined, replace: () => undefined }),
  usePathname: () => "/",
}));

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
  // jsdom has no PointerEvent and no pointer capture; a MouseEvent carries clientX just as well.
  if (!("PointerEvent" in window)) {
    class PE extends MouseEvent {
      pointerId: number;
      constructor(type: string, init: PointerEventInit = {}) {
        super(type, init);
        this.pointerId = init.pointerId ?? 1;
      }
    }
    (window as unknown as { PointerEvent: unknown }).PointerEvent = PE;
  }
  const captured = new Set<number>();
  HTMLElement.prototype.setPointerCapture = (id: number) => void captured.add(id);
  HTMLElement.prototype.releasePointerCapture = (id: number) => void captured.delete(id);
  HTMLElement.prototype.hasPointerCapture = (id: number) => captured.has(id);
});

afterEach(async () => {
  cleanup();
  vi.restoreAllMocks();
  // The demo backend keeps the white balances across tests: start each one neutral.
  await demoRequest("DELETE", `/api/v1/devices/${encodeURIComponent(STRIP.id)}/white-balance`, undefined).catch(() => undefined);
  patchSnap({ settings: null, devices: null, settingsError: null, devicesError: null });
});

const STRIP = DEVICES.find((d) => d.id === "openrgb:strip-1")!;

describe("SettingsView warmth (#50)", () => {
  it("sends warmth on commit, and shows Off at 6500", async () => {
    const spy = vi.spyOn(api, "patchOutput");
    patchSnap({ settings: { ...INITIAL_SETTINGS, warmth: 6500 } });
    render(<SettingsView />);

    const slider = screen.getByRole("slider", { name: "Warmth" });
    expect(slider.getAttribute("aria-valuenow")).toBe("6500");
    expect(screen.getByText("Off")).toBeTruthy();

    // One step warmer is impossible at the top, so step down instead.
    fireEvent.keyDown(slider, { key: "ArrowDown" });
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(1));
    expect(spy).toHaveBeenCalledWith({ power: null, brightness: null, warmth: 6400 });
  });
});

describe("DevicesView white balance (#50)", () => {
  async function page() {
    window.history.replaceState({}, "", "/devices/");
    patchSnap({ devices: DEVICES });
    render(<DevicesView />);
    await screen.findByLabelText(`White balance for ${STRIP.name}`);
  }

  it("PUTs the dragged channel on release", async () => {
    const spy = vi.spyOn(api, "putWhiteBalance");
    await page();

    const red = screen.getByRole("slider", { name: `Red gain for ${STRIP.name}` });
    expect(red.getAttribute("aria-valuenow")).toBe("100");
    // Home goes to the left end; releasing (here, the key step) commits red: 0.
    fireEvent.keyDown(red, { key: "Home" });
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(1));
    expect(spy).toHaveBeenCalledWith(STRIP.id, { red: 0, green: 1, blue: 1 });
  });

  it("DELETEs on Reset, and hides Reset while neutral", async () => {
    const del = vi.spyOn(api, "deleteWhiteBalance");
    await page();

    // Neutral at first: no Reset anywhere.
    expect(screen.queryByRole("button", { name: "Reset" })).toBeNull();

    // Pull Red down, then Reset appears and puts the device back to neutral.
    fireEvent.keyDown(screen.getByRole("slider", { name: `Red gain for ${STRIP.name}` }), { key: "ArrowDown" });
    await screen.findByRole("button", { name: "Reset" });
    expect(screen.getAllByRole("button", { name: "Reset" }).length).toBeGreaterThanOrEqual(1);
    fireEvent.click(screen.getAllByRole("button", { name: "Reset" })[0]!);
    await waitFor(() => expect(del).toHaveBeenCalledWith(STRIP.id));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Reset" })).toBeNull());
  });
});
