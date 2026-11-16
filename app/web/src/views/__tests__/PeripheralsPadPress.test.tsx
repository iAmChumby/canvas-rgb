// The controller drawing shows what the pad is doing (#375).
//
// The pad is read in the editor's own window, and the drawing is written from that read: a held
// button is filled, the triggers fill their slots, the sticks carry a dot, and the row of the
// button that is held says so. The frame loop is stepped by hand (src/test/padFrames.ts), so no
// test here waits for the clock.

import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { PeripheralsView } from "@/views/PeripheralsView";
import { StripSlot, StripSlotsProvider } from "@/shell/PageHeader";
import { PERIPHERALS } from "@/lib/peripheralFixtures";
import { PadFrames } from "@/test/padFrames";
import type { PeripheralDetail } from "@/proto/PeripheralDetail";
import type { PropValue } from "@/proto/PropValue";

const { api } = vi.hoisted(() => ({
  api: {
    peripherals: vi.fn(),
    peripheral: vi.fn(),
    patchPeripheral: vi.fn(),
    putPeripheralAccess: vi.fn(),
    peripheralRead: vi.fn(),
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** The pad the demo holds: the 8BitDo Ultimate 2 and its button map. */
const fixture = PERIPHERALS.find((p) => p.info.kind === "game_controller")!;

/** The pad a test holds: its buttons, its triggers and its sticks. */
const pad = {
  connected: true,
  buttons: new Set<number>(),
  lt: 0,
  rt: 0,
  lx: 0,
  ly: 0,
  rx: 0,
  ry: 0,
};

/** The browser's own list of pads, in the browser's own shape. */
function browserPad(extra: { id?: string; mapping?: string } = {}) {
  return {
    connected: pad.connected,
    ...extra,
    buttons: Array.from({ length: 21 }, (_, i) => {
      const value = i === 6 ? pad.lt : i === 7 ? pad.rt : pad.buttons.has(i) ? 1 : 0;
      return { pressed: value > 0, value };
    }),
    axes: [pad.lx, pad.ly, pad.rx, pad.ry],
  };
}

/** Gives the page a list of pads, made fresh on every read. */
function setGamepads(make: () => unknown[]) {
  Object.defineProperty(navigator, "getGamepads", { configurable: true, value: make });
}

/** The browser's own list of pads, with the one the test holds. */
function stubGamepads() {
  setGamepads(() => [browserPad()]);
}

/** The device the page shows, with the values a test gives it. */
function detailWith(values?: Record<string, PropValue>): PeripheralDetail {
  return {
    info: fixture.info,
    schema: fixture.schema,
    values: values ?? fixture.values,
    access: { reads: true, writes: true },
    problem: null,
  };
}

let frames: PadFrames;

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

beforeEach(() => {
  frames = new PadFrames();
  frames.install();
  stubGamepads();
  pad.connected = true;
  pad.buttons.clear();
  pad.lt = 0;
  pad.rt = 0;
  pad.lx = 0;
  pad.ly = 0;
  pad.rx = 0;
  pad.ry = 0;
  for (const fn of Object.values(api)) fn.mockReset();
  api.peripheralRead.mockResolvedValue({ stage: "idle", done: 0, total: 0 });
});

afterEach(() => {
  cleanup();
});

/** Opens the page on the pad, with the first frame already run. */
async function open(detail: PeripheralDetail = detailWith()) {
  api.peripherals.mockResolvedValue({ peripherals: [detail.info] });
  api.peripheral.mockResolvedValue(detail);
  render(
    <StripSlotsProvider>
      <header>
        <StripSlot kind="title" />
        <StripSlot kind="actions" />
      </header>
      <PeripheralsView />
    </StripSlotsProvider>,
  );
  await screen.findByRole("button", { name: "Map A" });
  // The panel's effects may still be waiting to run: let React finish them, so the loop is
  // scheduled and the painter is registered before the first frame.
  await act(async () => {});
  // The frames are run until the drawing has been written, or a few frames have gone by.
  for (let i = 0; i < 5; i++) {
    frames.step();
    if (document.querySelector('[data-pad-key][data-pressed]')) return;
  }
}

/** The key on the plan. */
function target(key: string): Element {
  const el = document.querySelector(`[data-pad-key="${key}"]`);
  if (!el) throw new Error(`no key ${key} on the plan`);
  return el;
}

/** The row of the button in the list. */
function row(key: string): Element {
  const el = document.querySelector(`.pad-map [data-key="${key}"]`);
  if (!el) throw new Error(`no row ${key} in the list`);
  return el;
}

/** The keys that are lit, in the plan's own words. */
function litKeys(): string[] {
  return [...document.querySelectorAll('[data-pad-key][data-pressed="true"]')].map(
    (el) => el.getAttribute("data-pad-key") ?? "",
  );
}

describe("the controller drawing", () => {
  it("lights the button that is held, and its row", async () => {
    await open();
    expect(target("button-a").getAttribute("data-pressed")).toBe("false");

    pad.buttons.add(0);
    frames.step();

    expect(target("button-a").getAttribute("data-pressed")).toBe("true");
    expect(row("button-a").getAttribute("data-pressed")).toBe("true");
    expect(litKeys()).toEqual(["button-a"]);
  });

  it("lights every button the pad reports, one at a time", async () => {
    await open();
    const keys: [number, string][] = [
      [0, "button-a"],
      [1, "button-b"],
      [2, "button-x"],
      [3, "button-y"],
      [4, "button-l1"],
      [5, "button-r1"],
      [8, "button-select"],
      [9, "button-start"],
      [10, "button-l3"],
      [11, "button-r3"],
      [16, "button-home"],
    ];
    for (const [index, key] of keys) {
      pad.buttons.clear();
      pad.buttons.add(index);
      frames.step();
      expect(target(key).getAttribute("data-pressed"), key).toBe("true");
      expect(row(key).getAttribute("data-pressed"), key).toBe("true");
      expect(litKeys(), key).toEqual([key]);
    }
  });

  it("lights the four ways of the D-pad", async () => {
    await open();
    const ways: [number, string][] = [
      [12, "button-dpad-up"],
      [13, "button-dpad-down"],
      [14, "button-dpad-left"],
      [15, "button-dpad-right"],
    ];
    for (const [index, key] of ways) {
      pad.buttons.clear();
      pad.buttons.add(index);
      frames.step();
      expect(target(key).getAttribute("data-pressed"), key).toBe("true");
      expect(litKeys(), key).toEqual([key]);
    }
  });

  it("shows the travel of the two triggers, at rest, half and full", async () => {
    await open();
    const fill = document.querySelector('[data-pad-travel="lt"]')!;
    const number = document.querySelector('[data-pad-value="lt"]')!;
    const slot = Number(document.querySelector('[data-pad-slot="lt"]')!.getAttribute("width"));

    expect(Number(fill.getAttribute("width"))).toBe(0);
    expect(number.textContent).toBe("000");

    pad.lt = 0.5;
    frames.step();
    // The travel is the 8-bit value the pad reports: 128 of 255.
    expect(Number(fill.getAttribute("width"))).toBeCloseTo((slot * 128) / 255, 5);
    expect(number.textContent).toBe("128");
    // The trigger is a button that is down: the key it is mapped to lights.
    expect(target("button-l2").getAttribute("data-pressed")).toBe("true");

    pad.lt = 1;
    frames.step();
    expect(Number(fill.getAttribute("width"))).toBeCloseTo(slot, 5);
    expect(number.textContent).toBe("255");

    // The other trigger stands still.
    expect(document.querySelector('[data-pad-value="rt"]')!.textContent).toBe("000");
    expect(Number(document.querySelector('[data-pad-travel="rt"]')!.getAttribute("width"))).toBe(0);
  });

  it("shows where the two sticks stand, in the middle and at an edge", async () => {
    await open();
    const dot = document.querySelector('[data-pad-dot="l"]')!;
    const cap = dot.parentElement!.querySelector(".pad-btn")!;
    const cx = Number(cap.getAttribute("cx"));
    const cy = Number(cap.getAttribute("cy"));

    // In the middle.
    expect(Number(dot.getAttribute("cx"))).toBeCloseTo(cx, 5);
    expect(Number(dot.getAttribute("cy"))).toBeCloseTo(cy, 5);
    expect(document.querySelector('[data-pad-stick="lx"]')!.textContent).toBe("+0");

    // At the edge.
    pad.lx = -1;
    pad.ly = -1;
    frames.step();
    expect(Number(dot.getAttribute("cx"))).toBeCloseTo(cx - 14, 5);
    expect(Number(dot.getAttribute("cy"))).toBeCloseTo(cy - 14, 5);
    expect(document.querySelector('[data-pad-stick="lx"]')!.textContent).toBe("-100");
    expect(document.querySelector('[data-pad-stick="ly"]')!.textContent).toBe("-100");

    // The other stick stands still.
    const right = document.querySelector('[data-pad-dot="r"]')!;
    const rightCap = right.parentElement!.querySelector(".pad-btn")!;
    expect(Number(right.getAttribute("cx"))).toBeCloseTo(Number(rightCap.getAttribute("cx")), 5);
  });

  it("says what the pad is doing, in the four states", async () => {
    await open();
    const words = () => document.querySelector(".pad-state-text")!.textContent;

    expect(words()).toBe("Pad live");

    // The editor goes to the background.
    act(() => {
      window.dispatchEvent(new Event("blur"));
    });
    expect(words()).toBe("Input paused: the editor is in the background.");
    // The keys are cleared when the input is paused.
    expect(litKeys()).toEqual([]);

    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(words()).toBe("Pad live");

    // The pad is gone.
    pad.connected = false;
    frames.step();
    expect(words()).toBe("No pad connected.");

    // No pad has been seen yet.
    cleanup();
    pad.connected = false;
    await open();
    expect(words()).toBe("No pad seen yet. Press any button on the pad.");
  });

  it("lights the button a paddle is mapped to, and never the paddle itself", async () => {
    const values: Record<string, PropValue> = {
      ...fixture.values,
      "profile-1-paddle-1": { type: "choice", value: "a" },
    };
    await open(detailWith(values));

    // The browser reports the paddle after the standard seventeen buttons.
    pad.buttons.add(17);
    frames.step();

    expect(target("button-a").getAttribute("data-pressed")).toBe("true");
    // The paddle is a ghost: XInput cannot see it, so it is never lit.
    expect(target("profile-1-paddle-1").getAttribute("data-pressed")).toBe("false");
    expect(row("profile-1-paddle-1").getAttribute("data-pressed")).toBe("false");
  });

  it("does not draw a focused key as pressed", async () => {
    await open();
    const key = target("button-a");
    key.dispatchEvent(new FocusEvent("focus"));

    frames.step();
    expect(key.getAttribute("data-pressed")).toBe("false");

    // A key that is held elsewhere does not make the focused key look pressed.
    pad.buttons.add(1);
    frames.step();
    expect(key.getAttribute("data-pressed")).toBe("false");
    expect(target("button-b").getAttribute("data-pressed")).toBe("true");
  });

  it("moves a held light to the key its button is mapped to, without waiting for a frame", async () => {
    await open();
    // X is held down.
    pad.buttons.add(2);
    frames.step();
    expect(target("button-x").getAttribute("data-pressed")).toBe("true");
    expect(target("button-b").getAttribute("data-pressed")).toBe("false");

    // B is mapped to X while X is still held. No frame is stepped after this: the light must
    // move on the map's own change, or a held key goes stale.
    fireEvent.click(screen.getByRole("combobox", { name: "B mapping" }));
    const list = await screen.findByRole("listbox", { name: "B mapping" });
    fireEvent.click(within(list).getByRole("option", { name: /^X$/ }));

    expect(target("button-b").getAttribute("data-pressed")).toBe("true");
    expect(row("button-b").getAttribute("data-pressed")).toBe("true");
  });

  it("says which pad it is showing when several are connected and none carries the device's ids", async () => {
    // Two pads, neither named with this device's ids: the first one in the standard layout is
    // read, and the strip says which one.
    setGamepads(() => [
      browserPad({ id: "Controller A", mapping: "" }),
      browserPad({ id: "Controller B", mapping: "standard" }),
    ]);
    await open();

    expect(document.querySelector(".pad-state-note")!.textContent).toBe("Showing pad 2 of 2.");
    expect(document.querySelector(".pad-state-text")!.textContent).toBe("Pad live");

    // Held buttons are read from the pad the page settled on.
    pad.buttons.add(0);
    frames.step();
    expect(target("button-a").getAttribute("data-pressed")).toBe("true");
  });

  it("lights no key, and says why, when the pad's buttons are not in the standard layout", async () => {
    setGamepads(() => [browserPad({ id: "Controller", mapping: "" })]);
    await open();
    expect(document.querySelector(".pad-state-note")!.textContent).toBe(
      "This pad reports its buttons in a layout the app cannot read.",
    );

    pad.buttons.add(0);
    pad.lt = 1;
    frames.step();

    // The indices would be the wrong ones, so nothing is lit, on the plan or in the list.
    expect(litKeys()).toEqual([]);
    expect(row("button-a").getAttribute("data-pressed")).toBe("false");
  });
});
