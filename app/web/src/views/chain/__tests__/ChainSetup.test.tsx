// The three steps of chain setup, drawn: the slider, the count and the confirm list.
//
// The routes are Task 8's, answered here the way its own tests answer them, so what is checked is
// the screen against the hook it is built on: the values the owner moves, the runs the daemon is
// told to light, the two writes that end the flow, and the words on the glass.

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const { api } = vi.hoisted(() => ({
  api: {
    chains: {
      list: vi.fn(),
      start: vi.fn(),
      light: vi.fn(),
      cancel: vi.fn(),
      empty: vi.fn(),
      commit: vi.fn(),
    },
    lights: { list: vi.fn() },
  },
}));

vi.mock("@/lib/endpoints", () => ({ api }));

import { ApiFailure } from "@/lib/errors";
import { patchSnap } from "@/lib/store";
import type { ChainPartKind } from "@/proto/ChainPartKind";
import type { ChainView } from "@/proto/ChainView";
import type { LightView } from "@/proto/LightView";
import type { SetupLight } from "@/proto/SetupLight";
import { ChainSetup } from "@/views/chain/ChainSetup";
import { evenSplit } from "@/views/chain/split";
import { PART_COLOURS } from "@/views/chain/partColours";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeAll(() => {
  // jsdom has no PointerEvent, no pointer capture and no ResizeObserver: the fader and the
  // segmented control read all three.
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
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof window.matchMedia;
});

const DEVICE = "openrgb:A1B2C3D4E5F6";
const SESSION = "session-1";
const WHY = "The app cannot see what is plugged into a header.";
const STEP_ONE = "Drag until the coloured light is the last one on the chain.";
const STEP_TWO = "How many things are plugged into this chain?";
const STEP_THREE = "Does each one have its own colour?";

/** The whole-zone light a header has before anything is set up: a ring, so a fan is its guess. */
function wholeZoneLight(): LightView {
  return {
    light: { id: `${DEVICE}#z1`, device_id: DEVICE, zone_index: 1, arrangement: "ring", reversed: false },
    led_count: 120,
    connected: true,
    label: "MSI MYSTIC LIGHT · JARGB 1",
  };
}

/** A header no one has described yet: first run sized it to 120, the device can drive 300. */
function notSetUp(over: Partial<ChainView> = {}): ChainView {
  return {
    device_id: DEVICE,
    device_name: "MSI MYSTIC LIGHT",
    zone_index: 1,
    zone_name: "JARGB 1",
    state: "not_set_up",
    leds: 120,
    min_leds: 0,
    max_leds: 300,
    parts: [wholeZoneLight()],
    ...over,
  };
}

/** One part of a set-up chain, by position (1-based), at `start` along the zone. */
function part(
  position: number,
  name: string,
  leds: number,
  start: number,
  kind: ChainPartKind,
): LightView {
  return {
    light: {
      id: `${DEVICE}#z1p${position}`,
      device_id: DEVICE,
      zone_index: 1,
      leds: { start, count: leds },
      name,
      arrangement: kind === "fan" ? "ring" : "strip",
      reversed: false,
      kind,
    },
    led_count: leds,
    connected: true,
    label: name,
  };
}

/** The chains the daemon lists after a first run: 7 fans and 1 strip, set up on 2 headers. */
function answered(): ChainView[] {
  const fans: [string, number][] = [
    ["Fan 1", 14],
    ["Fan 2", 14],
    ["Fan 3", 14],
    ["Fan 4", 14],
    ["Fan 5", 14],
    ["Fan 6", 13],
    ["Fan 7", 13],
  ];
  let start = 0;
  const fanParts = fans.map(([name, leds], i) => {
    const view = part(i + 1, name, leds, start, "fan");
    start += leds;
    return view;
  });
  const strip = part(1, "Front strip", 12, 0, "strip");
  return [
    notSetUp({ state: "set_up", leds: 96, parts: fanParts }),
    notSetUp({ zone_index: 2, zone_name: "JARGB 2", state: "set_up", leds: 12, parts: [strip] }),
  ];
}

/** Mounts the screen and lets the session's start answer land. Fake timers are on throughout. */
async function show(chain: ChainView = notSetUp()) {
  const onClose = vi.fn();
  const view = render(<ChainSetup chain={chain} onClose={onClose} />);
  await flush();
  return { ...view, onClose };
}

/** Lets pending answers land and moves the clock by `ms`. */
async function flush(ms = 0): Promise<void> {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

const slider = () => screen.getByRole("slider", { name: "Chain length" });
const valueNow = () => Number(slider().getAttribute("aria-valuenow"));

/** The keys that move the flow on: Next on the first two steps, Yes on the third. */
async function nextKey() {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
  });
}

/** Types a number into an entry and commits it with a blur, as the owner does. */
async function typeInto(input: HTMLElement, text: string) {
  fireEvent.change(input, { target: { value: text } });
  await act(async () => {
    fireEvent.blur(input);
  });
}

/** Walks to the count step and sets a count. */
async function toCount(count: number) {
  await nextKey();
  await typeInto(screen.getByRole("textbox", { name: "Parts on this chain" }), String(count));
}

/** Walks to the confirm step. */
async function toConfirm(count: number) {
  await toCount(count);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
  });
}

/** The runs of the newest light request. */
function lastRuns(): SetupLight["runs"] {
  const call = api.chains.light.mock.calls.at(-1) as [string, number, SetupLight] | undefined;
  return call?.[2].runs ?? [];
}

/** The colour of every part chip drawn, in order. */
function chipColours(): (string | null)[] {
  return [...document.querySelectorAll("[data-part-chip]")].map((el) => el.getAttribute("data-colour"));
}

describe("the chain setup steps", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    for (const fn of Object.values(api.chains)) fn.mockReset();
    api.lights.list.mockReset();
    api.chains.start.mockResolvedValue({ session: SESSION, restore_to: 120, raised_to: 300 });
    api.chains.light.mockResolvedValue({ ok: true });
    api.chains.cancel.mockResolvedValue({ ok: true });
    api.chains.empty.mockResolvedValue({ ok: true });
    api.chains.commit.mockResolvedValue({ lights: [], final_leds: 120 });
    api.chains.list.mockResolvedValue({ chains: [] });
    api.lights.list.mockResolvedValue({ lights: [] });
    patchSnap({ chains: null });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("step 1 shows the prompt and the counting number", async () => {
    const { getByTestId } = await show();

    expect(screen.getByText(STEP_ONE)).toBeTruthy();
    expect(screen.getByText(WHY)).toBeTruthy();
    // The number counts through its values, and it is the chain's own length to begin with.
    expect(getByTestId("chain-length").textContent).toBe("120");
    expect(slider().getAttribute("aria-valuemin")).toBe("1");
    expect(slider().getAttribute("aria-valuemax")).toBe("300");
    expect(valueNow()).toBe(120);
    expect(screen.getByRole("button", { name: "Nothing lit up" })).toBeTruthy();
  });

  it("the arrow keys move the length by one and Shift by ten", async () => {
    await show();

    fireEvent.keyDown(slider(), { key: "ArrowRight" });
    expect(valueNow()).toBe(121);
    fireEvent.keyDown(slider(), { key: "ArrowLeft" });
    expect(valueNow()).toBe(120);
    fireEvent.keyDown(slider(), { key: "ArrowRight", shiftKey: true });
    expect(valueNow()).toBe(130);
    fireEvent.keyDown(slider(), { key: "ArrowLeft", shiftKey: true });
    expect(valueNow()).toBe(120);
    await flush(60);
  });

  it("Nothing lit up calls the empty route and closes", async () => {
    const { onClose } = await show();

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Nothing lit up" }));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(api.chains.empty).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("the slider stops at the maximum and says why", async () => {
    await show();

    expect(screen.queryByText("This header takes at most 300 LEDs.")).toBeNull();
    fireEvent.keyDown(slider(), { key: "End" });
    expect(valueNow()).toBe(300);
    expect(screen.getByText("This header takes at most 300 LEDs.")).toBeTruthy();
    fireEvent.keyDown(slider(), { key: "ArrowRight" });
    expect(valueNow()).toBe(300);
    fireEvent.keyDown(slider(), { key: "ArrowLeft" });
    expect(screen.queryByText("This header takes at most 300 LEDs.")).toBeNull();
  });

  it("step 2 refuses 0 and a count above the length", async () => {
    await show();
    await nextKey();
    expect(screen.getByText(STEP_TWO)).toBeTruthy();

    const field = screen.getByRole("textbox", { name: "Parts on this chain" });
    expect(screen.getByText("Between 1 and 120 parts.")).toBeTruthy();

    // 0 is not a count: the entry gives the floor back, and the chain keeps one part.
    await typeInto(field, "0");
    expect(screen.getAllByRole("listitem")).toHaveLength(1);

    // 500 do not fit in 120 LEDs: the count stays and the line says why.
    await typeInto(field, "500");
    expect(screen.getByText("A chain of 120 LEDs cannot have more than 120 parts.")).toBeTruthy();
    expect(screen.getAllByRole("listitem")).toHaveLength(1);

    await typeInto(field, "7");
    expect(screen.queryByText(/cannot have more than/)).toBeNull();
    expect(screen.getAllByRole("listitem")).toHaveLength(7);
    expect(screen.getByText("Between 1 and 120 parts.")).toBeTruthy();
  });

  it("one row per part with the same colour as its light", async () => {
    await show();
    await toCount(7);
    await flush(60);

    const colours = chipColours();
    expect(colours).toEqual(PART_COLOURS.slice(0, 7));
    // The same colours the daemon was told to light, in chain order.
    expect(lastRuns().map((run) => run.color)).toEqual(colours);
    expect(lastRuns().map((run) => run.count)).toEqual(evenSplit(120, 7));
  });

  it("changing a row's LED count moves the boundary and calls light", async () => {
    await show();
    await toConfirm(2);
    expect(screen.getByText(STEP_THREE)).toBeTruthy();
    await flush(60);

    expect(lastRuns().map((run) => run.count)).toEqual([60, 60]);
    await typeInto(screen.getByRole("textbox", { name: "LEDs in part 1" }), "40");
    await flush(60);

    // The second part takes the 20 LEDs the first gave up, and the lights follow.
    expect(lastRuns()).toEqual([
      { start: 0, count: 40, color: PART_COLOURS[0] },
      { start: 40, count: 80, color: PART_COLOURS[1] },
    ]);
    // The figure beside it counts up to the new number rather than jumping.
    await flush(600);
    expect((screen.getByRole("textbox", { name: "LEDs in part 2" }) as HTMLInputElement).value).toBe("80");
  });

  it("Yes calls commit once", async () => {
    await show();
    await toConfirm(1);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Yes" }));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(api.chains.commit).toHaveBeenCalledTimes(1);
    expect(api.chains.commit.mock.calls[0]?.[2]).toEqual({
      session: SESSION,
      parts: [{ name: "Fan 1", kind: "fan", leds: 120 }],
    });
  });

  it("Back keeps the entered values", async () => {
    await show();
    // Four steps of ten down from 120.
    for (let i = 0; i < 4; i++) fireEvent.keyDown(slider(), { key: "ArrowLeft", shiftKey: true });
    expect(valueNow()).toBe(80);

    await toCount(3);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    });
    await act(async () => {
      fireEvent.change(screen.getByRole("textbox", { name: "Name of part 1" }), {
        target: { value: "Top intake" },
      });
    });

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Back" }));
    });
    expect(screen.getByText(STEP_TWO)).toBeTruthy();
    expect(screen.getAllByRole("listitem")).toHaveLength(3);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Back" }));
    });
    expect(screen.getByText(STEP_ONE)).toBeTruthy();
    expect(valueNow()).toBe(80);
    expect(screen.getByTestId("chain-length").textContent).toBe("80");

    await nextKey();
    await nextKey();
    expect(screen.getByText(STEP_THREE)).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Name of part 1" }) as HTMLInputElement).value).toBe(
      "Top intake",
    );
    expect(api.chains.cancel).not.toHaveBeenCalled();
  });

  it("a commit error shows the device's text and Retry resends", async () => {
    api.chains.commit.mockRejectedValueOnce(
      new ApiFailure(500, "internal", "the header could not be resized: no reply from OpenRGB"),
    );
    await show();
    await toConfirm(1);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Yes" }));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByText("the header could not be resized: no reply from OpenRGB")).toBeTruthy();
    expect(screen.getByText(STEP_THREE)).toBeTruthy();
    expect(api.chains.commit).toHaveBeenCalledTimes(1);

    api.chains.commit.mockResolvedValueOnce({ lights: [], final_leds: 120 });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Retry" }));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(api.chains.commit).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("the header could not be resized: no reply from OpenRGB")).toBeNull();
  });

  it("the last screen says what was found and closes", async () => {
    api.chains.list.mockResolvedValue({ chains: answered() });
    const { onClose } = await show();
    await toConfirm(1);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Yes" }));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByText("7 fans and 1 strip on 2 chains.")).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Close" }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("a failed start shows the device's text and Retry opens a new session", async () => {
    api.chains.start.mockRejectedValueOnce(
      new ApiFailure(503, "unavailable", "the header is busy: another setup holds it"),
    );
    const { getByRole, getByText } = await show();

    expect(getByText("the header is busy: another setup holds it")).toBeTruthy();
    api.chains.start.mockResolvedValue({ session: SESSION, restore_to: 120, raised_to: 300 });
    await act(async () => {
      fireEvent.click(getByRole("button", { name: "Retry" }));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(api.chains.start).toHaveBeenCalledTimes(2);
    expect(getByText(STEP_ONE)).toBeTruthy();
  });

  it("Later and Back on the first step both end the session and close", async () => {
    const first = await show();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Later" }));
    });
    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(first.onClose).toHaveBeenCalledTimes(1);
    cleanup();

    api.chains.cancel.mockClear();
    const second = await show();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Back" }));
    });
    expect(api.chains.cancel).toHaveBeenCalledWith(DEVICE, 1, { session: SESSION });
    expect(second.onClose).toHaveBeenCalledTimes(1);
  });

  it("no control is labelled Save or Revert", async () => {
    await show();
    const check = (where: string) => {
      const labels = [
        ...[...document.querySelectorAll("button")].map((b) => (b.textContent ?? "").trim()),
        ...[...document.querySelectorAll("[aria-label]")].map((e) => e.getAttribute("aria-label") ?? ""),
      ];
      expect(labels, where).not.toContain("Save");
      expect(labels, where).not.toContain("Revert");
    };
    check("step 1");
    await toCount(2);
    check("step 2");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    });
    check("step 3");
  });

  it("the word layout appears nowhere", async () => {
    await show();
    const check = (where: string) => {
      expect(document.body.textContent ?? "", where).not.toMatch(/layout/i);
      expect(document.body.innerHTML, where).not.toMatch(/layout/i);
    };
    check("step 1");
    await toCount(2);
    check("step 2");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    });
    check("step 3");
  });

  it("no raw button, select or input element is rendered outside the component layer", async () => {
    // The component layer's own class on the element is the proof it drew it: Button, IconButton,
    // Segmented, NumberField and NameField each carry one.
    const layer = ["btn", "ibtn", "seg-item", "num", "text-in"];
    const check = (where: string) => {
      const all = [...document.querySelectorAll("button, select, input")];
      expect(all.length, `${where}: some controls were drawn`).toBeGreaterThan(0);
      for (const el of all) {
        const own = [...el.classList];
        expect(own.some((c) => layer.includes(c)), `${where}: ${el.outerHTML}`).toBe(true);
      }
      expect(document.querySelectorAll("select"), where).toHaveLength(0);
    };
    await show();
    check("step 1");
    await toCount(2);
    check("step 2");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Next" }));
    });
    check("step 3");
  });
});
